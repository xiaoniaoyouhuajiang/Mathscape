//! One PDF with several Marker markdown snapshots projects as one canonical node
//! after the registry is synced. Historical graphs stay fragment-level until then.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use mathscape_artifact::{ArtifactMetadata, ArtifactPool};
use mathscape_domain::{
    ActorRef, CanonicalMarkdownKey, CommandRecordStatus, ConversionExitStatus,
    ConversionParameters, ConversionRun, EntityHeader, EntityKind, InclusivePageRange,
    LifecycleStatus, MediaType, PageIndexBase, PageRange, RevisionId, SchemaVersion, StableId,
    Timestamp,
};
use mathscape_graph::{IncidenceDirection, ProjectionLens, ProjectionRelationKind};
use mathscape_query::{LandscapeProjectionInput, LandscapeProjector, project_artifact_catalog};
use mathscape_store_sqlite::SqliteArtifactStore;
use rusqlite::params;

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn create() -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".test-runs")
            .join(format!(
                "canonical-map-{timestamp}-{}-{sequence}",
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

fn actor() -> ActorRef {
    ActorRef::parse("canonical-map-test").unwrap()
}

fn derived(media: &str, filename: &str) -> ArtifactMetadata {
    ArtifactMetadata::derived(MediaType::parse(media).unwrap(), Some(filename.to_owned())).unwrap()
}

fn record_conversion(
    pool: &mut ArtifactPool,
    id: &str,
    revision_digit: char,
    profile: &str,
    pages: (u32, u32),
    status: ConversionExitStatus,
    finished: &str,
    pdf: &StableId,
    outputs: Vec<StableId>,
) {
    let conversion = ConversionRun::new(
        "marker_single".to_owned(),
        Some("2.0.0".to_owned()),
        None,
        CommandRecordStatus::Recorded,
        ConversionParameters::new(
            Some(PageRange::new(pages.0, pages.1, PageIndexBase::Zero).unwrap()),
            "markdown".to_owned(),
            Some(false),
            BTreeMap::from([("marker_profile".to_owned(), profile.to_owned())]),
        )
        .unwrap(),
        None,
        Some(Timestamp::parse("2026-11-02T00:00:00+08:00").unwrap()),
        Some(Timestamp::parse(finished).unwrap()),
        status,
        vec![],
        None,
    )
    .unwrap()
    .into_record(
        EntityHeader::new(
            StableId::parse(id).unwrap(),
            RevisionId::parse(format!("sha256:{}", revision_digit.to_string().repeat(64))).unwrap(),
            EntityKind::ConversionRun,
            SchemaVersion::parse("canonical-map-test-v1").unwrap(),
            LifecycleStatus::Active,
            Timestamp::parse(finished).unwrap(),
            actor(),
            None,
        ),
        vec![pdf.clone()],
        outputs,
    )
    .unwrap();
    pool.record_conversion(&conversion).unwrap();
}

#[test]
fn one_canonical_markdown_node_per_pdf_after_registry_sync() {
    let directory = TestDirectory::create();
    let workspace = directory.path().join("workspace");
    let mut pool = ArtifactPool::create(&workspace).unwrap();
    let source = pool
        .import_file(
            &write_file(directory.path(), "source.pdf", b"%PDF-1.7\nsource\n"),
            ArtifactMetadata::new(
                None,
                MediaType::parse("application/pdf").unwrap(),
                Some("source.pdf".to_owned()),
                Some(6),
                mathscape_domain::RightsRecord::new(
                    mathscape_domain::RightsStatus::NotAssessed,
                    vec!["local_test".to_owned()],
                    None,
                )
                .unwrap(),
            ),
            actor(),
        )
        .unwrap();
    let early = pool
        .import_file(
            &write_file(directory.path(), "early.md", b"# pages 1-2\n"),
            derived("text/markdown", "early.md"),
            actor(),
        )
        .unwrap();
    let late = pool
        .import_file(
            &write_file(directory.path(), "late.md", b"# pages 3-4\n"),
            derived("text/markdown", "late.md"),
            actor(),
        )
        .unwrap();
    let failed = pool
        .import_file(
            &write_file(directory.path(), "failed.md", b"# failed\n"),
            derived("text/markdown", "failed.md"),
            actor(),
        )
        .unwrap();
    let metadata = pool
        .import_file(
            &write_file(
                directory.path(),
                "source_meta.json",
                br#"{"page_stats":[]}"#,
            ),
            derived("application/json", "source_meta.json"),
            actor(),
        )
        .unwrap();

    let profile = "mathscape.marker.math-ocr.v1";
    record_conversion(
        &mut pool,
        "mathscape:conversion/marker/early",
        'a',
        profile,
        (0, 1),
        ConversionExitStatus::Succeeded,
        "2026-11-02T00:01:00+08:00",
        source.stable_id(),
        vec![early.stable_id().clone(), metadata.stable_id().clone()],
    );
    record_conversion(
        &mut pool,
        "mathscape:conversion/marker/late",
        'b',
        profile,
        (2, 3),
        ConversionExitStatus::Succeeded,
        "2026-11-02T00:02:00+08:00",
        source.stable_id(),
        vec![late.stable_id().clone()],
    );
    record_conversion(
        &mut pool,
        "mathscape:conversion/marker/failed",
        'c',
        profile,
        (4, 5),
        ConversionExitStatus::Failed,
        "2026-11-02T00:03:00+08:00",
        source.stable_id(),
        vec![failed.stable_id().clone()],
    );
    drop(pool);

    let span_id = StableId::parse("mathscape:source-span/canonical-map").unwrap();
    insert_span(&workspace, &span_id, early.stable_id());

    let mut store = SqliteArtifactStore::open(&workspace.join("mathscape.sqlite3")).unwrap();
    let before = store.landscape_snapshot().unwrap();
    assert!(before.canonical_markdown().is_empty());
    let unsynced = LandscapeProjector
        .project(LandscapeProjectionInput::from_snapshot(&before))
        .unwrap();
    assert!(node_ids(&unsynced).contains(early.stable_id()));
    assert!(node_ids(&unsynced).contains(late.stable_id()));
    assert!(
        !unsynced
            .nodes()
            .iter()
            .any(|node| node.label().contains("规范解析正文"))
    );

    let recorded_at = Timestamp::parse("2026-11-02T01:00:00+08:00").unwrap();
    let views = store
        .sync_canonical_markdown_from_catalog(&recorded_at)
        .unwrap();
    assert_eq!(views.len(), 1);
    let canonical = CanonicalMarkdownKey::new(source.stable_id().clone(), profile, "marker-2.0.0")
        .unwrap()
        .canonical_id();
    assert_eq!(views[0].canonical_id, canonical);
    assert_eq!(views[0].head_artifact_id.as_ref(), Some(late.stable_id()));
    assert_eq!(views[0].total_pages, Some(6));
    let coverage = views[0].coverage(None);
    assert_eq!(
        coverage.covered,
        vec![InclusivePageRange { from: 1, to: 4 }]
    );
    assert_eq!(
        coverage.head_covered,
        vec![InclusivePageRange { from: 3, to: 4 }]
    );
    assert_eq!(
        coverage.missing,
        vec![InclusivePageRange { from: 5, to: 6 }]
    );
    assert!(coverage.partial);
    assert!(!coverage.complete);
    assert!(
        views[0]
            .segments
            .iter()
            .all(|segment| segment.conversion_id.as_str() != "mathscape:conversion/marker/failed")
    );

    let again = store
        .sync_canonical_markdown_from_catalog(&recorded_at)
        .unwrap();
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].canonical_id, canonical);

    let snapshot = store.landscape_snapshot().unwrap();
    let catalog_only =
        project_artifact_catalog(snapshot.artifacts(), ProjectionLens::Source).unwrap();
    assert!(
        node_ids(&catalog_only).contains(early.stable_id()),
        "the public catalog projection stays fragment-level until a caller passes the registry"
    );

    let landscape = LandscapeProjector
        .project(LandscapeProjectionInput::from_snapshot(&snapshot))
        .unwrap();
    let ids = node_ids(&landscape);
    assert!(ids.contains(&canonical));
    assert!(!ids.contains(early.stable_id()));
    assert!(!ids.contains(late.stable_id()));
    assert!(ids.contains(failed.stable_id()));
    assert!(ids.contains(metadata.stable_id()));
    assert!(ids.contains(source.stable_id()));
    assert_eq!(
        landscape
            .nodes()
            .iter()
            .filter(|node| node.label().contains("规范解析正文"))
            .count(),
        1
    );
    let canonical_node = landscape
        .nodes()
        .iter()
        .find(|node| node.entity_id() == &canonical)
        .unwrap();
    assert_eq!(canonical_node.label(), "source · 规范解析正文");
    assert_eq!(canonical_node.kind(), EntityKind::SourceArtifact);
    assert!(canonical_node.summary_ref().unwrap().contains("partial"));
    // Desktop 「阅读文件」 opens the head snapshot, not the logical canonical id.
    assert_eq!(canonical_node.openable_artifact_id(), Some(late.stable_id()));
    let incidence = mathscape_graph::IncidenceGraph::project(&landscape);
    let wire = serde_json::to_value(&incidence).unwrap();
    let wire_node = wire["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["domain_id"] == canonical.as_str())
        .unwrap();
    assert_eq!(wire_node["openable_artifact_id"], late.stable_id().as_str());
    let source_wire = wire["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["domain_id"] == source.stable_id().as_str())
        .unwrap();
    assert!(
        source_wire.get("openable_artifact_id").is_none(),
        "physical artifacts open by domain_id; the field is omitted"
    );

    let late_relation = landscape
        .relations()
        .iter()
        .find(|relation| relation.relation_id().as_str() == "mathscape:conversion/marker/late")
        .unwrap();
    assert!(late_relation.endpoints().iter().any(|endpoint| {
        endpoint.direction() == IncidenceDirection::OutOfRelation && endpoint.node() == &canonical
    }));
    assert!(
        !late_relation
            .endpoints()
            .iter()
            .any(|endpoint| endpoint.node() == late.stable_id())
    );
    let early_relation = landscape
        .relations()
        .iter()
        .find(|relation| relation.relation_id().as_str() == "mathscape:conversion/marker/early")
        .unwrap();
    assert!(early_relation.endpoints().iter().any(|endpoint| {
        endpoint.node() == &canonical && endpoint.direction() == IncidenceDirection::OutOfRelation
    }));
    assert!(
        early_relation
            .endpoints()
            .iter()
            .any(|endpoint| endpoint.node() == metadata.stable_id())
    );
    assert!(landscape.relations().iter().any(|relation| {
        relation.relation_id().as_str() == "mathscape:conversion/marker/failed"
            && relation
                .endpoints()
                .iter()
                .any(|endpoint| endpoint.node() == failed.stable_id())
    }));
    assert!(landscape.relations().iter().any(|relation| {
        relation.relation_kind_value() == ProjectionRelationKind::LocatedIn
            && relation
                .endpoints()
                .iter()
                .any(|endpoint| endpoint.node() == &canonical)
    }));
    assert!(!landscape.relations().iter().any(|relation| {
        relation.relation_kind_value() == ProjectionRelationKind::LocatedIn
            && relation
                .endpoints()
                .iter()
                .any(|endpoint| endpoint.node() == early.stable_id())
    }));

    drop(store);
    let mut pool = ArtifactPool::open(&workspace).unwrap();
    let other = pool
        .import_file(
            &write_file(directory.path(), "fast.md", b"# fast profile\n"),
            derived("text/markdown", "fast.md"),
            actor(),
        )
        .unwrap();
    let other_profile = "mathscape.marker.fast-native.v1";
    record_conversion(
        &mut pool,
        "mathscape:conversion/marker/fast",
        'd',
        other_profile,
        (0, 0),
        ConversionExitStatus::Succeeded,
        "2026-11-02T00:04:00+08:00",
        source.stable_id(),
        vec![other.stable_id().clone()],
    );
    drop(pool);

    let mut store = SqliteArtifactStore::open(&workspace.join("mathscape.sqlite3")).unwrap();
    let views = store
        .sync_canonical_markdown_from_catalog(&recorded_at)
        .unwrap();
    assert_eq!(views.len(), 2);
    let other_id =
        CanonicalMarkdownKey::new(source.stable_id().clone(), other_profile, "marker-2.0.0")
            .unwrap()
            .canonical_id();
    let landscape = LandscapeProjector
        .project(LandscapeProjectionInput::from_snapshot(
            &store.landscape_snapshot().unwrap(),
        ))
        .unwrap();
    let ids = node_ids(&landscape);
    assert!(ids.contains(&canonical));
    assert!(ids.contains(&other_id));
    assert!(!ids.contains(other.stable_id()));
    assert_eq!(
        landscape
            .nodes()
            .iter()
            .filter(|node| node.label().contains("规范解析正文"))
            .count(),
        2
    );

    drop(store);
    directory.cleanup();
}

fn write_file(directory: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = directory.join(name);
    fs::write(&path, bytes).unwrap();
    path
}

fn insert_span(workspace: &Path, span_id: &StableId, markdown: &StableId) {
    let span_revision = RevisionId::parse(format!("sha256:{}", "8".repeat(64))).unwrap();
    let connection = rusqlite::Connection::open(workspace.join("mathscape.sqlite3")).unwrap();
    connection
        .execute(
            "INSERT INTO map_objects (object_id, object_shape, entity_kind, current_revision)
             VALUES (?1, 'entity', 'SourceSpan', ?2)",
            params![span_id.as_str(), span_revision.as_str()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO map_object_revisions
             (object_id, revision_id, schema_version, lifecycle, created_at,
              created_by, generation_run, payload_json)
             VALUES (?1, ?2, 'mathscape-source-1', 'active',
                     '2026-11-02T00:00:00+08:00', 'canonical-map-test', NULL, ?3)",
            params![
                span_id.as_str(),
                span_revision.as_str(),
                serde_json::json!({
                    "payload_kind": "span",
                    "payload": {
                        "markdown": {
                            "artifact": markdown,
                            "byte_range": {"start": 0, "end": 10},
                            "displayed_lines": null
                        }
                    }
                })
                .to_string()
            ],
        )
        .unwrap();
}

fn node_ids(projection: &mathscape_graph::GraphProjection) -> Vec<StableId> {
    projection
        .nodes()
        .iter()
        .map(|node| node.entity_id().clone())
        .collect()
}
