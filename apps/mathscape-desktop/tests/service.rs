use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use lopdf::{Document, Object, Stream, content::Content, content::Operation, dictionary};
use mathscape_artifact::{ArtifactMetadata, ArtifactPool};
use mathscape_desktop::{
    AcceptDiscoveryRequest, AppendInkOperationRequest, CreateInkDocumentRequest,
    CreateInkSnapshotRequest, DesktopDiscoveryError, DesktopService, DesktopServiceError,
    MarkerProfileChoice, ParsePdfRequest, PdfImportOutcome, ResumeDiscoveryRequest,
    RunDiscoveryRequest, SaveWorkspaceSettingsRequest,
};
use mathscape_domain::{
    ActorRef, DISCOVERY_SELECTION_VERSION, DiscoveryAdmission, DiscoverySelectionRequest,
    InkLabDocument, InkSnapshotManifest, MediaType, RightsRecord, RightsStatus, SourceSelector,
    StableId,
};
use mathscape_llm::{CredentialRef, CredentialResolver, MemoryCredentialResolver};
use mathscape_store_sqlite::{DiscoveryCheckpointStatus, SqliteArtifactStore};
use wiremock::{
    Mock, MockServer, Request, Respond, ResponseTemplate,
    matchers::{header, method, path},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

#[test]
fn ink_operations_survive_desktop_reopen_and_duplicate_submission() {
    let directory = TestDirectory::create("ink-p1-service");
    let credentials = Arc::new(MemoryCredentialResolver::default());
    let workspace = directory.path().join("workspace");
    let service =
        DesktopService::open_with_credential_store(&workspace, credentials.clone()).unwrap();
    let mut document: InkLabDocument =
        serde_json::from_str(include_str!("../../../testdata/ink/p0/trace.json")).unwrap();
    let operation = document.operations[0].clone();
    document.operations.clear();
    let session = service
        .create_ink_document(CreateInkDocumentRequest {
            document: document.clone(),
            session_id: "session-p1".into(),
            writer_id: "tablet-p1".into(),
            device: serde_json::json!({"browser":"Chrome","model":"Tab S8+"}),
        })
        .unwrap();
    let request = AppendInkOperationRequest {
        request_id: "request-p1".into(),
        document_id: document.document_id.clone(),
        session_id: session.session_id.clone(),
        writer_epoch: session.writer_epoch,
        expected_head: 0,
        metadata: document.metadata(),
        operation,
    };
    let first = service.append_ink_operation(request.clone()).unwrap();
    assert!(first.durable);
    assert!(!first.replayed);
    let replay = service.append_ink_operation(request).unwrap();
    assert!(replay.replayed);

    let reopened = DesktopService::open_with_credential_store(&workspace, credentials).unwrap();
    let stored = reopened
        .open_ink_document(document.document_id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.head_seq, 1);
    assert_eq!(stored.document.operations.len(), 1);
    directory.cleanup();
}

#[test]
fn ink_snapshot_keeps_a_fixed_operation_prefix_after_later_editing() {
    let directory = TestDirectory::create("ink-p2-snapshot");
    let credentials = Arc::new(MemoryCredentialResolver::default());
    let workspace = directory.path().join("workspace");
    let service = DesktopService::open_with_credential_store(&workspace, credentials).unwrap();
    let mut document: InkLabDocument =
        serde_json::from_str(include_str!("../../../testdata/ink/p0/trace.json")).unwrap();
    let first_operation = document.operations[0].clone();
    let second_operation = document.operations[1].clone();
    document.operations.clear();
    let session = service
        .create_ink_document(CreateInkDocumentRequest {
            document: document.clone(),
            session_id: "session-p2".into(),
            writer_id: "desktop-local".into(),
            device: serde_json::json!({"source":"desktop"}),
        })
        .unwrap();
    let first_request = AppendInkOperationRequest {
        request_id: "request-p2-1".into(),
        document_id: document.document_id.clone(),
        session_id: session.session_id.clone(),
        writer_epoch: session.writer_epoch,
        expected_head: 0,
        metadata: document.metadata(),
        operation: first_operation,
    };
    service.append_ink_operation(first_request).unwrap();
    let snapshot = service
        .create_ink_snapshot(CreateInkSnapshotRequest {
            request_id: "snapshot-request-1".into(),
            document_id: document.document_id.clone(),
            expected_head: 1,
        })
        .unwrap();
    let second_request = AppendInkOperationRequest {
        request_id: "request-p2-2".into(),
        document_id: document.document_id.clone(),
        session_id: session.session_id,
        writer_epoch: session.writer_epoch,
        expected_head: 1,
        metadata: document.metadata(),
        operation: second_operation,
    };
    service.append_ink_operation(second_request).unwrap();
    let mut other_document = document.clone();
    other_document.document_id = "fixture-document-other".into();
    service
        .create_ink_document(CreateInkDocumentRequest {
            document: other_document.clone(),
            session_id: "session-p2-other".into(),
            writer_id: "desktop-local".into(),
            device: serde_json::json!({"source":"desktop"}),
        })
        .unwrap();
    let catalog = SqliteArtifactStore::open(&workspace.join("mathscape.sqlite3")).unwrap();
    assert_eq!(
        catalog
            .list_ink_snapshots(&document.document_id)
            .unwrap()
            .len(),
        1
    );
    assert!(
        catalog
            .list_ink_snapshots(&other_document.document_id)
            .unwrap()
            .is_empty()
    );
    let stored = catalog
        .open_ink_snapshot(&snapshot.snapshot_id)
        .unwrap()
        .unwrap();
    let manifest: InkSnapshotManifest = serde_json::from_str(&stored.manifest_json).unwrap();
    assert_eq!(manifest.through_document_seq, 1);
    assert_eq!(manifest.document.operations.len(), 1);
    assert_eq!(
        catalog
            .open_ink_document(&document.document_id)
            .unwrap()
            .unwrap()
            .head_seq,
        2
    );
    let retry = service
        .create_ink_snapshot(CreateInkSnapshotRequest {
            request_id: "snapshot-request-1".into(),
            document_id: document.document_id,
            expected_head: 1,
        })
        .unwrap();
    assert_eq!(retry.snapshot_id, snapshot.snapshot_id);
    let snapshot_artifact = StableId::parse(&snapshot.artifact_id).unwrap();
    let pool = ArtifactPool::open(&workspace).unwrap();
    let artifact_path = pool.managed_path(&snapshot_artifact).unwrap();
    #[cfg(unix)]
    fs::set_permissions(&artifact_path, fs::Permissions::from_mode(0o644)).unwrap();
    fs::write(&artifact_path, b"corrupted snapshot").unwrap();
    let error = service.open_ink_snapshot(snapshot.snapshot_id).unwrap_err();
    assert!(error.contains("完整性校验未通过"));
    directory.cleanup();
}

#[derive(Clone, Debug)]
struct StagedDiscoveryResponder;

impl Respond for StagedDiscoveryResponder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        let schema = body["response_format"]["json_schema"]["name"]
            .as_str()
            .unwrap();
        let content = body["messages"].as_array().unwrap().last().unwrap()["content"]
            .as_str()
            .unwrap();
        let input: serde_json::Value = serde_json::from_str(content).unwrap();
        let proposal = if schema == "unit_discovery_v1" {
            serde_json::json!({
                "version": "mathscape-unit-discovery-1",
                "request_ref": input["request_ref"],
                "units": [
                    {"ref":"u1","unit_kind":"problem","display_title":"常数列收敛习题","selectors":[{"kind":"block_range","first":"b1","last":"b1"}],"parent":null,"completeness":"complete"},
                    {"ref":"u2","unit_kind":"solution","display_title":"常数列收敛解答","selectors":[{"kind":"block_range","first":"b2","last":"b2"}],"parent":"u1","completeness":"complete"}
                ],
                "issues": [],
                "coverage": [{"selector":{"kind":"block_range","first":"b1","last":"b2"},"disposition":{"status":"processed"}}]
            })
        } else {
            assert_eq!(schema, "source_facts_v1");
            let tasks = input["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|task| {
                    let block = task["evidence_block_refs"][0].clone();
                    let labels = (task["owner_unit_ref"] == "u1")
                        .then(|| serde_json::json!({"anchor":{"kind":"text_occurrence","block":block.clone(),"text":"习题 1","occurrence":null}}))
                        .into_iter()
                        .collect::<Vec<_>>();
                    let references = (task["owner_unit_ref"] == "u1")
                        .then(|| serde_json::json!({"anchor":{"kind":"text_occurrence","block":block.clone(),"text":"定义","occurrence":null},"target_hint":null}))
                        .into_iter()
                        .collect::<Vec<_>>();
                    let issues = (task["owner_unit_ref"] == "u1")
                        .then(|| serde_json::json!({
                            "kind":"missing_context",
                            "evidence":[{"kind":"block_range","first":block.clone(),"last":block.clone()}],
                            "observation":"引用的定义不在当前选区中。",
                            "context_request":{"kind":"local_reference","target_label":"定义"},
                            "requires_unit_revision":false
                        }))
                        .into_iter()
                        .collect::<Vec<_>>();
                    let relation = (task["owner_unit_ref"] == "u2").then(|| {
                        serde_json::json!({
                            "relation":"solves","target":"u1",
                            "evidence":[{"kind":"block_range","first":block.clone(),"last":block}],
                            "basis":"原文给出该习题的解答。"
                        })
                    });
                    serde_json::json!({
                        "task_ref": task["task_ref"],
                        "disposition":{"status":"processed"},
                        "source_labels":labels,"source_title":null,"references":references,
                        "relations":relation.into_iter().collect::<Vec<_>>(),"issues":issues
                    })
                })
                .collect::<Vec<_>>();
            serde_json::json!({
                "version":"mathscape-source-facts-1",
                "request_ref":input["request_ref"],
                "tasks":tasks
            })
        };
        let chunk = serde_json::json!({
            "id":"chatcmpl-staged-discovery-fixture",
            "object":"chat.completion.chunk","created":1,"model":"resolved-discovery-model",
            "choices":[{"index":0,"delta":{"role":"assistant","content":proposal.to_string()},"finish_reason":"stop"}]
        });
        ResponseTemplate::new(200)
            .set_delay(std::time::Duration::from_millis(120))
            .insert_header("content-type", "text/event-stream")
            .set_body_string(format!("data: {chunk}\n\ndata: [DONE]\n\n"))
    }
}

#[test]
fn named_profiles_survive_reopen_and_default_switch_does_not_edit_credentials() {
    let directory = TestDirectory::create("named-profiles");
    let credentials = Arc::new(MemoryCredentialResolver::default());
    let service =
        DesktopService::open_with_credential_store(directory.path(), credentials.clone()).unwrap();
    for (id, name) in [("primary", "主配置"), ("secondary", "备用配置")] {
        service.save_llm_profile(serde_json::from_value(serde_json::json!({
            "profile_id": id, "display_name": name, "provider": "custom-relay",
            "wire_protocol": "open_ai_chat_completions", "model": id,
            "endpoint": "https://relay.example/v1/", "api_key": format!("fixture-{id}"), "make_default": false
        })).unwrap()).unwrap();
    }
    let initial = serde_json::to_value(service.llm_settings().unwrap()).unwrap();
    assert_eq!(initial["default_profile_id"], "primary");
    service.set_default_llm_profile("secondary").unwrap();
    assert!(service.set_default_llm_profile("missing").is_err());
    let reopened =
        DesktopService::open_with_credential_store(directory.path(), credentials).unwrap();
    let saved = serde_json::to_value(reopened.llm_settings().unwrap()).unwrap();
    assert_eq!(saved["default_profile_id"], "secondary");
    assert_eq!(saved["profiles"].as_array().unwrap().len(), 2);
    assert!(
        saved["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["has_credential"] == true && p["display_name"].is_string())
    );
    directory.cleanup();
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn create(case: &str) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".test-runs")
            .join(format!(
                "{case}-{timestamp}-{}-{sequence}",
                std::process::id()
            ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn cleanup(self) {
        let parent = self.0.parent().unwrap().to_owned();
        fs::remove_dir_all(self.0).unwrap();
        let _ = fs::remove_dir(parent);
    }
}

#[test]
fn persists_validated_workspace_runtime_settings_and_supplies_safe_defaults() {
    let directory = TestDirectory::create("workspace-settings");
    let workspace = directory.path().join("workspace");
    let credentials = Arc::new(MemoryCredentialResolver::default());
    let service =
        DesktopService::open_with_credential_store(&workspace, credentials.clone()).unwrap();
    let defaults = serde_json::to_value(service.workspace_settings().unwrap()).unwrap();
    assert_eq!(defaults["discovery_admission"], "review_before_publish");
    assert_eq!(defaults["discovery_timeout_seconds"], 600);
    assert_eq!(defaults["connect_timeout_seconds"], 15);

    let request: SaveWorkspaceSettingsRequest = serde_json::from_value(serde_json::json!({
        "discovery_admission": "ask_each_run",
        "discovery_timeout_seconds": 900,
        "connect_timeout_seconds": 20
    }))
    .unwrap();
    service.save_workspace_settings(request).unwrap();
    let reopened = DesktopService::open_with_credential_store(&workspace, credentials).unwrap();
    let settings = serde_json::to_value(reopened.workspace_settings().unwrap()).unwrap();
    assert_eq!(settings["version"], "mathscape-workspace-settings-1");
    assert_eq!(settings["discovery_admission"], "ask_each_run");
    assert_eq!(settings["discovery_timeout_seconds"], 900);
    assert_eq!(settings["connect_timeout_seconds"], 20);

    let invalid: SaveWorkspaceSettingsRequest = serde_json::from_value(serde_json::json!({
        "discovery_admission": "review_before_publish",
        "discovery_timeout_seconds": 29,
        "connect_timeout_seconds": 15
    }))
    .unwrap();
    assert!(reopened.save_workspace_settings(invalid).is_err());
    directory.cleanup();
}

#[test]
fn persists_llm_profile_while_keeping_api_key_out_of_sqlite() {
    let directory = TestDirectory::create("llm-settings");
    let workspace = directory.path().join("workspace");
    let credentials = Arc::new(MemoryCredentialResolver::default());
    let service =
        DesktopService::open_with_credential_store(&workspace, credentials.clone()).unwrap();
    let request = serde_json::from_value(serde_json::json!({
        "profile_id": "desktop-default",
        "provider": "custom-relay",
        "wire_protocol": "open_ai_chat_completions",
        "model": "fixture-model",
        "endpoint": "https://relay.example/v1",
        "api_key": "fixture-secret-never-in-sqlite",
        "clear_api_key": false,
        "make_default": true
    }))
    .unwrap();
    let snapshot = service.save_llm_profile(request).unwrap();
    let value = serde_json::to_value(snapshot).unwrap();
    assert_eq!(value["default_profile_id"], "desktop-default");
    assert_eq!(
        value["profiles"][0]["endpoint"],
        "https://relay.example/v1/"
    );
    assert_eq!(value["profiles"][0]["has_credential"], true);

    let workspace_id = service.snapshot().unwrap().workspace_id().to_string();
    let reference = CredentialRef::new(format!("{workspace_id}:desktop-default")).unwrap();
    assert!(credentials.resolve(&reference).is_ok());
    let database = fs::read(workspace.join("mathscape.sqlite3")).unwrap();
    assert!(!String::from_utf8_lossy(&database).contains("fixture-secret-never-in-sqlite"));

    let reopened =
        DesktopService::open_with_credential_store(&workspace, credentials.clone()).unwrap();
    let value = serde_json::to_value(reopened.llm_settings().unwrap()).unwrap();
    assert_eq!(value["profiles"][0]["model"], "fixture-model");

    let clear = serde_json::from_value(serde_json::json!({
        "profile_id": "desktop-default",
        "provider": "custom-relay",
        "wire_protocol": "open_ai_chat_completions",
        "model": "fixture-model",
        "endpoint": "https://relay.example/v1/",
        "api_key": null,
        "clear_api_key": true,
        "make_default": true
    }))
    .unwrap();
    reopened.save_llm_profile(clear).unwrap();
    assert!(credentials.resolve(&reference).is_err());
    directory.cleanup();
}

#[tokio::test]
async fn tests_a_persisted_llm_profile_through_the_desktop_service() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer desktop-fixture-secret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "chatcmpl-desktop-fixture",
            "object": "chat.completion",
            "created": 1,
            "model": "resolved-fixture-model",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "MATHSCAPE_OK"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 9, "completion_tokens": 3, "total_tokens": 12}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let directory = TestDirectory::create("llm-connection");
    let credentials = Arc::new(MemoryCredentialResolver::default());
    let service =
        DesktopService::open_with_credential_store(directory.path().join("workspace"), credentials)
            .unwrap();
    let request = serde_json::from_value(serde_json::json!({
        "profile_id": "desktop-default",
        "provider": "custom-relay",
        "wire_protocol": "open_ai_chat_completions",
        "model": "requested-fixture-model",
        "endpoint": format!("{}/v1/", server.uri()),
        "api_key": "desktop-fixture-secret",
        "clear_api_key": false,
        "make_default": true
    }))
    .unwrap();
    service.save_llm_profile(request).unwrap();

    let tested = serde_json::to_value(service.test_llm_profile(None).await.unwrap()).unwrap();
    assert_eq!(tested["profile_id"], "desktop-default");
    assert_eq!(tested["provider_model"], "resolved-fixture-model");
    assert_eq!(tested["response_preview"], "MATHSCAPE_OK");
    assert_eq!(tested["input_tokens"], 9);
    directory.cleanup();
}

#[tokio::test]
async fn runs_discovery_from_managed_markdown_through_the_configured_profile() {
    const SOURCE: &str = "习题 1：设 $a_n=a$，用定义证明 $a_n$ 收敛于 $a$。\n\n解答：任取 $\\epsilon>0$，取 $N=1$。当 $n>N$ 时，$|a_n-a|=0<\\epsilon$。因此收敛于 $a$。\n";

    let server = MockServer::start().await;
    let directory = TestDirectory::create("desktop-discovery");
    let workspace = directory.path().join("workspace");
    let markdown = directory.path().join("selection.md");
    fs::write(&markdown, SOURCE).unwrap();
    let credentials = Arc::new(MemoryCredentialResolver::default());
    let service =
        DesktopService::open_with_credential_store(&workspace, credentials.clone()).unwrap();

    let mut pool = ArtifactPool::open(&workspace).unwrap();
    let imported = pool
        .import_file(
            &markdown,
            ArtifactMetadata::new(
                None,
                MediaType::parse("text/markdown").unwrap(),
                Some("selection.md".into()),
                None,
                RightsRecord::new(RightsStatus::NotAssessed, vec![], None).unwrap(),
            ),
            ActorRef::parse("desktop-discovery-test").unwrap(),
        )
        .unwrap();
    let markdown_id = imported.stable_id().to_string();
    drop(pool);

    let profile = serde_json::from_value(serde_json::json!({
        "profile_id": "desktop-discovery",
        "provider": "custom-relay",
        "wire_protocol": "open_ai_chat_completions",
        "model": "discovery-fixture-model",
        "endpoint": format!("{}/v1/", server.uri()),
        "api_key": "desktop-discovery-secret",
        "clear_api_key": false,
        "make_default": true
    }))
    .unwrap();
    service.save_llm_profile(profile).unwrap();
    let settings: SaveWorkspaceSettingsRequest = serde_json::from_value(serde_json::json!({
        "discovery_admission": "review_before_publish",
        "discovery_timeout_seconds": 900,
        "connect_timeout_seconds": 20
    }))
    .unwrap();
    service.save_workspace_settings(settings).unwrap();

    let document = service.markdown_document(&markdown_id, None).unwrap();
    let selection = DiscoverySelectionRequest {
        version: DISCOVERY_SELECTION_VERSION.into(),
        snapshot: document.snapshot.clone(),
        selector: SourceSelector::WholeDocument,
        admission: DiscoveryAdmission::ReviewBeforePublish,
    };
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer desktop-discovery-secret"))
        .respond_with(StagedDiscoveryResponder)
        .expect(5)
        .mount(&server)
        .await;

    let request = RunDiscoveryRequest::new(selection.clone(), None);
    let worker = {
        let service = service.clone();
        let request = request.clone();
        tokio::spawn(async move { service.run_discovery(request).await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(40)).await;
    let active = service.active_discovery().unwrap();
    assert_eq!(active.artifact_id(), markdown_id);
    assert_eq!(
        serde_json::to_value(&active).unwrap()["request_timeout_seconds"],
        900
    );
    assert!(matches!(
        service.run_discovery(request.clone()).await,
        Err(DesktopDiscoveryError::AlreadyRunning)
    ));

    let outcome = worker.await.unwrap().unwrap();
    assert!(service.active_discovery().is_none());
    let value = serde_json::to_value(outcome).unwrap();
    assert_eq!(value["outcome"], "completed");
    assert_eq!(
        value["view"]["version"],
        "mathscape-desktop-discovery-view-1"
    );
    assert_eq!(value["view"]["status"], "completed");
    assert_eq!(value["view"]["units"].as_array().unwrap().len(), 2);
    assert_eq!(value["view"]["relations"][0]["relation"], "solves");
    assert_eq!(value["view"]["units"][0]["title"], "习题 1");
    assert_eq!(value["view"]["references"][0]["text"], "定义");
    assert_eq!(value["view"]["references"][0]["status"], "unresolved");
    assert!(
        value["view"]["references"][0]["anchor"]["block_ids"]
            .as_array()
            .unwrap()
            .iter()
            .all(|id| id.as_str().unwrap().starts_with("block:"))
    );
    assert!(
        value["view"]["relations"][0]["evidence"][0]["block_ids"]
            .as_array()
            .unwrap()
            .iter()
            .all(|id| id.as_str().unwrap().starts_with("block:"))
    );
    assert!(
        value["review"]["references"][0]["anchor"]["block_ids"]
            .as_array()
            .unwrap()
            .iter()
            .all(|id| id.as_str().unwrap().starts_with("block:"))
    );
    assert!(
        value["review"]["relations"][0]["evidence"][0]["block_ids"]
            .as_array()
            .unwrap()
            .iter()
            .all(|id| id.as_str().unwrap().starts_with("block:"))
    );
    assert_eq!(value["view"]["issues"][0]["kind"], "missing_context");
    assert!(
        value["view"]["issues"][0]["evidence"][0]["excerpt"]
            .as_str()
            .unwrap()
            .contains("习题 1")
    );
    assert_eq!(
        value["view"]["units"][0]["anchors"][0]["block_ids"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(value["run"]["attempts"], 2);
    assert!(value["run"]["total_tokens"].is_null());
    assert_eq!(value["run"]["admission"], "review_before_publish");
    assert_eq!(value["run"]["request_timeout_seconds"], 900);
    assert_eq!(value["run"]["connect_timeout_seconds"], 20);
    let run_id = StableId::parse(value["run"]["run_id"].as_str().unwrap()).unwrap();
    let listed =
        serde_json::to_value(service.list_discovery_runs(Some(&markdown_id)).unwrap()).unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["review_status"], "reviewable");
    assert_eq!(listed[0]["unresolved_reference_count"], 1);
    let recovered =
        serde_json::to_value(service.get_discovery_review(run_id.as_str()).unwrap()).unwrap();
    assert_eq!(
        recovered["outcome"]["review"]["result_fingerprint"],
        value["review"]["result_fingerprint"]
    );
    assert_eq!(
        recovered["outcome"]["view"]["references"][0]["status"],
        "unresolved"
    );
    assert!(
        recovered["outcome"]["view"]["references"][0]["anchor"]["block_ids"]
            .as_array()
            .unwrap()
            .iter()
            .all(|id| id.as_str().unwrap().starts_with("block:"))
    );
    let diagnostics =
        serde_json::to_value(service.get_discovery_diagnostics(run_id.as_str()).unwrap()).unwrap();
    assert_eq!(diagnostics["status"], "completed");
    assert_eq!(diagnostics["attempts"].as_array().unwrap().len(), 2);
    let store = SqliteArtifactStore::open(&workspace.join("mathscape.sqlite3")).unwrap();
    let persisted = store.discovery_checkpoint(&run_id).unwrap().unwrap();
    assert_eq!(
        persisted.checkpoint.status,
        DiscoveryCheckpointStatus::Completed
    );
    assert_eq!(persisted.checkpoint.units.len(), 2);
    assert_eq!(persisted.checkpoint.facts.len(), 2);
    assert!(persisted.checkpoint.pending_tasks.is_empty());
    assert_eq!(
        service.snapshot().unwrap().landscape_graph().nodes().len(),
        1
    );
    drop(store);

    let invalid_acceptance: AcceptDiscoveryRequest = serde_json::from_value(serde_json::json!({
        "run_id": run_id.as_str(),
        "expected_result_fingerprint": value["review"]["result_fingerprint"],
        "selection": {"kind": "explicit_units", "unit_refs": ["u2"]}
    }))
    .unwrap();
    assert!(matches!(
        service.accept_discovery_result(invalid_acceptance),
        Err(DesktopDiscoveryError::Acceptance(message)) if message.contains("requires its parent")
    ));
    let acceptance: AcceptDiscoveryRequest = serde_json::from_value(serde_json::json!({
        "run_id": run_id.as_str(),
        "expected_result_fingerprint": value["review"]["result_fingerprint"],
        "selection": {"kind": "explicit_units", "unit_refs": ["u1", "u2"]}
    }))
    .unwrap();
    let discard = mathscape_domain::DiscoveryDispositionRequest {
        run_id: run_id.clone(),
        request_id: StableId::parse("mathscape:test/discard-review").unwrap(),
        expected_result_fingerprint: serde_json::from_value(
            value["review"]["result_fingerprint"].clone(),
        )
        .unwrap(),
        expected_disposition_request_id: None,
        reason: "整理后再审核".into(),
    };
    let discarded = service.discard_discovery_run(discard).unwrap();
    let readonly =
        serde_json::to_value(service.get_discovery_review(run_id.as_str()).unwrap()).unwrap();
    assert_eq!(readonly["review_status"], "discarded");
    assert!(
        readonly["outcome"]["review"]["units"]
            .as_array()
            .unwrap()
            .iter()
            .all(|u| u["acceptance"] == "blocked")
    );
    assert!(service.accept_discovery_result(acceptance.clone()).is_err());
    let restore = mathscape_domain::DiscoveryDispositionRequest {
        run_id: run_id.clone(),
        request_id: StableId::parse("mathscape:test/restore-review").unwrap(),
        expected_result_fingerprint: serde_json::from_value(
            value["review"]["result_fingerprint"].clone(),
        )
        .unwrap(),
        expected_disposition_request_id: Some(discarded.request_id),
        reason: "恢复审核".into(),
    };
    let mut store = SqliteArtifactStore::open(&workspace.join("mathscape.sqlite3")).unwrap();
    let original = store
        .discovery_checkpoint(&run_id)
        .unwrap()
        .unwrap()
        .checkpoint;
    let mut incompatible = original.clone();
    incompatible.snapshot["source"]["snapshot"]["content_hash"] =
        serde_json::json!(format!("sha256:{}", "f".repeat(64)));
    store.save_discovery_checkpoint(&incompatible).unwrap();
    assert!(service.restore_discovery_review(restore.clone()).is_err());
    assert!(store.discovery_run_is_discarded(&run_id).unwrap());
    assert_eq!(
        store.discovery_disposition_history(&run_id).unwrap().len(),
        1
    );
    store.save_discovery_checkpoint(&original).unwrap();
    drop(store);
    let restored = service.restore_discovery_review(restore.clone()).unwrap();
    assert_eq!(restored.disposition, "restored");
    assert_eq!(
        service.restore_discovery_review(restore.clone()).unwrap(),
        restored
    );
    let reopened_review = serde_json::to_value(
        DesktopService::open(&workspace)
            .unwrap()
            .get_discovery_review(run_id.as_str())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(reopened_review["review_status"], "reviewable");
    assert!(
        reopened_review["outcome"]["review"]["units"]
            .as_array()
            .unwrap()
            .iter()
            .all(|u| u["acceptance"] != "blocked")
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    assert!(
        SqliteArtifactStore::open(&workspace.join("mathscape.sqlite3"))
            .unwrap()
            .map_objects()
            .unwrap()
            .is_empty()
    );
    let accepted =
        serde_json::to_value(service.accept_discovery_result(acceptance.clone()).unwrap()).unwrap();
    assert_eq!(accepted["state"], "registered");
    let standalone_context = service
        .registration_source_context(accepted["decision_id"].as_str().unwrap())
        .unwrap();
    assert_eq!(standalone_context.artifact_id.as_str(), markdown_id);
    assert!(standalone_context.conversion_id.is_none());
    assert_eq!(accepted["unit_count"], 2);
    assert_eq!(accepted["relation_count"], 1);
    assert_eq!(accepted["already_accepted"], false);
    assert_eq!(accepted["already_registered"], false);
    assert!(accepted["created_object_count"].as_u64().unwrap() >= 5);
    let repeated =
        serde_json::to_value(service.accept_discovery_result(acceptance.clone()).unwrap()).unwrap();
    assert_eq!(repeated["decision_id"], accepted["decision_id"]);
    assert_eq!(repeated["already_accepted"], true);
    assert_eq!(repeated["already_registered"], true);
    assert_eq!(repeated["commit_id"], accepted["commit_id"]);
    let decision_id = StableId::parse(accepted["decision_id"].as_str().unwrap()).unwrap();
    let store = SqliteArtifactStore::open(&workspace.join("mathscape.sqlite3")).unwrap();
    assert!(store.registration_plan(&decision_id).unwrap().is_some());
    assert!(
        store
            .source_registration_receipt(&decision_id)
            .unwrap()
            .is_some()
    );
    let objects = store.map_objects().unwrap();
    assert_eq!(
        objects.len() as u64,
        accepted["created_object_count"].as_u64().unwrap()
    );
    assert_eq!(
        objects
            .iter()
            .filter(|object| object.kind == mathscape_domain::EntityKind::SourceSemanticUnit)
            .count(),
        2
    );
    drop(store);
    let projected = service.snapshot().unwrap();
    assert_eq!(
        projected
            .landscape_graph()
            .nodes()
            .iter()
            .filter(
                |node| node.domain_kind() == Some(mathscape_domain::EntityKind::SourceSemanticUnit)
            )
            .count(),
        2
    );
    assert!(
        projected.landscape_graph().nodes().iter().any(|node| {
            node.domain_kind() == Some(mathscape_domain::EntityKind::SourceArtifact)
        })
    );
    assert!(projected.landscape_graph().nodes().len() > 2);
    assert!(projected.landscape_graph().nodes().iter().any(|node| {
        node.domain_kind() == Some(mathscape_domain::EntityKind::Evidence)
            && node.label() == "来源于 Markdown"
    }));
    let reopened = DesktopService::open(&workspace).unwrap();
    assert_eq!(
        reopened.snapshot().unwrap().landscape_graph().nodes().len(),
        projected.landscape_graph().nodes().len()
    );

    // Exercise the actual accepted plan, withdrawal transaction and a NEW LLM run.
    let mut store = SqliteArtifactStore::open(&workspace.join("mathscape.sqlite3")).unwrap();
    let old_receipt = store
        .source_registration_receipt(&decision_id)
        .unwrap()
        .unwrap();
    let reverse = mathscape_domain::ReverseDiscoveryRequest {
        request_id: "mathscape:test/withdraw-desktop".into(),
        decision_id: decision_id.to_string(),
        registration_commit_id: old_receipt.commit_id.to_string(),
        base_commit: store.map_edit_snapshot().unwrap().base_commit,
        reason: "重新整理".into(),
    };
    let actor = ActorRef::parse("mathscape-desktop-user").unwrap();
    let planned = mathscape_orchestrator::registration::plan_registration_reversal(
        &store.registration_snapshot().unwrap(),
        &reverse,
        &actor,
    );
    assert_eq!(
        planned.plan.impact.status,
        mathscape_domain::ImpactPlanStatus::Ready
    );
    store
        .commit_registration_reversal(
            &reverse,
            &planned.plan.impact.plan_fingerprint,
            &actor,
            mathscape_domain::Timestamp::parse("unix:2").unwrap(),
            |s| {
                let p = mathscape_orchestrator::registration::plan_registration_reversal(
                    s, &reverse, &actor,
                );
                (p.plan, p.patch)
            },
        )
        .unwrap();
    let history = store.registration_history(decision_id.as_str()).unwrap();
    assert!(history.objects.iter().all(|o| o.lifecycle == "active"));
    drop(store);
    let old_replay =
        serde_json::to_value(service.accept_discovery_result(acceptance).unwrap()).unwrap();
    assert_eq!(old_replay["state"], "withdrawn");
    let new_outcome = serde_json::to_value(
        service
            .run_discovery(RunDiscoveryRequest::new(selection.clone(), None))
            .await
            .unwrap(),
    )
    .unwrap();
    let new_accept:AcceptDiscoveryRequest=serde_json::from_value(serde_json::json!({"run_id":new_outcome["run"]["run_id"],"expected_result_fingerprint":new_outcome["review"]["result_fingerprint"],"selection":{"kind":"explicit_units","unit_refs":["u1","u2"]}})).unwrap();
    let pending =
        serde_json::to_value(service.accept_discovery_result(new_accept).unwrap()).unwrap();
    assert_eq!(pending["state"], "registration_review_required");
    assert_eq!(pending["registration_plan"]["impact"]["status"], "ready");
    let request: mathscape_domain::SourceRegistrationCommitRequest =
        serde_json::from_value(pending["registration_plan"]["request"].clone()).unwrap();
    let mut store = SqliteArtifactStore::open(&workspace.join("mathscape.sqlite3")).unwrap();
    let record = store
        .registration_plan(&StableId::parse(&request.decision_id).unwrap())
        .unwrap()
        .unwrap();
    let candidate = mathscape_orchestrator::materialize_source_registration(
        &serde_json::from_value::<mathscape_orchestrator::SourceRegistrationPlan>(record.plan)
            .unwrap(),
    )
    .unwrap();
    let registered = store
        .commit_bound_source_registration(
            &request,
            pending["registration_plan"]["impact"]["plan_fingerprint"]
                .as_str()
                .unwrap(),
            &actor,
            mathscape_domain::Timestamp::parse("unix:3").unwrap(),
            |s| {
                let p = mathscape_orchestrator::registration::plan_registration_commit(
                    s, &request, &candidate, &actor,
                );
                (p.plan, p.patch)
            },
        )
        .unwrap();
    assert_eq!(old_receipt.unit_ids, registered.unit_ids);
    assert_eq!(
        registered.reactivated_objects.len(),
        old_receipt.created_objects.len()
    );
    assert!(
        store
            .map_objects()
            .unwrap()
            .iter()
            .all(|o| o.lifecycle == mathscape_domain::LifecycleStatus::Active)
    );
    drop(store);

    let cancelled = {
        let service = service.clone();
        let request = RunDiscoveryRequest::new(selection, None);
        tokio::spawn(async move { service.run_discovery(request).await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(40)).await;
    assert!(service.cancel_discovery());
    assert_eq!(
        serde_json::to_value(service.active_discovery().unwrap()).unwrap()["phase"],
        "cancelling"
    );
    let cancelled = serde_json::to_value(cancelled.await.unwrap().unwrap()).unwrap();
    assert_eq!(cancelled["outcome"], "cancelled");
    let cancelled_run = StableId::parse(cancelled["run"]["run_id"].as_str().unwrap()).unwrap();
    let store = SqliteArtifactStore::open(&workspace.join("mathscape.sqlite3")).unwrap();
    assert_eq!(
        store
            .discovery_checkpoint(&cancelled_run)
            .unwrap()
            .unwrap()
            .checkpoint
            .status,
        DiscoveryCheckpointStatus::Cancelled
    );
    assert_eq!(store.discovery_run_count().unwrap(), 3);
    assert!(service.active_discovery().is_none());
    assert!(!service.cancel_discovery());

    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 5);
    let schema_names = received
        .iter()
        .map(|request| {
            let body: serde_json::Value = request.body_json().unwrap();
            assert_eq!(body["stream"], true);
            assert!(body.get("stream_options").is_none());
            body["response_format"]["json_schema"]["name"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        schema_names,
        [
            "unit_discovery_v1",
            "source_facts_v1",
            "unit_discovery_v1",
            "source_facts_v1",
            "unit_discovery_v1"
        ]
    );
    directory.cleanup();
}

const PATCH_SOURCE: &str =
    include_str!("../../../crates/mathscape-domain/tests/fixtures/staged-source.md");
const PATCH_UNITS: &str =
    include_str!("../../../crates/mathscape-domain/tests/fixtures/staged-units.json");
const PATCH_FACTS: &str =
    include_str!("../../../crates/mathscape-domain/tests/fixtures/staged-facts.json");

/// A reports one conversion issue inside b2, P proposes `N=1` → `N=2`, B
/// answers from the staged fixture.
#[derive(Clone, Debug)]
struct PatchReviewResponder;

impl Respond for PatchReviewResponder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        let schema = body["response_format"]["json_schema"]["name"]
            .as_str()
            .unwrap();
        let content = body["messages"].as_array().unwrap().last().unwrap()["content"]
            .as_str()
            .unwrap();
        let input: serde_json::Value = serde_json::from_str(content).unwrap();
        let output = match schema {
            "unit_discovery_v1" => {
                let mut units: serde_json::Value = serde_json::from_str(PATCH_UNITS).unwrap();
                units["request_ref"] = input["request_ref"].clone();
                units["issues"] = serde_json::json!([{
                    "kind": "suspected_conversion_error",
                    "unit": "u2",
                    "evidence": [{"kind": "block_range", "first": "b2", "last": "b2"}],
                    "observation": "fixture conversion damage",
                    "context_request": null
                }]);
                units
            }
            "mathscape_markdown_patch_response" => serde_json::json!({
                "version": mathscape_domain::MARKDOWN_PATCH_VERSION,
                "request_ref": input["request_ref"],
                "patches": [{
                    "patch_ref": "model-1",
                    "issue_ref": input["issues"][0]["issue_ref"],
                    "target": {"kind": "text_occurrence", "block": "b2", "text": "N=1", "occurrence": null},
                    "replacement": "N=2",
                    "rationale": "fixture repair"
                }]
            }),
            "source_facts_v1" => {
                let mut facts: serde_json::Value = serde_json::from_str(PATCH_FACTS).unwrap();
                facts["request_ref"] = input["request_ref"].clone();
                let requested = input["tasks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|task| task["task_ref"].clone())
                    .collect::<Vec<_>>();
                facts["tasks"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|task| requested.contains(&task["task_ref"]));
                facts
            }
            other => panic!("unexpected schema {other}"),
        };
        let chunk = serde_json::json!({
            "id":"chatcmpl-patch-review-fixture",
            "object":"chat.completion.chunk","created":1,"model":"patch-review-model",
            "choices":[{"index":0,"delta":{"role":"assistant","content":output.to_string()},"finish_reason":"stop"}]
        });
        ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string(format!("data: {chunk}\n\ndata: [DONE]\n\n"))
    }
}

#[tokio::test]
async fn discovery_pauses_for_markdown_patch_review_and_resumes_from_sqlite() {
    let server = MockServer::start().await;
    let directory = TestDirectory::create("desktop-patch-review");
    let workspace = directory.path().join("workspace");
    let markdown = directory.path().join("staged.md");
    fs::write(&markdown, PATCH_SOURCE).unwrap();
    let credentials = Arc::new(MemoryCredentialResolver::default());
    let service = DesktopService::open_with_credential_store(&workspace, credentials).unwrap();
    let mut pool = ArtifactPool::open(&workspace).unwrap();
    let markdown_id = pool
        .import_file(
            &markdown,
            ArtifactMetadata::new(
                None,
                MediaType::parse("text/markdown").unwrap(),
                Some("staged.md".into()),
                None,
                RightsRecord::new(RightsStatus::NotAssessed, vec![], None).unwrap(),
            ),
            ActorRef::parse("desktop-patch-review-test").unwrap(),
        )
        .unwrap()
        .stable_id()
        .to_string();
    drop(pool);
    service
        .save_llm_profile(
            serde_json::from_value(serde_json::json!({
                "profile_id": "patch-review",
                "provider": "custom-relay",
                "wire_protocol": "open_ai_chat_completions",
                "model": "patch-review-model",
                "endpoint": format!("{}/v1/", server.uri()),
                "api_key": "patch-review-secret",
                "clear_api_key": false,
                "make_default": true
            }))
            .unwrap(),
        )
        .unwrap();
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(PatchReviewResponder)
        .mount(&server)
        .await;
    let document = service.markdown_document(&markdown_id, None).unwrap();
    let request = RunDiscoveryRequest::new(
        DiscoverySelectionRequest {
            version: DISCOVERY_SELECTION_VERSION.into(),
            snapshot: document.snapshot.clone(),
            selector: SourceSelector::WholeDocument,
            admission: DiscoveryAdmission::ReviewBeforePublish,
        },
        None,
    );
    let database = workspace.join("mathscape.sqlite3");
    let status_of = |run_id: &StableId| {
        SqliteArtifactStore::open(&database)
            .unwrap()
            .discovery_checkpoint(run_id)
            .unwrap()
            .unwrap()
            .checkpoint
    };
    let pause = |outcome: serde_json::Value| {
        assert_eq!(outcome["outcome"], "awaiting_patch_review");
        assert!(outcome["review"].is_null());
        let run_id = StableId::parse(outcome["run"]["run_id"].as_str().unwrap()).unwrap();
        let checkpoint = status_of(&run_id);
        assert_eq!(
            checkpoint.status,
            DiscoveryCheckpointStatus::AwaitingPatchReview
        );
        let review = serde_json::to_value(
            service
                .get_markdown_patch_review(run_id.to_string())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(review["status"], "proposed");
        assert_eq!(review["patch_set"]["patches"][0]["patch_ref"], "p1");
        (run_id, review)
    };
    let server = &server;
    let schemas = |from: usize| async move {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .skip(from)
            .map(|request| {
                let body: serde_json::Value = request.body_json().unwrap();
                body["response_format"]["json_schema"]["name"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect::<Vec<_>>()
    };

    // Rejecting keeps the original snapshot; B still runs in the same run.
    let first =
        serde_json::to_value(service.run_discovery(request.clone()).await.unwrap()).unwrap();
    let (rejected_run, review) = pause(first);
    let pending_status = || {
        service
            .get_pending_markdown_patch_review(markdown_id.to_string())
            .unwrap()
            .map(|review| {
                let review = serde_json::to_value(review).unwrap();
                (
                    review["run_id"].as_str().unwrap().to_owned(),
                    review["status"].as_str().unwrap().to_owned(),
                )
            })
    };
    assert_eq!(
        pending_status(),
        Some((rejected_run.to_string(), "proposed".to_owned()))
    );
    assert!(matches!(
        service
            .resume_discovery_after_patch(ResumeDiscoveryRequest::new(rejected_run.as_str()))
            .await,
        Err(DesktopDiscoveryError::Resume(message)) if message.contains("接受或拒绝")
    ));
    service
        .submit_markdown_patch_decision(
            serde_json::from_value(serde_json::json!({
                "run_id": rejected_run.as_str(),
                "patch_set_id": review["patch_set_id"],
                "expected_patch_fingerprint": review["patch_set"]["fingerprint"],
                "accepted_patch_refs": []
            }))
            .unwrap(),
        )
        .unwrap();
    // A recorded decision stays recoverable until the run actually resumes.
    assert_eq!(
        pending_status(),
        Some((rejected_run.to_string(), "rejected".to_owned()))
    );
    let resumed = serde_json::to_value(
        service
            .resume_discovery_after_patch(ResumeDiscoveryRequest::new(rejected_run.as_str()))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(resumed["outcome"], "completed");
    assert_eq!(resumed["run"]["run_id"], rejected_run.as_str());
    assert_eq!(resumed["run"]["attempts"], 3);
    let checkpoint = status_of(&rejected_run);
    assert_eq!(checkpoint.status, DiscoveryCheckpointStatus::Completed);
    assert_eq!(checkpoint.artifact_id.as_str(), markdown_id);
    assert_eq!(pending_status(), None);
    let recovered_rejected =
        serde_json::to_value(service.get_discovery_review(rejected_run.as_str()).unwrap()).unwrap();
    assert_eq!(
        recovered_rejected["outcome"]["view"]["source"]["artifact_id"],
        markdown_id
    );
    assert_eq!(
        schemas(0).await,
        [
            "unit_discovery_v1",
            "mathscape_markdown_patch_response",
            "source_facts_v1"
        ]
    );
    assert!(
        service
            .resume_discovery_after_patch(ResumeDiscoveryRequest::new(rejected_run.as_str()))
            .await
            .is_err(),
        "a finished run cannot be resumed twice"
    );

    // Accepting promotes the patched Markdown and B runs on it.
    let second = serde_json::to_value(service.run_discovery(request).await.unwrap()).unwrap();
    let (applied_run, review) = pause(second);
    let receipt = serde_json::to_value(
        service
            .submit_markdown_patch_decision(
                serde_json::from_value(serde_json::json!({
                    "run_id": applied_run.as_str(),
                    "patch_set_id": review["patch_set_id"],
                    "expected_patch_fingerprint": review["patch_set"]["fingerprint"],
                    "accepted_patch_refs": ["p1"]
                }))
                .unwrap(),
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(receipt["status"], "applied");
    let patched_artifact = receipt["patched_artifact_id"].as_str().unwrap().to_owned();
    let resumed = serde_json::to_value(
        service
            .resume_discovery_after_patch(ResumeDiscoveryRequest::new(applied_run.as_str()))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(resumed["outcome"], "completed");
    // The reader switches to this snapshot to locate the resumed result.
    assert_eq!(
        resumed["view"]["source"]["artifact_id"],
        patched_artifact.as_str()
    );
    assert_eq!(
        resumed["view"]["source"]["conversion_id"],
        receipt["conversion_id"]
    );
    let checkpoint = status_of(&applied_run);
    assert_eq!(checkpoint.status, DiscoveryCheckpointStatus::Completed);
    assert_eq!(checkpoint.artifact_id.as_str(), patched_artifact);
    assert_eq!(
        checkpoint.conversion_id.as_ref().map(StableId::as_str),
        receipt["conversion_id"].as_str()
    );
    assert_eq!(
        checkpoint.snapshot["patch"]["applied"]["patched_artifact_id"],
        patched_artifact.as_str()
    );
    let recovered_applied =
        serde_json::to_value(service.get_discovery_review(applied_run.as_str()).unwrap()).unwrap();
    assert_eq!(
        recovered_applied["outcome"]["view"]["source"]["artifact_id"],
        patched_artifact
    );
    let acceptance: AcceptDiscoveryRequest = serde_json::from_value(serde_json::json!({
        "run_id":applied_run.as_str(),
        "expected_result_fingerprint":resumed["review"]["result_fingerprint"],
        "selection":{"kind":"all_validated"}
    }))
    .unwrap();
    let accepted =
        serde_json::to_value(service.accept_discovery_result(acceptance).unwrap()).unwrap();
    assert_eq!(accepted["state"], "registered");
    let decision = accepted["decision_id"].as_str().unwrap();
    let context = service.registration_source_context(decision).unwrap();
    assert_eq!(context.artifact_id.as_str(), patched_artifact);
    assert_eq!(
        context.conversion_id.as_ref().map(StableId::as_str),
        receipt["conversion_id"].as_str()
    );
    assert!(
        service
            .markdown_document(context.artifact_id.as_str(), None)
            .is_err()
    );
    let doc = service
        .markdown_document(
            context.artifact_id.as_str(),
            context.conversion_id.as_ref().map(StableId::as_str),
        )
        .unwrap();
    assert!(doc.source.contains("N=2"));
    let plan = SqliteArtifactStore::open(&workspace.join("mathscape.sqlite3"))
        .unwrap()
        .registration_plan(&StableId::parse(decision).unwrap())
        .unwrap()
        .unwrap();
    let unit_id = plan.plan["units"][0]["stable_id"].as_str().unwrap();
    let detail = service
        .registration_object_detail(decision, unit_id)
        .unwrap();
    assert_eq!(
        detail.artifact_id.as_deref(),
        Some(patched_artifact.as_str())
    );
    assert!(detail.excerpt.is_some());
    assert_eq!(
        schemas(3).await,
        [
            "unit_discovery_v1",
            "mathscape_markdown_patch_response",
            "source_facts_v1"
        ]
    );
    let facts_request: serde_json::Value = server.received_requests().await.unwrap()[5]
        .body_json()
        .unwrap();
    assert!(
        facts_request["messages"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["content"]
            .as_str()
            .unwrap()
            .contains("N=2"),
        "B reads the patched Markdown"
    );
    directory.cleanup();
}

#[test]
fn imports_a_pdf_into_a_reopenable_workspace_and_refreshes_the_graph() {
    let directory = TestDirectory::create("pdf-import");
    let source = directory.path().join("analysis.pdf");
    write_pdf(&source);
    let workspace = directory.path().join("workspace");
    let service = DesktopService::open(&workspace).unwrap();

    let initial = service.snapshot().unwrap();
    assert_eq!(initial.artifact_count(), 0);
    assert!(initial.landscape_graph().nodes().is_empty());

    let imported = service.import_pdf(&source).unwrap();
    assert_eq!(imported.outcome(), PdfImportOutcome::Inserted);
    assert_eq!(imported.page_count(), 1);
    assert_eq!(imported.snapshot().artifact_count(), 1);
    assert_eq!(imported.snapshot().conversion_count(), 0);
    assert_eq!(imported.snapshot().landscape_graph().nodes().len(), 1);
    assert!(
        imported
            .artifact_id()
            .starts_with("mathscape:artifact/sha256/")
    );

    let duplicate = service.import_pdf(&source).unwrap();
    assert_eq!(duplicate.outcome(), PdfImportOutcome::Deduplicated);
    assert_eq!(duplicate.snapshot().artifact_count(), 1);

    fs::remove_file(&source).unwrap();
    let reopened = DesktopService::open(&workspace).unwrap();
    assert_eq!(reopened.snapshot().unwrap().artifact_count(), 1);
    assert_eq!(
        reopened
            .read_artifact_bytes(imported.artifact_id(), 0, 5)
            .unwrap(),
        b"%PDF-"
    );
    let info =
        serde_json::to_value(reopened.content_info(imported.artifact_id()).unwrap()).unwrap();
    assert_eq!(info["media_type"], "application/pdf");
    let size = info["byte_size"].as_u64().unwrap();
    assert!(
        reopened
            .read_artifact_bytes(imported.artifact_id(), size + 1, 1)
            .is_err()
    );
    assert!(
        reopened
            .read_artifact_bytes(imported.artifact_id(), 0, 4 * 1024 * 1024 + 1)
            .is_err()
    );
    assert!(
        reopened
            .read_artifact_bytes("unknown:artifact", 0, 1)
            .is_err()
    );
    drop(reopened);
    directory.cleanup();
}

#[test]
fn rejects_malformed_pdf_bytes_before_artifact_registration() {
    let directory = TestDirectory::create("malformed-pdf");
    let source = directory.path().join("broken.pdf");
    fs::write(&source, b"this has a PDF extension but no PDF structure").unwrap();
    let service = DesktopService::open(directory.path().join("workspace")).unwrap();

    assert!(matches!(
        service.import_pdf(&source),
        Err(DesktopServiceError::Pdf(_))
    ));
    assert_eq!(service.snapshot().unwrap().artifact_count(), 0);
    directory.cleanup();
}

#[test]
fn rejects_non_pdf_imports_before_artifact_registration() {
    let directory = TestDirectory::create("non-pdf");
    let source = directory.path().join("notes.md");
    fs::write(&source, b"not a PDF").unwrap();
    let service = DesktopService::open(directory.path().join("workspace")).unwrap();

    assert!(matches!(
        service.import_pdf(&source),
        Err(DesktopServiceError::UnsupportedImportType(path)) if path == source
    ));
    assert_eq!(service.snapshot().unwrap().artifact_count(), 0);
    directory.cleanup();
}

#[cfg(unix)]
#[test]
fn parses_a_managed_pdf_promotes_products_and_refreshes_the_conversion_graph() {
    let directory = TestDirectory::create("marker-parse");
    let source = directory.path().join("analysis.pdf");
    write_pdf(&source);
    let runtime = directory.path().join("marker-runtime");
    let bin = runtime.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let marker = bin.join("marker_single");
    let python = bin.join("python");
    write_executable(&python, "#!/bin/sh\nprintf '2.1.0'\n");
    write_executable(
        &marker,
        r#"#!/bin/sh
source_path="$1"
output_root=''
paginate=0
while [ "$#" -gt 0 ]; do
    case "$1" in
        --output_dir) output_root="$2"; shift 2 ;;
        --paginate_output) paginate=1; shift ;;
        *) shift ;;
    esac
done
source_name="${source_path##*/}"
source_stem="${source_name%.*}"
mkdir -p "$output_root/$source_stem/images"
if [ "$paginate" -eq 1 ]; then
    printf '{0}----------------\n\n# Converted analysis\n![plot](images/plot.png)\n' > "$output_root/$source_stem/$source_stem.md"
else
    printf '# Converted analysis\n![plot](images/plot.png)\n' > "$output_root/$source_stem/$source_stem.md"
fi
printf '\211PNG\r\n\032\n' > "$output_root/$source_stem/images/plot.png"
printf '{"page_stats":[{"page_id":0}]}\n' > "$output_root/$source_stem/${source_stem}_meta.json"
"#,
    );

    let workspace = directory.path().join("workspace");
    let service = DesktopService::open_with_marker(&workspace, &marker).unwrap();
    assert!(service.snapshot().unwrap().marker().is_ready());
    let imported = service.import_pdf(&source).unwrap();
    let request =
        ParsePdfRequest::new(imported.artifact_id(), 1, 1, MarkerProfileChoice::MathOcrV1);

    let parsed = service.parse_pdf(&request).unwrap();
    assert!(
        parsed
            .conversion_id()
            .starts_with("mathscape:conversion/marker/")
    );
    assert_eq!(parsed.outputs().len(), 4);
    let result = serde_json::to_value(&parsed).unwrap();
    let markdown_id = result["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|output| output["role"] == "primary_markdown")
        .unwrap()["artifact_id"]
        .as_str()
        .unwrap();
    // Read after the successful run's staging has already been removed.
    let reopened = DesktopService::open(&workspace).unwrap();
    let info = serde_json::to_value(reopened.content_info(markdown_id).unwrap()).unwrap();
    let paths = info["contexts"][0]["paths"].as_array().unwrap();
    assert_eq!(paths.len(), 4);
    let image = paths
        .iter()
        .find(|p| p["path"].as_str().unwrap().ends_with("/images/plot.png"))
        .unwrap();
    // The storage fixture uses a PNG signature; image decoding is covered in the browser test.
    let image_id = image["artifact_id"].as_str().unwrap();
    assert_eq!(
        reopened.read_artifact_bytes(image_id, 0, 1024).unwrap(),
        b"\x89PNG\r\n\x1a\n"
    );
    assert_eq!(
        serde_json::to_value(reopened.content_info(image_id).unwrap()).unwrap()["media_type"],
        "image/png"
    );
    assert_eq!(info["contexts"][0]["conversion_id"], parsed.conversion_id());
    // A historical accepted plan's source identity remains readable without
    // reconstructing any A/B checkpoint or fabricating validator authority.
    let run_id = StableId::parse("mathscape:test/marker-preview-run").unwrap();
    let decision_id = StableId::parse("mathscape:test/marker-preview-decision").unwrap();
    let fp = mathscape_domain::ContentHash::parse(format!("sha256:{}", "1".repeat(64))).unwrap();
    let mut store = SqliteArtifactStore::open(&workspace.join("mathscape.sqlite3")).unwrap();
    store
        .save_discovery_checkpoint(&mathscape_store_sqlite::DiscoveryRunCheckpoint {
            run_id: run_id.clone(),
            artifact_id: StableId::parse(markdown_id).unwrap(),
            conversion_id: Some(StableId::parse(parsed.conversion_id()).unwrap()),
            pipeline_version: "legacy-preview-only".into(),
            input_fingerprint: fp.clone(),
            selection_fingerprint: fp.clone(),
            unit_set_fingerprint: None,
            result_fingerprint: Some(fp.clone()),
            status: DiscoveryCheckpointStatus::Completed,
            admission: DiscoveryAdmission::ReviewBeforePublish,
            profile_id: "test-no-live-profile".into(),
            snapshot: serde_json::json!({}),
            attempts: vec![],
            reports: vec![],
            units: vec![],
            facts: vec![],
            pending_tasks: vec![],
        })
        .unwrap();
    store.save_registration_plan(&mathscape_store_sqlite::AcceptedRegistrationPlanRecord {
        decision_id:decision_id.clone(),run_id,actor:ActorRef::parse("test-user").unwrap(),decided_at:mathscape_domain::Timestamp::parse("unix:1").unwrap(),
        authority:mathscape_store_sqlite::StoredAcceptanceAuthority::HumanReview,expected_result_fingerprint:fp.clone(),selection:serde_json::json!({"kind":"all_validated"}),
        plan_fingerprint:fp,plan:serde_json::json!({"markdown_artifact_id":markdown_id,"conversion_id":parsed.conversion_id()}),
    }).unwrap();
    drop(store);
    let context = reopened
        .registration_source_context(decision_id.as_str())
        .unwrap();
    assert_eq!(context.artifact_id.as_str(), markdown_id);
    assert_eq!(
        context.conversion_id.as_ref().map(StableId::as_str),
        Some(parsed.conversion_id())
    );
    let anchored = reopened
        .markdown_document(
            context.artifact_id.as_str(),
            context.conversion_id.as_ref().map(StableId::as_str),
        )
        .unwrap();
    assert!(anchored.source.contains("Converted analysis"));
    assert!(anchored.blocks.len() >= 2);
    let map_path = paths
        .iter()
        .find(|path| path["path"].as_str().unwrap().ends_with("_pdf_map.json"))
        .unwrap();
    let map_id = map_path["artifact_id"].as_str().unwrap();
    let map_value: serde_json::Value =
        serde_json::from_slice(&reopened.read_artifact_bytes(map_id, 0, 64 * 1024).unwrap())
            .unwrap();
    assert_eq!(map_value["marker_page_ids"], serde_json::json!([0]));
    let backlink = reopened
        .get_source_backlink(mathscape_desktop::SourceBacklinkRequest {
            artifact_id: markdown_id.to_owned(),
            conversion_id: Some(parsed.conversion_id().to_owned()),
            start: 0,
            end: anchored.source.len() as u64,
            block_ids: anchored
                .blocks
                .iter()
                .map(|block| block.id.clone())
                .collect(),
        })
        .unwrap();
    assert!(matches!(
        backlink,
        mathscape_desktop::SourceBacklink::ExactPdfPages { .. }
    ));
    let request = DiscoverySelectionRequest {
        version: DISCOVERY_SELECTION_VERSION.into(),
        snapshot: anchored.snapshot.clone(),
        selector: SourceSelector::WholeDocument,
        admission: DiscoveryAdmission::ReviewBeforePublish,
    };
    let selected = reopened
        .resolve_markdown_selection(request.clone())
        .unwrap();
    assert_eq!(selected.raw_text, anchored.source);
    assert_eq!(selected.range.end, anchored.source.len() as u64);
    let discovery_input = reopened.prepare_discovery_input(request.clone()).unwrap();
    let reconstructed = discovery_input
        .requested
        .segments
        .iter()
        .map(mathscape_domain::DiscoveryInputSegment::text)
        .collect::<String>();
    assert_eq!(reconstructed, anchored.source);
    assert_eq!(
        discovery_input.requested.utf8_bytes(),
        anchored.source.len()
    );
    assert_eq!(
        discovery_input.requested.block_count(),
        anchored.blocks.len()
    );
    assert_eq!(
        discovery_input.selection_fingerprint,
        selected.input_fingerprint
    );
    let serialized_input = serde_json::to_value(&discovery_input).unwrap();
    assert_eq!(serialized_input["version"], "mathscape-discovery-input-2");
    assert!(serialized_input.get("admission").is_none());
    assert!(serialized_input.get("request").is_none());
    assert!(reopened.markdown_document(markdown_id, None).is_err());
    assert!(
        reopened
            .markdown_document(markdown_id, Some("test:foreign-conversion"))
            .is_err()
    );
    assert!(
        reopened
            .markdown_document(imported.artifact_id(), None)
            .is_err()
    );
    let mut invalid = request.clone();
    invalid.snapshot.anchor_scheme = "unknown-version".into();
    assert!(reopened.resolve_markdown_selection(invalid).is_err());
    let mut invalid = request;
    invalid.selector = SourceSelector::BlockRange {
        first: "foreign-block".into(),
        last: "foreign-block".into(),
    };
    assert!(reopened.resolve_markdown_selection(invalid).is_err());
    assert!(
        String::from_utf8(reopened.read_artifact_bytes(markdown_id, 0, 1024).unwrap())
            .unwrap()
            .contains("Converted analysis")
    );
    assert_eq!(parsed.snapshot().artifact_count(), 5);
    assert_eq!(parsed.snapshot().conversion_count(), 1);
    assert_eq!(parsed.snapshot().landscape_graph().nodes().len(), 6);
    assert_eq!(parsed.snapshot().landscape_graph().edges().len(), 5);
    assert_eq!(parsed.snapshot().pdf_artifacts().len(), 1);
    assert_eq!(parsed.snapshot().pdf_artifacts()[0].page_count(), Some(1));
    assert!(
        fs::read_dir(workspace.join("cache/marker-runs"))
            .unwrap()
            .next()
            .is_none()
    );

    directory.cleanup();
}

fn write_pdf(path: &Path) {
    let mut document = Document::with_version("1.5");
    let pages_id = document.new_object_id();
    let font_id = document.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let resources_id = document.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });
    let operations = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec!["F1".into(), 18.into()]),
        Operation::new("Td", vec![72.into(), 700.into()]),
        Operation::new(
            "Tj",
            vec![Object::string_literal("Mathscape desktop import test")],
        ),
        Operation::new("ET", vec![]),
    ];
    let content_id = document.add_object(Stream::new(
        dictionary! {},
        Content { operations }.encode().unwrap(),
    ));
    let page_id = document.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
    });
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        }),
    );
    let catalog_id = document.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    let info_id = document.add_object(dictionary! {
        "Title" => Object::string_literal("Desktop import fixture"),
    });
    document.trailer.set("Root", catalog_id);
    document.trailer.set("Info", info_id);
    document.compress();
    document.save(path).unwrap();
}

#[cfg(unix)]
fn write_executable(path: &Path, content: &str) {
    fs::write(path, content).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}
