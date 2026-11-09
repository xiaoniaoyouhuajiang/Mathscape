//! Mathscape **Sandbox Flow S0**: checkpoint registry, session isolation by
//! full workspace copy, discard (physical delete) and export
//! (`mathscape.sandbox_export.v0`). Authoritative contract:
//! `docs/sandbox-flow.md` §3 and §5 (S0).
//!
//! Naming: a *sandbox session* here is the Mathscape product concept (a
//! disposable exploration session). It is unrelated to Codex CLI's
//! `SandboxMode` (`read-only | workspace-write | …`), which only governs what
//! the Codex harness may do to the host filesystem.
//!
//! S0 created, listed, exported and discarded the session copy. S1 adds the
//! dialogue storage ([`TranscriptEntry`] lines and raw `agent_events v0` in
//! `events.jsonl`), both inside the session dir; Agent tools stay read-only
//! (T2). The Codex mount itself lives in the Desktop app.
//!
//! On-disk layout under a [`SandboxHome`] root:
//!
//! ```text
//! <root>/
//!   checkpoints.json                 # registered checkpoints (aliases → local roots)
//!   discarded.jsonl                  # tombstones: one line per discarded session id
//!   sessions/<sandbox_session_id>/   # the whole isolation layer; discard deletes it
//!     session.json
//!     transcript.jsonl               # user / agent / system lines (S1), tagged by dialogue thread
//!     events.jsonl                   # raw agent_events v0 of every round (S1)
//!     harness.jsonl                  # per-round harness context: skill version/hash, grants
//!     scratch/                       # the only harness-writable dir (drafts)
//!     workspace/                     # full copy of the checkpoint workspace root
//!   exports/                         # default export destination (survives discard)
//!   .staging/  .trash/               # internal; make create/discard atomic
//! ```

mod copy;
mod dialogue;
mod fingerprint;

pub use dialogue::{
    DialogueThread, DialogueThreads, EVENTS_FILE, FIRST_THREAD_ID, HARNESS_FILE, MAX_MESSAGE_CHARS,
    SCRATCH_DIR, TranscriptEntry, TranscriptRole, build_dialogue_prompt,
};

pub use fingerprint::{
    CheckpointFingerprint, FINGERPRINT_ALGORITHM, FingerprintFile, fingerprint_workspace,
};

use std::{
    fs::{self, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

use chrono::{Local, SecondsFormat};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const WORKSPACE_DATABASE: &str = "mathscape.sqlite3";
pub const EXPORT_SCHEMA: &str = "mathscape.sandbox_export.v0";
pub const SESSION_SCHEMA: &str = "mathscape.sandbox_session.v0";
pub const CHECKPOINT_REGISTRY_SCHEMA: &str = "mathscape.sandbox_checkpoints.v0";
/// T1 (docs/sandbox-flow.md §6): v0 isolates by full copy.
pub const ISOLATION_FULL_COPY: &str = "full_copy";
/// S0 mounts no agent backend; S1 will record e.g. `codex`.
pub const BACKEND_NONE: &str = "none";
pub const MANIFEST_FILE: &str = "manifest.json";
pub const TRANSCRIPT_FILE: &str = "transcript.jsonl";
pub const SESSION_FILE: &str = "session.json";
pub const SESSION_WORKSPACE_DIR: &str = "workspace";

const CHECKPOINTS_FILE: &str = "checkpoints.json";
const TOMBSTONES_FILE: &str = "discarded.jsonl";
const SESSIONS_DIR: &str = "sessions";
const EXPORTS_DIR: &str = "exports";
const STAGING_DIR: &str = ".staging";
const TRASH_DIR: &str = ".trash";
const SESSION_ID_PREFIX: &str = "sbx-";
const MAX_NOTES_CHARS: usize = 2_000;
const MAX_LABEL_CHARS: usize = 120;

#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("沙箱文件操作失败（{path}）：{source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("沙箱元数据损坏（{path}）：{source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("checkpoint 目录中没有 {WORKSPACE_DATABASE}：{0}")]
    MissingDatabase(PathBuf),
    #[error("checkpoint 必须是已存在的目录：{0}")]
    NotADirectory(PathBuf),
    #[error("checkpoint 与沙箱目录互相包含，无法隔离：checkpoint={checkpoint}，sandbox={sandbox}")]
    OverlappingRoots {
        checkpoint: PathBuf,
        sandbox: PathBuf,
    },
    #[error("未登记的 checkpoint：{0}")]
    UnknownCheckpoint(String),
    #[error("未知的沙箱会话：{0}")]
    UnknownSession(String),
    #[error("沙箱会话已丢弃（终态，不可复活）：{0}")]
    SessionDiscarded(String),
    #[error("复制期间 checkpoint 发生变化（请关闭正在写它的程序后重试）：{0}")]
    CheckpointChangedDuringCopy(PathBuf),
    #[error("副本与 checkpoint 指纹不一致：{0}")]
    CopyMismatch(PathBuf),
    #[error("无法复制 {path}：{reason}")]
    UnsupportedEntry { path: PathBuf, reason: &'static str },
    #[error("导出目标不能位于{zone}内：{path}")]
    ForbiddenExportDestination { path: PathBuf, zone: &'static str },
    #[error("对话 {thread_id} 已不是当前对话（当前为 {active_thread_id}），只读")]
    InactiveThread {
        thread_id: String,
        active_thread_id: String,
    },
    #[error("参数无效：{0}")]
    Invalid(String),
    #[error("随机数生成失败：{0}")]
    Random(String),
}

impl SandboxError {
    pub(crate) fn io(path: &Path, source: io::Error) -> Self {
        Self::Io {
            path: path.to_owned(),
            source,
        }
    }
}

pub type Result<T> = std::result::Result<T, SandboxError>;

/// Lifecycle (docs/sandbox-flow.md §3.2). `Discarded` is terminal.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Active,
    Exported,
    Discarded,
}

/// A registered checkpoint. `checkpoint_id` is a stable alias for listing;
/// it is **not** a point in time — the bytes behind it are whatever the user's
/// test workspace holds, which is why every session also records a
/// [`CheckpointFingerprint`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointRecord {
    pub checkpoint_id: String,
    pub label: String,
    pub root: PathBuf,
    pub registered_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointRegistry {
    schema: String,
    checkpoints: Vec<CheckpointRecord>,
}

/// `session.json` inside the session isolation directory.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionRecord {
    pub schema: String,
    pub sandbox_session_id: String,
    pub state: SessionState,
    pub checkpoint_id: String,
    pub checkpoint_root: PathBuf,
    pub checkpoint_fingerprint: CheckpointFingerprint,
    pub isolation: String,
    pub backend: String,
    pub tool_allowlist: Vec<String>,
    pub created_at: String,
    pub exports: Vec<ExportRecord>,
    /// Dialogue threads sharing this session's isolated copy
    /// (docs/sandbox-flow.md §3.5). Empty for sessions written before
    /// threads existed: they implicitly hold one thread, [`FIRST_THREAD_ID`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dialogue_threads: Vec<DialogueThread>,
    /// The only thread that may receive new messages; `None` = first thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_thread_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportRecord {
    pub exported_at: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Tombstone {
    sandbox_session_id: String,
    checkpoint_id: String,
    created_at: String,
    discarded_at: String,
}

/// What listings return, for live and discarded sessions alike.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SessionView {
    pub sandbox_session_id: String,
    pub state: SessionState,
    pub checkpoint_id: String,
    pub checkpoint_digest: Option<String>,
    pub created_at: String,
    pub last_exported_at: Option<String>,
    pub export_count: usize,
    pub discarded_at: Option<String>,
    /// Isolated workspace copy; `None` once discarded.
    pub workspace_root: Option<PathBuf>,
}

/// `manifest.json` of an export package (docs/sandbox-flow.md §3.4).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportManifest {
    pub schema: String,
    pub sandbox_session_id: String,
    pub checkpoint_id: String,
    pub checkpoint_fingerprint: CheckpointFingerprint,
    pub created_at: String,
    pub exported_at: String,
    pub backend: String,
    pub tool_allowlist: Vec<String>,
    pub notes: Option<String>,
    pub isolation: String,
    pub transcript_entries: usize,
    /// Dialogue threads in the exported transcript (≥ 1). Older packages
    /// predate threads and read back as 0.
    #[serde(default)]
    pub dialogue_threads: usize,
    pub files: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExportOutcome {
    pub export_dir: PathBuf,
    pub manifest: ExportManifest,
    pub session: SessionView,
}

/// App-managed sandbox root. Not thread-safe across processes; callers in one
/// process should serialise operations (Desktop holds it behind a mutex).
#[derive(Clone, Debug)]
pub struct SandboxHome {
    root: PathBuf,
}

impl SandboxHome {
    /// Open (creating if needed) a sandbox home and sweep leftovers of
    /// interrupted creates/discards.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        for dir in [SESSIONS_DIR, EXPORTS_DIR, STAGING_DIR, TRASH_DIR] {
            let path = root.join(dir);
            fs::create_dir_all(&path).map_err(|e| SandboxError::io(&path, e))?;
        }
        let root = root.canonicalize().map_err(|e| SandboxError::io(root, e))?;
        let home = Self { root };
        home.sweep_internal();
        Ok(home)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn default_export_dir(&self) -> PathBuf {
        self.root.join(EXPORTS_DIR)
    }

    pub fn session_dir(&self, session_id: &str) -> PathBuf {
        self.root.join(SESSIONS_DIR).join(session_id)
    }

    pub fn session_workspace(&self, session_id: &str) -> PathBuf {
        self.session_dir(session_id).join(SESSION_WORKSPACE_DIR)
    }

    pub fn transcript_path(&self, session_id: &str) -> PathBuf {
        self.session_dir(session_id).join(TRANSCRIPT_FILE)
    }

    // ---------------------------------------------------------------- checkpoints

    pub fn list_checkpoints(&self) -> Result<Vec<CheckpointRecord>> {
        Ok(self.read_registry()?.checkpoints)
    }

    pub fn checkpoint(&self, checkpoint_id: &str) -> Result<CheckpointRecord> {
        self.list_checkpoints()?
            .into_iter()
            .find(|c| c.checkpoint_id == checkpoint_id)
            .ok_or_else(|| SandboxError::UnknownCheckpoint(checkpoint_id.to_owned()))
    }

    /// Register a local workspace root as a checkpoint. Idempotent per root:
    /// registering the same directory again returns the existing record.
    /// Only reads the checkpoint.
    pub fn register_checkpoint(
        &self,
        root: impl AsRef<Path>,
        label: Option<&str>,
    ) -> Result<CheckpointRecord> {
        let root = self.validate_checkpoint_root(root.as_ref())?;
        let mut registry = self.read_registry()?;
        if let Some(existing) = registry.checkpoints.iter().find(|c| c.root == root) {
            return Ok(existing.clone());
        }
        let label = match label.map(str::trim).filter(|l| !l.is_empty()) {
            Some(l) => truncate_chars(l, MAX_LABEL_CHARS),
            None => root
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("checkpoint")
                .to_owned(),
        };
        let checkpoint_id = unique_checkpoint_id(&label, &root, &registry.checkpoints);
        let record = CheckpointRecord {
            checkpoint_id,
            label,
            root,
            registered_at: now_rfc3339(),
        };
        registry.checkpoints.push(record.clone());
        self.write_registry(&registry)?;
        Ok(record)
    }

    /// Forget a registered alias. Never touches the checkpoint directory or
    /// any existing session.
    pub fn unregister_checkpoint(&self, checkpoint_id: &str) -> Result<()> {
        let mut registry = self.read_registry()?;
        let before = registry.checkpoints.len();
        registry
            .checkpoints
            .retain(|c| c.checkpoint_id != checkpoint_id);
        if registry.checkpoints.len() == before {
            return Err(SandboxError::UnknownCheckpoint(checkpoint_id.to_owned()));
        }
        self.write_registry(&registry)
    }

    fn validate_checkpoint_root(&self, root: &Path) -> Result<PathBuf> {
        if !root.is_dir() {
            return Err(SandboxError::NotADirectory(root.to_owned()));
        }
        let root = root.canonicalize().map_err(|e| SandboxError::io(root, e))?;
        if root.starts_with(&self.root) || self.root.starts_with(&root) {
            return Err(SandboxError::OverlappingRoots {
                checkpoint: root,
                sandbox: self.root.clone(),
            });
        }
        if !root.join(WORKSPACE_DATABASE).is_file() {
            return Err(SandboxError::MissingDatabase(root));
        }
        Ok(root)
    }

    fn read_registry(&self) -> Result<CheckpointRegistry> {
        let path = self.root.join(CHECKPOINTS_FILE);
        if !path.exists() {
            return Ok(CheckpointRegistry {
                schema: CHECKPOINT_REGISTRY_SCHEMA.to_owned(),
                checkpoints: Vec::new(),
            });
        }
        read_json(&path)
    }

    fn write_registry(&self, registry: &CheckpointRegistry) -> Result<()> {
        write_json_atomic(&self.root.join(CHECKPOINTS_FILE), registry)
    }

    // ---------------------------------------------------------------- sessions

    /// Open a sandbox session: fingerprint the checkpoint, copy the whole
    /// workspace root into a staging dir, verify the checkpoint did not change
    /// and the copy matches, then atomically publish it under `sessions/`.
    /// The checkpoint is only read.
    pub fn start_session(&self, checkpoint_id: &str) -> Result<SessionView> {
        let checkpoint = self.checkpoint(checkpoint_id)?;
        let checkpoint_root = self.validate_checkpoint_root(&checkpoint.root)?;
        let before = fingerprint_workspace(&checkpoint_root)?;

        let session_id = new_session_id()?;
        let staging = self.root.join(STAGING_DIR).join(&session_id);
        let result = (|| {
            let workspace = staging.join(SESSION_WORKSPACE_DIR);
            fs::create_dir_all(&workspace).map_err(|e| SandboxError::io(&workspace, e))?;
            copy::copy_tree(&checkpoint_root, &workspace)?;
            let after = fingerprint_workspace(&checkpoint_root)?;
            if !after.same_content(&before) {
                return Err(SandboxError::CheckpointChangedDuringCopy(
                    checkpoint_root.clone(),
                ));
            }
            let copied = fingerprint_workspace(&workspace)?;
            if !copied.same_content(&before) {
                return Err(SandboxError::CopyMismatch(workspace));
            }
            let transcript = staging.join(TRANSCRIPT_FILE);
            fs::File::create(&transcript).map_err(|e| SandboxError::io(&transcript, e))?;
            let record = SessionRecord {
                schema: SESSION_SCHEMA.to_owned(),
                sandbox_session_id: session_id.clone(),
                state: SessionState::Active,
                checkpoint_id: checkpoint.checkpoint_id.clone(),
                checkpoint_root: checkpoint_root.clone(),
                checkpoint_fingerprint: before.clone(),
                isolation: ISOLATION_FULL_COPY.to_owned(),
                backend: BACKEND_NONE.to_owned(),
                tool_allowlist: Vec::new(),
                created_at: now_rfc3339(),
                exports: Vec::new(),
                dialogue_threads: Vec::new(),
                active_thread_id: None,
            };
            write_json_atomic(&staging.join(SESSION_FILE), &record)?;
            let target = self.session_dir(&session_id);
            fs::rename(&staging, &target).map_err(|e| SandboxError::io(&target, e))?;
            Ok(record)
        })();
        match result {
            Ok(record) => Ok(self.live_view(&record)),
            Err(error) => {
                let _ = fs::remove_dir_all(&staging);
                Err(error)
            }
        }
    }

    /// Live sessions (active/exported) plus discarded tombstones, newest first.
    pub fn list_sessions(&self) -> Result<Vec<SessionView>> {
        self.sweep_internal();
        let mut views = Vec::new();
        let dir = self.root.join(SESSIONS_DIR);
        for entry in fs::read_dir(&dir).map_err(|e| SandboxError::io(&dir, e))? {
            let entry = entry.map_err(|e| SandboxError::io(&dir, e))?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if !name.starts_with(SESSION_ID_PREFIX) || !entry.path().is_dir() {
                continue;
            }
            let record: SessionRecord = read_json(&entry.path().join(SESSION_FILE))?;
            views.push(self.live_view(&record));
        }
        for tombstone in self.read_tombstones()? {
            views.push(SessionView {
                sandbox_session_id: tombstone.sandbox_session_id,
                state: SessionState::Discarded,
                checkpoint_id: tombstone.checkpoint_id,
                checkpoint_digest: None,
                created_at: tombstone.created_at,
                last_exported_at: None,
                export_count: 0,
                discarded_at: Some(tombstone.discarded_at),
                workspace_root: None,
            });
        }
        views.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(views)
    }

    /// The live record of a session; errors for discarded or unknown ids.
    pub fn session(&self, session_id: &str) -> Result<SessionRecord> {
        validate_session_id(session_id)?;
        let path = self.session_dir(session_id).join(SESSION_FILE);
        if path.is_file() {
            return read_json(&path);
        }
        if self
            .read_tombstones()?
            .iter()
            .any(|t| t.sandbox_session_id == session_id)
        {
            return Err(SandboxError::SessionDiscarded(session_id.to_owned()));
        }
        Err(SandboxError::UnknownSession(session_id.to_owned()))
    }

    /// Discard: physically delete the whole session isolation directory. The
    /// directory is first renamed out of `sessions/` (atomic), the tombstone
    /// is appended, then the bytes are removed; an interrupted removal is
    /// finished by the next [`SandboxHome::open`]/[`SandboxHome::list_sessions`].
    pub fn discard_session(&self, session_id: &str) -> Result<SessionView> {
        let record = self.session(session_id)?;
        let source = self.session_dir(session_id);
        let trash = self.root.join(TRASH_DIR).join(session_id);
        fs::rename(&source, &trash).map_err(|e| SandboxError::io(&source, e))?;
        let tombstone = Tombstone {
            sandbox_session_id: record.sandbox_session_id.clone(),
            checkpoint_id: record.checkpoint_id.clone(),
            created_at: record.created_at.clone(),
            discarded_at: now_rfc3339(),
        };
        self.append_tombstone(&tombstone)?;
        fs::remove_dir_all(&trash).map_err(|e| SandboxError::io(&trash, e))?;
        Ok(SessionView {
            sandbox_session_id: tombstone.sandbox_session_id,
            state: SessionState::Discarded,
            checkpoint_id: tombstone.checkpoint_id,
            checkpoint_digest: Some(record.checkpoint_fingerprint.digest),
            created_at: tombstone.created_at,
            last_exported_at: record.exports.last().map(|e| e.exported_at.clone()),
            export_count: record.exports.len(),
            discarded_at: Some(tombstone.discarded_at),
            workspace_root: None,
        })
    }

    /// Export `manifest.json` + `transcript.jsonl` (+ `events.jsonl` once a
    /// dialogue round ran) into
    /// `<parent>/sandbox_export_<id>[-N]/` (default parent: `<root>/exports`).
    /// The destination may not lie inside the session dir (it would vanish on
    /// discard) nor inside the checkpoint (never mutated). The session stays
    /// usable and moves to `exported`.
    pub fn export_session(
        &self,
        session_id: &str,
        destination_parent: Option<&Path>,
        notes: Option<&str>,
    ) -> Result<ExportOutcome> {
        let mut record = self.session(session_id)?;
        let parent = match destination_parent {
            Some(p) => p.to_owned(),
            None => self.default_export_dir(),
        };
        fs::create_dir_all(&parent).map_err(|e| SandboxError::io(&parent, e))?;
        let parent = parent
            .canonicalize()
            .map_err(|e| SandboxError::io(&parent, e))?;
        let session_dir = self.session_dir(session_id);
        if parent.starts_with(&session_dir) {
            return Err(SandboxError::ForbiddenExportDestination {
                path: parent,
                zone: "会话隔离目录（丢弃时会被删除）",
            });
        }
        if parent.starts_with(&record.checkpoint_root) {
            return Err(SandboxError::ForbiddenExportDestination {
                path: parent,
                zone: " checkpoint 目录（checkpoint 只读）",
            });
        }
        let notes = notes
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(|n| truncate_chars(n, MAX_NOTES_CHARS));

        let transcript_source = self.transcript_path(session_id);
        let transcript =
            fs::read(&transcript_source).map_err(|e| SandboxError::io(&transcript_source, e))?;
        let transcript_entries = transcript
            .split(|b| *b == b'\n')
            .filter(|line| !line.iter().all(u8::is_ascii_whitespace))
            .count();

        let events_source = self.events_path(session_id);
        let events = match fs::read(&events_source) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(SandboxError::io(&events_source, e)),
        };
        let harness_source = self.harness_path(session_id);
        let harness = match fs::read(&harness_source) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(SandboxError::io(&harness_source, e)),
        };

        let exported_at = now_rfc3339();
        let manifest = ExportManifest {
            schema: EXPORT_SCHEMA.to_owned(),
            sandbox_session_id: record.sandbox_session_id.clone(),
            checkpoint_id: record.checkpoint_id.clone(),
            checkpoint_fingerprint: record.checkpoint_fingerprint.clone(),
            created_at: record.created_at.clone(),
            exported_at: exported_at.clone(),
            backend: record.backend.clone(),
            tool_allowlist: record.tool_allowlist.clone(),
            notes,
            isolation: record.isolation.clone(),
            transcript_entries,
            dialogue_threads: dialogue::effective_threads(&record).threads.len(),
            files: {
                let mut files = vec![MANIFEST_FILE.to_owned(), TRANSCRIPT_FILE.to_owned()];
                if !events.is_empty() {
                    files.push(EVENTS_FILE.to_owned());
                }
                if !harness.is_empty() {
                    files.push(HARNESS_FILE.to_owned());
                }
                files
            },
        };

        let base = format!("sandbox_export_{session_id}");
        let mut export_dir = parent.join(&base);
        let mut n = 2;
        while export_dir.exists() {
            export_dir = parent.join(format!("{base}-{n}"));
            n += 1;
        }
        let partial = parent.join(format!(".{base}.partial-{}", random_hex(4)?));
        let written = (|| {
            fs::create_dir(&partial).map_err(|e| SandboxError::io(&partial, e))?;
            write_json_atomic(&partial.join(MANIFEST_FILE), &manifest)?;
            let path = partial.join(TRANSCRIPT_FILE);
            fs::write(&path, &transcript).map_err(|e| SandboxError::io(&path, e))?;
            if !events.is_empty() {
                let path = partial.join(EVENTS_FILE);
                fs::write(&path, &events).map_err(|e| SandboxError::io(&path, e))?;
            }
            if !harness.is_empty() {
                let path = partial.join(HARNESS_FILE);
                fs::write(&path, &harness).map_err(|e| SandboxError::io(&path, e))?;
            }
            fs::rename(&partial, &export_dir).map_err(|e| SandboxError::io(&export_dir, e))
        })();
        if let Err(error) = written {
            let _ = fs::remove_dir_all(&partial);
            return Err(error);
        }

        record.state = SessionState::Exported;
        record.exports.push(ExportRecord {
            exported_at,
            path: export_dir.clone(),
        });
        write_json_atomic(&session_dir.join(SESSION_FILE), &record)?;
        Ok(ExportOutcome {
            export_dir,
            manifest,
            session: self.live_view(&record),
        })
    }

    fn live_view(&self, record: &SessionRecord) -> SessionView {
        SessionView {
            sandbox_session_id: record.sandbox_session_id.clone(),
            state: record.state,
            checkpoint_id: record.checkpoint_id.clone(),
            checkpoint_digest: Some(record.checkpoint_fingerprint.digest.clone()),
            created_at: record.created_at.clone(),
            last_exported_at: record.exports.last().map(|e| e.exported_at.clone()),
            export_count: record.exports.len(),
            discarded_at: None,
            workspace_root: Some(self.session_workspace(&record.sandbox_session_id)),
        }
    }

    fn read_tombstones(&self) -> Result<Vec<Tombstone>> {
        let path = self.root.join(TOMBSTONES_FILE);
        let file = match fs::File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(SandboxError::io(&path, e)),
        };
        let mut out = Vec::new();
        for line in BufReader::new(file).lines() {
            let line = line.map_err(|e| SandboxError::io(&path, e))?;
            if line.trim().is_empty() {
                continue;
            }
            out.push(
                serde_json::from_str(&line).map_err(|source| SandboxError::Json {
                    path: path.clone(),
                    source,
                })?,
            );
        }
        Ok(out)
    }

    fn append_tombstone(&self, tombstone: &Tombstone) -> Result<()> {
        let path = self.root.join(TOMBSTONES_FILE);
        let mut line = serde_json::to_string(tombstone).map_err(|source| SandboxError::Json {
            path: path.clone(),
            source,
        })?;
        line.push('\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| SandboxError::io(&path, e))?;
        file.write_all(line.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|e| SandboxError::io(&path, e))
    }

    /// Best effort: finish interrupted discards and drop abandoned staging.
    fn sweep_internal(&self) {
        for dir in [STAGING_DIR, TRASH_DIR] {
            if let Ok(entries) = fs::read_dir(self.root.join(dir)) {
                for entry in entries.flatten() {
                    let _ = fs::remove_dir_all(entry.path());
                }
            }
        }
    }
}

// ---------------------------------------------------------------- helpers

fn now_rfc3339() -> String {
    Local::now().to_rfc3339_opts(SecondsFormat::Millis, false)
}

fn random_hex(bytes: usize) -> Result<String> {
    let mut buf = vec![0_u8; bytes];
    getrandom::fill(&mut buf).map_err(|e| SandboxError::Random(e.to_string()))?;
    Ok(fingerprint::hex(&buf))
}

fn new_session_id() -> Result<String> {
    Ok(format!(
        "{SESSION_ID_PREFIX}{}-{}",
        Local::now().format("%Y%m%dT%H%M%S"),
        random_hex(4)?
    ))
}

fn validate_session_id(id: &str) -> Result<()> {
    let ok = id.starts_with(SESSION_ID_PREFIX)
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(SandboxError::Invalid(format!("非法会话 id：{id}")))
    }
}

fn slug(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-');
    out.chars()
        .take(40)
        .collect::<String>()
        .trim_end_matches('-')
        .to_owned()
}

fn unique_checkpoint_id(label: &str, root: &Path, existing: &[CheckpointRecord]) -> String {
    let path_hash = {
        let digest = Sha256::digest(root.to_string_lossy().as_bytes());
        fingerprint::hex(&digest[..4])
    };
    let base = match slug(label) {
        s if s.is_empty() => format!("ckpt-{path_hash}"),
        s => s,
    };
    let taken = |id: &str| existing.iter().any(|c| c.checkpoint_id == id);
    if !taken(&base) {
        return base;
    }
    let hashed = format!("{base}-{path_hash}");
    if !taken(&hashed) {
        return hashed;
    }
    (2..)
        .map(|n| format!("{hashed}-{n}"))
        .find(|id| !taken(id))
        .expect("unbounded suffix search")
}

fn truncate_chars(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).map_err(|e| SandboxError::io(path, e))?;
    serde_json::from_slice(&bytes).map_err(|source| SandboxError::Json {
        path: path.to_owned(),
        source,
    })
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|source| SandboxError::Json {
        path: path.to_owned(),
        source,
    })?;
    bytes.push(b'\n');
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = fs::File::create(&tmp).map_err(|e| SandboxError::io(&tmp, e))?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|e| SandboxError::io(&tmp, e))?;
    }
    fs::rename(&tmp, path).map_err(|e| SandboxError::io(path, e))
}

/// Read and schema-check an export package's manifest.
pub fn read_export_manifest(export_dir: &Path) -> Result<ExportManifest> {
    let manifest: ExportManifest = read_json(&export_dir.join(MANIFEST_FILE))?;
    if manifest.schema != EXPORT_SCHEMA {
        return Err(SandboxError::Invalid(format!(
            "未知导出 schema：{}",
            manifest.schema
        )));
    }
    if !export_dir.join(TRANSCRIPT_FILE).is_file() {
        return Err(SandboxError::Invalid(format!(
            "导出包缺少 {TRANSCRIPT_FILE}：{}",
            export_dir.display()
        )));
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests;
