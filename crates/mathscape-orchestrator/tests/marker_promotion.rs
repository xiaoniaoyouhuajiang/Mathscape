#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use mathscape_artifact::{ArtifactMetadata, ArtifactPool};
use mathscape_domain::{ActorRef, MediaType, RightsRecord, RightsStatus};
use mathscape_orchestrator::promote_marker_inventory;
use mathscape_pdf::{PdfPageIndex, PdfSelection};
use mathscape_tool_marker::{
    MarkerCancellation, MarkerExecutionOptions, MarkerInstallation, MarkerProductRole,
    MarkerProductScanner, MarkerProfile, MarkerRequest, MarkerRunner, MarkerStagingArea,
};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn create() -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".test-runs")
            .join(format!(
                "marker-promotion-{timestamp}-{}-{sequence}",
                std::process::id()
            ));
        fs::create_dir_all(&root).unwrap();
        Self(root)
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

#[test]
fn promotes_marker_products_and_records_one_conversion() {
    let directory = TestDirectory::create();
    let source = directory.path().join("source.pdf");
    fs::write(&source, b"%PDF-1.7\nfixture\n").unwrap();
    let mut pool = ArtifactPool::create(directory.path().join("workspace")).unwrap();
    let source_artifact = pool
        .import_file(
            &source,
            ArtifactMetadata::new(
                None,
                MediaType::parse("application/pdf").unwrap(),
                Some("source.pdf".to_owned()),
                Some(1),
                RightsRecord::new(RightsStatus::NotAssessed, vec![], None).unwrap(),
            ),
            ActorRef::parse("marker-promotion-test").unwrap(),
        )
        .unwrap();

    let executable = directory.path().join("marker_single");
    fs::write(
        &executable,
        r#"#!/bin/sh
source_path="$1"
output_root=''
while [ "$#" -gt 0 ]; do
    case "$1" in
        --output_dir) output_root="$2"; shift 2 ;;
        *) shift ;;
    esac
done
source_name="${source_path##*/}"
source_stem="${source_name%.*}"
mkdir -p "$output_root/$source_stem"
printf '# converted\n' > "$output_root/$source_stem/$source_stem.md"
printf '{"page_stats":[{"page_id":0}]}\n' > "$output_root/$source_stem/${source_stem}_meta.json"
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&executable, permissions).unwrap();

    let managed_source = pool.workspace().object_path(source_artifact.content_hash());
    let request = MarkerRequest::new(
        managed_source,
        PdfSelection::inclusive_range(1, PdfPageIndex::new(0), PdfPageIndex::new(0)).unwrap(),
        MarkerProfile::MathOcrV1,
    )
    .unwrap();
    let runner = MarkerRunner::new(
        MarkerExecutionOptions::new(Duration::from_secs(2))
            .unwrap()
            .poll_interval(Duration::from_millis(10))
            .unwrap(),
    );
    let run = runner
        .execute(
            &MarkerInstallation::new(&executable).unwrap(),
            &request,
            &MarkerStagingArea::new(directory.path().join("marker-staging")).unwrap(),
            &MarkerCancellation::new(),
        )
        .unwrap();
    let inventory = MarkerProductScanner::default().scan(&run).unwrap();

    let result = promote_marker_inventory(
        &mut pool,
        &run,
        &inventory,
        source_artifact.stable_id(),
        Some("2.0.0"),
        ActorRef::parse("marker-promotion-test").unwrap(),
    )
    .unwrap();

    assert_eq!(result.outputs().len(), 2);
    assert!(
        result
            .outputs()
            .iter()
            .any(|output| output.role() == MarkerProductRole::PrimaryMarkdown)
    );
    assert!(
        result
            .outputs()
            .iter()
            .any(|output| output.role() == MarkerProductRole::Metadata)
    );
    assert_eq!(pool.artifact_count().unwrap(), 3);
    assert_eq!(pool.conversion_count().unwrap(), 1);
    assert!(pool.audit_portability().unwrap().is_self_contained());

    run.cleanup().unwrap();
    drop(pool);
    directory.cleanup();
}
