//! S0 sandbox sessions against a real Desktop workspace (docs/sandbox-flow.md §5 S0).

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use mathscape_desktop::{CreateInkDocumentRequest, DesktopService};
use mathscape_domain::InkLabDocument;
use mathscape_llm::MemoryCredentialResolver;
use mathscape_sandbox::{SandboxHome, SessionState, fingerprint_workspace, read_export_manifest};

fn create_ink(service: &DesktopService, document_id: &str) {
    let mut document: InkLabDocument =
        serde_json::from_str(include_str!("../../../testdata/ink/p0/trace.json")).unwrap();
    document.operations.clear();
    document.document_id = document_id.into();
    service
        .create_ink_document(CreateInkDocumentRequest {
            document,
            session_id: format!("session-{document_id}"),
            writer_id: "tablet-s0".into(),
            device: serde_json::json!({}),
            request_id: None,
            title_mode: None,
        })
        .unwrap();
}

fn ink_ids(service: &DesktopService) -> Vec<String> {
    let mut ids: Vec<String> = service
        .list_ink_documents()
        .unwrap()
        .into_iter()
        .map(|summary| summary.document_id)
        .collect();
    ids.sort();
    ids
}

fn test_root(case: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".test-runs")
        .join(format!("{case}-{nanos}-{}", std::process::id()));
    fs::create_dir_all(&path).unwrap();
    path
}

fn open(workspace: &Path) -> DesktopService {
    DesktopService::open_with_credential_store(
        workspace,
        Arc::new(MemoryCredentialResolver::default()),
    )
    .unwrap()
}

#[test]
fn session_copy_is_a_working_workspace_and_discard_leaves_the_checkpoint_untouched() {
    let root = test_root("sandbox-s0");
    let checkpoint = root.join("checkpoint");
    {
        let service = open(&checkpoint);
        create_ink(&service, "ink-in-checkpoint");
    }
    let before = fingerprint_workspace(&checkpoint).unwrap();

    let home = SandboxHome::open(root.join("sandbox")).unwrap();
    let record = home
        .register_checkpoint(&checkpoint, Some("S0 测试"))
        .unwrap();
    let session = home.start_session(&record.checkpoint_id).unwrap();
    assert_eq!(session.state, SessionState::Active);
    let copy = session.workspace_root.clone().unwrap();

    // The copy opens as a normal Desktop workspace and can diverge freely.
    {
        let sandboxed = open(&copy);
        assert_eq!(ink_ids(&sandboxed), vec!["ink-in-checkpoint".to_owned()]);
        create_ink(&sandboxed, "ink-only-in-sandbox");
        assert_eq!(ink_ids(&sandboxed).len(), 2);
    }
    // Byte-level proof first: the sandbox never wrote into the checkpoint.
    assert!(
        fingerprint_workspace(&checkpoint)
            .unwrap()
            .same_content(&before)
    );

    let export = home
        .export_session(&session.sandbox_session_id, None, Some("S0 desktop test"))
        .unwrap();
    let manifest = read_export_manifest(&export.export_dir).unwrap();
    assert_eq!(manifest.schema, "mathscape.sandbox_export.v0");
    assert_eq!(manifest.checkpoint_fingerprint.digest, before.digest);

    let discarded = home.discard_session(&session.sandbox_session_id).unwrap();
    assert_eq!(discarded.state, SessionState::Discarded);
    assert!(!copy.exists());
    assert!(export.export_dir.join("manifest.json").is_file());
    let after = fingerprint_workspace(&checkpoint).unwrap();
    assert!(after.same_content(&before), "checkpoint must never change");
    // Logical proof last (opening SQLite may fold the WAL and change bytes).
    {
        let original = open(&checkpoint);
        assert_eq!(ink_ids(&original), vec!["ink-in-checkpoint".to_owned()]);
    }

    fs::remove_dir_all(&root).unwrap();
}
