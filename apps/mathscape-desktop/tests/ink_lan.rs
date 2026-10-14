use futures::{SinkExt, StreamExt};
use mathscape_desktop::{CreateInkDocumentRequest, DesktopService};
use mathscape_domain::{
    INK_LAN_PROTOCOL_VERSION, InkLabDocument, InkLabOperation, InkLanOperation,
};
use mathscape_llm::MemoryCredentialResolver;
use mathscape_store_sqlite::{InkPairingOffer, SqliteArtifactStore};
use rustls::pki_types::{CertificateDer, pem::PemObject};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio_tungstenite::{
    Connector, MaybeTlsStream, WebSocketStream, connect_async_tls_with_config,
    tungstenite::{Message, client::IntoClientRequest},
};
type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

struct Fixture {
    root: PathBuf,
    service: DesktopService,
    base: String,
    port: u16,
    document: InkLabDocument,
    operation: InkLabOperation,
    client: reqwest::Client,
    tls: Arc<rustls::ClientConfig>,
}
impl Fixture {
    async fn new(name: &str) -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".test-runs")
            .join(format!(
                "lan-{name}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        fs::create_dir_all(&root).unwrap();
        let service = DesktopService::open_with_credential_store(
            &root,
            Arc::new(MemoryCredentialResolver::default()),
        )
        .unwrap();
        let mut document: InkLabDocument =
            serde_json::from_str(include_str!("../../../testdata/ink/p0/trace.json")).unwrap();
        let operation = document.operations[0].clone();
        document.operations.clear();
        service
            .create_ink_document(CreateInkDocumentRequest {
                document: document.clone(),
                session_id: "native-session".into(),
                writer_id: "desktop-local".into(),
                device: json!({"source":"desktop"}),
            })
            .unwrap();
        let status = service.start_ink_lan(Some(0)).await.unwrap();
        let base = format!("https://127.0.0.1:{}", status.port);
        let pem = fs::read(root.join("ink-lan/trust/ca.crt")).unwrap();
        let client = reqwest::Client::builder()
            .tls_certs_only([reqwest::Certificate::from_pem(&pem).unwrap()])
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(CertificateDer::from_pem_slice(&pem).unwrap())
            .unwrap();
        let tls = Arc::new(
            rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        );
        Self {
            root,
            service,
            base,
            port: status.port,
            document,
            operation,
            client,
            tls,
        }
    }
    async fn pair(&self) -> Value {
        let pairing = self
            .service
            .create_ink_pairing(self.document.document_id.clone(), 0)
            .await
            .unwrap();
        let token = pairing.urls[0].split("#pair=").nth(1).unwrap();
        self.client.post(format!("{}/api/pair",self.base)).header("Origin",&self.base).json(&json!({"pairing_token":token,"resume_secret":"2".repeat(64),"device":{"model":"fixture"}})).send().await.unwrap().error_for_status().unwrap().json::<Value>().await.unwrap()["lease"].clone()
    }
    async fn socket(&self, client: &str) -> Socket {
        let mut request = format!("wss://127.0.0.1:{}/api/ws", self.port)
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("origin", self.base.parse().unwrap());
        let (mut socket, _) = connect_async_tls_with_config(
            request,
            None,
            false,
            Some(Connector::Rustls(self.tls.clone())),
        )
        .await
        .unwrap();
        socket.send(Message::Text(json!({"type":"hello","protocol_version":INK_LAN_PROTOCOL_VERSION,"credential":"2".repeat(64),"client_id":client}).to_string().into())).await.unwrap();
        socket
    }
    async fn cleanup(self) {
        self.service.shutdown_ink_lan(false).await.unwrap();
        fs::remove_dir_all(self.root).unwrap();
    }
}
async fn next(socket: &mut Socket) -> Value {
    loop {
        match tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
        {
            Message::Text(value) => return serde_json::from_str(&value).unwrap(),
            Message::Ping(value) => {
                socket.send(Message::Pong(value)).await.unwrap();
            }
            other => panic!("unexpected socket message: {other:?}"),
        }
    }
}

#[tokio::test]
async fn https_pairing_ws_durable_ack_replay_and_snapshot_share_the_desktop_store() {
    let f = Fixture::new("durable").await;
    assert!(
        reqwest::Client::new()
            .get(format!("{}/health", f.base))
            .send()
            .await
            .is_err()
    ); // Unknown CA is not trusted.
    assert_eq!(
        f.client
            .get(format!("{}/health", f.base))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        f.client
            .get(format!("{}/mathscape.sqlite3", f.base))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    let lease = f.pair().await;
    let mut socket = f.socket("client-one").await;
    let context = next(&mut socket).await;
    assert_eq!(context["type"], "session_context");
    assert_eq!(context["head_seq"], 0);
    let request = InkLanOperation {
        protocol_version: INK_LAN_PROTOCOL_VERSION.into(),
        request_id: "durable-request".into(),
        document_id: f.document.document_id.clone(),
        session_id: lease["session_id"].as_str().unwrap().into(),
        writer_epoch: lease["writer_epoch"].as_u64().unwrap(),
        operation_id: f.operation.command_id().into(),
        client_seq: 1,
        previous_operation_id: None,
        metadata: f.document.metadata(),
        operation: f.operation.clone(),
    };
    socket
        .send(Message::Text(
            json!({"type":"operation_submit","request":request})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let ack = next(&mut socket).await;
    assert_eq!(ack["type"], "operation_ack");
    assert_eq!(ack["receipt"]["durable"], true);
    socket
        .send(Message::Text(
            json!({"type":"operation_submit","request":request})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let replay = next(&mut socket).await;
    assert_eq!(replay["receipt"]["replayed"], true);
    assert_eq!(
        f.service
            .open_ink_document(f.document.document_id.clone())
            .unwrap()
            .unwrap()
            .head_seq,
        1
    );
    socket
        .send(Message::Text(
            json!({"type":"snapshot_request","request_id":"remote-snapshot","expected_head":1})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let saved = next(&mut socket).await;
    assert_eq!(saved["type"], "snapshot_receipt");
    assert_eq!(saved["through_seq"], 1);
    assert_eq!(
        f.service
            .list_ink_snapshots(f.document.document_id.clone())
            .unwrap()
            .len(),
        1
    );
    socket.close(None).await.unwrap();
    f.cleanup().await;
}

#[tokio::test]
async fn pairing_is_single_use_origin_checked_and_scoped_to_one_document() {
    let f = Fixture::new("scope").await;
    let pairing = f
        .service
        .create_ink_pairing(f.document.document_id.clone(), 0)
        .await
        .unwrap();
    let token = pairing.urls[0].split("#pair=").nth(1).unwrap();
    let body = json!({"pairing_token":token,"resume_secret":"2".repeat(64),"device":{}});
    assert_eq!(
        f.client
            .post(format!("{}/api/pair", f.base))
            .header("Origin", "https://unrelated.example")
            .json(&body)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        f.client
            .post(format!("{}/api/pair", f.base))
            .json(&body)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let lease = f
        .client
        .post(format!("{}/api/pair", f.base))
        .header("Origin", &f.base)
        .json(&body)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["lease"]
        .clone();
    let retry = f
        .client
        .post(format!("{}/api/pair", f.base))
        .header("Origin", &f.base)
        .json(&body)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(retry["lease"], lease);
    let mut changed = body;
    changed["resume_secret"] = "3".repeat(64).into();
    assert_eq!(
        f.client
            .post(format!("{}/api/pair", f.base))
            .header("Origin", &f.base)
            .json(&changed)
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    let mut socket = f.socket("client-one").await;
    assert_eq!(next(&mut socket).await["type"], "session_context");
    let mut second = f.socket("other-tab").await;
    assert_eq!(next(&mut second).await["code"], "device_busy");
    let mut other = f.document.clone();
    other.document_id = "other-document".into();
    f.service
        .create_ink_document(CreateInkDocumentRequest {
            document: other.clone(),
            session_id: "other-local".into(),
            writer_id: "desktop-local".into(),
            device: json!({}),
        })
        .unwrap();
    let request = InkLanOperation {
        protocol_version: INK_LAN_PROTOCOL_VERSION.into(),
        request_id: "scope-request".into(),
        document_id: other.document_id.clone(),
        session_id: lease["session_id"].as_str().unwrap().into(),
        writer_epoch: lease["writer_epoch"].as_u64().unwrap(),
        operation_id: f.operation.command_id().into(),
        client_seq: 1,
        previous_operation_id: None,
        metadata: other.metadata(),
        operation: f.operation.clone(),
    };
    socket
        .send(Message::Text(
            json!({"type":"operation_submit","request":request})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    assert_eq!(next(&mut socket).await["type"], "operation_conflict");
    assert_eq!(
        f.service
            .open_ink_document(other.document_id)
            .unwrap()
            .unwrap()
            .head_seq,
        0
    );
    socket.close(None).await.unwrap();
    f.cleanup().await;
}

#[tokio::test]
async fn desktop_restart_preserves_resume_authentication_but_takeover_revokes_it() {
    let mut f = Fixture::new("resume").await;
    let lease = f.pair().await;
    let mut first = f.socket("same-client").await;
    assert_eq!(next(&mut first).await["type"], "session_context");
    f.service.shutdown_ink_lan(false).await.unwrap();
    drop(first);
    f.service = DesktopService::open_with_credential_store(
        &f.root,
        Arc::new(MemoryCredentialResolver::default()),
    )
    .unwrap();
    f.service.start_ink_lan(Some(f.port)).await.unwrap();
    let mut resumed = f.socket("same-client").await;
    let context = next(&mut resumed).await;
    assert_eq!(context["type"], "session_context");
    assert_eq!(context["lease"]["writer_epoch"], lease["writer_epoch"]);
    let claimed = f
        .service
        .reclaim_ink_writer(
            &f.document.document_id,
            lease["writer_epoch"].as_u64().unwrap(),
            0,
            "take-back",
        )
        .unwrap();
    assert!(claimed.writer_epoch > lease["writer_epoch"].as_u64().unwrap());
    assert_eq!(next(&mut resumed).await["code"], "writer_revoked");
    drop(resumed);
    let mut denied = f.socket("same-client").await;
    assert_eq!(next(&mut denied).await["code"], "writer_revoked");
    drop(denied);
    f.cleanup().await;
}

#[tokio::test]
async fn expiry_predecessor_and_conflicting_replay_never_change_committed_history() {
    let f = Fixture::new("validation").await;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let mut database = SqliteArtifactStore::open(&f.root.join("mathscape.sqlite3")).unwrap();
    database
        .issue_ink_pairing(
            &InkPairingOffer {
                token_hash: "a".repeat(64),
                document_id: f.document.document_id.clone(),
                expected_head: 0,
                expected_epoch: 1,
                session_id: "expired-session".into(),
                writer_id: "expired-peer".into(),
                expires_at: now - 1,
            },
            now - 120,
        )
        .unwrap();
    assert!(
        database
            .claim_ink_pairing(&"a".repeat(64), &"b".repeat(64), &json!({}), now)
            .unwrap_err()
            .to_string()
            .contains("pairing_expired")
    );
    let lease = f.pair().await;
    let mut socket = f.socket("validation-client").await;
    next(&mut socket).await;
    let request = InkLanOperation {
        protocol_version: INK_LAN_PROTOCOL_VERSION.into(),
        request_id: "valid-1".into(),
        document_id: f.document.document_id.clone(),
        session_id: lease["session_id"].as_str().unwrap().into(),
        writer_epoch: lease["writer_epoch"].as_u64().unwrap(),
        operation_id: f.operation.command_id().into(),
        client_seq: 1,
        previous_operation_id: None,
        metadata: f.document.metadata(),
        operation: f.operation.clone(),
    };
    let mut missing = request.clone();
    missing.previous_operation_id = Some("unknown".into());
    socket
        .send(Message::Text(
            json!({"type":"operation_submit","request":missing})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    assert_eq!(next(&mut socket).await["code"], "sequence_conflict");
    assert_eq!(
        f.service
            .open_ink_document(f.document.document_id.clone())
            .unwrap()
            .unwrap()
            .head_seq,
        0
    );
    socket
        .send(Message::Text(
            json!({"type":"operation_submit","request":request})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    assert_eq!(next(&mut socket).await["type"], "operation_ack");
    let mut altered = request;
    if let InkLabOperation::Stroke { stroke, .. } = &mut altered.operation {
        stroke.points[0].x += 1.0;
    }
    socket
        .send(Message::Text(
            json!({"type":"operation_submit","request":altered})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    assert_eq!(next(&mut socket).await["code"], "idempotency_conflict");
    let stored = f
        .service
        .open_ink_document(f.document.document_id.clone())
        .unwrap()
        .unwrap();
    assert_eq!(stored.document.operations[0], f.operation);
    let mirror = f
        .service
        .ink_lan_mirror(&f.document.document_id, 0)
        .unwrap();
    assert_eq!(mirror.delta.operations.len(), 1);
    assert_eq!(mirror.delta.server_head, 1);
    socket.close(None).await.unwrap();
    f.cleanup().await;
}

#[tokio::test]
async fn explicit_stop_fences_old_authentication_and_recovery_import_is_an_independent_draft() {
    let f = Fixture::new("recovery").await;
    f.pair().await;
    let digest = format!("{:x}", Sha256::digest("2".repeat(64).as_bytes()));
    f.service.shutdown_ink_lan(true).await.unwrap();
    let database = SqliteArtifactStore::open(&f.root.join("mathscape.sqlite3")).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    assert!(database.ink_remote_context(&digest, now).is_err());
    let mut recovery = f.document.clone();
    recovery.operations.push(f.operation.clone());
    let request = "a".repeat(32);
    let first = f
        .service
        .import_ink_recovery(recovery.clone(), &request)
        .unwrap();
    let retry = f.service.import_ink_recovery(recovery, &request).unwrap();
    assert_eq!(first.session_id, retry.session_id);
    assert_ne!(first.document_id, f.document.document_id);
    assert_eq!(
        f.service
            .open_ink_document(first.document_id)
            .unwrap()
            .unwrap()
            .head_seq,
        1
    );
    assert_eq!(
        f.service
            .open_ink_document(f.document.document_id.clone())
            .unwrap()
            .unwrap()
            .head_seq,
        0
    );
    f.cleanup().await;
}
