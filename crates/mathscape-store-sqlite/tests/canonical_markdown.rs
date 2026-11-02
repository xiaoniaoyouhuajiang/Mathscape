use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use mathscape_domain::{
    CanonicalMarkdownKey, CanonicalSegment, InclusivePageRange, StableId, Timestamp,
};
use mathscape_store_sqlite::{SqliteArtifactStore, SqliteStoreError};
use rusqlite::Connection;

fn open() -> (PathBuf, PathBuf, SqliteArtifactStore) {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".test-runs")
        .join(format!(
            "canonical-{stamp}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(&directory).unwrap();
    let database = directory.join("catalog.sqlite3");
    let store = SqliteArtifactStore::prepare(&database).unwrap();
    (directory, database, store)
}

fn key(policy: &str) -> CanonicalMarkdownKey {
    CanonicalMarkdownKey::new(
        StableId::parse("mathscape:artifact/pdf").unwrap(),
        "mathscape.marker.math-ocr.v1",
        policy,
    )
    .unwrap()
}

fn now() -> Timestamp {
    Timestamp::parse("2026-11-02T00:00:00+08:00").unwrap()
}

#[test]
fn registry_identity_is_unique_for_pdf_profile_and_policy() {
    let (directory, database, mut store) = open();
    let first = key("marker-2.0.0");
    let id = store
        .upsert_canonical_markdown(&first, None, None, Some(10), &now())
        .unwrap();
    assert_eq!(id, first.canonical_id());
    let head = StableId::parse("mathscape:artifact/markdown-head").unwrap();
    let again = store
        .upsert_canonical_markdown(&first, Some(&head), Some("sha256:ab"), Some(10), &now())
        .unwrap();
    assert_eq!(again, id);
    assert_eq!(store.list_canonical_markdown().unwrap().len(), 1);
    assert_eq!(
        store.list_canonical_markdown().unwrap()[0]
            .head_artifact_id
            .as_ref(),
        Some(&head)
    );

    let other = key("marker-2.1.0");
    store
        .upsert_canonical_markdown(&other, None, None, Some(10), &now())
        .unwrap();
    assert_eq!(store.list_canonical_markdown().unwrap().len(), 2);

    drop(store);
    let connection = Connection::open(database).unwrap();
    let error = connection.execute(
        "INSERT INTO canonical_markdown (
            canonical_id, source_pdf_artifact_id, parser_profile, parser_policy_version,
            head_artifact_id, head_revision, total_pages, created_at, updated_at
         ) VALUES ('mathscape:canonical-markdown/forged', 'mathscape:artifact/pdf',
                   'mathscape.marker.math-ocr.v1', 'marker-2.0.0', NULL, NULL, 10, 't', 't')",
        [],
    );
    assert!(
        error.is_err(),
        "the unique identity must reject a second row"
    );
    drop(connection);
    fs::remove_dir_all(directory).ok();
}

#[test]
fn coverage_segment_requires_a_registry_row_and_round_trips() {
    let (directory, _database, mut store) = open();
    let key = key("marker-2.0.0");
    let canonical_id = store
        .upsert_canonical_markdown(&key, None, None, Some(4), &now())
        .unwrap();
    let segment = CanonicalSegment {
        conversion_id: StableId::parse("mathscape:conversion/marker/1").unwrap(),
        markdown_artifact_id: StableId::parse("mathscape:artifact/markdown").unwrap(),
        pages: Some(InclusivePageRange::new(1, 2).unwrap()),
        finished_at: None,
    };
    store
        .record_canonical_coverage_segment(&canonical_id, &segment, &now())
        .unwrap();
    let stored = store.list_canonical_markdown().unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].segments.len(), 1);
    assert_eq!(stored[0].segments[0].pages, segment.pages);
    let coverage = stored[0].coverage(Some(InclusivePageRange::new(1, 2).unwrap()));
    assert!(coverage.partial);
    assert!(!coverage.complete);
    assert_eq!(
        coverage.missing,
        vec![InclusivePageRange { from: 3, to: 4 }]
    );

    let missing = CanonicalSegment {
        conversion_id: StableId::parse("mathscape:conversion/marker/missing").unwrap(),
        markdown_artifact_id: StableId::parse("mathscape:artifact/other").unwrap(),
        pages: Some(InclusivePageRange::new(1, 1).unwrap()),
        finished_at: None,
    };
    let error = store
        .record_canonical_coverage_segment(
            &StableId::parse("mathscape:canonical-markdown/v1/missing").unwrap(),
            &missing,
            &now(),
        )
        .unwrap_err();
    assert!(matches!(error, SqliteStoreError::Sqlite(_)));
    drop(store);
    fs::remove_dir_all(directory).ok();
}

#[test]
fn sync_on_an_empty_catalog_clears_the_registry() {
    let (directory, _database, mut store) = open();
    store
        .upsert_canonical_markdown(&key("marker-2.0.0"), None, None, None, &now())
        .unwrap();
    let views = store.sync_canonical_markdown_from_catalog(&now()).unwrap();
    assert!(views.is_empty());
    assert!(store.list_canonical_markdown().unwrap().is_empty());
    drop(store);
    fs::remove_dir_all(directory).ok();
}
