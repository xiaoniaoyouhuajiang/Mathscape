use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use mathscape_domain::{
    CanonicalMarkdownKey, CanonicalSegment, InclusivePageRange, StableId, Timestamp,
};
use mathscape_store_sqlite::{
    CanonicalEnsureClaim, CanonicalEnsureClaimOutcome, CanonicalEnsureLimits,
    CanonicalEnsureOutcome, SqliteArtifactStore, SqliteStoreError,
};
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

#[test]
fn scoped_sync_leaves_other_pdfs_untouched() {
    let (directory, _database, mut store) = open();
    let other = CanonicalMarkdownKey::new(
        StableId::parse("mathscape:artifact/other-pdf").unwrap(),
        "mathscape.marker.math-ocr.v1",
        "marker-2.0.0",
    )
    .unwrap();
    store
        .upsert_canonical_markdown(&key("marker-2.0.0"), None, None, Some(3), &now())
        .unwrap();
    store
        .upsert_canonical_markdown(&other, None, None, Some(9), &now())
        .unwrap();
    // The catalog has no conversions: the scoped sync clears only `pdf`.
    let views = store
        .sync_canonical_markdown_for_pdf(
            &StableId::parse("mathscape:artifact/pdf").unwrap(),
            &now(),
        )
        .unwrap();
    assert!(views.is_empty());
    let remaining = store.list_canonical_markdown().unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].canonical_id, other.canonical_id());
    drop(store);
    fs::remove_dir_all(directory).ok();
}

fn claim(id: &str, pdf: &str, from: u32, to: u32) -> CanonicalEnsureClaim {
    let key = CanonicalMarkdownKey::new(
        StableId::parse(pdf).unwrap(),
        "mathscape.marker.math-ocr.v1",
        "marker-2.1.0",
    )
    .unwrap();
    CanonicalEnsureClaim {
        ensure_id: id.to_owned(),
        idempotency_key: format!("{pdf}|{from}-{to}"),
        source_pdf_artifact_id: StableId::parse(pdf).unwrap(),
        canonical_id: key.canonical_id(),
        parser_profile: "mathscape.marker.math-ocr.v1".to_owned(),
        parser_policy_version: "marker-2.1.0".to_owned(),
        pages: InclusivePageRange::new(from, to).unwrap(),
        owner: "test".to_owned(),
    }
}

const LIMITS: CanonicalEnsureLimits = CanonicalEnsureLimits {
    max_running: 1,
    window_pages: 10,
    window_ms: 60_000,
    lease_ms: 1_000,
};

#[test]
fn ensure_claims_lock_per_pdf_share_one_global_slot_and_expire() {
    let (directory, _database, mut store) = open();
    let first = claim("e1", "mathscape:artifact/a", 1, 2);
    assert_eq!(
        store.claim_canonical_ensure(&first, LIMITS, 100).unwrap(),
        CanonicalEnsureClaimOutcome::Acquired
    );
    // Same PDF: blocked by the per-PDF lock, reported with the live lease.
    let busy = store
        .claim_canonical_ensure(&claim("e2", "mathscape:artifact/a", 3, 3), LIMITS, 200)
        .unwrap();
    assert!(
        matches!(busy, CanonicalEnsureClaimOutcome::PdfBusy(ref lease) if lease.ensure_id == "e1")
    );
    // Another PDF: blocked by the global Marker slot.
    assert_eq!(
        store
            .claim_canonical_ensure(&claim("e3", "mathscape:artifact/b", 1, 1), LIMITS, 200)
            .unwrap(),
        CanonicalEnsureClaimOutcome::SlotsBusy { running: 1 }
    );
    assert_eq!(store.active_canonical_ensures(200).unwrap().len(), 1);
    // The lease expires: it is abandoned and the PDF can be claimed again.
    assert!(store.active_canonical_ensures(1_100).unwrap().is_empty());
    assert_eq!(
        store
            .claim_canonical_ensure(&claim("e4", "mathscape:artifact/a", 1, 2), LIMITS, 1_200)
            .unwrap(),
        CanonicalEnsureClaimOutcome::Acquired
    );
    assert_eq!(
        store
            .canonical_ensure_receipt("e1")
            .unwrap()
            .unwrap()
            .status,
        "abandoned"
    );
    store
        .finish_canonical_ensure(
            "e4",
            CanonicalEnsureOutcome::Succeeded,
            Some("mathscape:conversion/x"),
            Some("mathscape:artifact/md"),
            None,
            1_300,
        )
        .unwrap();
    let receipt = store.canonical_ensure_receipt("e4").unwrap().unwrap();
    assert_eq!(receipt.status, "succeeded");
    assert_eq!(
        receipt.conversion_id.as_deref(),
        Some("mathscape:conversion/x")
    );
    assert!(store.active_canonical_ensures(1_300).unwrap().is_empty());
    drop(store);
    fs::remove_dir_all(directory).ok();
}

#[test]
fn ensure_claims_respect_the_rolling_page_budget() {
    let (directory, _database, mut store) = open();
    let limits = CanonicalEnsureLimits {
        max_running: 4,
        ..LIMITS
    };
    assert_eq!(
        store
            .claim_canonical_ensure(&claim("b1", "mathscape:artifact/a", 1, 8), limits, 0)
            .unwrap(),
        CanonicalEnsureClaimOutcome::Acquired
    );
    store
        .finish_canonical_ensure(
            "b1",
            CanonicalEnsureOutcome::Failed,
            None,
            None,
            Some("x"),
            10,
        )
        .unwrap();
    assert_eq!(
        store
            .claim_canonical_ensure(&claim("b2", "mathscape:artifact/b", 1, 3), limits, 20)
            .unwrap(),
        CanonicalEnsureClaimOutcome::BudgetExhausted {
            used_pages: 8,
            window_pages: 10
        }
    );
    // Outside the window the budget is free again.
    assert_eq!(
        store
            .claim_canonical_ensure(&claim("b3", "mathscape:artifact/b", 1, 3), limits, 70_000)
            .unwrap(),
        CanonicalEnsureClaimOutcome::Acquired
    );
    drop(store);
    fs::remove_dir_all(directory).ok();
}
