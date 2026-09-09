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
    DesktopDiscoveryError, DesktopService, DesktopServiceError, MarkerProfileChoice,
    ParsePdfRequest, PdfImportOutcome, RunDiscoveryRequest, SaveWorkspaceSettingsRequest,
};
use mathscape_domain::{
    ActorRef, DISCOVERY_SELECTION_VERSION, DiscoveryAdmission, DiscoverySelectionRequest,
    MediaType, RightsRecord, RightsStatus, SourceSelector,
};
use mathscape_llm::{CredentialRef, CredentialResolver, MemoryCredentialResolver};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

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
    const PROPOSAL: &str =
        include_str!("../../../crates/mathscape-domain/tests/fixtures/discovery-proposal.json");

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
    let input = service.prepare_discovery_input(selection.clone()).unwrap();
    let proposal = PROPOSAL
        .replace("b-problem", &document.blocks[0].id)
        .replace("b-solution", &document.blocks[1].id)
        .replace(
            "sha256:1111111111111111111111111111111111111111111111111111111111111111",
            input.fingerprint.as_str(),
        );
    let stream_chunk = serde_json::json!({
        "id": "chatcmpl-discovery-fixture",
        "object": "chat.completion.chunk",
        "created": 1,
        "model": "resolved-discovery-model",
        "choices": [{
            "index": 0,
            "delta": {"role": "assistant", "content": proposal},
            "finish_reason": "stop"
        }]
    });
    let stream_body = format!("data: {stream_chunk}\n\ndata: [DONE]\n\n");
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer desktop-discovery-secret"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_millis(120))
                .insert_header("content-type", "text/event-stream")
                .set_body_string(stream_body),
        )
        .expect(1)
        .mount(&server)
        .await;

    let request = RunDiscoveryRequest::new(selection, None);
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
        service.run_discovery(request).await,
        Err(DesktopDiscoveryError::AlreadyRunning)
    ));

    let outcome = worker.await.unwrap().unwrap();
    assert!(service.active_discovery().is_none());
    let value = serde_json::to_value(outcome).unwrap();
    assert_eq!(value["outcome"], "valid");
    assert_eq!(value["proposal"]["status"], "complete");
    assert_eq!(value["proposal"]["units"].as_array().unwrap().len(), 2);
    assert_eq!(value["proposal"]["relations"][0]["relation"], "solves");
    assert_eq!(
        value["run"]["response"]["provider_model"],
        "discovery-fixture-model"
    );
    assert!(value["run"]["response"]["total_tokens"].is_null());
    assert_eq!(value["run"]["admission"], "review_before_publish");
    assert_eq!(value["run"]["request_timeout_seconds"], 900);
    assert_eq!(value["run"]["connect_timeout_seconds"], 20);
    assert_eq!(service.snapshot().unwrap().graph().nodes().len(), 1);
    let received = server.received_requests().await.unwrap();
    let body: serde_json::Value = received[0].body_json().unwrap();
    assert_eq!(body["stream"], true);
    assert!(body.get("stream_options").is_none());
    let fingerprint_schema =
        &body["response_format"]["json_schema"]["schema"]["properties"]["input_fingerprint"];
    assert!(fingerprint_schema.get("$ref").is_some());
    assert!(fingerprint_schema.get("description").is_none());
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
    assert!(initial.graph().nodes().is_empty());

    let imported = service.import_pdf(&source).unwrap();
    assert_eq!(imported.outcome(), PdfImportOutcome::Inserted);
    assert_eq!(imported.page_count(), 1);
    assert_eq!(imported.snapshot().artifact_count(), 1);
    assert_eq!(imported.snapshot().conversion_count(), 0);
    assert_eq!(imported.snapshot().graph().nodes().len(), 1);
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
while [ "$#" -gt 0 ]; do
    case "$1" in
        --output_dir) output_root="$2"; shift 2 ;;
        *) shift ;;
    esac
done
source_name="${source_path##*/}"
source_stem="${source_name%.*}"
mkdir -p "$output_root/$source_stem/images"
printf '# Converted analysis\n![plot](images/plot.png)\n' > "$output_root/$source_stem/$source_stem.md"
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
    assert_eq!(parsed.outputs().len(), 3);
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
    assert_eq!(paths.len(), 3);
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
    let anchored = reopened
        .markdown_document(markdown_id, Some(parsed.conversion_id()))
        .unwrap();
    assert!(anchored.source.contains("Converted analysis"));
    assert_eq!(anchored.blocks.len(), 2);
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
    assert_eq!(parsed.snapshot().artifact_count(), 4);
    assert_eq!(parsed.snapshot().conversion_count(), 1);
    assert_eq!(parsed.snapshot().graph().nodes().len(), 5);
    assert_eq!(parsed.snapshot().graph().edges().len(), 4);
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
