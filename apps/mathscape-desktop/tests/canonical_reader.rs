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
        write(dir, name, text.as_bytes()),
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
            write(&dir, "book.pdf", b"%PDF-1.7\nbook\n"),
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

/// Like Marker with `--paginate_output`: one `{N}----` marker per page.
#[cfg(unix)]
const FAKE_MARKER: &str = r#"#!/bin/sh
here="$(dirname "$0")"
source_path="$1"
output_root=''
page_range=''
while [ "$#" -gt 0 ]; do
    case "$1" in
        --output_dir) output_root="$2"; shift 2 ;;
        --page_range) page_range="$2"; shift 2 ;;
        *) shift ;;
    esac
done
echo "$page_range" >> "$here/calls.log"
source_name="${source_path##*/}"
source_stem="${source_name%.*}"
mkdir -p "$output_root/$source_stem"
out="$output_root/$source_stem/$source_stem.md"
: > "$out"
page="${page_range%-*}"
while [ "$page" -le "${page_range#*-}" ]; do
    printf '{%s}----------------\n\nparsed page %s\n\n' "$page" "$((page + 1))" >> "$out"
    page=$((page + 1))
done
printf '{"page_stats":[{"page_id":0}]}\n' > "$output_root/$source_stem/${source_stem}_meta.json"
"#;

#[cfg(unix)]
fn write_executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, body).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

#[cfg(unix)]
fn shape(service: &DesktopService, canonical: &StableId) -> Vec<String> {
    let wire = serde_json::to_value(
        service
            .canonical_reader_projection(canonical.as_str())
            .unwrap(),
    )
    .unwrap();
    wire["pieces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|piece| {
            format!(
                "{} {}-{}",
                piece["kind"].as_str().unwrap(),
                piece["pages"]["from"],
                piece["pages"]["to"]
            )
        })
        .collect()
}

/// 「解析这几页」: the reader asks for the last pages of a gap; only those
/// are parsed, and they appear in the same view where the gap was.
#[cfg(unix)]
#[test]
fn parsing_pages_of_a_gap_fills_the_same_view_in_place() {
    use mathscape_desktop::ParseCanonicalPagesRequest;
    use mathscape_mcp::{EnsurePolicy, EnsureStatus};
    use std::time::Duration;

    let dir = directory();
    let workspace = dir.join("workspace");
    let mut pool = ArtifactPool::create(&workspace).unwrap();
    let pdf = pool
        .import_file(
            write(&dir, "book.pdf", b"%PDF-1.7\nbook\n"),
            ArtifactMetadata::new(
                None,
                MediaType::parse("application/pdf").unwrap(),
                Some("book.pdf".into()),
                Some(8),
                RightsRecord::new(RightsStatus::NotAssessed, vec!["local_test".into()], None)
                    .unwrap(),
            ),
            actor(),
        )
        .unwrap()
        .stable_id()
        .clone();
    let first = import_markdown(&mut pool, &dir, "a.md", &marker_pages(0..=1));
    record(
        &mut pool,
        "a",
        'a',
        (0, 1),
        "2026-11-03T00:02:00+08:00",
        &pdf,
        &first,
    );
    drop(pool);
    SqliteArtifactStore::open(&workspace.join("mathscape.sqlite3"))
        .unwrap()
        .sync_canonical_markdown_from_catalog(
            &Timestamp::parse("2026-11-03T01:00:00+08:00").unwrap(),
        )
        .unwrap();

    let bin = dir.join("marker-runtime").join("bin");
    fs::create_dir_all(&bin).unwrap();
    // Same Marker version as the registered snapshot: same lineage.
    write_executable(&bin.join("python"), "#!/bin/sh\nprintf '2.0.0'\n");
    write_executable(&bin.join("marker_single"), FAKE_MARKER);
    let service = DesktopService::open_with_marker(&workspace, bin.join("marker_single")).unwrap();
    let canonical =
        CanonicalMarkdownKey::new(pdf.clone(), "mathscape.marker.math-ocr.v1", "marker-2.0.0")
            .unwrap()
            .canonical_id();
    assert_eq!(shape(&service, &canonical), ["segment 1-2", "gap 3-8"]);

    let policy = EnsurePolicy {
        sync_timeout: Duration::from_secs(30),
        marker_timeout: Duration::from_secs(20),
        poll_interval: Duration::from_millis(50),
        ..EnsurePolicy::desktop()
    };
    let report = service
        .parse_canonical_pages_with(
            &ParseCanonicalPagesRequest::new(canonical.as_str(), 5, 8),
            policy,
        )
        .unwrap();
    assert_eq!(report.status, EnsureStatus::Expanded, "{}", report.message);
    assert_eq!(
        shape(&service, &canonical),
        ["segment 1-2", "gap 3-4", "segment 5-8"]
    );
    assert_eq!(
        fs::read_to_string(bin.join("calls.log")).unwrap().trim(),
        "4-7"
    );
    assert!(service.canonical_parse_leases().unwrap().leases.is_empty());

    // Asking again for pages that are now shown runs nothing.
    let again = service
        .parse_canonical_pages_with(
            &ParseCanonicalPagesRequest::new(canonical.as_str(), 5, 8),
            policy,
        )
        .unwrap();
    assert_eq!(again.status, EnsureStatus::Reused);
    assert_eq!(
        fs::read_to_string(bin.join("calls.log"))
            .unwrap()
            .lines()
            .count(),
        1
    );

    assert!(
        service
            .parse_canonical_pages_with(
                &ParseCanonicalPagesRequest::new(canonical.as_str(), 0, 2),
                policy
            )
            .is_err()
    );
    assert!(
        service
            .parse_canonical_pages_with(
                &ParseCanonicalPagesRequest::new(canonical.as_str(), 7, 9),
                policy
            )
            .is_err()
    );
    fs::remove_dir_all(dir).unwrap();
}
