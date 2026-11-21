use mathscape_domain::{InkLabDocument, InkLabOperation};
use mathscape_store_sqlite::{SqliteArtifactStore, SqliteStoreError};
use rusqlite::Connection;
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

fn empty_fixture() -> InkLabDocument {
    let mut document: InkLabDocument =
        serde_json::from_str(include_str!("../../../testdata/ink/p0/trace.json")).unwrap();
    document.operations.clear();
    document
}

fn operation(index: usize) -> InkLabOperation {
    let document: InkLabDocument =
        serde_json::from_str(include_str!("../../../testdata/ink/p0/trace.json")).unwrap();
    document.operations[index].clone()
}

#[test]
fn persists_ordered_ink_operations_and_replays_requests_idempotently() {
    let mut store = SqliteArtifactStore::open_in_memory().unwrap();
    let document = empty_fixture();
    let session = store
        .create_ink_document(
            &document,
            "session-1",
            "tablet-1",
            &serde_json::json!({"model":"Tab S8+","browser":"Chrome"}),
        )
        .unwrap();
    assert_eq!(session.writer_epoch, 1);

    let first = store
        .append_ink_operation(
            "request-1",
            &document.document_id,
            &session.session_id,
            session.writer_epoch,
            0,
            &document.metadata(),
            &operation(0),
        )
        .unwrap();
    assert_eq!(first.seq, 1);
    assert!(first.durable);
    assert!(!first.replayed);

    let replay = store
        .append_ink_operation(
            "request-1",
            &document.document_id,
            &session.session_id,
            session.writer_epoch,
            0,
            &document.metadata(),
            &operation(0),
        )
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.command_id, first.command_id);

    let opened = store
        .open_ink_document(&document.document_id)
        .unwrap()
        .unwrap();
    assert_eq!(opened.head_seq, 1);
    assert_eq!(opened.document.operations.len(), 1);

    let conflict = store.append_ink_operation(
        "request-1",
        &document.document_id,
        &session.session_id,
        session.writer_epoch,
        0,
        &document.metadata(),
        &operation(1),
    );
    assert!(matches!(
        conflict,
        Err(SqliteStoreError::InkRequestConflict(_))
    ));

    let head_conflict = store.append_ink_operation(
        "request-2",
        &document.document_id,
        &session.session_id,
        session.writer_epoch,
        0,
        &document.metadata(),
        &operation(1),
    );
    assert!(matches!(
        head_conflict,
        Err(SqliteStoreError::InkHeadConflict {
            expected: 0,
            actual: 1
        })
    ));
}

#[test]
fn session_epoch_and_recovery_reads_are_durable() {
    let mut store = SqliteArtifactStore::open_in_memory().unwrap();
    let document = empty_fixture();
    let first = store
        .create_ink_document(
            &document,
            "session-1",
            "tablet-1",
            &serde_json::json!({"kind":"browser"}),
        )
        .unwrap();
    store.close_ink_session(&first.session_id).unwrap();
    let second = store
        .start_ink_session(
            &document.document_id,
            "session-2",
            "tablet-1",
            &serde_json::json!({"kind":"browser","resumed":true}),
        )
        .unwrap();
    assert_eq!(second.writer_epoch, 2);

    let receipt = store
        .append_ink_operation(
            "request-2",
            &document.document_id,
            &second.session_id,
            second.writer_epoch,
            0,
            &document.metadata(),
            &operation(0),
        )
        .unwrap();
    assert_eq!(receipt.seq, 1);

    let summaries = store.list_ink_documents().unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].head_seq, 1);
    assert_eq!(summaries[0].session_count, 2);
    assert!(summaries[0].active_session_id.as_deref() == Some("session-2"));
}

#[test]
fn closed_session_can_resume_with_same_epoch_after_restart() {
    let mut store = SqliteArtifactStore::open_in_memory().unwrap();
    let document = empty_fixture();
    let session = store
        .create_ink_document(
            &document,
            "session-restart",
            "desktop-local",
            &serde_json::json!({"source":"desktop"}),
        )
        .unwrap();
    store.close_ink_session(&session.session_id).unwrap();
    let resumed = store
        .start_ink_session(
            &document.document_id,
            &session.session_id,
            "desktop-local",
            &serde_json::json!({"source":"desktop","resumed":true}),
        )
        .unwrap();
    assert_eq!(resumed.writer_epoch, session.writer_epoch);
    assert_eq!(resumed.status, "active");
    let receipt = store
        .append_ink_operation(
            "request-restart",
            &document.document_id,
            &resumed.session_id,
            resumed.writer_epoch,
            0,
            &document.metadata(),
            &operation(0),
        )
        .unwrap();
    assert_eq!(receipt.seq, 1);
    assert_eq!(
        store
            .list_ink_sessions(&document.document_id)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn failed_request_rolls_back_operation_head_and_can_retry() {
    let path = std::env::temp_dir().join(format!(
        "mathscape-ink-p1-{}-{}.sqlite3",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut store = SqliteArtifactStore::prepare(&path).unwrap();
    let document = empty_fixture();
    let session = store
        .create_ink_document(
            &document,
            "session-failure",
            "desktop-local",
            &serde_json::json!({}),
        )
        .unwrap();
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER fail_ink_request BEFORE INSERT ON ink_requests
         BEGIN SELECT RAISE(ABORT, 'injected ink request failure'); END;",
        )
        .unwrap();
    let failed = store.append_ink_operation(
        "request-failure",
        &document.document_id,
        &session.session_id,
        session.writer_epoch,
        0,
        &document.metadata(),
        &operation(0),
    );
    assert!(matches!(failed, Err(SqliteStoreError::Sqlite(_))));
    assert_eq!(
        store
            .open_ink_document(&document.document_id)
            .unwrap()
            .unwrap()
            .head_seq,
        0
    );
    connection
        .execute_batch("DROP TRIGGER fail_ink_request;")
        .unwrap();
    let retry = store
        .append_ink_operation(
            "request-failure",
            &document.document_id,
            &session.session_id,
            session.writer_epoch,
            0,
            &document.metadata(),
            &operation(0),
        )
        .unwrap();
    assert_eq!(retry.seq, 1);
    drop(connection);
    drop(store);
    let _ = fs::remove_file(path);
}
