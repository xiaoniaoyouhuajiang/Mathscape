use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use super::*;

/// Self-cleaning temp dir (no extra dev-dependency).
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "mathscape-sandbox-test-{tag}-{}",
            random_hex(6).unwrap()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A dummy checkpoint workspace: fake sqlite bytes + a nested artifact.
fn make_checkpoint(dir: &Path) -> PathBuf {
    let root = dir.join("my-test-workspace");
    fs::create_dir_all(root.join("artifacts/ab")).unwrap();
    fs::write(
        root.join(WORKSPACE_DATABASE),
        b"SQLite format 3\0dummy-bytes",
    )
    .unwrap();
    fs::write(root.join("workspace.toml"), b"workspace_id = \"ws-test\"\n").unwrap();
    fs::write(root.join("artifacts/ab/blob.bin"), b"artifact payload").unwrap();
    root
}

/// Bytes + mtime of every file in the tree, for "checkpoint unchanged" proofs.
fn snapshot_tree(root: &Path) -> Vec<(PathBuf, Vec<u8>, SystemTime)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_owned()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let mtime = path.metadata().unwrap().modified().unwrap();
                out.push((
                    path.strip_prefix(root).unwrap().to_owned(),
                    fs::read(&path).unwrap(),
                    mtime,
                ));
            }
        }
    }
    out.sort();
    out
}

fn setup() -> (TempDir, SandboxHome, PathBuf) {
    let tmp = TempDir::new("flow");
    let checkpoint = make_checkpoint(tmp.path());
    let home = SandboxHome::open(tmp.path().join("app_data/sandbox")).unwrap();
    (tmp, home, checkpoint)
}

#[test]
fn fingerprint_covers_sqlite_and_is_content_based() {
    let tmp = TempDir::new("fp");
    let root = make_checkpoint(tmp.path());
    let a = fingerprint_workspace(&root).unwrap();
    assert_eq!(a.algorithm, "sha256");
    assert!(a.digest.starts_with("sha256:"));
    let db = a
        .files
        .iter()
        .find(|f| f.path == WORKSPACE_DATABASE)
        .unwrap();
    assert_eq!(db.size, 27);
    assert!(db.modified_unix_ms.is_some());
    assert_eq!(db.sha256.len(), 64);
    assert!(a.files.iter().any(|f| f.path == "workspace.toml"));

    // Same bytes elsewhere → same digest even with different mtimes.
    let other = tmp.path().join("copy");
    fs::create_dir_all(&other).unwrap();
    fs::copy(
        root.join(WORKSPACE_DATABASE),
        other.join(WORKSPACE_DATABASE),
    )
    .unwrap();
    fs::copy(root.join("workspace.toml"), other.join("workspace.toml")).unwrap();
    assert!(fingerprint_workspace(&other).unwrap().same_content(&a));

    // Changing the sqlite bytes changes the digest.
    fs::write(other.join(WORKSPACE_DATABASE), b"SQLite format 3\0changed").unwrap();
    assert!(!fingerprint_workspace(&other).unwrap().same_content(&a));
}

#[test]
fn fingerprint_requires_database() {
    let tmp = TempDir::new("nodb");
    assert!(matches!(
        fingerprint_workspace(tmp.path()),
        Err(SandboxError::MissingDatabase(_))
    ));
}

#[test]
fn register_checkpoint_is_idempotent_and_validated() {
    let (tmp, home, checkpoint) = setup();
    let a = home
        .register_checkpoint(&checkpoint, Some("My Test WS"))
        .unwrap();
    assert_eq!(a.checkpoint_id, "my-test-ws");
    assert_eq!(a.label, "My Test WS");
    let again = home
        .register_checkpoint(&checkpoint, Some("ignored"))
        .unwrap();
    assert_eq!(again, a);
    assert_eq!(home.list_checkpoints().unwrap(), vec![a.clone()]);

    // Chinese-only label still gets a stable ascii alias.
    let second = tmp.path().join("second");
    fs::create_dir_all(&second).unwrap();
    fs::write(second.join(WORKSPACE_DATABASE), b"x").unwrap();
    let b = home.register_checkpoint(&second, Some("测试库")).unwrap();
    assert!(b.checkpoint_id.starts_with("ckpt-"), "{}", b.checkpoint_id);

    // Not a workspace / overlapping with sandbox home.
    let empty = tmp.path().join("empty");
    fs::create_dir_all(&empty).unwrap();
    assert!(matches!(
        home.register_checkpoint(&empty, None),
        Err(SandboxError::MissingDatabase(_))
    ));
    assert!(matches!(
        home.register_checkpoint(tmp.path(), None),
        Err(SandboxError::OverlappingRoots { .. })
    ));

    home.unregister_checkpoint(&b.checkpoint_id).unwrap();
    assert_eq!(home.list_checkpoints().unwrap().len(), 1);
    assert!(
        second.join(WORKSPACE_DATABASE).is_file(),
        "unregister never touches the checkpoint"
    );
}

#[test]
fn start_session_copies_workspace_into_isolated_dir() {
    let (_tmp, home, checkpoint) = setup();
    let before = snapshot_tree(&checkpoint);
    let ckpt = home.register_checkpoint(&checkpoint, None).unwrap();
    let view = home.start_session(&ckpt.checkpoint_id).unwrap();

    assert_eq!(view.state, SessionState::Active);
    let ws = view.workspace_root.clone().unwrap();
    assert!(ws.starts_with(home.root().join("sessions")));
    assert_eq!(ws, home.session_workspace(&view.sandbox_session_id));
    assert_eq!(
        fs::read(ws.join(WORKSPACE_DATABASE)).unwrap(),
        fs::read(checkpoint.join(WORKSPACE_DATABASE)).unwrap()
    );
    assert_eq!(
        fs::read(ws.join("artifacts/ab/blob.bin")).unwrap(),
        b"artifact payload"
    );
    assert!(home.transcript_path(&view.sandbox_session_id).is_file());

    let record = home.session(&view.sandbox_session_id).unwrap();
    assert_eq!(record.isolation, ISOLATION_FULL_COPY);
    assert_eq!(
        record.checkpoint_fingerprint,
        fingerprint_workspace(&checkpoint).unwrap()
    );

    // Writing in the session copy never reaches the checkpoint.
    fs::write(ws.join(WORKSPACE_DATABASE), b"mutated in sandbox").unwrap();
    assert_eq!(snapshot_tree(&checkpoint), before);

    // Nothing left in staging.
    assert_eq!(
        fs::read_dir(home.root().join(".staging")).unwrap().count(),
        0
    );
}

#[test]
fn export_writes_v0_manifest_and_transcript() {
    let (tmp, home, checkpoint) = setup();
    let ckpt = home.register_checkpoint(&checkpoint, Some("ws")).unwrap();
    let view = home.start_session(&ckpt.checkpoint_id).unwrap();
    let id = view.sandbox_session_id.clone();

    // Empty transcript in S0 still exports a transcript.jsonl.
    let first = home.export_session(&id, None, Some("  第一次  ")).unwrap();
    assert!(first.export_dir.starts_with(home.default_export_dir()));
    assert!(first.export_dir.ends_with(format!("sandbox_export_{id}")));
    let raw: serde_json::Value =
        serde_json::from_slice(&fs::read(first.export_dir.join(MANIFEST_FILE)).unwrap()).unwrap();
    for key in [
        "schema",
        "sandbox_session_id",
        "checkpoint_id",
        "checkpoint_fingerprint",
        "created_at",
        "exported_at",
        "backend",
        "tool_allowlist",
        "notes",
    ] {
        assert!(raw.get(key).is_some(), "manifest missing {key}");
    }
    assert_eq!(raw["schema"], "mathscape.sandbox_export.v0");
    assert_eq!(raw["notes"], "第一次");
    // RFC3339 with an explicit offset.
    let exported_at = raw["exported_at"].as_str().unwrap();
    assert!(chrono::DateTime::parse_from_rfc3339(exported_at).is_ok());
    let manifest = read_export_manifest(&first.export_dir).unwrap();
    assert_eq!(manifest.transcript_entries, 0);
    assert_eq!(
        fs::read(first.export_dir.join(TRANSCRIPT_FILE)).unwrap(),
        b""
    );
    assert_eq!(
        manifest.checkpoint_fingerprint,
        fingerprint_workspace(&checkpoint).unwrap()
    );
    assert_eq!(first.session.state, SessionState::Exported);

    // Transcript lines are carried; a second export gets its own directory.
    fs::write(
        home.transcript_path(&id),
        b"{\"role\":\"user\",\"text\":\"hi\"}\n{\"role\":\"agent\",\"text\":\"hello\"}\n",
    )
    .unwrap();
    let custom = tmp.path().join("chosen");
    let second = home.export_session(&id, Some(&custom), None).unwrap();
    assert_eq!(second.manifest.transcript_entries, 2);
    assert!(
        second
            .export_dir
            .starts_with(custom.canonicalize().unwrap())
    );
    let again = home.export_session(&id, Some(&custom), None).unwrap();
    assert!(again.export_dir.ends_with(format!("sandbox_export_{id}-2")));
    assert_eq!(home.session(&id).unwrap().exports.len(), 3);
    // Session continues to exist after export.
    assert!(home.session_workspace(&id).is_dir());
}

#[test]
fn export_refuses_session_and_checkpoint_destinations() {
    let (_tmp, home, checkpoint) = setup();
    let before = snapshot_tree(&checkpoint);
    let ckpt = home.register_checkpoint(&checkpoint, None).unwrap();
    let id = home
        .start_session(&ckpt.checkpoint_id)
        .unwrap()
        .sandbox_session_id;
    assert!(matches!(
        home.export_session(&id, Some(&home.session_dir(&id)), None),
        Err(SandboxError::ForbiddenExportDestination { .. })
    ));
    assert!(matches!(
        home.export_session(&id, Some(&checkpoint), None),
        Err(SandboxError::ForbiddenExportDestination { .. })
    ));
    assert_eq!(snapshot_tree(&checkpoint), before);
}

#[test]
fn discard_physically_deletes_session_and_leaves_checkpoint_untouched() {
    let (_tmp, home, checkpoint) = setup();
    let before_tree = snapshot_tree(&checkpoint);
    let before_fp = fingerprint_workspace(&checkpoint).unwrap();
    let ckpt = home.register_checkpoint(&checkpoint, None).unwrap();
    let view = home.start_session(&ckpt.checkpoint_id).unwrap();
    let id = view.sandbox_session_id.clone();
    let exported = home.export_session(&id, None, None).unwrap();
    fs::write(
        view.workspace_root.unwrap().join("scratch.txt"),
        b"side effect",
    )
    .unwrap();

    let discarded = home.discard_session(&id).unwrap();
    assert_eq!(discarded.state, SessionState::Discarded);
    assert!(
        !home.session_dir(&id).exists(),
        "isolation dir must be gone"
    );
    assert_eq!(fs::read_dir(home.root().join(".trash")).unwrap().count(), 0);

    // Checkpoint: identical bytes and mtimes, identical fingerprint (incl. mtime).
    assert_eq!(snapshot_tree(&checkpoint), before_tree);
    assert_eq!(fingerprint_workspace(&checkpoint).unwrap(), before_fp);

    // Exports survive discard (they are the kept sample).
    assert!(read_export_manifest(&exported.export_dir).is_ok());

    // Terminal: listed as discarded, cannot be read, exported or discarded again.
    let listed = home.list_sessions().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].state, SessionState::Discarded);
    assert!(listed[0].workspace_root.is_none());
    assert!(matches!(
        home.session(&id),
        Err(SandboxError::SessionDiscarded(_))
    ));
    assert!(matches!(
        home.export_session(&id, None, None),
        Err(SandboxError::SessionDiscarded(_))
    ));
    assert!(matches!(
        home.discard_session(&id),
        Err(SandboxError::SessionDiscarded(_))
    ));

    // Re-entering the same checkpoint = a brand-new session with a new id.
    let fresh = home.start_session(&ckpt.checkpoint_id).unwrap();
    assert_ne!(fresh.sandbox_session_id, id);
    assert_eq!(home.list_sessions().unwrap().len(), 2);
}

#[test]
fn reopen_home_sweeps_interrupted_discard_and_staging() {
    let (_tmp, home, checkpoint) = setup();
    fs::create_dir_all(home.root().join(".trash/sbx-x/workspace")).unwrap();
    fs::create_dir_all(home.root().join(".staging/sbx-y/workspace")).unwrap();
    let reopened = SandboxHome::open(home.root()).unwrap();
    assert_eq!(
        fs::read_dir(reopened.root().join(".trash"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        fs::read_dir(reopened.root().join(".staging"))
            .unwrap()
            .count(),
        0
    );
    assert!(checkpoint.join(WORKSPACE_DATABASE).is_file());
}

#[test]
fn rejects_invalid_session_ids() {
    let (_tmp, home, _checkpoint) = setup();
    assert!(matches!(
        home.session("../etc"),
        Err(SandboxError::Invalid(_))
    ));
    assert!(matches!(
        home.discard_session("sbx-../x"),
        Err(SandboxError::Invalid(_))
    ));
    assert!(matches!(
        home.session("sbx-nope"),
        Err(SandboxError::UnknownSession(_))
    ));
}

#[test]
fn shm_is_not_copied() {
    let (_tmp, home, checkpoint) = setup();
    fs::write(checkpoint.join("mathscape.sqlite3-shm"), b"index").unwrap();
    let ckpt = home.register_checkpoint(&checkpoint, None).unwrap();
    let ws = home
        .start_session(&ckpt.checkpoint_id)
        .unwrap()
        .workspace_root
        .unwrap();
    assert!(!ws.join("mathscape.sqlite3-shm").exists());
    assert!(checkpoint.join("mathscape.sqlite3-shm").exists());
}
