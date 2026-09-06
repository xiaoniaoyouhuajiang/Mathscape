use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::{PermissionsExt, symlink};

use mathscape_artifact::{
    ArtifactImportOutcome, ArtifactMetadata, ArtifactPool, ArtifactPoolError, WorkspaceRoot,
};
use mathscape_domain::{
    ActorRef, ArtifactCheckResult, ArtifactUri, ContentHash, MediaType, RightsRecord, RightsStatus,
};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn create(case: &str) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".test-runs")
            .join(format!(
                "{case}-{timestamp}-{}-{sequence}",
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

fn metadata(filename: &str) -> ArtifactMetadata {
    ArtifactMetadata::new(
        None,
        MediaType::parse("application/pdf").unwrap(),
        Some(filename.to_owned()),
        Some(3),
        RightsRecord::new(
            RightsStatus::NotAssessed,
            vec!["local_test".to_owned()],
            None,
        )
        .unwrap(),
    )
}

fn actor() -> ActorRef {
    ActorRef::parse("artifact-pool-test").unwrap()
}

#[test]
fn managed_import_is_deduplicated_and_survives_workspace_move() {
    let directory = TestDirectory::create("portable");
    let source = directory.path().join("source.pdf");
    fs::write(&source, b"%PDF-1.7\nportable artifact\n").unwrap();
    let first_workspace = directory.path().join("workspace-a");
    let mut pool = ArtifactPool::create(&first_workspace).unwrap();
    let workspace_id = pool.workspace().workspace_id().clone();

    let first = pool
        .import_file(&source, metadata("source.pdf"), actor())
        .unwrap();
    assert_eq!(first.outcome(), ArtifactImportOutcome::Inserted);
    assert_eq!(pool.artifact_count().unwrap(), 1);
    assert!(pool.workspace().object_path(first.content_hash()).is_file());
    assert_eq!(
        first.uri().as_str(),
        format!(
            "pool:sha256/{}",
            first
                .content_hash()
                .as_str()
                .strip_prefix("sha256:")
                .unwrap()
        )
    );

    let duplicate = pool
        .import_file(&source, metadata("renamed.pdf"), actor())
        .unwrap();
    assert_eq!(duplicate.outcome(), ArtifactImportOutcome::Deduplicated);
    assert_eq!(duplicate.stable_id(), first.stable_id());
    assert_eq!(pool.artifact_count().unwrap(), 1);
    assert_eq!(pool.check_count().unwrap(), 2);
    assert!(pool.audit_portability().unwrap().is_self_contained());
    assert!(
        fs::read_dir(pool.workspace().staging_path())
            .unwrap()
            .next()
            .is_none()
    );

    fs::remove_file(&source).unwrap();
    drop(pool);
    let moved_workspace = directory.path().join("workspace-b");
    fs::rename(&first_workspace, &moved_workspace).unwrap();
    let reopened = ArtifactPool::open(&moved_workspace).unwrap();
    assert_eq!(reopened.workspace().workspace_id(), &workspace_id);
    assert_eq!(
        reopened.read_bytes(first.stable_id(), 0, 5).unwrap(),
        b"%PDF-"
    );
    assert_eq!(
        reopened.read_bytes(first.stable_id(), 9, 1024).unwrap(),
        b"portable artifact\n"
    );
    assert!(
        reopened
            .read_bytes(first.stable_id(), first.byte_size(), 1)
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        reopened.read_bytes(first.stable_id(), u64::MAX, 1),
        Err(ArtifactPoolError::InvalidReadRange)
    ));
    assert!(matches!(
        reopened.read_bytes(first.stable_id(), 0, 0),
        Err(ArtifactPoolError::InvalidReadRange)
    ));
    assert_eq!(
        fs::read(reopened.managed_path(first.stable_id()).unwrap()).unwrap(),
        b"%PDF-1.7\nportable artifact\n"
    );
    assert!(reopened.audit_portability().unwrap().is_self_contained());
    drop(reopened);
    directory.cleanup();
}

#[test]
fn digest_mismatch_is_quarantined_before_catalog_registration() {
    let directory = TestDirectory::create("quarantine");
    let source = directory.path().join("source.pdf");
    fs::write(&source, b"not the expected bytes").unwrap();
    let mut pool = ArtifactPool::create(directory.path().join("workspace")).unwrap();
    let expected = ContentHash::parse(format!("sha256:{}", "0".repeat(64))).unwrap();

    let error = pool
        .import_file_with_expected_hash(&source, &expected, metadata("source.pdf"), actor())
        .unwrap_err();
    let ArtifactPoolError::DigestMismatch { quarantine, .. } = error else {
        panic!("expected digest mismatch");
    };
    assert!(quarantine.is_file());
    assert_eq!(pool.artifact_count().unwrap(), 0);
    assert!(
        fs::read_dir(pool.workspace().staging_path())
            .unwrap()
            .next()
            .is_none()
    );
    drop(pool);
    directory.cleanup();
}

#[test]
fn verification_records_corruption_and_changes_portability() {
    let directory = TestDirectory::create("verification");
    let source = directory.path().join("source.pdf");
    fs::write(&source, b"immutable bytes").unwrap();
    let mut pool = ArtifactPool::create(directory.path().join("workspace")).unwrap();
    let artifact = pool
        .import_file(&source, metadata("source.pdf"), actor())
        .unwrap();
    let object = pool.workspace().object_path(artifact.content_hash());
    make_writable(&object);
    fs::write(&object, b"tampered bytes").unwrap();

    let verification = pool.verify(artifact.stable_id()).unwrap();
    assert_eq!(verification.result(), ArtifactCheckResult::Corrupted);
    assert_eq!(pool.check_count().unwrap(), 2);
    let report = pool.audit_portability().unwrap();
    assert_eq!(report.corrupted_artifacts(), 1);
    assert!(!report.is_self_contained());
    drop(pool);
    directory.cleanup();
}

#[test]
fn verification_records_a_missing_managed_object() {
    let directory = TestDirectory::create("missing");
    let source = directory.path().join("source.pdf");
    fs::write(&source, b"managed bytes").unwrap();
    let mut pool = ArtifactPool::create(directory.path().join("workspace")).unwrap();
    let artifact = pool
        .import_file(&source, metadata("source.pdf"), actor())
        .unwrap();
    fs::remove_file(pool.workspace().object_path(artifact.content_hash())).unwrap();

    let verification = pool.verify(artifact.stable_id()).unwrap();
    assert_eq!(verification.result(), ArtifactCheckResult::Missing);
    let report = pool.audit_portability().unwrap();
    assert_eq!(report.missing_artifacts(), 1);
    assert!(!report.is_self_contained());
    drop(pool);
    directory.cleanup();
}

#[cfg(unix)]
fn make_writable(path: &Path) {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(path, permissions).unwrap();
}

#[cfg(not(unix))]
#[allow(clippy::permissions_set_readonly_false)]
fn make_writable(path: &Path) {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions).unwrap();
}

#[test]
fn workspace_resolver_rejects_non_pool_uris() {
    let directory = TestDirectory::create("resolver");
    let workspace = WorkspaceRoot::create(directory.path().join("workspace")).unwrap();
    let external = ArtifactUri::parse("file:///outside/book.pdf").unwrap();
    assert!(matches!(
        workspace.resolve_pool_uri(&external),
        Err(ArtifactPoolError::InvalidPoolUri(_))
    ));
    directory.cleanup();
}

#[cfg(unix)]
#[test]
fn managed_locations_never_follow_symbolic_links() {
    let directory = TestDirectory::create("symlink");
    let source = directory.path().join("source.pdf");
    fs::write(&source, b"immutable bytes").unwrap();
    let mut pool = ArtifactPool::create(directory.path().join("workspace")).unwrap();
    let artifact = pool
        .import_file(&source, metadata("source.pdf"), actor())
        .unwrap();
    let object = pool.workspace().object_path(artifact.content_hash());
    fs::remove_file(&object).unwrap();
    symlink(&source, &object).unwrap();

    assert!(matches!(
        pool.managed_path(artifact.stable_id()),
        Err(ArtifactPoolError::UnsafeManagedPath(_))
    ));
    assert!(matches!(
        pool.read_bytes(artifact.stable_id(), 0, 5),
        Err(ArtifactPoolError::UnsafeManagedPath(_))
    ));
    assert_eq!(
        pool.verify(artifact.stable_id()).unwrap().result(),
        ArtifactCheckResult::Corrupted
    );
    assert!(!pool.audit_portability().unwrap().is_self_contained());
    drop(pool);
    directory.cleanup();
}
