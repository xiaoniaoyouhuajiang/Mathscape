pub use crate::ink_tls::AddressKind;
use crate::{CreateInkSnapshotRequest, DesktopService, DesktopState, ink_tls};
use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, OriginalUri, Path, Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use futures::SinkExt;
use mathscape_domain::{
    INK_LAN_DOCUMENT_BYTES, INK_LAN_OPERATION_BYTES, INK_LAN_PROTOCOL_VERSION, InkLabStroke,
    InkLanOperation, validate_ink_preview,
};
use mathscape_store_sqlite::{
    InkLanSettings, InkLifecycle, InkMirrorDelta, InkNoteState, InkPairingOffer,
    SqliteArtifactStore, SqliteStoreError,
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
/// A setup link only exposes the public CA and guide; it never grants writing.
const SETUP_SECONDS: i64 = 15 * 60;
/// Setup hand-off pairings are reissued on demand, so they stay short-lived.
const HANDOFF_PAIRING_SECONDS: i64 = 300;
const HANDOFF_CREDENTIALS: usize = 16;
const HTTPS_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' blob: data:; connect-src 'self' wss:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";
const SETUP_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self' https:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";
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
fn ct_eq(left: &str, right: &str) -> bool {
    let (a, b) = (left.as_bytes(), right.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct StandbyKeyFile {
    token: String,
    #[serde(default)]
    revoked_after_stop: bool,
}
fn standby_key_path(workspace_root: &std::path::Path) -> std::path::PathBuf {
    workspace_root.join("ink-lan/standby.key")
}
fn write_standby_key(workspace_root: &std::path::Path, key: &StandbyKeyFile) -> Result<(), String> {
    use std::io::Write;
    let dir = workspace_root.join("ink-lan");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = standby_key_path(workspace_root);
    let tmp = dir.join(format!("standby.key.tmp.{}", secret()?));
    let payload = serde_json::to_vec(key).map_err(|e| e.to_string())?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp).map_err(|e| e.to_string())?;
    file.write_all(&payload)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}
fn valid_standby_token(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
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
    /// N3.0: kind of each entry in `urls` (same order); LAN entries come first.
    pub url_kinds: Vec<AddressKind>,
    /// N3a: host the user last preferred for the QR / default URL.
    pub preferred_host: Option<String>,
    pub certificate_file_sha256: Option<String>,
    pub assets_ready: bool,
    pub online_tablets: usize,
    pub error: Option<String>,
}
#[derive(Serialize)]
pub struct InkPairingLink {
    pub urls: Vec<String>,
    /// N3.0: kind of each entry in `urls` (same order).
    pub url_kinds: Vec<AddressKind>,
    pub expires_at: i64,
    pub document_id: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct InkPushResult {
    pub pushed: bool,
    pub online: usize,
    pub requires_takeover: bool,
    pub expires_at: Option<i64>,
    pub document_id: String,
    pub title: Option<String>,
    pub message: String,
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
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InkSetupCheck {
    pub secure_context: bool,
    pub indexed_db: bool,
    pub service_worker: bool,
    pub user_agent: String,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct InkSetupStatus {
    pub active: bool,
    pub expires_at: Option<i64>,
    /// Only returned when the link is created; the server keeps a token digest.
    pub urls: Vec<String>,
    /// N3.0: kind of each entry in `urls` (same order).
    pub url_kinds: Vec<AddressKind>,
    pub opened: bool,
    pub certificate_downloaded: bool,
    pub https_trusted: bool,
    pub check: Option<InkSetupCheck>,
    pub complete: bool,
    /// Manuscript the tablet enters after setup; `None` keeps P3.1 behavior.
    pub document_id: Option<String>,
    /// `unbound`, `idle`, `requested`, `ready`, `resumed` or `claimed`.
    pub handoff: String,
    /// Another device holds the lease and the desktop user must confirm.
    pub transfer_confirmation: bool,
}
struct SetupPairing {
    token: String,
    token_hash: String,
    expires_at: i64,
}
struct SetupState {
    token_hash: String,
    /// Writing hand-off code. It only travels in URL fragments and HTTPS bodies.
    grant_hash: String,
    expires_at: i64,
    http_port: u16,
    https_port: u16,
    opened: bool,
    downloaded: bool,
    trusted: bool,
    check: Option<InkSetupCheck>,
    document_id: Option<String>,
    title: Option<String>,
    requested: bool,
    confirm_needed: bool,
    pairing: Option<SetupPairing>,
    outcome: Option<&'static str>,
}
impl SetupState {
    fn complete(&self) -> bool {
        self.trusted
            && self
                .check
                .as_ref()
                .is_some_and(|c| c.secure_context && c.indexed_db)
    }
    fn live_pairing(&self) -> Option<&SetupPairing> {
        self.pairing.as_ref().filter(|p| p.expires_at > now())
    }
    fn handoff(&self) -> &'static str {
        if self.document_id.is_none() {
            "unbound"
        } else if let Some(outcome) = self.outcome {
            outcome
        } else if self.live_pairing().is_some() {
            "ready"
        } else if self.requested {
            "requested"
        } else {
            "idle"
        }
    }
    fn status(&self) -> InkSetupStatus {
        let active = self.expires_at > now();
        InkSetupStatus {
            active,
            expires_at: Some(self.expires_at),
            urls: vec![],
            url_kinds: vec![],
            opened: self.opened,
            certificate_downloaded: self.downloaded,
            https_trusted: self.trusted,
            complete: self.complete(),
            check: self.check.clone(),
            document_id: self.document_id.clone(),
            handoff: self.handoff().into(),
            transfer_confirmation: self.confirm_needed && self.handoff() == "requested",
        }
    }
}
struct SetupServer {
    handle: axum_server::Handle<std::net::SocketAddr>,
    task: tokio::task::JoinHandle<()>,
    port: u16,
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
    /// The document moved to the trash; its tablet connection must close.
    Trashed(String),
    /// Title metadata changed; connected tablets refresh their header.
    Note(InkNoteState),
    /// N3a: push a one-time switch_offer to every connected tablet.
    SwitchOffer(Value),
    /// N3b: primary-link problem card for one document (or clear).
    ProblemCard {
        document_id: String,
        payload: Value,
    },
}
/// Old pads only fence on known codes. Trash therefore reuses
/// `writer_revoked` and adds a `reason` that new pads show specifically (D7).
const TRASHED_REASON: &str = "document_trashed";
const TRASHED_MESSAGE: &str = "这份手稿已在电脑上移到回收站，平板不能继续写入。恢复后需要在电脑重新配对；本机未同步内容会保留，可导出恢复草稿。";

/// Put the preferred host first when it is still among the candidates.
fn prefer_host(
    mut addresses: Vec<ink_tls::HostAddress>,
    preferred: Option<&str>,
) -> Vec<ink_tls::HostAddress> {
    if let Some(host) = preferred
        && let Some(index) = addresses.iter().position(|address| address.host == host)
    {
        let chosen = addresses.remove(index);
        addresses.insert(0, chosen);
    }
    addresses
}

fn trashed_closed() -> Value {
    json!({"type":"session_closed","code":"writer_revoked","reason":TRASHED_REASON,"message":TRASHED_MESSAGE})
}
struct Live {
    slots: Mutex<HashMap<String, Slot>>,
    /// Read-only push-ready standby sockets keyed by `client_id -> instance`.
    standby_slots: Mutex<HashMap<String, String>>,
    standby_key_lock: Mutex<()>,
    previews: Mutex<HashMap<String, Preview>>,
    reports: Mutex<HashMap<String, Report>>,
    setup: Mutex<Option<SetupState>>,
    control: broadcast::Sender<Control>,
    connections: Arc<Semaphore>,
}
impl Live {
    fn new() -> Self {
        let (control, _) = broadcast::channel(32);
        Self {
            slots: Mutex::new(HashMap::new()),
            standby_slots: Mutex::new(HashMap::new()),
            standby_key_lock: Mutex::new(()),
            previews: Mutex::new(HashMap::new()),
            reports: Mutex::new(HashMap::new()),
            setup: Mutex::new(None),
            control,
            connections: Arc::new(Semaphore::new(16)),
        }
    }
    fn online_tablet_count(&self) -> usize {
        let slots = self.slots.lock().unwrap();
        let standby = self.standby_slots.lock().unwrap();
        let mut ids: HashSet<&str> = slots.values().map(|slot| slot.client_id.as_str()).collect();
        ids.extend(standby.keys().map(String::as_str));
        ids.len()
    }
    fn load_or_create_standby_key(
        &self,
        workspace_root: &std::path::Path,
    ) -> Result<StandbyKeyFile, String> {
        let _guard = self.standby_key_lock.lock().unwrap();
        let path = standby_key_path(workspace_root);
        if let Ok(bytes) = std::fs::read(&path)
            && let Ok(parsed) = serde_json::from_slice::<StandbyKeyFile>(&bytes)
            && valid_standby_token(&parsed.token)
        {
            return Ok(parsed);
        }
        let created = StandbyKeyFile {
            token: secret()?,
            revoked_after_stop: false,
        };
        write_standby_key(workspace_root, &created)?;
        Ok(created)
    }
    fn rotate_standby_key_on_stop(&self, workspace_root: &std::path::Path) -> Result<(), String> {
        let _guard = self.standby_key_lock.lock().unwrap();
        let rotated = StandbyKeyFile {
            token: secret()?,
            revoked_after_stop: true,
        };
        write_standby_key(workspace_root, &rotated)
    }
}
struct Running {
    handle: axum_server::Handle<std::net::SocketAddr>,
    task: tokio::task::JoinHandle<()>,
    status: InkLanStatus,
}
pub struct InkLanRuntime {
    running: tokio::sync::Mutex<Option<Running>>,
    setup_server: tokio::sync::Mutex<Option<SetupServer>>,
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
            setup_server: tokio::sync::Mutex::new(None),
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
    // A trashed document keeps the code old pads already fence on (D7).
    if message.contains(TRASHED_REASON)
        || message.contains("writer_revoked")
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
    if message.contains(TRASHED_REASON) {
        let mut reply = error("writer_revoked", TRASHED_MESSAGE);
        reply["reason"] = TRASHED_REASON.into();
        return reply;
    }
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
fn secure_response(response: Response) -> Response {
    secure_response_with(response, HTTPS_CSP)
}
fn secure_response_with(mut response: Response, csp: &'static str) -> Response {
    for (name, value) in [
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        ("referrer-policy", "no-referrer"),
        ("cache-control", "no-store"),
        ("content-security-policy", csp),
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
    asset_response_with(name, HTTPS_CSP)
}
fn asset_response_with(name: &str, csp: &'static str) -> Response {
    let Some((_, bytes)) = INKPAD_ASSETS.iter().find(|(path, _)| *path == name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mime = if name.ends_with(".js") {
        "text/javascript; charset=utf-8"
    } else if name.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if name.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if name.ends_with(".woff2") {
        "font/woff2"
    } else if name.ends_with(".woff") {
        "font/woff"
    } else if name.ends_with(".ttf") {
        "font/ttf"
    } else {
        "application/octet-stream"
    };
    secure_response_with(
        ([(header::CONTENT_TYPE, mime)], *bytes).into_response(),
        csp,
    )
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
    let service = state.service.clone();
    let token_hash = token.clone();
    let result = tokio::task::spawn_blocking(move || {
        store(&service)?
            .claim_ink_pairing_detailed(&token, &credential, &request.device, now())
            .map_err(|e| e.to_string())
    })
    .await;
    match result {
        Ok(Ok((lease, transferred)))=>{
            if transferred {
                // The previous holder's credential is revoked in the same transaction.
                let _ = state.live.control.send(Control::Revoke(lease.document_id.clone()));
                if let Some(setup) = state.live.setup.lock().unwrap().as_mut()
                    && setup.pairing.as_ref().is_some_and(|p| p.token_hash == token_hash) {
                        setup.outcome = Some("claimed");
                    }
            }
            let mut body = json!({"protocol_version":INK_LAN_PROTOCOL_VERSION,"lease":lease});
            if let Ok(key) = state
                .live
                .load_or_create_standby_key(state.service.workspace_root())
            {
                body["standby_token"] = key.token.into();
            }
            secure_response(Json(body).into_response())
        }
        Ok(Err(message)) if message.contains(TRASHED_REASON)=>secure_response((StatusCode::CONFLICT,Json(json!({"code":"writer_revoked","reason":TRASHED_REASON,"message":"这份手稿已在电脑上移到回收站，无法配对。"}))).into_response()),
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
    /// Read-only push-ready standby channel for paired tablets without an active note lease.
    HelloStandby {
        protocol_version: String,
        client_id: String,
        #[serde(default)]
        standby_token: Option<String>,
        #[serde(default)]
        credential: Option<String>,
    },
    OperationSubmit {
        request: Box<InkLanOperation>,
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
    /// Tablet rename (N1). Old pads never send it; the reply is either
    /// `rename_receipt` or an `operation_conflict` with a title code.
    RenameRequest {
        request_id: String,
        title: String,
        expected_title_revision: u64,
    },
    Ping,
}
/// Remote rename outcome mapped to a wire reply.
fn rename_reply(
    request_id: &str,
    result: Result<mathscape_store_sqlite::InkNoteReceipt, SqliteStoreError>,
) -> Value {
    match result {
        Ok(receipt) => {
            json!({"type":"rename_receipt","request_id":request_id,"note":receipt.note,"replayed":receipt.replayed})
        }
        Err(SqliteStoreError::InkTitleConflict {
            current_title,
            current_title_mode,
            current_revision,
            ..
        }) => json!({
            "type":"operation_conflict","code":"title_conflict","request_id":request_id,
            "current_title":current_title,"current_title_mode":current_title_mode,
            "current_title_revision":current_revision,
            "message":"标题已在电脑上修改，请选择保留哪一个。"
        }),
        Err(SqliteStoreError::InkTitleInvalid(message)) => {
            json!({"type":"operation_conflict","code":"title_invalid","request_id":request_id,"message":message})
        }
        Err(SqliteStoreError::InkRequestConflict(_)) => {
            json!({"type":"operation_conflict","code":"rename_request_conflict","request_id":request_id,"message":"同一改名请求包含不同内容，已忽略。"})
        }
        Err(error) => {
            let mut reply = public_error(&error.to_string());
            reply["request_id"] = request_id.into();
            reply
        }
    }
}
async fn send(socket: &mut WebSocket, value: Value) -> bool {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .is_ok()
}
async fn socket_loop(mut socket: WebSocket, state: HttpState) {
    serve_socket(&mut socket, state).await;
    let _ = tokio::time::timeout(Duration::from_secs(1), async {
        let _ = socket.send(Message::Close(None)).await;
        if socket.flush().await.is_err() {
            return;
        }
        while let Some(Ok(message)) = socket.recv().await {
            if matches!(message, Message::Close(_)) {
                break;
            }
        }
        let _ = socket.flush().await;
    })
    .await;
}
async fn serve_socket(socket: &mut WebSocket, state: HttpState) {
    let first = tokio::time::timeout(Duration::from_secs(10), socket.recv()).await;
    let Ok(Some(Ok(Message::Text(text)))) = first else {
        return;
    };
    let (protocol_version, credential, client_id) =
        match serde_json::from_str::<ClientMessage>(&text) {
            Ok(ClientMessage::Hello {
                protocol_version,
                credential,
                client_id,
            }) => (protocol_version, credential, client_id),
            Ok(ClientMessage::HelloStandby {
                protocol_version,
                client_id,
                standby_token,
                credential,
            }) => {
                standby_loop(
                    socket,
                    state,
                    protocol_version,
                    client_id,
                    standby_token,
                    credential,
                )
                .await;
                return;
            }
            _ => {
                send(socket, error("authentication_failed", "连接认证失败。")).await;
                return;
            }
        };
    if protocol_version != INK_LAN_PROTOCOL_VERSION || client_id.is_empty() || client_id.len() > 200
    {
        send(
            socket,
            error("protocol_mismatch", "InkPad 版本不兼容，请重新加载页面。"),
        )
        .await;
        return;
    }
    let Ok(credential_hash) = hash(&credential) else {
        send(socket, error("authentication_failed", "连接认证失败。")).await;
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
        Ok(Err(message)) if message.contains(TRASHED_REASON) => {
            send(socket, public_error(&message)).await;
            return;
        }
        Ok(Err(message)) => {
            send(
                socket,
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
            socket,
            error(
                "device_busy",
                "此手稿已在另一个画板窗口打开，请关闭那个窗口后重试。",
            ),
        )
        .await;
        return;
    }
    let mut full = json!({"type":"session_context","protocol_version":INK_LAN_PROTOCOL_VERSION,"lease":lease,"note":document.note,"document":document.document,"head_seq":document.head_seq,"limits":{"operation_bytes":INK_LAN_OPERATION_BYTES,"document_bytes":INK_LAN_DOCUMENT_BYTES,"queue_bytes":INK_LAN_DOCUMENT_BYTES}});
    if let Ok(key) = state
        .live
        .load_or_create_standby_key(state.service.workspace_root())
    {
        full.as_object_mut()
            .unwrap()
            .insert("standby_token".into(), key.token.into());
    }
    // N3b: primary-link card for this authorized note only (§10).
    match state.service.ink_problem_card(&lease.document_id) {
        Ok(card) => {
            full.as_object_mut().unwrap().insert(
                "problem_card".into(),
                serde_json::to_value(card).unwrap_or(Value::Null),
            );
        }
        Err(_) => {
            full.as_object_mut()
                .unwrap()
                .insert("problem_card".into(), Value::Null);
        }
    }
    if full.to_string().len() > INK_LAN_DOCUMENT_BYTES {
        send(
            socket,
            error(
                "operation_limit",
                "此手稿超过当前传输上限，请在电脑查看或导出原稿。",
            ),
        )
        .await;
        release(&state, &credential_hash, &instance);
        return;
    }
    if !send(socket, full).await {
        release(&state, &credential_hash, &instance);
        return;
    }
    let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
    let mut last_seen = Instant::now();
    loop {
        tokio::select! {
            event=control.recv()=>{
                if let Ok(Control::Trashed(doc))=&event {
                    if *doc==lease.document_id {send(socket,trashed_closed()).await;break;}
                    continue;
                }
                if let Ok(Control::Note(note))=&event {
                    if note.document_id==lease.document_id && !send(socket,json!({"type":"note_metadata","note":note})).await {break;}
                    continue;
                }
                if let Ok(Control::SwitchOffer(payload))=&event {
                    // Every connected tablet receives the offer; the first to claim wins.
                    if !send(socket, payload.clone()).await {break;}
                    continue;
                }
                if let Ok(Control::ProblemCard{document_id,payload})=&event {
                    // §10: only the tablet writing this note sees its card.
                    if document_id==&lease.document_id && !send(socket, payload.clone()).await {break;}
                    continue;
                }
                let code=match event {Ok(Control::Stop(revoke))=>Some(if revoke{"writer_revoked"}else{"server_restart"}),Ok(Control::Revoke(doc)) if doc==lease.document_id=>Some("writer_revoked"),Ok(Control::Replace(old)) if old==instance=>Some("connection_replaced"),Err(broadcast::error::RecvError::Lagged(_))=>Some("reconnect_required"),_=>None};
                if let Some(code)=code {send(socket,json!({"type":"session_closed","code":code})).await;break;}
            }
            _=heartbeat.tick()=>{if last_seen.elapsed()>Duration::from_secs(50){break;}
                if socket.send(Message::Ping(Vec::new().into())).await.is_err(){break;}}
            incoming=socket.recv()=>{
                let Some(Ok(message))=incoming else{break;};last_seen=Instant::now();
                match message {Message::Pong(_)=>continue,Message::Ping(data)=>{if socket.send(Message::Pong(data)).await.is_err(){break;}continue;},Message::Close(_)=>break,Message::Text(text)=>{
                    if !state.live.slots.lock().unwrap().get(&credential_hash).is_some_and(|slot|slot.instance==instance){break;}
                    let parsed=serde_json::from_str::<ClientMessage>(&text);
                    let Ok(message)=parsed else {if !send(socket,error("invalid_message","消息格式无效，内容未写入。")).await{break;}continue;};
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
                                service.create_ink_snapshot(CreateInkSnapshotRequest{request_id:id,document_id:reservation.document_id,expected_head}).map_err(|e|e.message)
                            }).await {Ok(Ok(snapshot))=>Some(json!({"type":"snapshot_receipt","request_id":request_id,"snapshot_id":snapshot.snapshot_id,"through_seq":snapshot.through_seq})),Ok(Err(message))=>{let mut reply=public_error(&message);reply["request_id"]=request_id.into();Some(reply)},Err(_)=>Some(error("service_unavailable","快照服务暂时不可用。"))}
                        }
                        ClientMessage::RenameRequest{request_id,title,expected_title_revision}=>{
                            let service=state.service.clone();let credential=credential_hash.clone();let id=request_id.clone();
                            let outcome=tokio::task::spawn_blocking(move||{
                                let mut database=SqliteArtifactStore::open(&service.workspace_root().join("mathscape.sqlite3"))?;
                                database.rename_remote_ink_note(&credential,&id,&title,expected_title_revision,now())
                            }).await;
                            match outcome {
                                Ok(result)=>{
                                    // A replay carries the note as it was then; never re-broadcast it.
                                    if let Ok(receipt)=&result && !receipt.replayed {note_changed(&state.live,&receipt.note);}
                                    Some(rename_reply(&request_id,result))
                                }
                                Err(_)=>Some(error("service_unavailable","电脑暂时无法改名，请稍后重试。")),
                            }
                        }
                        ClientMessage::Ping=>Some(json!({"type":"pong"})),
                        ClientMessage::Hello{..}|ClientMessage::HelloStandby{..}=>Some(error("invalid_message","连接已经认证。")),
                    };
                    if let Some(reply)=reply&& !send(socket,reply).await{break;}
                },_=>{send(socket,error("invalid_message","只接受版本化 JSON 消息。")).await;}}
            }
        }
    }
    release(&state, &credential_hash, &instance);
}
async fn standby_loop(
    socket: &mut WebSocket,
    state: HttpState,
    protocol_version: String,
    client_id: String,
    standby_token: Option<String>,
    credential: Option<String>,
) {
    if protocol_version != INK_LAN_PROTOCOL_VERSION || client_id.is_empty() || client_id.len() > 200
    {
        send(
            socket,
            error("protocol_mismatch", "InkPad 版本不兼容，请重新加载页面。"),
        )
        .await;
        return;
    }
    let Ok(key) = state
        .live
        .load_or_create_standby_key(state.service.workspace_root())
    else {
        send(socket, error("service_unavailable", "待命服务暂时不可用。")).await;
        return;
    };
    let token_valid = standby_token
        .as_deref()
        .is_some_and(|candidate| valid_standby_token(candidate) && ct_eq(candidate, &key.token));
    let authenticated = if token_valid {
        true
    } else if !key.revoked_after_stop {
        if let Some(raw_credential) = credential.as_deref()
            && let Ok(cred_hash) = hash(raw_credential)
        {
            let service = state.service.clone();
            matches!(
                tokio::task::spawn_blocking(move || {
                    store(&service)?
                        .can_ink_standby(&cred_hash, now())
                        .map_err(|e| e.to_string())
                })
                .await,
                Ok(Ok(true))
            )
        } else {
            false
        }
    } else {
        false
    };
    if !authenticated {
        let code = if standby_token.is_some() || credential.is_some() {
            "writer_revoked"
        } else {
            "authentication_failed"
        };
        send(socket, error(code, "待命认证已失效，请重新扫码配对。")).await;
        return;
    }
    let Ok(instance) = secret() else {
        return;
    };
    let mut control = state.live.control.subscribe();
    {
        let mut standby = state.live.standby_slots.lock().unwrap();
        if let Some(old_instance) = standby.insert(client_id.clone(), instance.clone()) {
            let _ = state.live.control.send(Control::Replace(old_instance));
        }
    }
    if !send(
        socket,
        json!({
            "type": "standby_ready",
            "protocol_version": INK_LAN_PROTOCOL_VERSION,
            "standby_token": key.token,
        }),
    )
    .await
    {
        release_standby(&state, &client_id, &instance);
        return;
    }
    let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
    let mut last_seen = Instant::now();
    loop {
        tokio::select! {
            event = control.recv() => {
                if let Ok(Control::SwitchOffer(payload)) = &event {
                    if !send(socket, payload.clone()).await {
                        break;
                    }
                    continue;
                }
                let code = match event {
                    Ok(Control::Stop(revoke)) => Some(if revoke { "writer_revoked" } else { "server_restart" }),
                    Ok(Control::Replace(old)) if old == instance => Some("connection_replaced"),
                    Err(broadcast::error::RecvError::Lagged(_)) => Some("reconnect_required"),
                    _ => None,
                };
                if let Some(code) = code {
                    send(socket, json!({"type": "session_closed", "code": code})).await;
                    break;
                }
            }
            _ = heartbeat.tick() => {
                if last_seen.elapsed() > Duration::from_secs(50) {
                    break;
                }
                if socket.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
            incoming = socket.recv() => {
                let Some(Ok(message)) = incoming else { break; };
                last_seen = Instant::now();
                match message {
                    Message::Pong(_) => continue,
                    Message::Ping(data) => {
                        if socket.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                        continue;
                    }
                    Message::Close(_) => break,
                    Message::Text(text) => {
                        if state
                            .live
                            .standby_slots
                            .lock()
                            .unwrap()
                            .get(&client_id)
                            .is_none_or(|current| current != &instance)
                        {
                            break;
                        }
                        let reply = match serde_json::from_str::<ClientMessage>(&text) {
                            Ok(ClientMessage::Ping) => Some(json!({"type": "pong"})),
                            Ok(_) => Some(error(
                                "standby_read_only",
                                "当前处于只读待命状态，不能写入手稿。",
                            )),
                            Err(_) => Some(error("invalid_message", "消息格式无效，内容未写入。")),
                        };
                        if let Some(reply) = reply
                            && !send(socket, reply).await
                        {
                            break;
                        }
                    }
                    _ => {
                        send(socket, error("invalid_message", "只接受版本化 JSON 消息。")).await;
                    }
                }
            }
        }
    }
    release_standby(&state, &client_id, &instance);
}
fn release_standby(state: &HttpState, client_id: &str, instance: &str) {
    let mut standby = state.live.standby_slots.lock().unwrap();
    if standby
        .get(client_id)
        .is_some_and(|current| current == instance)
    {
        standby.remove(client_id);
    }
}
/// Keeps the setup hand-off title current and pushes the note to tablets.
fn note_changed(live: &Live, note: &InkNoteState) {
    if let Some(setup) = live.setup.lock().unwrap().as_mut()
        && setup.document_id.as_deref() == Some(note.document_id.as_str())
    {
        setup.title = Some(note.title.clone());
    }
    let _ = live.control.send(Control::Note(note.clone()));
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

#[derive(Clone)]
struct SetupHttpState {
    service: DesktopService,
    hosts: Arc<HashSet<String>>,
}
#[derive(Deserialize)]
struct SetupQuery {
    t: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetupCheckRequest {
    token: String,
    check: InkSetupCheck,
}
/// Applies `update` only for the active, unexpired setup token.
fn with_setup<T>(
    live: &Live,
    token: &str,
    update: impl FnOnce(&mut SetupState) -> T,
) -> Result<T, StatusCode> {
    let digest = hash(token).map_err(|_| StatusCode::NOT_FOUND)?;
    let mut setup = live.setup.lock().unwrap();
    match setup.as_mut() {
        Some(state) if state.token_hash == digest && state.expires_at > now() => Ok(update(state)),
        _ => Err(StatusCode::GONE),
    }
}
fn setup_host(headers: &HeaderMap, hosts: &HashSet<String>) -> Result<String, StatusCode> {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .ok_or(StatusCode::FORBIDDEN)?;
    if hosts.contains(host) {
        Ok(host.to_owned())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}
async fn setup_index(State(state): State<SetupHttpState>, headers: HeaderMap) -> Response {
    if let Err(code) = setup_host(&headers, &state.hosts) {
        return code.into_response();
    }
    asset_response_with("setup.html", SETUP_CSP)
}
async fn setup_asset(
    State(state): State<SetupHttpState>,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Response {
    if let Err(code) = setup_host(&headers, &state.hosts) {
        return code.into_response();
    }
    asset_response_with(&format!("assets/{path}"), SETUP_CSP)
}
async fn setup_open(
    State(state): State<SetupHttpState>,
    headers: HeaderMap,
    Query(query): Query<SetupQuery>,
) -> Response {
    if let Err(code) = setup_host(&headers, &state.hosts) {
        return code.into_response();
    }
    let opened = with_setup(&state.service.ink_lan.live, &query.t, |setup| {
        setup.opened = true;
        (setup.https_port, setup.expires_at, setup.trusted)
    });
    let Ok((https_port, expires_at, trusted)) = opened else {
        return StatusCode::GONE.into_response();
    };
    let Ok((_, fingerprint)) = ink_tls::certificate(state.service.workspace_root()) else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    secure_response_with(
        Json(json!({"https_port":https_port,"expires_at":expires_at,"trusted":trusted,"certificate_name":format!("Mathscape-InkPad-{}.crt",&fingerprint[..8]),"fingerprint":fingerprint})).into_response(),
        SETUP_CSP,
    )
}
async fn setup_certificate(
    State(state): State<SetupHttpState>,
    headers: HeaderMap,
    Query(query): Query<SetupQuery>,
) -> Response {
    use rustls::pki_types::{CertificateDer, pem::PemObject};
    if let Err(code) = setup_host(&headers, &state.hosts) {
        return code.into_response();
    }
    if with_setup(&state.service.ink_lan.live, &query.t, |setup| {
        setup.downloaded = true
    })
    .is_err()
    {
        return StatusCode::GONE.into_response();
    }
    let Ok((pem, fingerprint)) = ink_tls::certificate(state.service.workspace_root()) else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let Ok(der) = CertificateDer::from_pem_slice(pem.as_bytes()) else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    // A generic binary type keeps Android browsers from routing the file to an
    // installer that rejects CA certificates; the user installs it in Settings.
    let mut response = (
        [(header::CONTENT_TYPE, "application/octet-stream")],
        der.as_ref().to_vec(),
    )
        .into_response();
    if let Ok(value) = HeaderValue::from_str(&format!(
        "attachment; filename=\"Mathscape-InkPad-{}.crt\"",
        &fingerprint[..8]
    )) {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, value);
    }
    secure_response_with(response, SETUP_CSP)
}
/// Reachable from the HTTP guide only after the device trusts the HTTPS chain.
async fn setup_probe(
    State(state): State<HttpState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Query(query): Query<SetupQuery>,
) -> Response {
    // HTTP/2 carries the authority in the URI rather than a Host header.
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .or_else(|| uri.authority().map(|authority| authority.as_str()))
        .filter(|host| state.hosts.contains(*host))
    else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let hostname = host
        .rsplit_once(':')
        .map_or(host, |(name, _)| name)
        .to_owned();
    let origin = headers.get(header::ORIGIN).and_then(|h| h.to_str().ok());
    let result = with_setup(&state.live, &query.t, |setup| {
        let expected = format!("http://{hostname}:{}", setup.http_port);
        if origin != Some(expected.as_str()) {
            return None;
        }
        setup.trusted = true;
        Some(expected)
    });
    let Ok(Some(allowed)) = result else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let mut response = secure_response(Json(json!({"trusted":true})).into_response());
    if let Ok(value) = HeaderValue::from_str(&allowed) {
        response
            .headers_mut()
            .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
        response
            .headers_mut()
            .insert(header::VARY, HeaderValue::from_static("Origin"));
    }
    response
}
async fn setup_check_page(
    State(state): State<HttpState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Response {
    if let Err(code) = origin(&headers, &state, false, &uri) {
        return code.into_response();
    }
    asset_response("setup.html")
}
async fn setup_check(
    State(state): State<HttpState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Json(request): Json<SetupCheckRequest>,
) -> Response {
    if let Err(code) = origin(&headers, &state, true, &uri) {
        return code.into_response();
    }
    let mut check = request.check;
    check.user_agent = check.user_agent.chars().take(512).collect();
    match with_setup(&state.live, &request.token, |setup| {
        setup.trusted = true;
        setup.check = Some(check);
        (
            setup.complete(),
            setup.document_id.clone(),
            setup.title.clone(),
        )
    }) {
        Ok((complete, document_id, title)) => secure_response(
            Json(json!({"complete":complete,"document_id":document_id,"title":title}))
                .into_response(),
        ),
        Err(code) => code.into_response(),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetupHandoffRequest {
    token: String,
    grant: String,
    #[serde(default)]
    credentials: Vec<String>,
}
/// Lets the checked tablet enter the bound manuscript without a second QR code.
/// It resumes with an existing device credential or receives a one-time pairing
/// token prepared by the desktop; the HTTP setup token alone is never enough.
async fn setup_handoff(
    State(state): State<HttpState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Json(request): Json<SetupHandoffRequest>,
) -> Response {
    if let Err(code) = origin(&headers, &state, true, &uri) {
        return code.into_response();
    }
    if request.credentials.len() > HANDOFF_CREDENTIALS {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    let Ok(grant) = hash(&request.grant) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let snapshot = with_setup(&state.live, &request.token, |setup| {
        if setup.grant_hash != grant || !setup.complete() {
            return None;
        }
        Some((
            setup.token_hash.clone(),
            setup.document_id.clone(),
            setup.title.clone(),
        ))
    });
    let (setup_hash, document, title) = match snapshot {
        Ok(Some(value)) => value,
        Ok(None) => return StatusCode::FORBIDDEN.into_response(),
        Err(code) => return code.into_response(),
    };
    let Some(document) = document else {
        return secure_response(Json(json!({"state":"unbound"})).into_response());
    };
    let digests: Vec<String> = request
        .credentials
        .iter()
        .filter_map(|value| hash(value).ok())
        .collect();
    let service = state.service.clone();
    let target = document.clone();
    let resumable = tokio::task::spawn_blocking(move || {
        let database = store(&service).ok()?;
        let time = now();
        Some(digests.iter().any(|digest| {
            database
                .authenticate_ink_device(digest, time)
                .is_ok_and(|lease| lease.document_id == target)
        }))
    })
    .await
    .ok()
    .flatten()
    .unwrap_or(false);
    let mut guard = state.live.setup.lock().unwrap();
    let Some(setup) = guard
        .as_mut()
        .filter(|setup| setup.token_hash == setup_hash && setup.expires_at > now())
    else {
        return StatusCode::GONE.into_response();
    };
    let reply = if resumable || setup.outcome == Some("resumed") {
        setup.outcome = Some("resumed");
        json!({"state":"resume","document_id":document,"title":title})
    } else if let Some(pairing) = setup.live_pairing() {
        json!({"state":"pair","pairing_token":pairing.token,"document_id":document,"title":title})
    } else if setup.outcome == Some("claimed") {
        json!({"state":"claimed","document_id":document,"title":title})
    } else {
        setup.requested = true;
        let reason = if setup.confirm_needed {
            "confirm"
        } else {
            "desktop"
        };
        json!({"state":"waiting","reason":reason,"document_id":document,"title":title})
    };
    drop(guard);
    secure_response(Json(reply).into_response())
}

impl DesktopService {
    /// Starts the HTTPS service if needed, then opens a short HTTP guide that
    /// only serves the public CA and setup page.
    pub async fn start_ink_setup(&self) -> Result<InkSetupStatus, String> {
        self.start_ink_setup_for(None).await
    }
    /// Binds the setup to a manuscript so the tablet can start writing directly.
    pub async fn start_ink_setup_for(
        &self,
        document: Option<String>,
    ) -> Result<InkSetupStatus, String> {
        let title = match &document {
            Some(id) => {
                let database = store(self)?;
                let note = database
                    .ink_note(id)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| "手稿不存在或已被移除。".to_string())?;
                if note.lifecycle == InkLifecycle::Trashed {
                    return Err("这份手稿在回收站中，恢复后才能连接平板。".into());
                }
                Some(note.title)
            }
            None => None,
        };
        let mut status = self.ink_lan_status().await?;
        if !status.running {
            status = self.start_ink_lan(None).await?;
        }
        if !status.assets_ready {
            return Err("InkPad 页面尚未构建，无法开始平板设置。".into());
        }
        let https_port = status.port;
        let preferred = store(self)?
            .ink_lan_settings()
            .map_err(|e| e.to_string())?
            .preferred_host;
        let addresses = prefer_host(ink_tls::host_addresses(), preferred.as_deref());
        let hosts = addresses
            .iter()
            .map(|address| address.host.clone())
            .collect::<Vec<_>>();
        let mut server = self.ink_lan.setup_server.lock().await;
        if server
            .as_ref()
            .is_some_and(|active| active.task.is_finished())
        {
            server.take();
        }
        if server.is_none() {
            let preferred = https_port.saturating_sub(1).max(1024);
            let listener =
                std::net::TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, preferred))
                    .or_else(|_| std::net::TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, 0)))
                    .map_err(|e| format!("无法开启平板设置端口：{e}"))?;
            let port = listener.local_addr().map_err(|e| e.to_string())?.port();
            listener.set_nonblocking(true).map_err(|e| e.to_string())?;
            let state = SetupHttpState {
                service: self.clone(),
                hosts: Arc::new(hosts.iter().map(|host| format!("{host}:{port}")).collect()),
            };
            let router = Router::new()
                .route("/", get(|| async { Redirect::temporary("/setup/") }))
                .route("/setup", get(|| async { Redirect::temporary("/setup/") }))
                .route("/setup/", get(setup_index))
                .route("/setup/ca.crt", get(setup_certificate))
                .route("/pad/assets/{*path}", get(setup_asset))
                .route("/api/setup/open", get(setup_open))
                .layer(DefaultBodyLimit::max(4 * 1024))
                .with_state(state);
            let handle = axum_server::Handle::new();
            let shutdown = handle.clone();
            let http = axum_server::from_tcp(listener).map_err(|e| e.to_string())?;
            let task = tokio::spawn(async move {
                let _ = http
                    .handle(shutdown)
                    .serve(router.into_make_service())
                    .await;
            });
            *server = Some(SetupServer { handle, task, port });
        }
        let http_port = server
            .as_ref()
            .map(|active| active.port)
            .unwrap_or_default();
        drop(server);
        let token = secret()?;
        let digest = hash(&token)?;
        let grant = secret()?;
        let expires_at = now() + SETUP_SECONDS;
        let state = SetupState {
            token_hash: digest.clone(),
            grant_hash: hash(&grant)?,
            expires_at,
            http_port,
            https_port,
            opened: false,
            downloaded: false,
            trusted: false,
            check: None,
            document_id: document,
            title,
            requested: false,
            confirm_needed: false,
            pairing: None,
            outcome: None,
        };
        let mut result = state.status();
        *self.ink_lan.live.setup.lock().unwrap() = Some(state);
        let service = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(SETUP_SECONDS as u64)).await;
            let current = service
                .ink_lan
                .live
                .setup
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|setup| setup.token_hash == digest);
            if current {
                service.cancel_ink_setup().await;
            }
        });
        result.urls = hosts
            .iter()
            .map(|host| format!("http://{host}:{http_port}/setup/#setup={token}&grant={grant}"))
            .collect();
        result.url_kinds = addresses.iter().map(|address| address.kind).collect();
        Ok(result)
    }
    /// Called by the desktop panel after the tablet reaches "start writing".
    /// The panel flushes local writes first; a remote holder needs confirmation.
    pub async fn prepare_ink_setup_handoff(
        &self,
        expected_head: Option<u64>,
        confirm_transfer: bool,
    ) -> Result<InkSetupStatus, String> {
        let (setup_hash, document) = {
            let guard = self.ink_lan.live.setup.lock().unwrap();
            let Some(setup) = guard.as_ref().filter(|setup| setup.expires_at > now()) else {
                return Err("平板设置已结束，请重新开始设置。".into());
            };
            let Some(document) = setup.document_id.clone() else {
                return Ok(setup.status());
            };
            if !setup.complete() || setup.handoff() != "requested" {
                return Ok(setup.status());
            }
            (setup.token_hash.clone(), document)
        };
        let view = store(self)?
            .ink_writer_view(&document, now())
            .map_err(|e| e.to_string())?;
        if view.remote && !confirm_transfer {
            let mut guard = self.ink_lan.live.setup.lock().unwrap();
            let setup = guard
                .as_mut()
                .filter(|setup| setup.token_hash == setup_hash)
                .ok_or("平板设置已结束，请重新开始设置。")?;
            setup.confirm_needed = true;
            return Ok(setup.status());
        }
        let (token, expires_at, _, _) = self
            .issue_pairing(
                &document,
                expected_head.unwrap_or(view.head_seq),
                view.remote,
                HANDOFF_PAIRING_SECONDS,
                "tablet_pairing",
            )
            .await?;
        let mut guard = self.ink_lan.live.setup.lock().unwrap();
        let Some(setup) = guard
            .as_mut()
            .filter(|setup| setup.token_hash == setup_hash)
        else {
            drop(guard);
            let _ = store(self)?.cancel_ink_pairing(&document, now());
            return Err("平板设置已结束，请重新开始设置。".into());
        };
        setup.pairing = Some(SetupPairing {
            token_hash: hash(&token)?,
            token,
            expires_at,
        });
        setup.confirm_needed = false;
        Ok(setup.status())
    }
    pub fn ink_setup_status(&self) -> InkSetupStatus {
        self.ink_lan
            .live
            .setup
            .lock()
            .unwrap()
            .as_ref()
            .map(SetupState::status)
            .unwrap_or_default()
    }
    pub async fn cancel_ink_setup(&self) {
        let previous = self.ink_lan.live.setup.lock().unwrap().take();
        // An unclaimed hand-off pairing would otherwise keep the desktop read-only.
        if let Some(setup) = previous
            && let (Some(document), Some(_), None) = (
                setup.document_id.as_ref(),
                setup.live_pairing(),
                setup.outcome,
            )
            && let Ok(mut database) = store(self)
        {
            let _ = database.cancel_ink_pairing(document, now());
        }
        let active = self.ink_lan.setup_server.lock().await.take();
        if let Some(active) = active {
            active.handle.shutdown();
            let _ = tokio::time::timeout(Duration::from_secs(2), active.task).await;
        }
    }
}

impl DesktopService {
    /// Called after a desktop rename so tablets and the setup page follow it.
    pub(crate) fn ink_note_renamed(&self, note: &InkNoteState) {
        note_changed(&self.ink_lan.live, note);
    }
    /// N3b: push the current primary-link card (or clear) to tablets on this note.
    pub(crate) fn ink_problem_card_changed(&self, document_id: &str) {
        let payload = match self.ink_problem_card(document_id) {
            Ok(Some(card)) => {
                crate::ink_problem_card::InkProblemCard::wire_push(document_id, &card)
            }
            Ok(None) => crate::ink_problem_card::InkProblemCard::wire_clear(document_id),
            Err(_) => crate::ink_problem_card::InkProblemCard::wire_clear(document_id),
        };
        let _ = self.ink_lan.live.control.send(Control::ProblemCard {
            document_id: document_id.into(),
            payload,
        });
    }
    /// Called after documents move to the trash: closes their tablet
    /// connections, drops live previews and ends a setup bound to them.
    pub(crate) async fn ink_notes_trashed(&self, documents: &[String]) {
        let live = &self.ink_lan.live;
        for document in documents {
            let _ = live.control.send(Control::Trashed(document.clone()));
            live.previews.lock().unwrap().remove(document);
            live.reports.lock().unwrap().remove(document);
        }
        let bound = live
            .setup
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|setup| setup.document_id.clone())
            .is_some_and(|bound| documents.contains(&bound));
        if bound {
            self.cancel_ink_setup().await;
        }
    }
    /// Whether a tablet connection is currently open for the document.
    pub(crate) fn ink_note_connected(&self, document: &str) -> bool {
        self.ink_lan
            .live
            .slots
            .lock()
            .unwrap()
            .values()
            .any(|slot| slot.document_id == document)
    }
    pub fn import_ink_recovery(
        &self,
        document: mathscape_domain::InkLabDocument,
        request: &str,
    ) -> Result<mathscape_store_sqlite::StoredInkSession, String> {
        store(self)?
            .import_ink_recovery(&document, request, now())
            .map_err(|error| error.to_string())
    }
    /// "导入恢复草稿": accepts an Ink JSON document or the A1 unsaved-operations
    /// export (`mathscape-ink-unsaved-operations`), which is rebuilt on the
    /// history prefix each queue was written against.
    pub fn import_ink_recovery_file(
        &self,
        bytes: &[u8],
        request: &str,
    ) -> Result<InkRecoveryImport, String> {
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|error| format!("Ink 恢复文件无效：{error}"))?;
        if value.get("format").and_then(serde_json::Value::as_str)
            == Some(mathscape_store_sqlite::INK_UNSAVED_OPERATIONS_FORMAT)
        {
            let export: mathscape_store_sqlite::InkUnsavedOperationsExport =
                serde_json::from_value(value)
                    .map_err(|error| format!("未保存笔迹导出文件无效：{error}"))?;
            let sessions = store(self)?
                .import_ink_unsaved_operations(&export, request, now())
                .map_err(|error| error.to_string())?;
            return Ok(InkRecoveryImport {
                source: "unsaved_operations".into(),
                original_document_id: Some(export.document_id),
                sessions,
            });
        }
        let document: mathscape_domain::InkLabDocument =
            serde_json::from_value(value).map_err(|error| format!("Ink 恢复文件无效：{error}"))?;
        let original = document.document_id.clone();
        let session = self.import_ink_recovery(document, request)?;
        Ok(InkRecoveryImport {
            source: "document".into(),
            original_document_id: Some(original),
            sessions: vec![session],
        })
    }
    pub async fn start_ink_lan(&self, port: Option<u16>) -> Result<InkLanStatus, String> {
        let mut running = self.ink_lan.running.lock().await;
        if let Some(active) = running.as_ref()
            && !active.task.is_finished()
        {
            return Ok(active.status.clone());
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
        let settings_now = store(self)?.ink_lan_settings().map_err(|e| e.to_string())?;
        let addresses = prefer_host(
            ink_tls::host_addresses(),
            settings_now.preferred_host.as_deref(),
        );
        let hosts = addresses
            .iter()
            .map(|address| address.host.clone())
            .collect::<Vec<_>>();
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
            url_kinds: addresses.iter().map(|address| address.kind).collect(),
            preferred_host: settings_now.preferred_host.clone(),
            certificate_file_sha256: Some(fingerprint),
            assets_ready: !INKPAD_ASSETS.is_empty(),
            online_tablets: self.ink_lan.live.online_tablet_count(),
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
            .route("/pad/setup.html", get(setup_check_page))
            .route("/api/setup/probe", get(setup_probe))
            .route("/api/setup/check", post(setup_check))
            .route("/api/setup/handoff", post(setup_handoff))
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
        let preferred_host = store(self)?
            .ink_lan_settings()
            .map_err(|e| e.to_string())?
            .preferred_host;
        store(self)?
            .save_ink_lan_settings(&InkLanSettings {
                enabled: true,
                port: actual_port,
                preferred_host,
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
        let settings = store(self)?.ink_lan_settings().map_err(|e| e.to_string())?;
        let running = self.ink_lan.running.lock().await;
        if let Some(active) = running.as_ref() {
            let mut status = active.status.clone();
            // Keep preferred_host live after set_ink_lan_preferred_host without restart.
            status.preferred_host = settings.preferred_host.clone();
            if active.task.is_finished() {
                status.running = false;
                status.online_tablets = 0;
                status.error = Some("InkPad 服务已停止，请重新开启服务。".into());
            } else {
                status.online_tablets = self.ink_lan.live.online_tablet_count();
            }
            return Ok(status);
        }
        Ok(InkLanStatus {
            enabled: settings.enabled,
            running: false,
            port: settings.port,
            urls: vec![],
            url_kinds: vec![],
            preferred_host: settings.preferred_host,
            certificate_file_sha256: None,
            assets_ready: !INKPAD_ASSETS.is_empty(),
            online_tablets: 0,
            error: None,
        })
    }
    /// Process restart preserves lease authentication. Explicit Stop revokes it.
    pub async fn shutdown_ink_lan(&self, revoke: bool) -> Result<(), String> {
        if revoke {
            store(self)?
                .revoke_ink_lan(now())
                .map_err(|e| e.to_string())?;
            self.ink_lan
                .live
                .rotate_standby_key_on_stop(self.workspace_root())?;
        }
        self.cancel_ink_setup().await;
        let _ = self.ink_lan.live.control.send(Control::Stop(revoke));
        let active = self.ink_lan.running.lock().await.take();
        if let Some(active) = active {
            active.handle.shutdown();
            let _ = tokio::time::timeout(Duration::from_secs(2), active.task).await;
        }
        self.ink_lan.live.slots.lock().unwrap().clear();
        self.ink_lan.live.standby_slots.lock().unwrap().clear();
        self.ink_lan.live.previews.lock().unwrap().clear();
        self.ink_lan.live.reports.lock().unwrap().clear();
        Ok(())
    }
    pub async fn create_ink_pairing(
        &self,
        document: String,
        head: u64,
    ) -> Result<InkPairingLink, String> {
        self.create_ink_pairing_with(document, head, false).await
    }
    /// `allow_transfer` hands the lease from another device to whichever device
    /// scans next; the current holder keeps writing until that moment.
    pub async fn create_ink_pairing_with(
        &self,
        document: String,
        head: u64,
        allow_transfer: bool,
    ) -> Result<InkPairingLink, String> {
        let (token, expires_at, urls, url_kinds) = self
            .issue_pairing(&document, head, allow_transfer, 120, "tablet_pairing")
            .await?;
        Ok(InkPairingLink {
            url_kinds,
            urls: urls
                .into_iter()
                .map(|url| format!("{url}#pair={token}"))
                .collect(),
            expires_at,
            document_id: document,
        })
    }
    async fn issue_pairing(
        &self,
        document: &str,
        head: u64,
        allow_transfer: bool,
        seconds: i64,
        claim_reason: &str,
    ) -> Result<(String, i64, Vec<String>, Vec<AddressKind>), String> {
        let status = self.ink_lan_status().await?;
        if !status.running {
            return Err("请先开启平板书写服务。".into());
        }
        let token = secret()?;
        let mut database = store(self)?;
        let view = database
            .ink_writer_view(document, now())
            .map_err(|e| e.to_string())?;
        let expiry = now() + seconds;
        database
            .issue_ink_pairing(
                &InkPairingOffer {
                    token_hash: hash(&token)?,
                    document_id: document.into(),
                    expected_head: if view.remote { view.head_seq } else { head },
                    expected_epoch: view.writer_epoch,
                    session_id: format!("inkpad-session-{}", &secret()?[..32]),
                    writer_id: format!("inkpad-{}", &secret()?[..32]),
                    expires_at: expiry,
                    allow_transfer,
                    claim_reason: claim_reason.into(),
                },
                now(),
            )
            .map_err(|e| e.to_string())?;
        // Reorder QR URLs with the latest preferred_host without restarting LAN.
        let preferred = database
            .ink_lan_settings()
            .map_err(|e| e.to_string())?
            .preferred_host;
        let addresses = prefer_host(ink_tls::host_addresses(), preferred.as_deref());
        let urls = addresses
            .iter()
            .map(|address| format!("https://{}:{}/pad/", address.host, status.port))
            .collect::<Vec<_>>();
        let url_kinds = addresses.iter().map(|address| address.kind).collect();
        Ok((token, expiry, urls, url_kinds))
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

    /// N3a: push a one-time `switch_offer` to every connected tablet. When no
    /// tablet is online the caller falls back to the QR flow. `allow_transfer`
    /// must be true when the target note already has a remote writer, matching
    /// the existing pairing takeover confirm.
    pub async fn push_ink_to_tablet(
        &self,
        document: &str,
        allow_transfer: bool,
    ) -> Result<InkPushResult, String> {
        let online = self.ink_lan.live.online_tablet_count();
        if online == 0 {
            return Ok(InkPushResult {
                pushed: false,
                online: 0,
                requires_takeover: false,
                expires_at: None,
                document_id: document.into(),
                title: None,
                message: "平板未连接。请用二维码配对，或确认平板已打开这份电脑上的手稿。".into(),
            });
        }
        let note = store(self)?
            .open_ink_document(document)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "找不到这份手稿。".to_string())?;
        if note.note.lifecycle != InkLifecycle::Active {
            return Err("回收站中的手稿不能推送到平板。".into());
        }
        let view = store(self)?
            .ink_writer_view(document, now())
            .map_err(|e| e.to_string())?;
        // Desktop holds the writer when there is no remote session.
        let requires_takeover = view.remote;
        if requires_takeover && !allow_transfer {
            return Ok(InkPushResult {
                pushed: false,
                online,
                requires_takeover: true,
                expires_at: None,
                document_id: document.into(),
                title: Some(note.note.title.clone()),
                message: if view.remote {
                    "这份手稿正在由平板书写。推送会转交写入权，对方未同步的内容会被隔离，可以导出恢复草稿。".into()
                } else {
                    "这份手稿正在由电脑书写。".into()
                },
            });
        }
        let (token, expires_at, _, _) = self
            .issue_pairing(
                document,
                view.head_seq,
                allow_transfer || !view.remote,
                120,
                "desktop_push",
            )
            .await?;
        let payload = json!({
            "type": "switch_offer",
            "offer_id": token.get(..16).unwrap_or(&token),
            "document_id": document,
            "title": note.note.title,
            "pairing_token": token,
            "expires_at": expires_at,
            "requires_takeover": requires_takeover,
            "message": format!("电脑请求将平板切换到「{}」。", note.note.title),
        });
        let _ = self
            .ink_lan
            .live
            .control
            .send(Control::SwitchOffer(payload));
        Ok(InkPushResult {
            pushed: true,
            online,
            requires_takeover: false,
            expires_at: Some(expires_at),
            document_id: document.into(),
            title: Some(note.note.title),
            message: format!("已向 {online} 台在线平板发送切换请求。"),
        })
    }

    pub fn set_ink_lan_preferred_host(
        &self,
        host: Option<String>,
    ) -> Result<InkLanSettings, String> {
        let mut settings = store(self)?.ink_lan_settings().map_err(|e| e.to_string())?;
        settings.preferred_host = match host {
            Some(value) if value.trim().is_empty() => None,
            other => other,
        };
        store(self)?
            .save_ink_lan_settings(&settings)
            .map_err(|e| e.to_string())?;
        Ok(settings)
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
    state.service().start_ink_lan(port).await
}
#[tauri::command]
pub(crate) async fn get_ink_lan_status(
    state: TauriState<'_, DesktopState>,
) -> Result<InkLanStatus, String> {
    state.service().ink_lan_status().await
}
#[tauri::command]
pub(crate) async fn stop_ink_lan(state: TauriState<'_, DesktopState>) -> Result<(), String> {
    state.service().shutdown_ink_lan(true).await
}
#[tauri::command]
pub(crate) async fn push_ink_to_tablet(
    state: TauriState<'_, DesktopState>,
    document_id: String,
    allow_transfer: Option<bool>,
) -> Result<InkPushResult, String> {
    state
        .service()
        .push_ink_to_tablet(&document_id, allow_transfer.unwrap_or(false))
        .await
}
#[tauri::command]
pub(crate) fn set_ink_lan_preferred_host(
    state: TauriState<'_, DesktopState>,
    host: Option<String>,
) -> Result<InkLanSettings, String> {
    state.service().set_ink_lan_preferred_host(host)
}
#[tauri::command]
pub(crate) async fn create_ink_pairing(
    state: TauriState<'_, DesktopState>,
    document_id: String,
    expected_head: u64,
    allow_transfer: Option<bool>,
) -> Result<InkPairingLink, String> {
    state
        .service()
        .create_ink_pairing_with(document_id, expected_head, allow_transfer.unwrap_or(false))
        .await
}
#[tauri::command]
pub(crate) async fn cancel_ink_pairing(
    state: TauriState<'_, DesktopState>,
    document_id: String,
) -> Result<(), String> {
    store(&state.service())?
        .cancel_ink_pairing(&document_id, now())
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub(crate) async fn get_ink_lan_mirror(
    state: TauriState<'_, DesktopState>,
    document_id: String,
    after_seq: u64,
) -> Result<InkLanMirror, String> {
    let service = state.service();
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
    let service = state.service();
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
    let (pem, fingerprint) = ink_tls::certificate(state.service().workspace_root())?;
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
) -> Result<Option<InkRecoveryImport>, String> {
    let selected = app
        .dialog()
        .file()
        .add_filter("Ink JSON recovery", &["json"])
        .blocking_pick_file();
    let Some(selected) = selected else {
        return Ok(None);
    };
    let path = selected.into_path().map_err(|error| error.to_string())?;
    let service = state.service();
    tokio::task::spawn_blocking(move || {
        if std::fs::metadata(&path)
            .map_err(|error| error.to_string())?
            .len()
            > 128 * 1024 * 1024
        {
            return Err("恢复文件超过读取上限。".into());
        }
        let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
        service
            .import_ink_recovery_file(&bytes, &request_id)
            .map(Some)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn start_ink_setup(
    state: TauriState<'_, DesktopState>,
    document_id: Option<String>,
) -> Result<InkSetupStatus, String> {
    state.service().start_ink_setup_for(document_id).await
}
#[tauri::command]
pub(crate) async fn prepare_ink_setup_handoff(
    state: TauriState<'_, DesktopState>,
    expected_head: Option<u64>,
    confirm_transfer: bool,
) -> Result<InkSetupStatus, String> {
    state
        .service()
        .prepare_ink_setup_handoff(expected_head, confirm_transfer)
        .await
}
#[tauri::command]
pub(crate) async fn get_ink_setup_status(
    state: TauriState<'_, DesktopState>,
) -> Result<InkSetupStatus, String> {
    Ok(state.service().ink_setup_status())
}
#[tauri::command]
pub(crate) async fn cancel_ink_setup(state: TauriState<'_, DesktopState>) -> Result<(), String> {
    state.service().cancel_ink_setup().await;
    Ok(())
}

/// Result of "导入恢复草稿": one recovery draft per rebuilt writer queue.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct InkRecoveryImport {
    pub source: String,
    pub original_document_id: Option<String>,
    pub sessions: Vec<mathscape_store_sqlite::StoredInkSession>,
}
