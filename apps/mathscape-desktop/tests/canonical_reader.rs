//! 「阅读文件」 on a canonical markdown node: one read-only view stitched from
//! every registered coverage segment in page order, with gaps, no new Artifact.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use mathscape_artifact::{ArtifactMetadata, ArtifactPool};
use mathscape_desktop::DesktopService;
use mathscape_domain::{
    ActorRef, CanonicalMarkdownKey, CommandRecordStatus, ConversionExitStatus,
    ConversionParameters, ConversionRun, EntityHeader, EntityKind, LifecycleStatus, MediaType,
    PageIndexBase, PageRange, RevisionId, RightsRecord, RightsStatus, SchemaVersion, StableId,
    Timestamp,
};
use mathscape_llm::MemoryCredentialResolver;
use mathscape_store_sqlite::SqliteArtifactStore;
use serde_json::Value;

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn directory() -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".test-runs")
        .join(format!(
            "canonical-reader-{timestamp}-{}-{sequence}",
            std::process::id()
        ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn actor() -> ActorRef {
    ActorRef::parse("canonical-reader-test").unwrap()
}

fn write(directory: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = directory.join(name);
    fs::write(&path, bytes).unwrap();
    path
}

fn marker_pages(pages: std::ops::RangeInclusive<u32>) -> String {
    pages
        .map(|id| {
            format!(
                "{{{id}}}------------------------------------------------\n\nbody of page {}\n\n",
                id + 1
            )
        })
        .collect()
}

fn import_markdown(pool: &mut ArtifactPool, dir: &Path, name: &str, text: &str) -> StableId {
    pool.import_file(
        &write(dir, name, text.as_bytes()),
        ArtifactMetadata::derived(
            MediaType::parse("text/markdown").unwrap(),
            Some(name.into()),
        )
        .unwrap(),
        actor(),
    )
    .unwrap()
    .stable_id()
    .clone()
}

fn record(
    pool: &mut ArtifactPool,
    name: &str,
    digit: char,
    zero_based: (u32, u32),
    finished: &str,
    pdf: &StableId,
    output: &StableId,
) {
    let run = ConversionRun::new(
        "marker_single".into(),
        Some("2.0.0".into()),
        None,
        CommandRecordStatus::Recorded,
        ConversionParameters::new(
            Some(PageRange::new(zero_based.0, zero_based.1, PageIndexBase::Zero).unwrap()),
            "markdown".into(),
            Some(true),
            BTreeMap::from([(
                "marker_profile".to_owned(),
                "mathscape.marker.math-ocr.v1".to_owned(),
            )]),
        )
        .unwrap(),
        None,
        Some(Timestamp::parse("2026-11-03T00:00:00+08:00").unwrap()),
        Some(Timestamp::parse(finished).unwrap()),
        ConversionExitStatus::Succeeded,
        vec![],
        None,
    )
    .unwrap()
    .into_record(
        EntityHeader::new(
            StableId::parse(format!("mathscape:conversion/marker/{name}")).unwrap(),
            RevisionId::parse(format!("sha256:{}", digit.to_string().repeat(64))).unwrap(),
            EntityKind::ConversionRun,
            SchemaVersion::parse("canonical-reader-test-v1").unwrap(),
            LifecycleStatus::Active,
            Timestamp::parse(finished).unwrap(),
            actor(),
            None,
        ),
        vec![pdf.clone()],
        vec![output.clone()],
    )
    .unwrap();
    pool.record_conversion(&run).unwrap();
}

#[test]
fn canonical_reader_stitches_registered_segments_in_page_order() {
    let dir = directory();
    let workspace = dir.join("workspace");
    let mut pool = ArtifactPool::create(&workspace).unwrap();
    let pdf = pool
        .import_file(
            &write(&dir, "book.pdf", b"%PDF-1.7\nbook\n"),
            ArtifactMetadata::new(
                None,
                MediaType::parse("application/pdf").unwrap(),
                Some("book.pdf".into()),
                Some(10),
                RightsRecord::new(RightsStatus::NotAssessed, vec!["local_test".into()], None)
                    .unwrap(),
            ),
            actor(),
        )
        .unwrap()
        .stable_id()
        .clone();
    let first_text = marker_pages(0..=3);
    let second_text = marker_pages(2..=5);
    let unpaginated = "# 第九页\n\n无页标记的快照\n";
    let first = import_markdown(&mut pool, &dir, "a.md", &first_text);
    let second = import_markdown(&mut pool, &dir, "b.md", &second_text);
    let ninth = import_markdown(&mut pool, &dir, "c.md", unpaginated);
    // The latest run (pages 3–6) is the head, but page order decides the view.
    record(
        &mut pool,
        "c",
        'c',
        (8, 8),
        "2026-11-03T00:01:00+08:00",
        &pdf,
        &ninth,
    );
    record(
        &mut pool,
        "a",
        'a',
        (0, 3),
        "2026-11-03T00:02:00+08:00",
        &pdf,
        &first,
    );
    record(
        &mut pool,
        "b",
        'b',
        (2, 5),
        "2026-11-03T00:03:00+08:00",
        &pdf,
        &second,
    );
    drop(pool);
    let mut store = SqliteArtifactStore::open(&workspace.join("mathscape.sqlite3")).unwrap();
    store
        .sync_canonical_markdown_from_catalog(
            &Timestamp::parse("2026-11-03T01:00:00+08:00").unwrap(),
        )
        .unwrap();
    let artifacts_before = store.read_catalog_snapshot().unwrap().artifacts().len();
    drop(store);

    let service = DesktopService::open_with_credential_store(
        &workspace,
        Arc::new(MemoryCredentialResolver::default()),
    )
    .unwrap();
    let canonical =
        CanonicalMarkdownKey::new(pdf.clone(), "mathscape.marker.math-ocr.v1", "marker-2.0.0")
            .unwrap()
            .canonical_id();
    let projection = service
        .canonical_reader_projection(canonical.as_str())
        .unwrap();
    let wire = serde_json::to_value(&projection).unwrap();
    assert_eq!(wire["head_artifact_id"], second.as_str());
    assert_eq!(wire["total_pages"], 10);
    let pieces = wire["pieces"].as_array().unwrap();
    let shape: Vec<String> = pieces
        .iter()
        .map(|piece| {
            let pages = &piece["pages"];
            let range = format!("{}-{}", pages["from"], pages["to"]);
            match piece["kind"].as_str().unwrap() {
                "gap" => format!("gap {range}"),
                _ => format!("{} {range}", piece["artifact_id"].as_str().unwrap()),
            }
        })
        .collect();
    assert_eq!(
        shape,
        [
            format!("{first} 1-4"),
            format!("{second} 5-6"),
            "gap 7-8".to_owned(),
            format!("{ninth} 9-9"),
            "gap 10-10".to_owned(),
        ]
    );
    assert_eq!(pieces[0]["markdown"], first_text.as_str());
    assert_eq!(pieces[0]["is_head"], false);
    // Overlap 3–4 is cut at the `{4}----` marker; the head keeps its own bytes.
    assert_eq!(
        pieces[1]["overlap"],
        serde_json::json!({ "from": 3, "to": 4 })
    );
    assert_eq!(pieces[1]["overlap_trimmed"], true);
    assert_eq!(pieces[1]["is_head"], true);
    let shown = pieces[1]["markdown"].as_str().unwrap();
    assert!(shown.starts_with("{4}----"));
    assert_eq!(
        pieces[1]["byte_start"].as_u64().unwrap() as usize + shown.len(),
        second_text.len()
    );
    assert!(!shown.contains("body of page 3"));
    assert_eq!(pieces[3]["markdown"], unpaginated);
    assert_eq!(pieces[3]["overlap"], Value::Null);
    assert_eq!(
        wire["coverage"]["missing"],
        serde_json::json!([{ "from": 7, "to": 8 }, { "from": 10, "to": 10 }])
    );

    // Read-only: no Artifact written, snapshot bytes untouched, physical reads unchanged.
    let store = SqliteArtifactStore::open(&workspace.join("mathscape.sqlite3")).unwrap();
    assert_eq!(
        store.read_catalog_snapshot().unwrap().artifacts().len(),
        artifacts_before
    );
    assert_eq!(
        service
            .read_artifact_bytes(second.as_str(), 0, 4096)
            .unwrap(),
        second_text.as_bytes()
    );
    assert!(service.content_info(canonical.as_str()).is_err());
    assert!(
        service
            .canonical_reader_projection(
                "mathscape:canonical-markdown/v1/p/q/mathscape:artifact/none"
            )
            .is_err()
    );
    drop(store);
    drop(service);
    let _ = fs::remove_dir_all(&dir);
}
