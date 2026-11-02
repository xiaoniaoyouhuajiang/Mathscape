//! N1 note metadata: title authority, rename, trash/restore and clean-up.
use chrono::NaiveDate;
use mathscape_domain::{InkLabDocument, InkLabOperation};
use mathscape_store_sqlite::{
    InkCleanBlankRequest, InkLifecycle, InkLifecycleRequest, InkPairingOffer, InkRenameRequest,
    InkTitleMode, SqliteArtifactStore, SqliteStoreError, format_ink_auto_title, ink_auto_title,
    parse_ink_timestamp_ms,
};
use rusqlite::Connection;
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

fn fixture(id: &str) -> InkLabDocument {
    let mut document: InkLabDocument =
        serde_json::from_str(include_str!("../../../testdata/ink/p0/trace.json")).unwrap();
    document.operations.clear();
    document.document_id = id.into();
    document
}

fn operation(index: usize) -> InkLabOperation {
    let document: InkLabDocument =
        serde_json::from_str(include_str!("../../../testdata/ink/p0/trace.json")).unwrap();
    document.operations[index].clone()
}

fn test_directory(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("mathscape-ink-note-{name}-{nanos}"));
    fs::create_dir_all(&directory).unwrap();
    directory
}

fn create(store: &mut SqliteArtifactStore, id: &str, mode: InkTitleMode) -> InkLabDocument {
    let document = fixture(id);
    store
        .create_ink_note(
            &document,
            &format!("{id}-session"),
            "desktop-local",
            &json!({"source":"desktop"}),
            mode,
            None,
        )
        .unwrap();
    document
}

fn append(store: &mut SqliteArtifactStore, document: &InkLabDocument, request: &str, seq: u64) {
    let session = format!("{}-session", document.document_id);
    let epoch = store.ink_session(&session).unwrap().unwrap().writer_epoch;
    store
        .append_ink_operation(
            request,
            &document.document_id,
            &session,
            epoch,
            seq,
            &document.metadata(),
            &operation(seq as usize),
        )
        .unwrap();
}

fn rename(id: &str, request: &str, title: &str, expected: u64) -> InkRenameRequest {
    InkRenameRequest {
        request_id: request.into(),
        document_id: id.into(),
        title: title.into(),
        expected_title_revision: expected,
    }
}

fn lifecycle(id: &str, request: &str, expected: u64) -> InkLifecycleRequest {
    InkLifecycleRequest {
        request_id: request.into(),
        document_id: id.into(),
        expected_lifecycle_revision: expected,
    }
}

#[test]
fn auto_titles_follow_the_local_creation_time() {
    let at = NaiveDate::from_ymd_opt(2026, 3, 7)
        .unwrap()
        .and_hms_opt(9, 5, 0)
        .unwrap();
    assert_eq!(format_ink_auto_title(at, 2026), "手稿 · 3月7日 09:05");
    assert_eq!(format_ink_auto_title(at, 2027), "手稿 · 2026年3月7日 09:05");
    assert_eq!(parse_ink_timestamp_ms("unix:12.5"), Some(12_500));
    assert_eq!(
        parse_ink_timestamp_ms("2026-10-09T01:00:00.000Z"),
        Some(1_791_507_600_000)
    );
    assert_eq!(parse_ink_timestamp_ms("yesterday"), None);
    assert!(
        ink_auto_title("2026-10-09T01:00:00.000Z")
            .unwrap()
            .starts_with("手稿 · ")
    );
}

#[test]
fn create_is_idempotent_and_materializes_the_auto_title() {
    let mut store = SqliteArtifactStore::open_in_memory().unwrap();
    let document = fixture("auto-doc");
    let device = json!({"source":"desktop"});
    let first = store
        .create_ink_note(
            &document,
            "auto-session",
            "desktop-local",
            &device,
            InkTitleMode::Auto,
            Some("create-1"),
        )
        .unwrap();
    assert!(!first.replayed);
    assert_eq!(first.note.title_mode, InkTitleMode::Auto);
    assert_eq!(
        Some(first.note.title.clone()),
        ink_auto_title(&document.created_at)
    );
    let replay = store
        .create_ink_note(
            &document,
            "auto-session",
            "desktop-local",
            &device,
            InkTitleMode::Auto,
            Some("create-1"),
        )
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.note, first.note);
    assert_eq!(replay.session.session_id, first.session.session_id);
    assert_eq!(store.list_ink_documents().unwrap().len(), 1);

    let mut other = fixture("other-doc");
    other.title = "另一份".into();
    let conflict = store.create_ink_note(
        &other,
        "other-session",
        "desktop-local",
        &device,
        InkTitleMode::Manual,
        Some("create-1"),
    );
    assert!(matches!(
        conflict,
        Err(SqliteStoreError::InkRequestConflict(_))
    ));

    // The legacy entry point stays manual and keeps the submitted title.
    let legacy = fixture("legacy-doc");
    store
        .create_ink_document(&legacy, "legacy-session", "desktop-local", &device)
        .unwrap();
    let note = store.ink_note("legacy-doc").unwrap().unwrap();
    assert_eq!(note.title_mode, InkTitleMode::Manual);
    assert_eq!(note.title, "Synthetic ink trace");
    assert!(note.last_used_at > 0);
}

#[test]
fn old_operation_metadata_never_overwrites_a_rename() {
    let mut store = SqliteArtifactStore::open_in_memory().unwrap();
    let document = create(&mut store, "doc", InkTitleMode::Manual);
    append(&mut store, &document, "op-1", 0);
    let before = store.ink_note("doc").unwrap().unwrap();

    let renamed = store
        .rename_ink_note(&rename("doc", "rename-1", "  极限练习  ", 0))
        .unwrap();
    assert_eq!(renamed.note.title, "极限练习");
    assert_eq!(renamed.note.title_mode, InkTitleMode::Manual);
    assert_eq!(renamed.note.title_revision, 1);
    // Rename is not a content edit (D9).
    assert_eq!(renamed.note.last_used_at, before.last_used_at);
    let opened = store.open_ink_document("doc").unwrap().unwrap();
    assert_eq!(opened.head_seq, 1);
    assert_eq!(opened.document.title, "极限练习");

    // A pre-rename request replays with its original fingerprint, and a new
    // operation that still carries the old title leaves the title alone.
    let session = store.ink_session("doc-session").unwrap().unwrap();
    let replay = store
        .append_ink_operation(
            "op-1",
            "doc",
            "doc-session",
            session.writer_epoch,
            0,
            &document.metadata(),
            &operation(0),
        )
        .unwrap();
    assert!(replay.replayed);
    append(&mut store, &document, "op-2", 1);
    let opened = store.open_ink_document("doc").unwrap().unwrap();
    assert_eq!(opened.head_seq, 2);
    assert_eq!(opened.document.title, "极限练习");
    assert_eq!(opened.note.title_revision, 1);
    assert!(opened.note.last_used_at >= before.last_used_at);
    assert_eq!(store.list_ink_documents().unwrap()[0].title, "极限练习");
}

#[test]
fn rename_checks_revision_validates_and_replays() {
    let mut store = SqliteArtifactStore::open_in_memory().unwrap();
    create(&mut store, "doc", InkTitleMode::Auto);
    let auto = store.ink_note("doc").unwrap().unwrap().title;

    let first = store
        .rename_ink_note(&rename("doc", "r1", "第一稿", 0))
        .unwrap();
    assert_eq!(first.note.title_revision, 1);
    let replay = store
        .rename_ink_note(&rename("doc", "r1", "第一稿", 0))
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.note.title_revision, 1);
    assert!(matches!(
        store.rename_ink_note(&rename("doc", "r1", "别的", 0)),
        Err(SqliteStoreError::InkRequestConflict(_))
    ));

    match store.rename_ink_note(&rename("doc", "r2", "第二稿", 0)) {
        Err(SqliteStoreError::InkTitleConflict {
            current_title,
            current_revision,
            current_title_mode,
            ..
        }) => {
            assert_eq!(current_title, "第一稿");
            assert_eq!(current_revision, 1);
            assert_eq!(current_title_mode, "manual");
        }
        other => panic!("expected title conflict, got {other:?}"),
    }

    let error = store
        .rename_ink_note(&rename("doc", "r3", &"长".repeat(101), 1))
        .unwrap_err();
    assert_eq!(error.ink_code(), "title_invalid");
    assert!(
        store
            .rename_ink_note(&rename("doc", "r4", "a\u{7}b", 1))
            .is_err()
    );
    assert!(
        store
            .rename_ink_note(&rename("doc", "r5", &"长".repeat(100), 1))
            .is_ok()
    );

    // Unchanged titles do not advance the revision.
    let same = store
        .rename_ink_note(&rename("doc", "r6", &"长".repeat(100), 2))
        .unwrap();
    assert_eq!(same.note.title_revision, 2);

    // Empty restores the automatic title.
    let reset = store
        .rename_ink_note(&rename("doc", "r7", "   ", 2))
        .unwrap();
    assert_eq!(reset.note.title, auto);
    assert_eq!(reset.note.title_mode, InkTitleMode::Auto);
    assert_eq!(reset.note.title_revision, 3);
}

#[test]
fn trash_fences_writers_and_restore_requires_a_new_session() {
    let mut store = SqliteArtifactStore::open_in_memory().unwrap();
    let document = create(&mut store, "doc", InkTitleMode::Manual);
    append(&mut store, &document, "op-1", 0);
    let old = store.ink_session("doc-session").unwrap().unwrap();
    assert!(
        store
            .reserve_ink_snapshot("snap-before", "doc", 1, "2026-10-09T02:00:00.000Z")
            .is_ok()
    );

    let impact = store.ink_trash_impact("doc").unwrap().unwrap();
    assert_eq!(impact.head_seq, 1);
    assert_eq!(impact.snapshot_count, 1);
    assert!(!impact.blank);

    let trashed = store.trash_ink_note(&lifecycle("doc", "t1", 0)).unwrap();
    assert_eq!(trashed.note.lifecycle, InkLifecycle::Trashed);
    assert_eq!(trashed.note.lifecycle_revision, 1);
    assert!(trashed.note.trashed_at.is_some());
    assert_eq!(trashed.closed_sessions, 1);
    assert!(store.list_ink_documents().unwrap().is_empty());
    assert_eq!(store.list_trashed_ink_documents().unwrap().len(), 1);
    // Trashed documents stay readable.
    assert_eq!(store.open_ink_document("doc").unwrap().unwrap().head_seq, 1);

    let replay = store.trash_ink_note(&lifecycle("doc", "t1", 0)).unwrap();
    assert!(replay.replayed);
    let again = store.trash_ink_note(&lifecycle("doc", "t2", 0)).unwrap();
    assert!(!again.replayed);
    assert_eq!(again.note.lifecycle_revision, 1);

    let write = store.append_ink_operation(
        "op-2",
        "doc",
        "doc-session",
        old.writer_epoch,
        1,
        &document.metadata(),
        &operation(1),
    );
    assert!(matches!(
        write,
        Err(SqliteStoreError::InkDocumentTrashed(_))
    ));
    assert_eq!(write.unwrap_err().ink_code(), "document_trashed");
    // A request accepted before the trash still replays.
    assert!(
        store
            .append_ink_operation(
                "op-1",
                "doc",
                "doc-session",
                old.writer_epoch,
                0,
                &document.metadata(),
                &operation(0),
            )
            .unwrap()
            .replayed
    );
    assert!(matches!(
        store.reserve_ink_snapshot("snap-after", "doc", 1, "2026-10-09T03:00:00.000Z"),
        Err(SqliteStoreError::InkDocumentTrashed(_))
    ));
    assert!(matches!(
        store.rename_ink_note(&rename("doc", "rename-trashed", "x", 0)),
        Err(SqliteStoreError::InkDocumentTrashed(_))
    ));
    assert!(matches!(
        store.start_ink_session("doc", "doc-session", "desktop-local", &json!({})),
        Err(SqliteStoreError::InkDocumentTrashed(_))
    ));
    let offer = InkPairingOffer {
        token_hash: "a".repeat(64),
        document_id: "doc".into(),
        expected_head: 1,
        expected_epoch: old.writer_epoch + 1,
        session_id: "pad-session".into(),
        writer_id: "pad".into(),
        expires_at: 1_000 + 60,
        allow_transfer: false,
        claim_reason: "tablet_pairing".into(),
    };
    assert!(matches!(
        store.issue_ink_pairing(&offer, 1_000),
        Err(SqliteStoreError::InkDocumentTrashed(_))
    ));

    // Restore: stale revision conflicts, matching revision succeeds.
    assert!(matches!(
        store.restore_ink_note(&lifecycle("doc", "restore-stale", 0)),
        Err(SqliteStoreError::InkLifecycleConflict { .. })
    ));
    let restored = store
        .restore_ink_note(&lifecycle("doc", "restore-1", 1))
        .unwrap();
    assert_eq!(restored.note.lifecycle, InkLifecycle::Active);
    assert_eq!(restored.note.lifecycle_revision, 2);
    assert!(restored.note.trashed_at.is_none());
    assert!(!restored.requires_repairing);

    // The pre-trash session belongs to an older epoch and cannot be revived.
    let resumed = store.start_ink_session("doc", "doc-session", "desktop-local", &json!({}));
    assert!(matches!(
        resumed,
        Err(SqliteStoreError::InkWriterEpochConflict { .. })
    ));
    let write = store.append_ink_operation(
        "op-2",
        "doc",
        "doc-session",
        old.writer_epoch,
        1,
        &document.metadata(),
        &operation(1),
    );
    assert!(write.is_err());

    let fresh = store
        .start_ink_session("doc", "doc-session-2", "desktop-local", &json!({}))
        .unwrap();
    assert!(fresh.writer_epoch > old.writer_epoch + 1);
    store
        .append_ink_operation(
            "op-2",
            "doc",
            "doc-session-2",
            fresh.writer_epoch,
            1,
            &document.metadata(),
            &operation(1),
        )
        .unwrap();
}

#[test]
fn takeover_epoch_prevents_resurrecting_an_old_closed_session() {
    let mut store = SqliteArtifactStore::open_in_memory().unwrap();
    create(&mut store, "doc", InkTitleMode::Manual);
    store.close_ink_session("doc-session").unwrap();
    let next = store
        .start_ink_session("doc", "doc-session-b", "desktop-local", &json!({}))
        .unwrap();
    assert_eq!(next.writer_epoch, 2);
    store.close_ink_session("doc-session-b").unwrap();
    assert!(matches!(
        store.start_ink_session("doc", "doc-session", "desktop-local", &json!({})),
        Err(SqliteStoreError::InkWriterEpochConflict { .. })
    ));
    // The session of the current epoch still resumes (restart case).
    let resumed = store
        .start_ink_session("doc", "doc-session-b", "desktop-local", &json!({}))
        .unwrap();
    assert_eq!(resumed.writer_epoch, 2);
}

#[test]
fn clean_blank_only_trashes_untouched_auto_documents() {
    let mut store = SqliteArtifactStore::open_in_memory().unwrap();
    create(&mut store, "blank", InkTitleMode::Auto);
    let inked = create(&mut store, "inked", InkTitleMode::Auto);
    append(&mut store, &inked, "op-1", 0);
    create(&mut store, "manual", InkTitleMode::Manual);
    create(&mut store, "snap", InkTitleMode::Auto);
    store
        .reserve_ink_snapshot("snap-1", "snap", 0, "2026-10-09T02:00:00.000Z")
        .unwrap();
    assert!(store.ink_trash_impact("blank").unwrap().unwrap().blank);

    let request = InkCleanBlankRequest {
        request_id: "clean-1".into(),
        document_ids: vec![
            "snap".into(),
            "blank".into(),
            "inked".into(),
            "manual".into(),
            "missing".into(),
            "blank".into(),
        ],
    };
    let receipt = store.clean_blank_ink_notes(&request).unwrap();
    assert_eq!(
        receipt
            .trashed
            .iter()
            .map(|note| note.document_id.as_str())
            .collect::<Vec<_>>(),
        ["blank"]
    );
    let skipped = receipt
        .skipped
        .iter()
        .map(|skip| (skip.document_id.as_str(), skip.reason.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        skipped,
        [
            ("inked", "has_ink"),
            ("manual", "manual_title"),
            ("missing", "not_found"),
            ("snap", "has_snapshot"),
        ]
    );
    assert!(store.clean_blank_ink_notes(&request).unwrap().replayed);
    let active = store.list_ink_documents().unwrap();
    assert_eq!(active.len(), 3);
    assert!(active.iter().all(|summary| summary.document_id != "blank"));
}

#[test]
fn active_list_is_ordered_by_last_ink_use() {
    let mut store = SqliteArtifactStore::open_in_memory().unwrap();
    let first = create(&mut store, "first", InkTitleMode::Manual);
    std::thread::sleep(std::time::Duration::from_millis(5));
    create(&mut store, "second", InkTitleMode::Manual);
    assert_eq!(store.list_ink_documents().unwrap()[0].document_id, "second");
    std::thread::sleep(std::time::Duration::from_millis(5));
    append(&mut store, &first, "op-1", 0);
    let list = store.list_ink_documents().unwrap();
    assert_eq!(list[0].document_id, "first");
    assert!(list[0].page_count >= 1);
    assert_eq!(list[0].lifecycle, InkLifecycle::Active);
    // Renaming does not reorder (D9).
    store
        .rename_ink_note(&rename("second", "r", "新名字", 0))
        .unwrap();
    assert_eq!(store.list_ink_documents().unwrap()[0].document_id, "first");
}

fn downgrade_to_v16(path: &std::path::Path) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(
            "DROP TABLE ink_note_links;
             DELETE FROM schema_migrations WHERE version IN (18,19);
             ALTER TABLE ink_lan_settings DROP COLUMN preferred_host;
             ALTER TABLE ink_pairings DROP COLUMN claim_reason;
             DROP INDEX ink_documents_by_last_used;
             DROP INDEX ink_documents_by_trashed;
             DROP TABLE ink_note_requests;
             ALTER TABLE ink_documents DROP COLUMN trashed_at;
             ALTER TABLE ink_documents DROP COLUMN last_used_at;
             ALTER TABLE ink_documents DROP COLUMN lifecycle_revision;
             ALTER TABLE ink_documents DROP COLUMN lifecycle;
             ALTER TABLE ink_documents DROP COLUMN title_revision;
             ALTER TABLE ink_documents DROP COLUMN title_mode;
             DELETE FROM schema_migrations WHERE version=17;
             PRAGMA user_version=16;",
        )
        .unwrap();
}

#[test]
fn upgrading_v16_backfills_titles_and_keeps_old_requests_replayable() {
    let directory = test_directory("upgrade");
    let database = directory.join("catalog.sqlite3");
    let mut store = SqliteArtifactStore::prepare(&database).unwrap();
    let mut default = fixture("default-doc");
    default.title = "数学手稿".into();
    store
        .create_ink_document(&default, "default-session", "desktop-local", &json!({}))
        .unwrap();
    store
        .append_ink_operation(
            "legacy-op",
            "default-doc",
            "default-session",
            1,
            0,
            &default.metadata(),
            &operation(0),
        )
        .unwrap();
    let mut named = fixture("named-doc");
    named.title = "函数作业".into();
    store
        .create_ink_document(&named, "named-session", "desktop-local", &json!({}))
        .unwrap();
    let mut odd = fixture("odd-date-doc");
    odd.title = "数学手稿".into();
    odd.created_at = "not-a-date".into();
    store
        .create_ink_document(&odd, "odd-session", "desktop-local", &json!({}))
        .unwrap();
    drop(store);
    downgrade_to_v16(&database);
    assert!(matches!(
        SqliteArtifactStore::open(&database),
        Err(SqliteStoreError::UnsupportedSchemaVersion { actual: 16, .. })
    ));

    let mut store = SqliteArtifactStore::prepare(&database).unwrap();
    let default_note = store.ink_note("default-doc").unwrap().unwrap();
    assert_eq!(default_note.title_mode, InkTitleMode::Auto);
    assert_eq!(
        Some(default_note.title.clone()),
        ink_auto_title(&default.created_at)
    );
    assert_eq!(default_note.lifecycle, InkLifecycle::Active);
    assert!(default_note.last_used_at > 0);
    let named_note = store.ink_note("named-doc").unwrap().unwrap();
    assert_eq!(
        (named_note.title.as_str(), named_note.title_mode),
        ("函数作业", InkTitleMode::Manual)
    );
    let odd_note = store.ink_note("odd-date-doc").unwrap().unwrap();
    assert_eq!(
        (odd_note.title.as_str(), odd_note.title_mode),
        ("数学手稿", InkTitleMode::Manual)
    );
    // A request accepted before the upgrade still replays: its fingerprint
    // covers the old metadata (title "数学手稿") and is unchanged.
    assert!(
        store
            .append_ink_operation(
                "legacy-op",
                "default-doc",
                "default-session",
                1,
                0,
                &default.metadata(),
                &operation(0),
            )
            .unwrap()
            .replayed
    );
    drop(store);
    assert!(SqliteArtifactStore::open(&database).is_ok());

    // Guard path: migration 17 recorded but user_version left behind.
    Connection::open(&database)
        .unwrap()
        .execute_batch("PRAGMA user_version=16;")
        .unwrap();
    let store = SqliteArtifactStore::prepare(&database).unwrap();
    assert_eq!(
        store.ink_note("named-doc").unwrap().unwrap().title,
        "函数作业"
    );
    drop(store);
    let version: i64 = Connection::open(&database)
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 21);
    fs::remove_dir_all(directory).ok();
}

#[test]
fn recovery_import_is_manual_and_active() {
    let mut store = SqliteArtifactStore::open_in_memory().unwrap();
    let mut document = fixture("recovered");
    document.operations.push(operation(0));
    let session = store
        .import_ink_recovery(&document, &"b".repeat(32), 1_800_000_000)
        .unwrap();
    let note = store.ink_note(&session.document_id).unwrap().unwrap();
    assert_eq!(note.title_mode, InkTitleMode::Manual);
    assert!(note.title.ends_with("恢复草稿"));
    assert_eq!(note.lifecycle, InkLifecycle::Active);
    assert_eq!(note.last_used_at, 1_800_000_000_000);
}
