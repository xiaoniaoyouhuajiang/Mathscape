use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use mathscape_artifact::{ArtifactMetadata, ArtifactPool};
use mathscape_domain::{
    ActorRef, CommandRecordStatus, ConversionExitStatus, ConversionParameters, ConversionRun,
    EntityHeader, EntityKind, LifecycleStatus, MediaType, PageIndexBase, PageRange, RevisionId,
    RightsRecord, RightsStatus, SchemaVersion, SourcePayload, StableId, Timestamp,
};
use mathscape_graph::{IncidenceDirection, IncidenceGraph, ProjectionLens};
use mathscape_query::{
    ArtifactGraphProjector, LandscapeProjectionInput, LandscapeProjector, project_artifact_catalog,
};
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
                "artifact-graph-{timestamp}-{}-{sequence}",
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
    ActorRef::parse("artifact-graph-test").unwrap()
}

fn source_metadata() -> ArtifactMetadata {
    ArtifactMetadata::new(
        None,
        MediaType::parse("application/pdf").unwrap(),
        Some("source.pdf".to_owned()),
        Some(2),
        RightsRecord::new(
            RightsStatus::NotAssessed,
            vec!["local_test".to_owned()],
            None,
        )
        .unwrap(),
    )
}

#[test]
fn reads_the_catalog_and_projects_conversion_hyperedges() {
    let directory = TestDirectory::create();
    let workspace = directory.path().join("workspace");
    let source_path = directory.path().join("source.pdf");
    let markdown_path = directory.path().join("source.md");
    let metadata_path = directory.path().join("source_meta.json");
    fs::write(&source_path, b"%PDF-1.7\nsource\n").unwrap();
    fs::write(&markdown_path, b"# converted\n").unwrap();
    fs::write(&metadata_path, br#"{"page_stats":[]}"#).unwrap();

    let mut pool = ArtifactPool::create(&workspace).unwrap();
    let source = pool
        .import_file(&source_path, source_metadata(), actor())
        .unwrap();
    let markdown = pool
        .import_file(
            &markdown_path,
            ArtifactMetadata::derived(
                MediaType::parse("text/markdown").unwrap(),
                Some("source.md".to_owned()),
            )
            .unwrap(),
            actor(),
        )
        .unwrap();
    let metadata = pool
        .import_file(
            &metadata_path,
            ArtifactMetadata::derived(
                MediaType::parse("application/json").unwrap(),
                Some("source_meta.json".to_owned()),
            )
            .unwrap(),
            actor(),
        )
        .unwrap();

    let conversion_id =
        mathscape_domain::StableId::parse("mathscape:conversion/test-marker").unwrap();
    let conversion = ConversionRun::new(
        "marker_single".to_owned(),
        Some("2.0.0".to_owned()),
        Some("marker_single {source_pdf}".to_owned()),
        CommandRecordStatus::Recorded,
        ConversionParameters::new(
            Some(PageRange::new(0, 1, PageIndexBase::Zero).unwrap()),
            "markdown".to_owned(),
            Some(false),
            BTreeMap::from([("marker_profile".to_owned(), "math-ocr-v1".to_owned())]),
        )
        .unwrap(),
        None,
        Some(Timestamp::parse("2026-09-06T10:00:00+08:00").unwrap()),
        Some(Timestamp::parse("2026-09-06T10:01:00+08:00").unwrap()),
        ConversionExitStatus::Succeeded,
        vec![],
        None,
    )
    .unwrap()
    .into_record(
        EntityHeader::new(
            conversion_id.clone(),
            RevisionId::parse(format!("sha256:{}", "7".repeat(64))).unwrap(),
            EntityKind::ConversionRun,
            SchemaVersion::parse("artifact-graph-test-v1").unwrap(),
            LifecycleStatus::Active,
            Timestamp::parse("2026-09-06T10:01:00+08:00").unwrap(),
            actor(),
            None,
        ),
        vec![source.stable_id().clone()],
        vec![markdown.stable_id().clone(), metadata.stable_id().clone()],
    )
    .unwrap();
    pool.record_conversion(&conversion).unwrap();
    drop(pool);

    // Persist one current SourceSpan with the same payload shape produced by
    // source registration. This anchors the cross-view provenance edge.
    drop(SqliteArtifactStore::open(&workspace.join("mathscape.sqlite3")).unwrap());
    let span_id = StableId::parse("mathscape:source-span/landscape-fixture").unwrap();
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
                     '2026-09-14T00:00:00+08:00', 'landscape-test', NULL, ?3)",
            params![
                span_id.as_str(),
                span_revision.as_str(),
                serde_json::json!({
                    "payload_kind": "span",
                    "payload": {
                        "markdown": {
                            "artifact": markdown.stable_id(),
                            "byte_range": {"start": 0, "end": 10},
                            "displayed_lines": null
                        }
                    }
                })
                .to_string()
            ],
        )
        .unwrap();
    drop(connection);

    let store = SqliteArtifactStore::open(&workspace.join("mathscape.sqlite3")).unwrap();
    let snapshot = store.read_catalog_snapshot().unwrap();
    assert_eq!(snapshot.artifacts().len(), 3);
    assert_eq!(snapshot.conversions().len(), 1);
    assert!(
        snapshot
            .artifacts()
            .iter()
            .all(|artifact| artifact.locations().len() == 1)
    );
    let source_record = snapshot
        .artifacts()
        .iter()
        .find(|record| record.entity().header().stable_id() == source.stable_id())
        .unwrap();
    assert_eq!(
        source_record.entity().payload().original_filename(),
        Some("source.pdf")
    );
    assert_eq!(source_record.entity().payload().page_count(), Some(2));

    let stored_conversion = &snapshot.conversions()[0];
    let SourcePayload::ConversionRun(run) = stored_conversion.payload() else {
        panic!("expected ConversionRun");
    };
    assert_eq!(run.tool_name(), "marker_single");
    assert_eq!(run.tool_version(), Some("2.0.0"));
    assert_eq!(run.parameters().page_range().unwrap().start(), 0);
    assert_eq!(stored_conversion.endpoints().len(), 3);

    let projection = ArtifactGraphProjector
        .project(&snapshot, ProjectionLens::Source)
        .unwrap();
    assert_eq!(projection.nodes().len(), 3);
    assert_eq!(projection.relations().len(), 1);
    let projected_conversion = &projection.relations()[0];
    assert_eq!(projected_conversion.relation_id(), &conversion_id);
    assert_eq!(projected_conversion.label(), "source · PDF 1–2 页 · 解析");
    assert!(
        projected_conversion
            .summary_ref()
            .unwrap()
            .contains("marker_single 2.0.0")
    );
    assert!(
        projection
            .nodes()
            .iter()
            .any(|node| node.label() == "source · PDF 1–2 页 · 解析正文")
    );
    assert!(
        projection
            .nodes()
            .iter()
            .any(|node| node.label() == "source · PDF 1–2 页 · 解析元数据")
    );
    assert_eq!(projected_conversion.endpoints().len(), 3);
    assert_eq!(
        projected_conversion
            .endpoints()
            .iter()
            .filter(|endpoint| endpoint.direction() == IncidenceDirection::IntoRelation)
            .count(),
        1
    );
    assert_eq!(
        projected_conversion
            .endpoints()
            .iter()
            .filter(|endpoint| endpoint.direction() == IncidenceDirection::OutOfRelation)
            .count(),
        2
    );

    let incidence = IncidenceGraph::project(&projection);
    assert_eq!(incidence.nodes().len(), 4);
    assert_eq!(incidence.edges().len(), 3);
    assert!(
        project_artifact_catalog(&snapshot, ProjectionLens::Rdtx).is_err(),
        "an Artifact catalog must not pretend to provide an RDTX lens"
    );

    let landscape_snapshot = store.landscape_snapshot().unwrap();
    let landscape = LandscapeProjector
        .project(LandscapeProjectionInput::from_snapshot(&landscape_snapshot))
        .unwrap();
    assert_eq!(landscape.lens(), ProjectionLens::Landscape);
    assert!(
        landscape
            .nodes()
            .iter()
            .any(|node| node.entity_id() == &span_id)
    );
    assert!(landscape.relations().iter().any(|relation| {
        relation.kind() == EntityKind::Evidence
            && relation.label() == "来源于 Markdown"
            && relation
                .endpoints()
                .iter()
                .any(|endpoint| endpoint.node() == markdown.stable_id())
    }));
    assert!(landscape.relations().iter().any(|relation| {
        relation.kind() == EntityKind::ConversionRun
            && relation
                .endpoints()
                .iter()
                .any(|endpoint| endpoint.node() == source.stable_id())
    }));

    drop(store);
    directory.cleanup();
}
