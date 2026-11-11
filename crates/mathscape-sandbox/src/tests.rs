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
    // A directory that *contains* the sandbox root is fine (the copy skips
    // the sandbox); this one is just not a workspace.
    assert!(matches!(
        home.register_checkpoint(tmp.path(), None),
        Err(SandboxError::MissingDatabase(_))
    ));
    // A directory inside the sandbox root can never be isolated.
    let inside = home.root().join("sessions/not-a-session");
    fs::create_dir_all(&inside).unwrap();
    fs::write(inside.join(WORKSPACE_DATABASE), b"x").unwrap();
    assert!(matches!(
        home.register_checkpoint(&inside, None),
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

// ---------------------------------------------------------------- S1 dialogue

#[test]
fn dialogue_lines_and_events_stay_inside_session_and_export() {
    let (_tmp, home, checkpoint) = setup();
    let before = snapshot_tree(&checkpoint);
    let ckpt = home.register_checkpoint(&checkpoint, Some("t")).unwrap();
    let session = home.start_session(&ckpt.checkpoint_id).unwrap();
    let id = session.sandbox_session_id.clone();

    home.append_transcript(&id, TranscriptRole::User, "紧集是什么？", None, None)
        .unwrap();
    home.append_event(
        &id,
        &serde_json::json!({"type": "turn_start", "round_id": "r1", "seq": 0}),
    )
    .unwrap();
    home.append_transcript(
        &id,
        TranscriptRole::Agent,
        "紧集是……",
        Some("r1"),
        Some("completed"),
    )
    .unwrap();
    let view = home
        .record_backend(&id, "codex", &["workspace.search".to_owned()])
        .unwrap();
    assert_eq!(view.sandbox_session_id, id);

    let transcript = home.read_transcript(&id).unwrap();
    assert_eq!(transcript.len(), 2);
    assert_eq!(transcript[0].role, TranscriptRole::User);
    assert_eq!(transcript[1].round_id.as_deref(), Some("r1"));
    assert_eq!(home.read_events(&id).unwrap().len(), 1);
    let scratch = home.ensure_session_scratch(&id).unwrap();
    assert!(scratch.is_dir() && scratch.starts_with(home.session_dir(&id)));
    assert!(!scratch.starts_with(home.session_workspace(&id)));
    home.append_harness_context(&id, &serde_json::json!({"skill": "mathscape-worknet"}))
        .unwrap();
    assert_eq!(home.read_harness_contexts(&id).unwrap().len(), 1);
    assert!(home.events_path(&id).starts_with(home.session_dir(&id)));

    let outcome = home.export_session(&id, None, None).unwrap();
    assert_eq!(outcome.manifest.backend, "codex");
    assert_eq!(outcome.manifest.tool_allowlist, vec!["workspace.search"]);
    assert_eq!(outcome.manifest.transcript_entries, 2);
    assert!(outcome.manifest.files.iter().any(|f| f == EVENTS_FILE));
    assert!(outcome.export_dir.join(EVENTS_FILE).is_file());
    assert!(outcome.manifest.files.iter().any(|f| f == HARNESS_FILE));
    assert!(outcome.export_dir.join(HARNESS_FILE).is_file());

    home.discard_session(&id).unwrap();
    assert!(!home.session_dir(&id).exists());
    // A late write from a still-running round must not revive the session.
    assert!(matches!(
        home.append_transcript(&id, TranscriptRole::Agent, "late", None, None),
        Err(SandboxError::SessionDiscarded(_))
    ));
    assert!(matches!(
        home.append_event(&id, &serde_json::json!({})),
        Err(SandboxError::SessionDiscarded(_))
    ));
    assert!(!home.session_dir(&id).exists());
    assert_eq!(snapshot_tree(&checkpoint), before);
}

#[test]
fn export_without_rounds_has_no_events_file() {
    let (_tmp, home, checkpoint) = setup();
    let ckpt = home.register_checkpoint(&checkpoint, None).unwrap();
    let session = home.start_session(&ckpt.checkpoint_id).unwrap();
    let outcome = home
        .export_session(&session.sandbox_session_id, None, None)
        .unwrap();
    assert_eq!(outcome.manifest.files, vec![MANIFEST_FILE, TRANSCRIPT_FILE]);
}

#[test]
fn dialogue_prompt_replays_recent_history_without_system_lines() {
    let entry = |role, text: &str| TranscriptEntry {
        ts: "2026-10-31T00:00:00.000+08:00".into(),
        role,
        text: text.into(),
        round_id: None,
        outcome: None,
        thread_id: None,
    };
    let history = vec![
        entry(TranscriptRole::User, "第一问"),
        entry(TranscriptRole::Agent, "第一答"),
        entry(TranscriptRole::System, "本轮已取消"),
    ];
    let prompt = build_dialogue_prompt(&history, "  第二问 ", false);
    assert!(
        prompt.contains("[用户] 第一问\n[助手] 第一答\n"),
        "{prompt}"
    );
    assert!(!prompt.contains("本轮已取消"));
    assert!(prompt.ends_with("用户本轮消息：\n第二问"));
    // Same tool wording as the persistent session preamble: proposals included.
    assert!(prompt.starts_with(DIALOGUE_INTRO), "{prompt}");
    assert!(!prompt.contains("只读工具"), "{prompt}");
    assert!(prompt.contains("propose_changes"), "{prompt}");
    assert!(prompt.contains(REBUILT_CONTEXT_NOTE), "{prompt}");

    let long = "长".repeat(13_000);
    let prompt = build_dialogue_prompt(&[entry(TranscriptRole::User, &long)], "q", false);
    assert!(!prompt.contains("此前的对话"));
    assert!(build_dialogue_prompt(&[], "q", false).ends_with("q"));
    assert!(!build_dialogue_prompt(&[], "q", false).contains("新开的一条对话"));
    assert!(!build_dialogue_prompt(&[], "q", false).contains(REBUILT_CONTEXT_NOTE));
    let later = build_dialogue_prompt(&[], "q", true);
    assert!(later.contains("新开的一条对话"), "{later}");
    assert!(later.ends_with("q"));
}

// ---------------------------------------------------------------- dialogue threads

#[test]
fn new_thread_shares_copy_and_resets_only_history() {
    let (_tmp, home, checkpoint) = setup();
    let before = snapshot_tree(&checkpoint);
    let ckpt = home.register_checkpoint(&checkpoint, None).unwrap();
    let session = home.start_session(&ckpt.checkpoint_id).unwrap();
    let id = session.sandbox_session_id.clone();
    let workspace = session.workspace_root.unwrap();

    let threads = home.dialogue_threads(&id).unwrap();
    assert_eq!(threads.active_thread_id, FIRST_THREAD_ID);
    assert_eq!(threads.threads.len(), 1);
    // Empty active thread: opening a new one is a no-op.
    assert_eq!(home.start_dialogue_thread(&id).unwrap(), threads);

    home.append_transcript(&id, TranscriptRole::User, "调试 A", None, None)
        .unwrap();
    home.append_transcript(
        &id,
        TranscriptRole::Agent,
        "A 的回复",
        Some("r1"),
        Some("completed"),
    )
    .unwrap();
    // Side effect on the shared copy made during thread d1.
    fs::write(workspace.join("from-d1.txt"), "state").unwrap();

    let threads = home.start_dialogue_thread(&id).unwrap();
    assert_eq!(threads.active_thread_id, "d2");
    assert_eq!(threads.threads.len(), 2);
    // Same session, same copy: state carries over.
    assert_eq!(home.session_workspace(&id), workspace);
    assert!(workspace.join("from-d1.txt").is_file());
    assert!(home.read_thread_transcript(&id, "d2").unwrap().is_empty());

    // Old thread is read-only history; late lines for it are refused.
    assert!(matches!(
        home.append_thread_transcript(
            &id,
            FIRST_THREAD_ID,
            TranscriptRole::Agent,
            "late",
            None,
            None
        ),
        Err(SandboxError::InactiveThread { .. })
    ));
    let entry = home
        .append_transcript(&id, TranscriptRole::User, "调试 B", None, None)
        .unwrap();
    assert_eq!(entry.thread(), "d2");
    assert_eq!(
        home.read_thread_transcript(&id, FIRST_THREAD_ID)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(home.read_transcript(&id).unwrap().len(), 3);

    // Export / discard still act on the whole session.
    let outcome = home.export_session(&id, None, None).unwrap();
    assert_eq!(outcome.manifest.transcript_entries, 3);
    assert_eq!(outcome.manifest.dialogue_threads, 2);
    assert_eq!(home.dialogue_threads(&id).unwrap().active_thread_id, "d2");
    home.discard_session(&id).unwrap();
    assert!(!home.session_dir(&id).exists());
    assert_eq!(snapshot_tree(&checkpoint), before);
}

#[test]
fn legacy_session_without_threads_reads_as_first_thread() {
    let (_tmp, home, checkpoint) = setup();
    let ckpt = home.register_checkpoint(&checkpoint, None).unwrap();
    let id = home
        .start_session(&ckpt.checkpoint_id)
        .unwrap()
        .sandbox_session_id;
    // A pre-thread transcript line has no `thread_id`.
    fs::write(
        home.transcript_path(&id),
        "{\"ts\":\"2026-11-01T00:00:00.000+08:00\",\"role\":\"user\",\"text\":\"旧消息\"}\n",
    )
    .unwrap();
    let session_json = fs::read_to_string(home.session_dir(&id).join(SESSION_FILE)).unwrap();
    assert!(!session_json.contains("dialogue_threads"));

    let old = home.read_thread_transcript(&id, FIRST_THREAD_ID).unwrap();
    assert_eq!(old.len(), 1);
    assert_eq!(old[0].thread_id, None);
    let threads = home.start_dialogue_thread(&id).unwrap();
    assert_eq!(
        threads
            .threads
            .iter()
            .map(|t| t.thread_id.as_str())
            .collect::<Vec<_>>(),
        vec![FIRST_THREAD_ID, "d2"]
    );
}

/// Sandbox root inside the mother workspace (`<ws>/.mathscape/sandbox`).
fn setup_in_workspace() -> (TempDir, SandboxHome, PathBuf) {
    let tmp = TempDir::new("inws");
    let checkpoint = make_checkpoint(tmp.path());
    let home = SandboxHome::open(SandboxHome::workspace_root_for(&checkpoint)).unwrap();
    (tmp, home, checkpoint)
}

#[test]
fn sandbox_inside_the_mother_workspace_is_not_copied_into_sessions() {
    let (_tmp, home, checkpoint) = setup_in_workspace();
    fs::create_dir_all(checkpoint.join(".mathscape/lean")).unwrap();
    fs::write(checkpoint.join(".mathscape/lean/keep.txt"), b"k").unwrap();
    let record = home.register_checkpoint(&checkpoint, None).unwrap();
    let first = home.start_session(&record.checkpoint_id).unwrap();
    let second = home.start_session(&record.checkpoint_id).unwrap();
    let copy = second.workspace_root.unwrap();
    assert!(copy.join(WORKSPACE_DATABASE).is_file());
    assert!(copy.join(".mathscape/lean/keep.txt").is_file());
    assert!(
        !copy.join(".mathscape/sandbox").exists(),
        "the sandbox root must not be copied into a session"
    );
    assert!(first.workspace_root.unwrap().is_dir());
}

#[test]
fn moving_the_workspace_rebases_checkpoint_roots() {
    let (tmp, home, checkpoint) = setup_in_workspace();
    let record = home.register_checkpoint(&checkpoint, Some("ws")).unwrap();
    let session = home.start_session(&record.checkpoint_id).unwrap();
    drop(home);
    let moved = tmp.path().join("moved-workspace");
    fs::rename(&checkpoint, &moved).unwrap();
    let home = SandboxHome::open(SandboxHome::workspace_root_for(&moved)).unwrap();
    let moved = moved.canonicalize().unwrap();
    assert_eq!(home.checkpoint("ws").unwrap().root, moved);
    let session = home.session(&session.sandbox_session_id).unwrap();
    assert_eq!(session.checkpoint_root, moved);
    // A new session from the moved checkpoint still works.
    assert!(home.start_session("ws").is_ok());
}

#[test]
fn legacy_sessions_are_copied_without_touching_the_legacy_root() {
    let (tmp, legacy, checkpoint) = setup();
    let record = legacy.register_checkpoint(&checkpoint, Some("ws")).unwrap();
    let mine = legacy.start_session(&record.checkpoint_id).unwrap();
    legacy
        .append_transcript(
            &mine.sandbox_session_id,
            TranscriptRole::User,
            "旧会话",
            None,
            None,
        )
        .unwrap();
    let other_root = tmp.path().join("other");
    fs::create_dir_all(&other_root).unwrap();
    fs::write(other_root.join(WORKSPACE_DATABASE), b"o").unwrap();
    let other = legacy
        .register_checkpoint(&other_root, Some("other"))
        .unwrap();
    let theirs = legacy.start_session(&other.checkpoint_id).unwrap();
    let legacy_before = snapshot_tree(legacy.root());

    let home = SandboxHome::open(SandboxHome::workspace_root_for(&checkpoint)).unwrap();
    let report = home
        .import_legacy_sessions(legacy.root(), &checkpoint)
        .unwrap();
    assert_eq!(report.copied, vec![mine.sandbox_session_id.clone()]);
    assert!(home.session(&theirs.sandbox_session_id).is_err());
    let copied = home.session(&mine.sandbox_session_id).unwrap();
    assert_eq!(copied.checkpoint_root, checkpoint.canonicalize().unwrap());
    assert_eq!(
        home.checkpoint(&copied.checkpoint_id).unwrap().root,
        copied.checkpoint_root
    );
    let transcript = home.read_transcript(&mine.sandbox_session_id).unwrap();
    assert_eq!(transcript[0].text, "旧会话");
    // The legacy root is byte-for-byte unchanged.
    assert_eq!(snapshot_tree(legacy.root()), legacy_before);

    // Idempotent, and a discarded copy is never revived.
    let again = home
        .import_legacy_sessions(legacy.root(), &checkpoint)
        .unwrap();
    assert!(again.copied.is_empty());
    home.discard_session(&mine.sandbox_session_id).unwrap();
    let after_discard = home
        .import_legacy_sessions(legacy.root(), &checkpoint)
        .unwrap();
    assert!(after_discard.copied.is_empty());
    assert!(home.session(&mine.sandbox_session_id).is_err());
}

#[test]
fn harness_binding_round_trips_through_session_json() {
    let (_tmp, home, checkpoint) = setup();
    let record = home.register_checkpoint(&checkpoint, None).unwrap();
    let session = home.start_session(&record.checkpoint_id).unwrap();
    let id = session.sandbox_session_id;
    let binding = HarnessThreadBinding {
        backend: "codex".into(),
        thread_id: "01a1c59d-fd72-7831-83e0-10f1b2afeff8".into(),
        rollout_file: Some("rollout-x.jsonl".into()),
        workspace: home.session_workspace(&id),
        tool_schema_version: Some("mathscape-mcp/16".into()),
        skill_version: Some("0.6.5".into()),
        skill_sha256: Some("ab".into()),
        last_turn_at: None,
    };
    let threads = home
        .set_thread_harness(&id, FIRST_THREAD_ID, Some(binding.clone()))
        .unwrap();
    assert_eq!(threads.threads[0].harness.as_ref(), Some(&binding));
    assert_eq!(
        home.dialogue_threads(&id).unwrap().threads[0].harness,
        Some(binding)
    );
    assert!(home.set_thread_harness(&id, "d9", None).is_err());
    let dir = home.ensure_harness_state_dir(&id, "codex").unwrap();
    assert!(dir.ends_with("harness/codex"));
    assert!(home.ensure_harness_state_dir(&id, "../x").is_err());
    assert!(home.current_round_path(&id).ends_with(CURRENT_ROUND_FILE));
}
