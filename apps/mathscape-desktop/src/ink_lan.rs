use crate::{CreateInkSnapshotRequest, DesktopService, DesktopState, ink_tls};
use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, OriginalUri, Path, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use mathscape_domain::{
    INK_LAN_DOCUMENT_BYTES, INK_LAN_OPERATION_BYTES, INK_LAN_PROTOCOL_VERSION, InkLabStroke,
    InkLanOperation, validate_ink_preview,
};
use mathscape_store_sqlite::{
    InkLanSettings, InkMirrorDelta, InkPairingOffer, SqliteArtifactStore,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, State as TauriState};
use tauri_plugin_dialog::DialogExt;
use tokio::sync::{Semaphore, broadcast};

include!(concat!(env!("OUT_DIR"), "/inkpad_assets.rs"));
pub(crate) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .try_into()
        .unwrap_or(i64::MAX)
}
fn secret() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn hash(value: &str) -> Result<String, String> {
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid_credential".into());
    }
    Ok(format!("{:x}", Sha256::digest(value.as_bytes())))
}
fn store(service: &DesktopService) -> Result<SqliteArtifactStore, String> {
    SqliteArtifactStore::open(&service.workspace_root().join("mathscape.sqlite3"))
        .map_err(|e| e.to_string())
}

#[derive(Clone, Debug, Serialize)]
pub struct InkLanStatus {
    pub enabled: bool,
    pub running: bool,
    pub port: u16,
    pub urls: Vec<String>,
    pub certificate_file_sha256: Option<String>,
    pub assets_ready: bool,
    pub error: Option<String>,
}
#[derive(Serialize)]
pub struct InkPairingLink {
    pub urls: Vec<String>,
    pub expires_at: i64,
    pub document_id: String,
}
#[derive(Clone, Serialize)]
pub struct InkClientReport {
    pub pending_count: u32,
    pub locally_durable: bool,
    pub secure_context: bool,
    pub page_id: String,
}
#[derive(Serialize)]
pub struct InkLanMirror {
    pub delta: InkMirrorDelta,
    pub connected: bool,
    pub preview: Option<InkLabStroke>,
    pub client: Option<InkClientReport>,
}
struct Preview {
    stroke: InkLabStroke,
    epoch: u64,
    updated: Instant,
}
struct Report {
    value: InkClientReport,
    epoch: u64,
}
struct Slot {
    client_id: String,
    instance: String,
    document_id: String,
    epoch: u64,
}
#[derive(Clone)]
enum Control {
    Stop(bool),
    Revoke(String),
    Replace(String),
}
struct Live {
    slots: Mutex<HashMap<String, Slot>>,
    previews: Mutex<HashMap<String, Preview>>,
    reports: Mutex<HashMap<String, Report>>,
    control: broadcast::Sender<Control>,
    connections: Arc<Semaphore>,
}
impl Live {
    fn new() -> Self {
        let (control, _) = broadcast::channel(32);
        Self {
            slots: Mutex::new(HashMap::new()),
            previews: Mutex::new(HashMap::new()),
            reports: Mutex::new(HashMap::new()),
            control,
            connections: Arc::new(Semaphore::new(8)),
        }
    }
}
struct Running {
    handle: axum_server::Handle<std::net::SocketAddr>,
    task: tokio::task::JoinHandle<()>,
    status: InkLanStatus,
}
pub struct InkLanRuntime {
    running: tokio::sync::Mutex<Option<Running>>,
    live: Arc<Live>,
}
impl std::fmt::Debug for InkLanRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InkLanRuntime").finish_non_exhaustive()
    }
}
impl Default for InkLanRuntime {
    fn default() -> Self {
        Self {
            running: tokio::sync::Mutex::new(None),
            live: Arc::new(Live::new()),
        }
    }
}
#[derive(Clone)]
struct HttpState {
    service: DesktopService,
    hosts: Arc<HashSet<String>>,
    live: Arc<Live>,
}
fn origin(
    headers: &HeaderMap,
    state: &HttpState,
    required: bool,
    uri: &axum::http::Uri,
) -> Result<(), StatusCode> {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .or_else(|| uri.authority().map(|authority| authority.as_str()))
        .ok_or(StatusCode::FORBIDDEN)?;
    if !state.hosts.contains(host) {
        return Err(StatusCode::FORBIDDEN);
    }
    let actual = headers.get(header::ORIGIN).and_then(|h| h.to_str().ok());
    if actual.is_some_and(|value| value != format!("https://{host}"))
        || (required && actual.is_none())
    {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(())
}
fn error(code: &str, message: &str) -> Value {
    json!({"type":"operation_conflict","code":code,"message":message})
}
fn error_code(message: &str) -> &'static str {
    if message.contains("writer_revoked")
        || message.contains("writer epoch")
        || message.contains("writer_epoch_conflict")
    {
        "writer_revoked"
    } else if message.contains("credential_expired") {
        "credential_expired"
    } else if message.contains("pairing_already_used") {
        "pairing_already_used"
    } else if message.contains("pairing_expired") {
        "pairing_expired"
    } else if message.contains("authentication_failed") {
        "authentication_failed"
    } else if message.contains("head conflict")
        || message.contains("sequence conflict")
        || message.contains("predecessor")
    {
        "sequence_conflict"
    } else if message.contains("different content") {
        "idempotency_conflict"
    } else if message.contains("document_byte_limit") {
        "operation_limit"
    } else {
        "request_rejected"
    }
}
fn public_error(message: &str) -> Value {
    let code = error_code(message);
    let text = match code {
        "writer_revoked" | "credential_expired" | "authentication_failed" => {
            "会话或写入权已失效，原队列会保留，可导出恢复草稿。"
        }
        "sequence_conflict" => "操作顺序与电脑记录不一致，原队列会保留，请导出恢复草稿后检查。",
        "idempotency_conflict" => "同一操作请求包含不同内容，原队列保持不变。",
        "operation_limit" => "手稿达到当前传输上限，原内容已保留，请导出恢复草稿。",
        _ => "电脑未接受此请求，内容未写入，请保留本机恢复队列后重试。",
    };
    error(code, text)
}
fn secure_response(mut response: Response) -> Response {
    for (name, value) in [
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        ("referrer-policy", "no-referrer"),
        ("cache-control", "no-store"),
        (
            "content-security-policy",
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' blob: data:; connect-src 'self' wss:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ),
    ] {
        response.headers_mut().insert(
            axum::http::HeaderName::from_static(name),
            axum::http::HeaderValue::from_static(value),
        );
    }
    response
}
async fn index(
    State(state): State<HttpState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Response {
    if let Err(code) = origin(&headers, &state, false, &uri) {
        return code.into_response();
    }
    asset_response("index.html")
}
fn asset_response(name: &str) -> Response {
    let Some((_, bytes)) = INKPAD_ASSETS.iter().find(|(path, _)| *path == name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mime = if name.ends_with(".js") {
        "text/javascript; charset=utf-8"
    } else if name.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if name.ends_with(".html") {
        "text/html; charset=utf-8"
    } else {
        "application/octet-stream"
    };
    secure_response(([(header::CONTENT_TYPE, mime)], *bytes).into_response())
}
async fn asset(
    State(state): State<HttpState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Path(path): Path<String>,
) -> Response {
    if let Err(code) = origin(&headers, &state, false, &uri) {
        return code.into_response();
    }
    asset_response(&format!("assets/{path}"))
}
async fn sw(
    State(state): State<HttpState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Response {
    if let Err(code) = origin(&headers, &state, false, &uri) {
        return code.into_response();
    }
    asset_response("sw.js")
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PairRequest {
    pairing_token: String,
    resume_secret: String,
    device: Value,
}
async fn pair(
    State(state): State<HttpState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Json(request): Json<PairRequest>,
) -> Response {
    if let Err(code) = origin(&headers, &state, true, &uri) {
        return code.into_response();
    }
    let (Ok(token), Ok(credential)) = (hash(&request.pairing_token), hash(&request.resume_secret))
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let service = state.service;
    let result = tokio::task::spawn_blocking(move || {
        store(&service)?
            .claim_ink_pairing(&token, &credential, &request.device, now())
            .map_err(|e| e.to_string())
    })
    .await;
    match result {
        Ok(Ok(lease))=>secure_response(Json(json!({"protocol_version":INK_LAN_PROTOCOL_VERSION,"lease":lease})).into_response()),
        Ok(Err(message))=>secure_response((StatusCode::CONFLICT,Json(json!({"code":error_code(&message),"message":"配对无效、已使用或书写状态已变化，请在电脑重新配对。"}))).into_response()),
        Err(_)=>StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}
async fn ws(
    State(state): State<HttpState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    upgrade: WebSocketUpgrade,
) -> Response {
    if let Err(code) = origin(&headers, &state, true, &uri) {
        return code.into_response();
    }
    let Ok(permit) = state.live.connections.clone().try_acquire_owned() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    upgrade
        .max_message_size(INK_LAN_OPERATION_BYTES)
        .max_frame_size(INK_LAN_OPERATION_BYTES)
        .on_upgrade(move |socket| async move {
            let _permit = permit;
            socket_loop(socket, state).await;
        })
        .into_response()
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ClientMessage {
    Hello {
        protocol_version: String,
        credential: String,
        client_id: String,
    },
    OperationSubmit {
        request: InkLanOperation,
    },
    PreviewStroke {
        stroke: InkLabStroke,
    },
    ClientStatus {
        pending_count: u32,
        locally_durable: bool,
        secure_context: bool,
        page_id: String,
    },
    SnapshotRequest {
        request_id: String,
        expected_head: u64,
    },
    Ping,
}
async fn send(socket: &mut WebSocket, value: Value) -> bool {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .is_ok()
}
async fn socket_loop(mut socket: WebSocket, state: HttpState) {
    let first = tokio::time::timeout(Duration::from_secs(10), socket.recv()).await;
    let Ok(Some(Ok(Message::Text(text)))) = first else {
        return;
    };
    let Ok(ClientMessage::Hello {
        protocol_version,
        credential,
        client_id,
    }) = serde_json::from_str::<ClientMessage>(&text)
    else {
        send(
            &mut socket,
            error("authentication_failed", "连接认证失败。"),
        )
        .await;
        return;
    };
    if protocol_version != INK_LAN_PROTOCOL_VERSION || client_id.is_empty() || client_id.len() > 200
    {
        send(
            &mut socket,
            error("protocol_mismatch", "InkPad 版本不兼容，请重新加载页面。"),
        )
        .await;
        return;
    }
    let Ok(credential_hash) = hash(&credential) else {
        send(
            &mut socket,
            error("authentication_failed", "连接认证失败。"),
        )
        .await;
        return;
    };
    let service = state.service.clone();
    let auth_hash = credential_hash.clone();
    let context = tokio::task::spawn_blocking(move || {
        store(&service)?
            .ink_remote_context(&auth_hash, now())
            .map_err(|e| e.to_string())
    })
    .await;
    let (lease, document) = match context {
        Ok(Ok(value)) => value,
        Ok(Err(message)) => {
            send(
                &mut socket,
                error(
                    error_code(&message),
                    "会话已失效或书写权已转移，待同步内容仍保留在本机。",
                ),
            )
            .await;
            return;
        }
        Err(_) => return,
    };
    let Ok(instance) = secret() else {
        return;
    };
    let mut control = state.live.control.subscribe();
    let admission = {
        let mut slots = state.live.slots.lock().unwrap();
        if let Some(old) = slots.get(&credential_hash) {
            if old.client_id != client_id {
                Err(())
            } else {
                let old_instance = old.instance.clone();
                slots.insert(
                    credential_hash.clone(),
                    Slot {
                        client_id,
                        instance: instance.clone(),
                        document_id: lease.document_id.clone(),
                        epoch: lease.writer_epoch,
                    },
                );
                let _ = state.live.control.send(Control::Replace(old_instance));
                Ok(())
            }
        } else {
            slots.insert(
                credential_hash.clone(),
                Slot {
                    client_id,
                    instance: instance.clone(),
                    document_id: lease.document_id.clone(),
                    epoch: lease.writer_epoch,
                },
            );
            Ok(())
        }
    };
    if admission.is_err() {
        send(
            &mut socket,
            error(
                "device_busy",
                "此手稿已在另一个画板窗口打开，请关闭那个窗口后重试。",
            ),
        )
        .await;
        return;
    }
    let full = json!({"type":"session_context","protocol_version":INK_LAN_PROTOCOL_VERSION,"lease":lease,"document":document.document,"head_seq":document.head_seq,"limits":{"operation_bytes":INK_LAN_OPERATION_BYTES,"document_bytes":INK_LAN_DOCUMENT_BYTES,"queue_bytes":INK_LAN_DOCUMENT_BYTES}});
    if full.to_string().len() > INK_LAN_DOCUMENT_BYTES {
        send(
            &mut socket,
            error(
                "operation_limit",
                "此手稿超过当前传输上限，请在电脑查看或导出原稿。",
            ),
        )
        .await;
        release(&state, &credential_hash, &instance);
        return;
    }
    if !send(&mut socket, full).await {
        release(&state, &credential_hash, &instance);
        return;
    }
    let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
    let mut last_seen = Instant::now();
    loop {
        tokio::select! {
            event=control.recv()=>{
                let code=match event {Ok(Control::Stop(revoke))=>Some(if revoke{"writer_revoked"}else{"server_restart"}),Ok(Control::Revoke(doc)) if doc==lease.document_id=>Some("writer_revoked"),Ok(Control::Replace(old)) if old==instance=>Some("connection_replaced"),Err(broadcast::error::RecvError::Lagged(_))=>Some("reconnect_required"),_=>None};
                if let Some(code)=code {send(&mut socket,json!({"type":"session_closed","code":code})).await;break;}
            }
            _=heartbeat.tick()=>{if last_seen.elapsed()>Duration::from_secs(50){break;}if socket.send(Message::Ping(Vec::new().into())).await.is_err(){break;}}
            incoming=socket.recv()=>{
                let Some(Ok(message))=incoming else{break;};last_seen=Instant::now();
                match message {Message::Pong(_)=>continue,Message::Ping(data)=>{if socket.send(Message::Pong(data)).await.is_err(){break;}continue;},Message::Close(_)=>break,Message::Text(text)=>{
                    if !state.live.slots.lock().unwrap().get(&credential_hash).is_some_and(|slot|slot.instance==instance){break;}
                    let parsed=serde_json::from_str::<ClientMessage>(&text);
                    let Ok(message)=parsed else {if !send(&mut socket,error("invalid_message","消息格式无效，内容未写入。")).await{break;}continue;};
                    let reply=match message {
                        ClientMessage::OperationSubmit{request}=>{
                            let request_id=request.request_id.clone();let command=match &request.operation {mathscape_domain::InkLabOperation::Stroke{stroke,..}=>Some(stroke.id.clone()),_=>None};let service=state.service.clone();let credential=credential_hash.clone();
                            match tokio::task::spawn_blocking(move||store(&service)?.append_remote_ink_operation(&credential,&request,now()).map_err(|e|e.to_string())).await {
                                Ok(Ok(receipt))=>{let mut previews=state.live.previews.lock().unwrap();if previews.get(&lease.document_id).is_some_and(|p|Some(&p.stroke.id)==command.as_ref()){previews.remove(&lease.document_id);}Some(json!({"type":"operation_ack","receipt":receipt}))},
                                Ok(Err(message))=>{let mut reply=public_error(&message);reply["request_id"]=request_id.into();Some(reply)},
                                Err(_)=>Some(error("service_unavailable","电脑保存暂时不可用，请保留本机队列后重试。")),
                            }
                        }
                        ClientMessage::PreviewStroke{stroke}=>{
                            if validate_ink_preview(&stroke).is_ok(){state.live.previews.lock().unwrap().insert(lease.document_id.clone(),Preview{stroke,epoch:lease.writer_epoch,updated:Instant::now()});}None
                        }
                        ClientMessage::ClientStatus{pending_count,locally_durable,secure_context,page_id}=>{
                            if pending_count<=20_000 && page_id.len()<=200{state.live.reports.lock().unwrap().insert(lease.document_id.clone(),Report{value:InkClientReport{pending_count,locally_durable,secure_context,page_id},epoch:lease.writer_epoch});}None
                        }
                        ClientMessage::SnapshotRequest{request_id,expected_head}=>{
                            let service=state.service.clone();let credential=credential_hash.clone();let id=request_id.clone();
                            match tokio::task::spawn_blocking(move||{
                                let reservation=store(&service)?.reserve_remote_ink_snapshot(&credential,&id,expected_head,now()).map_err(|e|e.to_string())?;
                                service.create_ink_snapshot(CreateInkSnapshotRequest{request_id:id,document_id:reservation.document_id,expected_head})
                            }).await {Ok(Ok(snapshot))=>Some(json!({"type":"snapshot_receipt","request_id":request_id,"snapshot_id":snapshot.snapshot_id,"through_seq":snapshot.through_seq})),Ok(Err(message))=>{let mut reply=public_error(&message);reply["request_id"]=request_id.into();Some(reply)},Err(_)=>Some(error("service_unavailable","快照服务暂时不可用。"))}
                        }
                        ClientMessage::Ping=>Some(json!({"type":"pong"})),
                        ClientMessage::Hello{..}=>Some(error("invalid_message","连接已经认证。")),
                    };
                    if let Some(reply)=reply {if !send(&mut socket,reply).await{break;}}
                },_=>{send(&mut socket,error("invalid_message","只接受版本化 JSON 消息。")).await;}}
            }
        }
    }
    release(&state, &credential_hash, &instance);
    let _ = socket.send(Message::Close(None)).await;
}
fn release(state: &HttpState, credential: &str, instance: &str) {
    let mut slots = state.live.slots.lock().unwrap();
    if slots
        .get(credential)
        .is_some_and(|s| s.instance == instance)
    {
        slots.remove(credential);
    }
}

impl DesktopService {
    pub fn import_ink_recovery(
        &self,
        document: mathscape_domain::InkLabDocument,
        request: &str,
    ) -> Result<mathscape_store_sqlite::StoredInkSession, String> {
        store(self)?
            .import_ink_recovery(&document, request, now())
            .map_err(|error| error.to_string())
    }
    pub async fn start_ink_lan(&self, port: Option<u16>) -> Result<InkLanStatus, String> {
        let mut running = self.ink_lan.running.lock().await;
        if let Some(active) = running.as_ref() {
            if !active.task.is_finished() {
                return Ok(active.status.clone());
            }
        }
        running.take();
        let settings = store(self)?.ink_lan_settings().map_err(|e| e.to_string())?;
        let listener = std::net::TcpListener::bind((
            std::net::Ipv4Addr::UNSPECIFIED,
            port.unwrap_or(settings.port),
        ))
        .map_err(|e| format!("无法开启 InkPad 端口：{e}"))?;
        let actual_port = listener.local_addr().map_err(|e| e.to_string())?.port();
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let hosts = ink_tls::addresses();
        let config = ink_tls::config(self.workspace_root(), hosts.clone()).await?;
        let (_, fingerprint) = ink_tls::certificate(self.workspace_root())?;
        let status = InkLanStatus {
            enabled: true,
            running: true,
            port: actual_port,
            urls: hosts
                .iter()
                .map(|host| format!("https://{host}:{actual_port}/pad/"))
                .collect(),
            certificate_file_sha256: Some(fingerprint),
            assets_ready: !INKPAD_ASSETS.is_empty(),
            error: None,
        };
        let state = HttpState {
            service: self.clone(),
            hosts: Arc::new(
                hosts
                    .iter()
                    .map(|host| format!("{host}:{actual_port}"))
                    .collect(),
            ),
            live: self.ink_lan.live.clone(),
        };
        let router = Router::new()
            .route("/", get(|| async { Redirect::temporary("/pad/") }))
            .route("/pad", get(|| async { Redirect::temporary("/pad/") }))
            .route("/pad/", get(index))
            .route("/pad/assets/{*path}", get(asset))
            .route("/pad/sw.js", get(sw))
            .route("/api/pair", post(pair))
            .route("/api/ws", get(ws))
            .route(
                "/health",
                get(|| async {
                    Json(json!({"protocol_version":INK_LAN_PROTOCOL_VERSION,"tls":true}))
                }),
            )
            .layer(DefaultBodyLimit::max(16 * 1024))
            .with_state(state);
        let handle = axum_server::Handle::new();
        let shutdown = handle.clone();
        let server = axum_server::from_tcp_rustls(listener, config).map_err(|e| e.to_string())?;
        store(self)?
            .save_ink_lan_settings(&InkLanSettings {
                enabled: true,
                port: actual_port,
            })
            .map_err(|e| e.to_string())?;
        let task = tokio::spawn(async move {
            let _ = server
                .handle(shutdown)
                .serve(router.into_make_service())
                .await;
        });
        *running = Some(Running {
            handle,
            task,
            status: status.clone(),
        });
        Ok(status)
    }
    pub async fn ink_lan_status(&self) -> Result<InkLanStatus, String> {
        let running = self.ink_lan.running.lock().await;
        if let Some(active) = running.as_ref() {
            let mut status = active.status.clone();
            if active.task.is_finished() {
                status.running = false;
                status.error = Some("InkPad 服务已停止，请重新开启服务。".into());
            }
            return Ok(status);
        }
        let settings = store(self)?.ink_lan_settings().map_err(|e| e.to_string())?;
        Ok(InkLanStatus {
            enabled: settings.enabled,
            running: false,
            port: settings.port,
            urls: vec![],
            certificate_file_sha256: None,
            assets_ready: !INKPAD_ASSETS.is_empty(),
            error: None,
        })
    }
    /// Process restart preserves lease authentication. Explicit Stop revokes it.
    pub async fn shutdown_ink_lan(&self, revoke: bool) -> Result<(), String> {
        if revoke {
            store(self)?
                .revoke_ink_lan(now())
                .map_err(|e| e.to_string())?;
        }
        let _ = self.ink_lan.live.control.send(Control::Stop(revoke));
        let active = self.ink_lan.running.lock().await.take();
        if let Some(active) = active {
            active.handle.shutdown();
            let _ = tokio::time::timeout(Duration::from_secs(2), active.task).await;
        }
        self.ink_lan.live.slots.lock().unwrap().clear();
        self.ink_lan.live.previews.lock().unwrap().clear();
        self.ink_lan.live.reports.lock().unwrap().clear();
        Ok(())
    }
    pub async fn create_ink_pairing(
        &self,
        document: String,
        head: u64,
    ) -> Result<InkPairingLink, String> {
        let status = self.ink_lan_status().await?;
        if !status.running {
            return Err("请先开启平板书写服务。".into());
        }
        let token = secret()?;
        let mut database = store(self)?;
        let view = database
            .ink_writer_view(&document, now())
            .map_err(|e| e.to_string())?;
        let expiry = now() + 120;
        database
            .issue_ink_pairing(
                &InkPairingOffer {
                    token_hash: hash(&token)?,
                    document_id: document.clone(),
                    expected_head: head,
                    expected_epoch: view.writer_epoch,
                    session_id: format!("inkpad-session-{}", &secret()?[..32]),
                    writer_id: format!("inkpad-{}", &secret()?[..32]),
                    expires_at: expiry,
                },
                now(),
            )
            .map_err(|e| e.to_string())?;
        Ok(InkPairingLink {
            urls: status
                .urls
                .into_iter()
                .map(|url| format!("{url}#pair={token}"))
                .collect(),
            expires_at: expiry,
            document_id: document,
        })
    }
    pub fn ink_lan_mirror(&self, document: &str, head: u64) -> Result<InkLanMirror, String> {
        let delta = store(self)?
            .ink_mirror_delta(document, head, now())
            .map_err(|e| e.to_string())?;
        let connected = self
            .ink_lan
            .live
            .slots
            .lock()
            .unwrap()
            .values()
            .any(|slot| slot.document_id == document && slot.epoch == delta.writer.writer_epoch);
        let preview = self
            .ink_lan
            .live
            .previews
            .lock()
            .unwrap()
            .get(document)
            .filter(|p| {
                p.updated.elapsed() < Duration::from_secs(3)
                    && p.epoch == delta.writer.writer_epoch
                    && delta
                        .metadata
                        .pages
                        .iter()
                        .any(|page| page.id == p.stroke.page_id)
            })
            .map(|p| p.stroke.clone());
        let client = self
            .ink_lan
            .live
            .reports
            .lock()
            .unwrap()
            .get(document)
            .filter(|report| report.epoch == delta.writer.writer_epoch)
            .map(|report| report.value.clone());
        Ok(InkLanMirror {
            delta,
            connected,
            preview,
            client,
        })
    }
    pub fn reclaim_ink_writer(
        &self,
        document: &str,
        epoch: u64,
        head: u64,
        request: &str,
    ) -> Result<mathscape_store_sqlite::StoredInkSession, String> {
        let result = store(self)?
            .take_ink_writer(
                document,
                epoch,
                head,
                &format!("desktop-session-{request}"),
                request,
                now(),
            )
            .map_err(|e| e.to_string())?;
        let _ = self
            .ink_lan
            .live
            .control
            .send(Control::Revoke(document.into()));
        Ok(result)
    }
}

#[tauri::command]
pub(crate) async fn start_ink_lan(
    state: TauriState<'_, DesktopState>,
    port: Option<u16>,
) -> Result<InkLanStatus, String> {
    state.service.start_ink_lan(port).await
}
#[tauri::command]
pub(crate) async fn get_ink_lan_status(
    state: TauriState<'_, DesktopState>,
) -> Result<InkLanStatus, String> {
    state.service.ink_lan_status().await
}
#[tauri::command]
pub(crate) async fn stop_ink_lan(state: TauriState<'_, DesktopState>) -> Result<(), String> {
    state.service.shutdown_ink_lan(true).await
}
#[tauri::command]
pub(crate) async fn create_ink_pairing(
    state: TauriState<'_, DesktopState>,
    document_id: String,
    expected_head: u64,
) -> Result<InkPairingLink, String> {
    state
        .service
        .create_ink_pairing(document_id, expected_head)
        .await
}
#[tauri::command]
pub(crate) async fn cancel_ink_pairing(
    state: TauriState<'_, DesktopState>,
    document_id: String,
) -> Result<(), String> {
    store(&state.service)?
        .cancel_ink_pairing(&document_id, now())
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub(crate) async fn get_ink_lan_mirror(
    state: TauriState<'_, DesktopState>,
    document_id: String,
    after_seq: u64,
) -> Result<InkLanMirror, String> {
    let service = state.service.clone();
    tokio::task::spawn_blocking(move || service.ink_lan_mirror(&document_id, after_seq))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub(crate) async fn reclaim_ink_writer(
    state: TauriState<'_, DesktopState>,
    document_id: String,
    expected_epoch: u64,
    expected_head: u64,
    request_id: String,
) -> Result<mathscape_store_sqlite::StoredInkSession, String> {
    let service = state.service.clone();
    tokio::task::spawn_blocking(move || {
        service.reclaim_ink_writer(&document_id, expected_epoch, expected_head, &request_id)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub(crate) async fn export_ink_trust_certificate(
    app: AppHandle,
    state: TauriState<'_, DesktopState>,
) -> Result<Option<String>, String> {
    let (pem, fingerprint) = ink_tls::certificate(state.service.workspace_root())?;
    let selected = app
        .dialog()
        .file()
        .add_filter("InkPad trust certificate", &["crt"])
        .set_file_name(format!("Mathscape-InkPad-{}.crt", &fingerprint[..8]))
        .blocking_save_file();
    let Some(selected) = selected else {
        return Ok(None);
    };
    let path = selected.into_path().map_err(|e| e.to_string())?;
    std::fs::write(&path, pem).map_err(|e| e.to_string())?;
    Ok(Some(path.to_string_lossy().into()))
}

#[tauri::command]
pub(crate) async fn choose_ink_recovery(
    app: AppHandle,
    state: TauriState<'_, DesktopState>,
    request_id: String,
) -> Result<Option<mathscape_store_sqlite::StoredInkSession>, String> {
    let selected = app
        .dialog()
        .file()
        .add_filter("Ink JSON recovery", &["json"])
        .blocking_pick_file();
    let Some(selected) = selected else {
        return Ok(None);
    };
    let path = selected.into_path().map_err(|error| error.to_string())?;
    let service = state.service.clone();
    tokio::task::spawn_blocking(move || {
        if std::fs::metadata(&path)
            .map_err(|error| error.to_string())?
            .len()
            > 128 * 1024 * 1024
        {
            return Err("恢复文件超过读取上限。".into());
        }
        let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
        let document =
            serde_json::from_slice(&bytes).map_err(|error| format!("Ink 恢复文件无效：{error}"))?;
        service.import_ink_recovery(document, &request_id).map(Some)
    })
    .await
    .map_err(|error| error.to_string())?
}
