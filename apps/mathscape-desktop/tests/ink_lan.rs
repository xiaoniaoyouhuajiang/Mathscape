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
        self.socket_as(client, &"2".repeat(64)).await
    }
    async fn socket_as(&self, client: &str, credential: &str) -> Socket {
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
        socket.send(Message::Text(json!({"type":"hello","protocol_version":INK_LAN_PROTOCOL_VERSION,"credential":credential,"client_id":client}).to_string().into())).await.unwrap();
        socket
    }
    async fn claim(&self, url: &str, credential: &str) -> reqwest::Response {
        let token = url.split("#pair=").nth(1).unwrap_or(url);
        self.client
            .post(format!("{}/api/pair", self.base))
            .header("Origin", &self.base)
            .json(&json!({"pairing_token":token,"resume_secret":credential,"device":{"model":"fixture"}}))
            .send()
            .await
            .unwrap()
    }
    async fn handoff(&self, body: &Value) -> reqwest::Response {
        self.client
            .post(format!("{}/api/setup/handoff", self.base))
            .header("Origin", &self.base)
            .json(body)
            .send()
            .await
            .unwrap()
    }
    async fn complete_setup(&self, token: &str) {
        self.client
            .post(format!("{}/api/setup/check", self.base))
            .header("Origin", &self.base)
            .json(&json!({"token":token,"check":{"secure_context":true,"indexed_db":true,"service_worker":true,"user_agent":"fixture"}}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
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
                allow_transfer: false,
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

#[tokio::test]
async fn setup_guide_only_exposes_the_public_ca_and_reports_trust_progress() {
    let f = Fixture::new("setup").await;
    let setup = f.service.start_ink_setup().await.unwrap();
    let url = setup
        .urls
        .iter()
        .find(|url| url.contains("127.0.0.1"))
        .unwrap();
    let http_base = url.split("/setup/").next().unwrap().to_owned();
    let token = url
        .split("#setup=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap()
        .to_owned();
    let plain = reqwest::Client::builder().no_proxy().build().unwrap();
    assert_eq!(
        plain
            .get(format!("{http_base}/api/setup/open?t={}", "0".repeat(64)))
            .send()
            .await
            .unwrap()
            .status(),
        410
    );
    let rebound = http_base.clone();
    assert_eq!(
        plain
            .get(format!("{rebound}/api/setup/open?t={token}"))
            .header("Host", "evil.example")
            .send()
            .await
            .unwrap()
            .status(),
        403,
        "DNS-rebound hosts must not reach the setup helper"
    );
    let opened: Value = plain
        .get(format!("{http_base}/api/setup/open?t={token}"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(opened["https_port"], f.port);
    let der = plain
        .get(format!("{http_base}/setup/ca.crt?t={token}"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let pem = fs::read(f.root.join("ink-lan/trust/ca.crt")).unwrap();
    assert_eq!(
        der.as_ref(),
        CertificateDer::from_pem_slice(&pem).unwrap().as_ref()
    );
    assert!(!String::from_utf8_lossy(&der).contains("PRIVATE KEY"));
    assert!(
        plain
            .get(format!("{http_base}/pad/"))
            .send()
            .await
            .unwrap()
            .status()
            .is_client_error(),
        "the HTTP helper must not serve the writing pad"
    );
    assert_eq!(
        plain
            .post(format!("{http_base}/api/pair"))
            .json(&json!({}))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    let status = f.service.ink_setup_status();
    assert!(status.opened && status.certificate_downloaded && !status.https_trusted);
    let setup_origin = http_base.clone();
    assert_eq!(
        f.client
            .get(format!("{}/api/setup/probe?t={token}", f.base))
            .header("Origin", "http://evil.example")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let probe = f
        .client
        .get(format!("{}/api/setup/probe?t={token}", f.base))
        .header("Origin", &setup_origin)
        .send()
        .await
        .unwrap();
    assert_eq!(probe.status(), 200);
    assert_eq!(
        probe.headers()["access-control-allow-origin"],
        setup_origin.as_str()
    );
    assert!(f.service.ink_setup_status().https_trusted);
    let check = json!({"token":token,"check":{"secure_context":true,"indexed_db":true,"service_worker":true,"user_agent":"fixture"}});
    assert_eq!(
        f.client
            .post(format!("{}/api/setup/check", f.base))
            .json(&check)
            .send()
            .await
            .unwrap()
            .status(),
        403,
        "the check report must come from the HTTPS setup page"
    );
    f.client
        .post(format!("{}/api/setup/check", f.base))
        .header("Origin", &f.base)
        .json(&check)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    assert!(f.service.ink_setup_status().complete);
    f.service.cancel_ink_setup().await;
    assert!(!f.service.ink_setup_status().active);
    assert!(
        plain
            .get(format!("{http_base}/setup/ca.crt?t={token}"))
            .send()
            .await
            .is_err(),
        "ending setup closes the HTTP helper"
    );
    assert_eq!(
        f.client
            .get(format!("{}/api/setup/probe?t={token}", f.base))
            .header("Origin", &setup_origin)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    f.cleanup().await;
}

#[tokio::test]
async fn transfer_offer_keeps_the_tablet_writing_until_a_new_device_claims_it() {
    let f = Fixture::new("transfer").await;
    let lease = f.pair().await;
    let mut old = f.socket("old-tablet").await;
    assert_eq!(next(&mut old).await["type"], "session_context");
    let document = f.document.document_id.clone();
    let Err(refused) = f.service.create_ink_pairing(document.clone(), 0).await else {
        panic!("pairing without a declared transfer must be refused");
    };
    assert!(refused.contains("takeover_required"), "{refused}");
    let offer = f
        .service
        .create_ink_pairing_with(document.clone(), 0, true)
        .await
        .unwrap();
    let view = f.service.ink_lan_mirror(&document, 0).unwrap().delta.writer;
    assert!(view.remote && view.pairing_pending);
    // The holder keeps syncing after the offer is issued; its ops move with the lease.
    let request = InkLanOperation {
        protocol_version: INK_LAN_PROTOCOL_VERSION.into(),
        request_id: "before-transfer".into(),
        document_id: document.clone(),
        session_id: lease["session_id"].as_str().unwrap().into(),
        writer_epoch: lease["writer_epoch"].as_u64().unwrap(),
        operation_id: f.operation.command_id().into(),
        client_seq: 1,
        previous_operation_id: None,
        metadata: f.document.metadata(),
        operation: f.operation.clone(),
    };
    old.send(Message::Text(
        json!({"type":"operation_submit","request":request})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    assert_eq!(next(&mut old).await["type"], "operation_ack");
    let claimed = f.claim(&offer.urls[0], &"4".repeat(64)).await;
    assert_eq!(claimed.status(), 200);
    let new_lease = claimed.json::<Value>().await.unwrap()["lease"].clone();
    assert!(new_lease["writer_epoch"].as_u64() > lease["writer_epoch"].as_u64());
    let closed = next(&mut old).await;
    assert_eq!(closed["type"], "session_closed");
    assert_eq!(closed["code"], "writer_revoked");
    let mut fresh = f.socket_as("new-tablet", &"4".repeat(64)).await;
    let context = next(&mut fresh).await;
    assert_eq!(context["type"], "session_context");
    assert_eq!(context["head_seq"], 1);
    let mut stale = f.socket("old-tablet").await;
    assert_eq!(next(&mut stale).await["code"], "writer_revoked");
    // A transfer offer dies when the desktop reclaims first.
    let again = f
        .service
        .create_ink_pairing_with(document.clone(), 0, true)
        .await
        .unwrap();
    let view = f.service.ink_lan_mirror(&document, 0).unwrap().delta.writer;
    f.service
        .reclaim_ink_writer(&document, view.writer_epoch, view.head_seq, "reclaim-first")
        .unwrap();
    assert_eq!(f.claim(&again.urls[0], &"6".repeat(64)).await.status(), 409);
    drop((old, fresh, stale));
    f.cleanup().await;
}

#[tokio::test]
async fn setup_handoff_needs_the_fragment_grant_and_then_pairs_or_resumes() {
    let f = Fixture::new("handoff").await;
    let document = f.document.document_id.clone();
    let setup = f
        .service
        .start_ink_setup_for(Some(document.clone()))
        .await
        .unwrap();
    let url = setup.urls.iter().find(|url| url.contains("127.0.0.1")).unwrap();
    let fragment = url.split('#').nth(1).unwrap();
    let token = fragment.split("setup=").nth(1).unwrap().split('&').next().unwrap().to_owned();
    let grant = fragment.split("grant=").nth(1).unwrap().to_owned();
    assert_eq!(f.service.ink_setup_status().handoff, "idle");
    let body = json!({"token":token,"grant":grant,"credentials":[]});
    assert_eq!(f.handoff(&body).await.status(), 403, "check must finish first");
    f.complete_setup(&token).await;
    let without_origin = f
        .client
        .post(format!("{}/api/setup/handoff", f.base))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(without_origin.status(), 403);
    let sniffed = json!({"token":token,"grant":token,"credentials":[]});
    assert_eq!(
        f.handoff(&sniffed).await.status(),
        403,
        "the HTTP setup token alone cannot obtain writing access"
    );
    let waiting: Value = f.handoff(&body).await.json().await.unwrap();
    assert_eq!(waiting["state"], "waiting");
    assert_eq!(waiting["reason"], "desktop");
    assert_eq!(f.service.ink_setup_status().handoff, "requested");
    let ready = f
        .service
        .prepare_ink_setup_handoff(Some(0), false)
        .await
        .unwrap();
    assert_eq!(ready.handoff, "ready");
    assert!(
        f.service
            .ink_lan_mirror(&document, 0)
            .unwrap()
            .delta
            .writer
            .pairing_pending,
        "the desktop becomes a read-only mirror only after the tablet asked"
    );
    let pair: Value = f.handoff(&body).await.json().await.unwrap();
    assert_eq!(pair["state"], "pair");
    let claimed = f
        .claim(pair["pairing_token"].as_str().unwrap(), &"5".repeat(64))
        .await;
    assert_eq!(claimed.status(), 200);
    assert_eq!(f.service.ink_setup_status().handoff, "claimed");
    f.service.cancel_ink_setup().await;

    // A second setup on the holder resumes; another device must be confirmed.
    let setup = f
        .service
        .start_ink_setup_for(Some(document.clone()))
        .await
        .unwrap();
    let url = setup.urls.iter().find(|url| url.contains("127.0.0.1")).unwrap();
    let fragment = url.split('#').nth(1).unwrap();
    let token = fragment.split("setup=").nth(1).unwrap().split('&').next().unwrap().to_owned();
    let grant = fragment.split("grant=").nth(1).unwrap().to_owned();
    f.complete_setup(&token).await;
    let other = json!({"token":token,"grant":grant,"credentials":["7".repeat(64)]});
    assert_eq!(f.handoff(&other).await.json::<Value>().await.unwrap()["state"], "waiting");
    let pending = f.service.prepare_ink_setup_handoff(None, false).await.unwrap();
    assert!(pending.transfer_confirmation);
    assert_eq!(pending.handoff, "requested");
    assert_eq!(f.handoff(&other).await.json::<Value>().await.unwrap()["reason"], "confirm");
    let holder = json!({"token":token,"grant":grant,"credentials":["5".repeat(64)]});
    let resume: Value = f.handoff(&holder).await.json().await.unwrap();
    assert_eq!(resume["state"], "resume");
    assert_eq!(resume["document_id"], document.as_str());
    assert_eq!(f.service.ink_setup_status().handoff, "resumed");
    let mut socket = f.socket_as("holder", &"5".repeat(64)).await;
    assert_eq!(next(&mut socket).await["type"], "session_context");
    drop(socket);
    f.cleanup().await;
}

#[tokio::test]
async fn ending_setup_before_the_tablet_claims_returns_writing_to_the_desktop() {
    let f = Fixture::new("handoff-cancel").await;
    let document = f.document.document_id.clone();
    let setup = f.service.start_ink_setup_for(Some(document.clone())).await.unwrap();
    let url = setup.urls.iter().find(|url| url.contains("127.0.0.1")).unwrap();
    let fragment = url.split('#').nth(1).unwrap();
    let token = fragment.split("setup=").nth(1).unwrap().split('&').next().unwrap().to_owned();
    let grant = fragment.split("grant=").nth(1).unwrap().to_owned();
    f.complete_setup(&token).await;
    let body = json!({"token":token,"grant":grant,"credentials":[]});
    assert_eq!(f.handoff(&body).await.json::<Value>().await.unwrap()["state"], "waiting");
    assert_eq!(f.service.prepare_ink_setup_handoff(Some(0), false).await.unwrap().handoff, "ready");
    let pairing = f.handoff(&body).await.json::<Value>().await.unwrap()["pairing_token"].clone();
    f.service.cancel_ink_setup().await;
    assert!(!f.service.ink_lan_mirror(&document, 0).unwrap().delta.writer.pairing_pending);
    assert_eq!(f.claim(pairing.as_str().unwrap(), &"8".repeat(64)).await.status(), 409);
    f.cleanup().await;
}
