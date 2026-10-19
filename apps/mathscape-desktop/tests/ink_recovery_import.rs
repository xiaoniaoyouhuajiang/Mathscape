//! PR #4 regression: the A1 unsaved-operations export ("导出到文件…") must be
//! importable through "导入恢复草稿", including when a newer writer already
//! extended the history past the point the queued operations were written on.
use mathscape_desktop::{
    AppendInkOperationRequest, CreateInkDocumentRequest, DesktopService, InkRecoveryImport,
};
use mathscape_domain::{InkLabDocument, InkLabOperation};
use mathscape_llm::MemoryCredentialResolver;
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

const USER_EXPORT: &str =
    include_str!("../../../testdata/ink/n2/unsaved-operations-epoch-conflict.json");
const TRACE: &str = include_str!("../../../testdata/ink/p0/trace.json");

struct Fixture {
    root: PathBuf,
    service: DesktopService,
}

impl Fixture {
    fn new(case: &str) -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".test-runs")
            .join(format!(
                "recovery-import-{case}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        fs::create_dir_all(&root).unwrap();
        let service = DesktopService::open_with_credential_store(
            root.join("workspace"),
            Arc::new(MemoryCredentialResolver::default()),
        )
        .unwrap();
        Self { root, service }
    }

    /// The stored note: `prefix` operations written by the current (newer)
    /// writer `session-new`.
    fn store(&self, document: &InkLabDocument, prefix: &[InkLabOperation]) {
        let mut empty = document.clone();
        empty.operations.clear();
        let receipt = self
            .service
            .create_ink_document(CreateInkDocumentRequest {
                document: empty.clone(),
                session_id: "session-new".into(),
                writer_id: "desktop-local".into(),
                device: json!({}),
                request_id: None,
                title_mode: None,
            })
            .unwrap();
        for (index, operation) in prefix.iter().enumerate() {
            self.service
                .append_ink_operation(AppendInkOperationRequest {
                    request_id: format!("{:032x}", index + 1),
                    document_id: document.document_id.clone(),
                    session_id: "session-new".into(),
                    writer_epoch: receipt.session.writer_epoch,
                    expected_head: index as u64,
                    metadata: empty.metadata(),
                    operation: operation.clone(),
                })
                .unwrap();
        }
    }

    fn import(&self, bytes: &[u8], request: &str) -> Result<InkRecoveryImport, String> {
        self.service.import_ink_recovery_file(bytes, request)
    }

    fn operations(&self, document_id: &str) -> Vec<InkLabOperation> {
        self.service
            .open_ink_document(document_id.into())
            .unwrap()
            .unwrap()
            .document
            .operations
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn stroke_ids(operations: &[InkLabOperation]) -> Vec<String> {
    operations
        .iter()
        .filter_map(|operation| match operation {
            InkLabOperation::Stroke { stroke, .. } => Some(stroke.id.clone()),
            _ => None,
        })
        .collect()
}

/// The first trace stroke with fresh identities, as another writer's stroke.
fn other_writer_stroke(trace: &InkLabDocument, seq: u32) -> InkLabOperation {
    let mut value = serde_json::to_value(&trace.operations[0]).unwrap();
    value["seq"] = json!(seq);
    value["command_id"] = json!(format!("{:032x}", 0xabc + seq));
    value["stroke"]["id"] = json!(format!("{:032x}", 0xdef + seq));
    serde_json::from_value(value).unwrap()
}

#[test]
fn the_reported_epoch_conflict_export_imports_as_a_recovery_draft() {
    let f = Fixture::new("reported");
    let export: Value = serde_json::from_str(USER_EXPORT).unwrap();
    let item = &export["items"][0];
    let exported_stroke = item["operation"]["stroke"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    // The stored note as the user had it: the old writer's stroke was never
    // saved, and a newer writer already wrote seq 1.
    let mut metadata = item["metadata"].clone();
    metadata["operations"] = json!([]);
    let document: InkLabDocument = serde_json::from_value(metadata).unwrap();
    let trace: InkLabDocument = serde_json::from_str(TRACE).unwrap();
    let mut newer = serde_json::to_value(other_writer_stroke(&trace, 1)).unwrap();
    newer["stroke"]["page_id"] = item["operation"]["stroke"]["page_id"].clone();
    newer["stroke"]["capture_segment_id"] =
        item["operation"]["stroke"]["capture_segment_id"].clone();
    let newer: InkLabOperation = serde_json::from_value(newer).unwrap();
    f.store(&document, std::slice::from_ref(&newer));

    let request = "1".repeat(32);
    let imported = f.import(USER_EXPORT.as_bytes(), &request).unwrap();
    assert_eq!(imported.source, "unsaved_operations");
    assert_eq!(
        imported.original_document_id.as_deref(),
        Some(document.document_id.as_str())
    );
    assert_eq!(imported.sessions.len(), 1);
    let draft = &imported.sessions[0];
    assert_ne!(draft.document_id, document.document_id);
    let recovered = f.operations(&draft.document_id);
    assert_eq!(stroke_ids(&recovered), vec![exported_stroke]);
    assert_eq!(recovered[0].seq(), 1);
    let stored = f
        .service
        .open_ink_document(draft.document_id.clone())
        .unwrap()
        .unwrap();
    assert!(
        stored.note.title.ends_with("恢复草稿"),
        "{}",
        stored.note.title
    );
    // The original keeps the newer writer's history untouched.
    assert_eq!(f.operations(&document.document_id), vec![newer]);
    // Retrying the same import is idempotent.
    let retry = f.import(USER_EXPORT.as_bytes(), &request).unwrap();
    assert_eq!(retry.sessions[0].session_id, draft.session_id);
}

/// Two writer queues: queue A was written on seq 1 and overlaps the newer
/// seq 2; queue B was written on an empty note. Items are listed out of order
/// on purpose.
fn two_queue_export(trace: &InkLabDocument) -> Vec<u8> {
    let metadata = serde_json::to_value(trace.metadata()).unwrap();
    let item = |request: u32,
                session: &str,
                head: u64,
                client: u64,
                operation: &InkLabOperation| {
        json!({
            "request_id": format!("{request:032x}"), "document_id": trace.document_id,
            "session_id": session, "writer_epoch": 1, "expected_head": head, "client_seq": client,
            "queued_at": "2026-10-18T09:36:06.081Z", "metadata": metadata,
            "operation": operation,
            "quarantined": { "code": "writer_epoch_conflict", "message": "", "at": "2026-10-18T09:36:06.101Z" }
        })
    };
    let export = json!({
        "format": "mathscape-ink-unsaved-operations", "version": 1,
        "document_id": trace.document_id, "exported_at": "2026-10-18T09:36:06.229Z",
        "items": [
            item(11, "session-old", 2, 3, &trace.operations[2]),
            item(10, "session-old", 1, 2, &trace.operations[1]),
            item(20, "session-other", 0, 1, &trace.operations[7]),
        ]
    });
    serde_json::to_vec(&export).unwrap()
}

#[test]
fn overlapping_queues_are_rebuilt_on_their_own_history_prefix() {
    let f = Fixture::new("overlap");
    let trace: InkLabDocument = serde_json::from_str(TRACE).unwrap();
    // Stored: trace seq 1 (shared), then a newer writer's stroke at seq 2.
    let newer = other_writer_stroke(&trace, 2);
    f.store(&trace, &[trace.operations[0].clone(), newer.clone()]);
    let bytes = two_queue_export(&trace);
    let imported = f.import(&bytes, &"2".repeat(32)).unwrap();
    assert_eq!(imported.sessions.len(), 2);
    let drafts: Vec<_> = imported
        .sessions
        .iter()
        .map(|session| f.operations(&session.document_id))
        .collect();
    // Queue A: the shared prefix, then its own operations (not the newer stroke).
    let a = drafts
        .iter()
        .find(|ops| ops.len() == 3)
        .expect("queue A draft");
    assert_eq!(a[0], trace.operations[0]);
    assert_eq!(stroke_ids(a), stroke_ids(&trace.operations[..3]));
    assert_eq!(
        a.iter().map(InkLabOperation::seq).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    // Queue B: only its stroke, renumbered onto the empty prefix.
    let b = drafts
        .iter()
        .find(|ops| ops.len() == 1)
        .expect("queue B draft");
    assert_eq!(stroke_ids(b), stroke_ids(&trace.operations[7..8]));
    assert_eq!(b[0].seq(), 1);
    assert_eq!(
        f.operations(&trace.document_id),
        vec![trace.operations[0].clone(), newer]
    );
    // Idempotent across both drafts.
    let retry = f.import(&bytes, &"2".repeat(32)).unwrap();
    assert_eq!(
        retry
            .sessions
            .iter()
            .map(|s| &s.session_id)
            .collect::<Vec<_>>(),
        imported
            .sessions
            .iter()
            .map(|s| &s.session_id)
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_queue_whose_history_prefix_is_gone_is_refused_and_documents_still_import() {
    let f = Fixture::new("missing");
    let trace: InkLabDocument = serde_json::from_str(TRACE).unwrap();
    let export = json!({
        "format": "mathscape-ink-unsaved-operations", "version": 1, "document_id": trace.document_id,
        "items": [{
            "request_id": "a".repeat(32), "document_id": trace.document_id, "session_id": "s",
            "writer_epoch": 1, "expected_head": 1, "client_seq": 2,
            "metadata": trace.metadata(), "operation": trace.operations[1]
        }]
    });
    let error = f
        .import(&serde_json::to_vec(&export).unwrap(), &"3".repeat(32))
        .unwrap_err();
    assert!(error.contains("前 1 个操作"), "{error}");
    // A plain Ink JSON document still imports through the same entry point.
    let imported = f.import(TRACE.as_bytes(), &"4".repeat(32)).unwrap();
    assert_eq!(imported.source, "document");
    assert_eq!(
        f.operations(&imported.sessions[0].document_id).len(),
        trace.operations.len()
    );
    // Anything else keeps the original parse error.
    let error = f
        .import(br#"{"format":"other"}"#, &"5".repeat(32))
        .unwrap_err();
    assert!(error.contains("Ink 恢复文件无效"), "{error}");
}

/// PR #4 review (P2): all drafts of a multi-queue import and their request
/// receipts commit in one transaction. A failure writing the second draft
/// must leave no draft and no receipt behind.
#[test]
fn a_failure_writing_the_second_draft_persists_no_draft_and_no_receipt() {
    let f = Fixture::new("atomic");
    let trace: InkLabDocument = serde_json::from_str(TRACE).unwrap();
    f.store(
        &trace,
        &[trace.operations[0].clone(), other_writer_stroke(&trace, 2)],
    );
    let bytes = two_queue_export(&trace);
    let database = f.root.join("workspace").join("mathscape.sqlite3");
    let counts = || -> (i64, i64) {
        let connection = rusqlite::Connection::open(&database).unwrap();
        let drafts = connection
            .query_row(
                "SELECT count(*) FROM ink_documents WHERE document_id LIKE 'recovery-%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let receipts = connection
            .query_row("SELECT count(*) FROM ink_recovery_imports", [], |r| {
                r.get(0)
            })
            .unwrap();
        (drafts, receipts)
    };
    // Fail the second draft: once the first draft's receipt exists in the
    // (still open) transaction, the next draft insert aborts.
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_second_recovery_draft BEFORE INSERT ON ink_documents
             WHEN NEW.document_id LIKE 'recovery-%'
              AND (SELECT count(*) FROM ink_recovery_imports) >= 1
             BEGIN SELECT RAISE(ABORT, 'injected second draft failure'); END;",
        )
        .unwrap();

    let request = "6".repeat(32);
    let error = f.import(&bytes, &request).unwrap_err();
    assert!(error.contains("injected second draft failure"), "{error}");
    assert_eq!(counts(), (0, 0), "no draft and no receipt may be persisted");
    assert_eq!(f.operations(&trace.document_id).len(), 2);

    // Once the failure is gone, the same request imports both drafts.
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch("DROP TRIGGER fail_second_recovery_draft;")
        .unwrap();
    let imported = f.import(&bytes, &request).unwrap();
    assert_eq!(imported.sessions.len(), 2);
    assert_eq!(counts(), (2, 2));
}
