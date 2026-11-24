//! S1 dialogue shell over a sandbox session (docs/sandbox-flow.md §4.2, §5 S1).
//!
//! One user message = one `agent_events v0` round on the selected local
//! executor ([`CodexExecutor`] or [`OpenCodeExecutor`]), with Mathscape MCP
//! tools mounted against the session's *isolated copy* (never the checkpoint,
//! never the mother workspace). Workflow is "diluted" to chat: there is no
//! planner and no propose/accept; Mathscape MCP tools stay read-only (T2).
//!
//! The harness itself runs under a per-session permission profile
//! ([`round_harness`]): it may read the workspace copy, the versioned skill
//! pack and the managed Lean profile, and write only the session's
//! `scratch/` (drafts). OpenCode uses approval-layer checks, not OS enforcement;
//! its isolation is weaker than Codex's, and shell/network tools are disabled.
//! Each round's harness context
//! (skill version + hash, grants, Lean profile root) is appended to the
//! session's `harness.jsonl`.
//!
//! Each sandbox session has one long-lived Codex app-server (a harness
//! session keyed by the sandbox session id; tool allow-list fixed per
//! session) and each dialogue thread is one persistent Codex thread
//! (docs/codex-session-persistence.md). The thread binding lives in
//! `session.json`, a copy of the rollout in `harness/codex/`, so a resumed
//! or moved session continues the same agent context; only when the harness
//! cannot reload the thread is the context rebuilt from recent transcript
//! lines ([`build_dialogue_prompt`]) and the dialogue shows 「上下文已降级」.
//! Per-turn notes (first-turn skill read, workspace moved, tool/skill
//! updated, 「自上轮以来工作区变化」) go into a separate context item
//! ([`crate::sandbox_continuity`]). 「新对话」 opens another thread on the
//! same isolated copy: the agent context starts empty, workspace state
//! carries over (docs/sandbox-flow.md §3.5). Everything the round produces stays inside the
//! session directory: `transcript.jsonl` (user / agent / system lines) and
//! `events.jsonl` (the raw event stream). The UI receives the same events
//! live on [`DIALOGUE_EVENT`]; it never shows a reply that did not come from
//! the event stream (`turn_end` + [`AgentExecutor::final_text`]).

use std::{
    collections::{HashMap, HashSet},
    env,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use futures::StreamExt;
use mathscape_agent::{
    AgentBackend, AgentEffort, AgentExecutor, AgentModel, CapabilityProbe, CodexConfig,
    CodexExecutor, ContextNote, EventPayload, HarnessContext, LoadThread, OpenCodeConfig,
    OpenCodeExecutor, Prompt, RoundId, SessionSpec, ThreadBinding, ThreadContinuity, TurnOutcome,
};
use mathscape_sandbox::{
    DialogueThread, HarnessThreadBinding, MAX_MESSAGE_CHARS, SandboxError, SandboxHome,
    TranscriptEntry, TranscriptRole, build_dialogue_prompt,
};
use mathscape_store_sqlite::{PendingQuestionAnswer, PendingQuestionRecord};
use mathscape_tool_lean::{LEAN_PROFILE_ROOT_ENV, discover_profile_root};
use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, State};

use crate::sandbox::SandboxState;
use crate::sandbox_continuity::{
    NoteInputs, WorkspaceDigest, context_notes, digest_workspace, keep_rollout_copy, load_digest,
    save_digest,
};
use crate::skill_pack::{SkillPack, load_worknet_skill, locate_skills_root};

/// Codex CLI used for dialogue rounds (default `codex` on `PATH`).
pub(crate) const CODEX_PROGRAM_ENVIRONMENT_VARIABLE: &str = "MATHSCAPE_CODEX_PROGRAM";
/// `mathscape` CLI that Codex spawns as `mathscape mcp` (default: the binary
/// beside this executable if present, else `mathscape` on `PATH`). Setting
/// this variable always wins over the sibling binary. `tauri dev` / `tauri
/// build` build the sibling first (`beforeDevCommand` / `beforeBuildCommand`
/// in `tauri.conf.json` run `cargo build -p mathscape-cli [--release]`).
pub(crate) const MCP_PROGRAM_ENVIRONMENT_VARIABLE: &str = "MATHSCAPE_MCP_PROGRAM";
/// Tauri event carrying [`DialogueUpdate`]s.
pub(crate) const DIALOGUE_EVENT: &str = "sandbox://dialogue";
const OPENCODE_PROGRAM_ENVIRONMENT_VARIABLE: &str = "MATHSCAPE_OPENCODE_PROGRAM";
const BACKEND_ENVIRONMENT_VARIABLE: &str = "MATHSCAPE_AGENT_BACKEND";

fn parse_backend(value: &str) -> Result<AgentBackend, String> {
    match value.trim() {
        "codex" => Ok(AgentBackend::Codex),
        "opencode" => Ok(AgentBackend::OpenCode),
        _ => Err(format!("不支持的执行器：{value}")),
    }
}

fn backend_name(backend: AgentBackend) -> &'static str {
    match backend {
        AgentBackend::OpenCode => "OpenCode",
        _ => "Codex",
    }
}

/// Live update for the dialogue view.
/// Blocking-side result of [`SandboxDialogueState::status`].
#[derive(Default)]
struct StatusSnapshot {
    backends: Vec<BackendStatus>,
    probes: Vec<CapabilityProbe>,
    model: Option<String>,
    models: Vec<AgentModel>,
    model_selection: bool,
    effort: Option<String>,
    efforts: Vec<AgentEffort>,
    effort_selection: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingQuestionView {
    pub question_id: String,
    pub round_id: String,
    pub thread_id: String,
    pub question: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
    pub ink_requested: bool,
    pub state: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxInkSnapshotChoice {
    pub snapshot_id: String,
    pub document_id: String,
    pub document_title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DialogueUpdate {
    /// One `agent_events v0` event, verbatim.
    AgentEvent {
        sandbox_session_id: String,
        event: Value,
    },
    /// A transcript line was appended (user message, agent reply, system note).
    Transcript {
        sandbox_session_id: String,
        entry: TranscriptEntry,
    },
    /// Updated `user.ask` pending question list (emitted when questions change or a round ends).
    PendingQuestions {
        sandbox_session_id: String,
        pending_questions: Vec<PendingQuestionView>,
        ink_snapshots: Vec<SandboxInkSnapshotChoice>,
    },
    /// The round's event pump stopped; the session accepts a new message.
    RoundFinished {
        sandbox_session_id: String,
        round_id: String,
    },
}

type Sink = Arc<dyn Fn(DialogueUpdate) + Send + Sync>;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub backend: &'static str,
    pub codex_program: PathBuf,
    pub program: PathBuf,
    pub backends: Vec<BackendStatus>,
    pub backend_selection: bool,
    pub mcp_program: PathBuf,
    pub tools: Vec<String>,
    pub probes: Vec<CapabilityProbe>,
    pub available: bool,
    /// Current model id from the adapter (`AgentExecutor::current_model`).
    pub model: Option<String>,
    /// Selectable models from the adapter (`list_models`). Empty when unsupported.
    pub models: Vec<AgentModel>,
    /// True when the selected adapter implements model selection.
    pub model_selection: bool,
    /// Reasoning effort the next round will use (`AgentExecutor::current_effort`).
    pub effort: Option<String>,
    /// Efforts the current model advertises (`list_efforts(None)`). Empty when
    /// the catalog says nothing for this model.
    pub efforts: Vec<AgentEffort>,
    /// True when the selected adapter implements effort selection.
    pub effort_selection: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendStatus {
    pub backend: &'static str,
    pub program: PathBuf,
    pub available: bool,
    pub probes: Vec<CapabilityProbe>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDialogue {
    pub sandbox_session_id: String,
    /// Every thread's lines in write order (each carries `thread_id`).
    pub transcript: Vec<TranscriptEntry>,
    /// Threads in creation order; all share the session's isolated copy.
    pub threads: Vec<DialogueThread>,
    /// The only thread that accepts new messages; others are read-only.
    pub active_thread_id: String,
    pub events: Vec<Value>,
    /// Round currently streaming for this session, if any.
    pub active_round_id: Option<String>,
    /// Open `state = 'pending'` clarification questions for this session.
    pub pending_questions: Vec<PendingQuestionView>,
    /// Candidate ink snapshots from the session copy when a pending question sets `ink_requested`.
    pub ink_snapshots: Vec<SandboxInkSnapshotChoice>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendOutcome {
    pub round_id: String,
    pub user_entry: TranscriptEntry,
    pub active_thread_id: String,
    pub pending_questions: Vec<PendingQuestionView>,
    pub ink_snapshots: Vec<SandboxInkSnapshotChoice>,
}

#[derive(Clone)]
pub(crate) struct SandboxDialogueState {
    inner: Arc<Inner>,
}

struct Inner {
    executor: Arc<dyn AgentExecutor>,
    opencode: Arc<dyn AgentExecutor>,
    opencode_program: PathBuf,
    selected: Mutex<AgentBackend>,
    backend_override: Option<AgentBackend>,
    backend_file: PathBuf,
    round_backends: Mutex<HashMap<RoundId, AgentBackend>>,
    codex_program: PathBuf,
    mcp_program: PathBuf,
    /// One round at a time per session. `None` = reserved, round starting.
    active: Mutex<HashMap<String, Option<RoundId>>>,
    /// Sandbox sessions with an open harness session (app-server).
    opened: Mutex<HashSet<String>>,
}

/// Everything `send` gathers under the sandbox lock before talking to Codex.
struct Prepared {
    backend: AgentBackend,
    workspace: PathBuf,
    user_entry: TranscriptEntry,
    thread: String,
    dialogue_thread: Option<DialogueThread>,
    later_thread: bool,
    history: Vec<TranscriptEntry>,
    harness: HarnessContext,
    harness_record: Value,
    round_file: PathBuf,
    state_dir: PathBuf,
    skill: Option<SkillPack>,
    before: Option<WorkspaceDigest>,
    now: Option<WorkspaceDigest>,
    extra_notes: Vec<ContextNote>,
    pending_questions: Vec<PendingQuestionView>,
    ink_snapshots: Vec<SandboxInkSnapshotChoice>,
}

enum Submission {
    FreeMessage {
        text: String,
    },
    QuestionAnswer {
        question_id: String,
        text: Option<String>,
        selected_option: Option<String>,
        ink_snapshot_ref: Option<String>,
    },
}

/// Mathscape MCP tools mounted in sandbox rounds. Proposal writes are safe
/// because they only create staged versions in the isolated copy; the
/// Desktop-only accept/discard commands remain outside MCP.
/// The skill's runtime-capabilities sandbox section must list exactly these
/// (checked by `skill_pack` tests).
pub(crate) fn sandbox_tool_allowlist() -> Vec<String> {
    mathscape_mcp::tools::TOOL_NAMES
        .iter()
        .filter(|name| **name != mathscape_mcp::tools::ARTIFACT_STAGE)
        .map(|name| (*name).to_owned())
        .collect()
}

/// Harness grants for one sandbox round and the line recorded in
/// `harness.jsonl`.
///
/// The Lean profile root is resolved from the *checkpoint* (the real
/// workspace): the session copy has no parent `.mathscape/lean/profiles`.
/// It is passed as `MATHSCAPE_LEAN_PROFILE_ROOT` to the Codex child (and on
/// to the MCP child) and granted read-only to harness tools.
#[cfg(test)]
pub(crate) fn round_harness(
    checkpoint_root: Option<&Path>,
    scratch: PathBuf,
    tools: &[String],
) -> (HarnessContext, Value) {
    round_harness_for_investigation(checkpoint_root, scratch, tools, None)
}

/// Same permission profile with a durable investigation binding. The
/// generated Codex round remains a separate execution token; the MCP child
/// receives this value through the explicit forwarded environment seam.
pub(crate) fn round_harness_for_investigation(
    checkpoint_root: Option<&Path>,
    scratch: PathBuf,
    tools: &[String],
    investigation_id: Option<&str>,
) -> (HarnessContext, Value) {
    let mut harness = HarnessContext {
        scratch_dir: Some(scratch.clone()),
        ..HarnessContext::default()
    };
    let lean = discover_profile_root(checkpoint_root).filter(|found| found.root.is_dir());
    if let Some(found) = &lean {
        harness.read_roots.push(found.root.clone());
        harness.env.push((
            LEAN_PROFILE_ROOT_ENV.to_owned(),
            found.root.display().to_string(),
        ));
    }
    // Resident Loogle socket (the desktop owns the process; the per-round MCP
    // child only connects to it). Absent: mathlib.search stays one-shot.
    let loogle_socket = crate::loogle_host::round_env();
    if let Some(entry) = loogle_socket.clone() {
        harness.env.push(entry);
    }
    if let Some(investigation_id) = investigation_id.filter(|value| !value.trim().is_empty()) {
        harness.env.push((
            "MATHSCAPE_INVESTIGATION_ID".to_owned(),
            investigation_id.to_owned(),
        ));
    }
    let skill = locate_skills_root().map(|root| load_worknet_skill(&root));
    let skill_record = match &skill {
        Some(Ok(pack)) => {
            harness.skill_roots.push(pack.root.clone());
            harness.instructions = Some(pack.instructions());
            json!({
                "name": pack.name,
                "version": pack.version,
                "tool_schema_version": pack.tool_schema_version,
                "sha256": pack.sha256,
                "path": pack.dir,
            })
        }
        Some(Err(problem)) => json!({ "problem": problem }),
        None => json!({ "problem": "未找到 skill 目录（可设置 MATHSCAPE_SKILLS_ROOT）" }),
    };
    let record = json!({
        "schema": "mathscape.sandbox_harness.v0",
        "permission_profile": "mathscape-session",
        "skill": skill_record,
        "lean_profile_root": lean,
        "loogle_resident_socket": loogle_socket.map(|(_, path)| path),
        "investigation_id": investigation_id,
        "read_roots": harness.read_roots,
        "skill_roots": harness.skill_roots,
        "scratch": scratch,
        "network": false,
        "tools": tools,
    });
    (harness, record)
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn default_mcp_program() -> PathBuf {
    let file = format!("mathscape{}", env::consts::EXE_SUFFIX);
    env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(&file)))
        .filter(|candidate| candidate.is_file())
        .unwrap_or_else(|| PathBuf::from("mathscape"))
}

async fn with_home<T, F>(home: &Arc<Mutex<SandboxHome>>, work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&SandboxHome) -> Result<T, SandboxError> + Send + 'static,
{
    with_home_str(home, move |home| {
        work(home).map_err(|error| error.to_string())
    })
    .await
}

async fn with_home_str<T, F>(home: &Arc<Mutex<SandboxHome>>, work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&SandboxHome) -> Result<T, String> + Send + 'static,
{
    let home = Arc::clone(home);
    tauri::async_runtime::spawn_blocking(move || {
        let home = home
            .lock()
            .map_err(|_| "沙箱状态锁已损坏，请重启应用".to_owned())?;
        work(&home)
    })
    .await
    .map_err(|error| format!("sandbox dialogue worker failed: {error}"))?
}

fn open_or_upgrade_session_store(
    workspace: &Path,
) -> Result<Option<mathscape_store_sqlite::SqliteArtifactStore>, String> {
    let db = workspace.join("mathscape.sqlite3");
    if !db.is_file() {
        return Ok(None);
    }
    mathscape_store_sqlite::SqliteArtifactStore::prepare(&db)
        .map(Some)
        .map_err(|error| format!("升级沙箱副本数据库失败：{error}"))
}

fn resolve_question_thread_id(
    transcript: &[TranscriptEntry],
    active_thread_id: &str,
    round_id: &str,
) -> String {
    transcript
        .iter()
        .rev()
        .find(|entry| entry.round_id.as_deref() == Some(round_id))
        .map(|entry| entry.thread().to_owned())
        .unwrap_or_else(|| active_thread_id.to_owned())
}

fn collect_session_questions_and_ink(
    store: Option<&mathscape_store_sqlite::SqliteArtifactStore>,
    session_id: &str,
    transcript: &[TranscriptEntry],
    active_thread_id: &str,
) -> (Vec<PendingQuestionView>, Vec<SandboxInkSnapshotChoice>) {
    let Some(store) = store else {
        return (Vec::new(), Vec::new());
    };
    let investigation_id = mathscape_mcp::investigation::sandbox_investigation_id(session_id);
    let pending_questions: Vec<PendingQuestionView> = store
        .list_pending_questions(&investigation_id, true)
        .unwrap_or_default()
        .into_iter()
        .map(|q| PendingQuestionView {
            question_id: q.question_id,
            thread_id: resolve_question_thread_id(transcript, active_thread_id, &q.round_id),
            round_id: q.round_id,
            question: q.question,
            options: q.options,
            ink_requested: q.ink_requested,
            state: q.state,
            created_at: q.created_at,
        })
        .collect();
    let mut ink_snapshots = Vec::new();
    if pending_questions.iter().any(|q| q.ink_requested)
        && let Ok(docs) = store.list_ink_documents()
    {
        for doc in docs.into_iter().filter(|doc| doc.snapshot_count > 0) {
            if let Ok(snaps) = store.list_ink_snapshots(&doc.document_id) {
                for snap in snaps {
                    ink_snapshots.push(SandboxInkSnapshotChoice {
                        snapshot_id: snap.snapshot_id,
                        document_id: doc.document_id.clone(),
                        document_title: doc.title.clone(),
                        label: snap.label,
                        created_at: snap.created_at,
                    });
                }
            }
        }
        ink_snapshots.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    }
    (pending_questions, ink_snapshots)
}

fn read_session_questions_and_ink(
    home: &SandboxHome,
    session_id: &str,
    transcript: &[TranscriptEntry],
    active_thread_id: &str,
) -> (Vec<PendingQuestionView>, Vec<SandboxInkSnapshotChoice>) {
    let workspace = home
        .live_session(session_id)
        .ok()
        .and_then(|view| view.workspace_root)
        .unwrap_or_else(|| home.session_workspace(session_id));
    let store = open_or_upgrade_session_store(&workspace)
        .map_err(|error| eprintln!("mathscape: {error}"))
        .ok()
        .flatten();
    collect_session_questions_and_ink(store.as_ref(), session_id, transcript, active_thread_id)
}

fn trim_non_empty(value: Option<String>) -> Option<String> {
    value.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
}

fn format_answer_user_bubble(
    selected_option: Option<&str>,
    note_text: Option<&str>,
    ink_snapshot_ref: Option<&str>,
) -> String {
    let mut lines = Vec::new();
    if let Some(opt) = selected_option {
        lines.push(format!("【确认：{opt}】"));
    }
    if let Some(note) = note_text {
        lines.push(note.to_owned());
    }
    if let Some(snap) = ink_snapshot_ref {
        lines.push(format!("（演算快照：{snap}）"));
    }
    lines.join("\n")
}

fn format_answer_context_note(
    record: &PendingQuestionRecord,
    answer: &PendingQuestionAnswer,
) -> ContextNote {
    ContextNote {
        key: "mathscape.user_answer".to_owned(),
        text: format!(
            "用户已回答你上一轮的 `user.ask` 追问：\n{}",
            serde_json::to_string_pretty(&json!({
                "question_id": record.question_id,
                "question": record.question,
                "selected_option": answer.selected_option,
                "answer_text": answer.text,
                "ink_snapshot_ref": answer.ink_snapshot_ref,
            }))
            .unwrap_or_default()
        ),
    }
}

impl SandboxDialogueState {
    /// `resident_log`: where app-server lifecycle events go
    /// (`resident-processes.jsonl`, shared with the resident Loogle).
    pub(crate) fn from_environment(resident_log: Option<PathBuf>, state_dir: PathBuf) -> Self {
        let config = CodexConfig {
            program: env_path(CODEX_PROGRAM_ENVIRONMENT_VARIABLE)
                .unwrap_or_else(|| PathBuf::from("codex")),
            mcp_program: env_path(MCP_PROGRAM_ENVIRONMENT_VARIABLE)
                .unwrap_or_else(default_mcp_program),
            ..CodexConfig::default()
        };
        let opencode = OpenCodeConfig {
            program: env_path(OPENCODE_PROGRAM_ENVIRONMENT_VARIABLE)
                .unwrap_or_else(|| PathBuf::from("opencode")),
            mcp_program: config.mcp_program.clone(),
            state_dir: state_dir.join("opencode"),
            ..OpenCodeConfig::default()
        };
        Self::with_configs(
            config,
            opencode,
            state_dir.join("agent-backend"),
            env::var(BACKEND_ENVIRONMENT_VARIABLE)
                .ok()
                .and_then(|value| parse_backend(&value).ok()),
            crate::resident_log::jsonl_sink(resident_log),
        )
    }

    #[cfg(test)]
    fn with_config_and_sink(config: CodexConfig, sink: mathscape_resident::EventSink) -> Self {
        let root = config.home.as_ref().unwrap().parent().unwrap();
        let opencode = OpenCodeConfig {
            mcp_program: config.mcp_program.clone(),
            state_dir: root.join("opencode"),
            ..OpenCodeConfig::default()
        };
        let backend_file = root.join("agent-backend");
        Self::with_configs(config, opencode, backend_file, None, sink)
    }

    fn with_configs(
        config: CodexConfig,
        opencode: OpenCodeConfig,
        backend_file: PathBuf,
        backend_override: Option<AgentBackend>,
        sink: mathscape_resident::EventSink,
    ) -> Self {
        let selected = std::fs::read_to_string(&backend_file)
            .ok()
            .and_then(|value| parse_backend(&value).ok())
            .unwrap_or(AgentBackend::Codex);
        Self {
            inner: Arc::new(Inner {
                codex_program: config.program.clone(),
                mcp_program: config.mcp_program.clone(),
                opencode_program: opencode.program.clone(),
                opencode: Arc::new(
                    OpenCodeExecutor::new(opencode).with_event_sink(Arc::clone(&sink)),
                ),
                executor: Arc::new(CodexExecutor::new(config).with_event_sink(sink)),
                selected: Mutex::new(selected),
                backend_override,
                backend_file,
                round_backends: Mutex::new(HashMap::new()),
                active: Mutex::new(HashMap::new()),
                opened: Mutex::new(HashSet::new()),
            }),
        }
    }

    fn backend(&self) -> AgentBackend {
        self.inner
            .backend_override
            .unwrap_or_else(|| *self.inner.selected.lock().unwrap())
    }

    fn executor(&self, backend: AgentBackend) -> Arc<dyn AgentExecutor> {
        Arc::clone(if backend == AgentBackend::OpenCode {
            &self.inner.opencode
        } else {
            &self.inner.executor
        })
    }

    fn round_executor(&self, round: &RoundId) -> Option<Arc<dyn AgentExecutor>> {
        let backend = self
            .inner
            .round_backends
            .lock()
            .unwrap()
            .get(round)
            .copied();
        backend.map(|backend| self.executor(backend))
    }

    async fn set_backend(&self, value: String) -> Result<AgentStatus, String> {
        let backend = parse_backend(&value)?;
        if self.inner.backend_override.is_some() {
            return Err("执行器已由 MATHSCAPE_AGENT_BACKEND 固定".into());
        }
        let executor = self.executor(backend);
        let probes = tauri::async_runtime::spawn_blocking(move || executor.probe())
            .await
            .map_err(|error| error.to_string())?;
        if probes.is_empty() || !probes.iter().all(CapabilityProbe::is_available) {
            return Err(format!("{} 未安装或不可用", backend_name(backend)));
        }
        {
            let mut selected = self.inner.selected.lock().unwrap();
            let path = &self.inner.backend_file;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let temporary = path.with_extension("tmp");
            std::fs::write(&temporary, backend.as_str()).map_err(|e| e.to_string())?;
            std::fs::rename(&temporary, path).map_err(|e| e.to_string())?;
            *selected = backend;
        }
        Ok(self.status().await)
    }

    fn active(&self) -> std::sync::MutexGuard<'_, HashMap<String, Option<RoundId>>> {
        self.inner
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn active_round(&self, session_id: &str) -> Option<RoundId> {
        self.active().get(session_id).cloned().flatten()
    }

    fn release(&self, session_id: &str, round: Option<&RoundId>) {
        let mut active = self.active();
        if active
            .get(session_id)
            .is_some_and(|slot| slot.as_ref() == round)
        {
            active.remove(session_id);
            if let Some(round) = round {
                self.inner.round_backends.lock().unwrap().remove(round);
            }
        }
    }

    fn opened(&self) -> std::sync::MutexGuard<'_, HashSet<String>> {
        self.inner
            .opened
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Stop the session's app-server (discard). Threads stay resumable.
    pub(crate) async fn close_harness_session(&self, session_id: &str) {
        self.opened().remove(session_id);
        let _ = self.inner.executor.close_session(session_id).await;
        let _ = self.inner.opencode.close_session(session_id).await;
    }

    /// Stop the app-servers of every idle session except `keep` (session
    /// switch). A session with a streaming round keeps its process.
    pub(crate) async fn close_other_harness_sessions(&self, keep: Option<&str>) {
        let busy: HashSet<String> = self.active().keys().cloned().collect();
        let victims: Vec<String> = self
            .opened()
            .iter()
            .filter(|id| Some(id.as_str()) != keep && !busy.contains(*id))
            .cloned()
            .collect();
        for id in victims {
            self.close_harness_session(&id).await;
        }
    }

    /// App exit: stop every app-server (blocking, bounded).
    pub(crate) fn close_all(&self) {
        self.opened().clear();
        self.inner.executor.close_all();
        self.inner.opencode.close_all();
    }

    /// Best-effort cancel of the session's running round (used by discard).
    pub(crate) async fn cancel_session_round(&self, session_id: &str) {
        if let Some(round) = self.active_round(session_id)
            && let Some(executor) = self.round_executor(&round)
        {
            let _ = executor.cancel(&round).await;
        }
    }

    async fn status(&self) -> AgentStatus {
        let backend = self.backend();
        let executor = self.executor(backend);
        let peers = [
            (
                AgentBackend::Codex,
                self.inner.codex_program.clone(),
                self.executor(AgentBackend::Codex),
            ),
            (
                AgentBackend::OpenCode,
                self.inner.opencode_program.clone(),
                self.executor(AgentBackend::OpenCode),
            ),
        ];
        let snapshot = tauri::async_runtime::spawn_blocking(move || {
            let backends: Vec<_> = peers
                .into_iter()
                .map(|(backend, program, executor)| {
                    let probes = executor.probe();
                    BackendStatus {
                        backend: backend.as_str(),
                        program,
                        available: !probes.is_empty()
                            && probes.iter().all(CapabilityProbe::is_available),
                        probes,
                    }
                })
                .collect();
            let selected = backends
                .iter()
                .find(|item| item.backend == backend.as_str())
                .unwrap();
            let probes = selected.probes.clone();
            let model = executor.current_model();
            let (models, model_selection) = match selected.available.then(|| executor.list_models())
            {
                Some(Ok(models)) => (models, true),
                _ => (Vec::new(), false),
            };
            let effort = executor.current_effort();
            let efforts = executor.list_efforts(None);
            // Selectable only when the adapter supports it and the model
            // advertises at least one level (otherwise nothing to validate).
            let (efforts, effort_selection) = match efforts {
                Ok(efforts) => {
                    let selectable = !efforts.is_empty();
                    (efforts, selectable)
                }
                Err(_) => (Vec::new(), false),
            };
            StatusSnapshot {
                backends,
                probes,
                model,
                models,
                model_selection,
                effort,
                efforts,
                effort_selection,
            }
        })
        .await
        .unwrap_or_default();
        let probes = snapshot.probes;
        AgentStatus {
            backend: backend.as_str(),
            program: if backend == AgentBackend::OpenCode {
                self.inner.opencode_program.clone()
            } else {
                self.inner.codex_program.clone()
            },
            backends: snapshot.backends,
            backend_selection: self.inner.backend_override.is_none(),
            codex_program: self.inner.codex_program.clone(),
            mcp_program: self.inner.mcp_program.clone(),
            tools: sandbox_tool_allowlist(),
            available: !probes.is_empty() && probes.iter().all(CapabilityProbe::is_available),
            probes,
            model: snapshot.model,
            models: snapshot.models,
            model_selection: snapshot.model_selection,
            effort: snapshot.effort,
            efforts: snapshot.efforts,
            effort_selection: snapshot.effort_selection,
        }
    }

    async fn set_effort(&self, effort: String) -> Result<AgentStatus, String> {
        let executor = self.executor(self.backend());
        tauri::async_runtime::spawn_blocking(move || executor.set_effort(&effort))
            .await
            .map_err(|error| format!("set_effort worker failed: {error}"))?
            .map_err(|error| error.to_string())?;
        Ok(self.status().await)
    }

    async fn set_model(&self, id: String) -> Result<AgentStatus, String> {
        let executor = self.executor(self.backend());
        tauri::async_runtime::spawn_blocking(move || executor.set_model(&id))
            .await
            .map_err(|error| format!("set_model worker failed: {error}"))?
            .map_err(|error| error.to_string())?;
        Ok(self.status().await)
    }

    async fn dialogue(
        &self,
        home: &Arc<Mutex<SandboxHome>>,
        session_id: String,
    ) -> Result<SessionDialogue, String> {
        let active_round_id = self
            .active_round(&session_id)
            .map(|r| r.as_str().to_owned());
        with_home(home, move |home| {
            let threads = home.dialogue_threads(&session_id)?;
            let transcript = home.read_transcript(&session_id)?;
            let (pending_questions, ink_snapshots) = read_session_questions_and_ink(
                home,
                &session_id,
                &transcript,
                &threads.active_thread_id,
            );
            Ok(SessionDialogue {
                transcript,
                threads: threads.threads,
                active_thread_id: threads.active_thread_id,
                events: home.read_events(&session_id)?,
                sandbox_session_id: session_id,
                active_round_id,
                pending_questions,
                ink_snapshots,
            })
        })
        .await
    }

    async fn send(
        &self,
        home: Arc<Mutex<SandboxHome>>,
        session_id: String,
        text: String,
        sink: Sink,
    ) -> Result<SendOutcome, String> {
        self.send_turn(home, session_id, Submission::FreeMessage { text }, sink)
            .await
    }

    async fn answer_question(
        &self,
        home: Arc<Mutex<SandboxHome>>,
        session_id: String,
        question_id: String,
        text: Option<String>,
        selected_option: Option<String>,
        ink_snapshot_ref: Option<String>,
        sink: Sink,
    ) -> Result<SendOutcome, String> {
        self.send_turn(
            home,
            session_id,
            Submission::QuestionAnswer {
                question_id,
                text,
                selected_option,
                ink_snapshot_ref,
            },
            sink,
        )
        .await
    }

    async fn dismiss_question(
        &self,
        home: &Arc<Mutex<SandboxHome>>,
        session_id: String,
        question_id: String,
    ) -> Result<SessionDialogue, String> {
        let question_id = question_id.trim().to_owned();
        if question_id.is_empty() {
            return Err("question_id 不能为空".into());
        }
        {
            let mut active = self.active();
            if active.contains_key(&session_id) {
                return Err("上一轮还没结束，请等待或先取消".into());
            }
            active.insert(session_id.clone(), None);
        }
        let sid = session_id.clone();
        let cancelled = with_home_str(home, move |home| {
            let view = home.live_session(&sid).map_err(|e| e.to_string())?;
            let workspace = view
                .workspace_root
                .unwrap_or_else(|| home.session_workspace(&sid));
            let mut store = open_or_upgrade_session_store(&workspace)?
                .ok_or_else(|| "沙箱会话副本不可用，无法忽略问题".to_owned())?;
            let investigation_id = mathscape_mcp::investigation::sandbox_investigation_id(&sid);
            store
                .cancel_pending_question(
                    &investigation_id,
                    &question_id,
                    &mathscape_sandbox::now_rfc3339(),
                )
                .map(|_| ())
                .map_err(|error| error.to_string())
        })
        .await;
        self.release(&session_id, None);
        cancelled?;
        self.dialogue(home, session_id).await
    }

    async fn send_turn(
        &self,
        home: Arc<Mutex<SandboxHome>>,
        session_id: String,
        submission: Submission,
        sink: Sink,
    ) -> Result<SendOutcome, String> {
        let backend = self.backend();
        let normalized = match submission {
            Submission::FreeMessage { text } => {
                let text = text.trim().to_owned();
                if text.is_empty() {
                    return Err("消息为空".into());
                }
                if text.chars().count() > MAX_MESSAGE_CHARS {
                    return Err(format!("消息过长（上限 {MAX_MESSAGE_CHARS} 字符）"));
                }
                Submission::FreeMessage { text }
            }
            Submission::QuestionAnswer {
                question_id,
                text,
                selected_option,
                ink_snapshot_ref,
            } => {
                let question_id = question_id.trim().to_owned();
                if question_id.is_empty() {
                    return Err("question_id 不能为空".into());
                }
                let text = trim_non_empty(text);
                let selected_option = trim_non_empty(selected_option);
                let ink_snapshot_ref = trim_non_empty(ink_snapshot_ref);
                if let Some(ref note) = text
                    && note.chars().count() > MAX_MESSAGE_CHARS
                {
                    return Err(format!("补充说明过长（上限 {MAX_MESSAGE_CHARS} 字符）"));
                }
                if text.is_none() && selected_option.is_none() && ink_snapshot_ref.is_none() {
                    return Err("请至少选择一项、附上演算快照或输入补充说明".into());
                }
                Submission::QuestionAnswer {
                    question_id,
                    text,
                    selected_option,
                    ink_snapshot_ref,
                }
            }
        };
        {
            let mut active = self.active();
            if active.contains_key(&session_id) {
                return Err("上一轮还没结束，请等待或先取消".into());
            }
            active.insert(session_id.clone(), None);
        }

        let tools = sandbox_tool_allowlist();
        let prepared = {
            let (session_id, tools) = (session_id.clone(), tools.clone());
            with_home_str(&home, move |home| {
                let view = home.live_session(&session_id).map_err(|e| e.to_string())?;
                let workspace = view
                    .workspace_root
                    .unwrap_or_else(|| home.session_workspace(&session_id));
                // One sandbox session = one investigation; the thread is metadata.
                let investigation_id =
                    mathscape_mcp::investigation::sandbox_investigation_id(&session_id);
                let mut store = open_or_upgrade_session_store(&workspace)?;
                let mut threads = home
                    .dialogue_threads(&session_id)
                    .map_err(|e| e.to_string())?;
                let full_transcript = home
                    .read_transcript(&session_id)
                    .map_err(|e| e.to_string())?;
                let mut extra_notes = Vec::new();

                let (thread, user_text) = match normalized {
                    Submission::FreeMessage { text } => {
                        let thread = threads.active_thread_id.clone();
                        if let Some(store_mut) = store.as_mut() {
                            let open_for_thread: Vec<PendingQuestionRecord> = store_mut
                                .list_pending_questions(&investigation_id, true)
                                .map_err(|e| e.to_string())?
                                .into_iter()
                                .filter(|q| {
                                    resolve_question_thread_id(
                                        &full_transcript,
                                        &thread,
                                        &q.round_id,
                                    ) == thread
                                })
                                .collect();
                            for open_q in open_for_thread {
                                let answer = PendingQuestionAnswer {
                                    text: text.clone(),
                                    selected_option: None,
                                    ink_snapshot_ref: None,
                                    answered_by: "mathscape-desktop-user".to_owned(),
                                    answered_at: mathscape_sandbox::now_rfc3339(),
                                };
                                let settled = store_mut
                                    .answer_pending_question(
                                        &investigation_id,
                                        &open_q.question_id,
                                        &answer,
                                    )
                                    .map_err(|e| e.to_string())?;
                                extra_notes.push(format_answer_context_note(&settled, &answer));
                            }
                        }
                        (thread, text)
                    }
                    Submission::QuestionAnswer {
                        question_id,
                        text,
                        selected_option,
                        ink_snapshot_ref,
                    } => {
                        let store_mut = store
                            .as_mut()
                            .ok_or_else(|| "沙箱会话副本不可用，无法回答问题".to_owned())?;
                        let target_q = store_mut
                            .list_pending_questions(&investigation_id, false)
                            .map_err(|e| e.to_string())?
                            .into_iter()
                            .find(|q| q.question_id == question_id)
                            .ok_or_else(|| format!("未找到待答问题 {question_id}"))?;
                        if target_q.state != "pending" {
                            return Err(format!("该问题已处理（当前状态：{}）", target_q.state));
                        }
                        let q_thread = resolve_question_thread_id(
                            &full_transcript,
                            &threads.active_thread_id,
                            &target_q.round_id,
                        );
                        if q_thread != threads.active_thread_id {
                            threads = home
                                .activate_dialogue_thread(&session_id, &q_thread)
                                .map_err(|e| e.to_string())?;
                        }
                        if let Some(ref snap_id) = ink_snapshot_ref
                            && store_mut
                                .open_ink_snapshot(snap_id)
                                .map_err(|e| e.to_string())?
                                .is_none()
                        {
                            return Err(format!("未找到演算快照 {snap_id}"));
                        }
                        let answer = PendingQuestionAnswer {
                            text: text.clone().unwrap_or_default(),
                            selected_option: selected_option.clone(),
                            ink_snapshot_ref: ink_snapshot_ref.clone(),
                            answered_by: "mathscape-desktop-user".to_owned(),
                            answered_at: mathscape_sandbox::now_rfc3339(),
                        };
                        let settled = store_mut
                            .answer_pending_question(&investigation_id, &question_id, &answer)
                            .map_err(|e| e.to_string())?;
                        extra_notes.push(format_answer_context_note(&settled, &answer));
                        let bubble = format_answer_user_bubble(
                            selected_option.as_deref(),
                            text.as_deref(),
                            ink_snapshot_ref.as_deref(),
                        );
                        (q_thread, bubble)
                    }
                };

                let later_thread = threads.threads.len() > 1;
                let dialogue_thread = threads
                    .threads
                    .iter()
                    .find(|candidate| candidate.thread_id == thread)
                    .cloned();
                let history = home
                    .read_thread_transcript(&session_id, &thread)
                    .map_err(|e| e.to_string())?;
                let user_entry = home
                    .append_thread_transcript(
                        &session_id,
                        &thread,
                        TranscriptRole::User,
                        &user_text,
                        None,
                        None,
                    )
                    .map_err(|e| e.to_string())?;
                let (pending_questions, ink_snapshots) = collect_session_questions_and_ink(
                    store.as_ref(),
                    &session_id,
                    &full_transcript,
                    &thread,
                );
                drop(store);

                home.record_backend(&session_id, backend.as_str(), &tools)
                    .map_err(|e| e.to_string())?;
                let checkpoint_root = home.checkpoint(&view.checkpoint_id).ok().map(|c| c.root);
                let scratch = home
                    .ensure_session_scratch(&session_id)
                    .map_err(|e| e.to_string())?;
                let (harness, harness_record) = round_harness_for_investigation(
                    checkpoint_root.as_deref(),
                    scratch,
                    &tools,
                    Some(&investigation_id),
                );
                let state_dir = home
                    .ensure_harness_state_dir(&session_id, backend.as_str())
                    .map_err(|e| e.to_string())?;
                let before = load_digest(&state_dir, &thread);
                let now = digest_workspace(&workspace, &investigation_id)
                    .map_err(|error| eprintln!("mathscape: workspace digest failed: {error}"))
                    .ok();
                Ok(Prepared {
                    backend,
                    round_file: home.current_round_path(&session_id),
                    skill: locate_skills_root().and_then(|root| load_worknet_skill(&root).ok()),
                    workspace,
                    user_entry,
                    thread,
                    dialogue_thread,
                    later_thread,
                    history,
                    harness,
                    harness_record,
                    state_dir,
                    before,
                    now,
                    extra_notes,
                    pending_questions,
                    ink_snapshots,
                })
            })
            .await
        };
        let prepared = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                self.release(&session_id, None);
                return Err(error);
            }
        };
        let user_entry = prepared.user_entry.clone();
        let prompt_user_text = user_entry.text.clone();
        let thread = prepared.thread.clone();
        let workspace = prepared.workspace.clone();
        let pending_questions = prepared.pending_questions.clone();
        let ink_snapshots = prepared.ink_snapshots.clone();
        let mut harness_record = prepared.harness_record.clone();
        sink(DialogueUpdate::PendingQuestions {
            sandbox_session_id: session_id.clone(),
            pending_questions: pending_questions.clone(),
            ink_snapshots: ink_snapshots.clone(),
        });
        sink(DialogueUpdate::Transcript {
            sandbox_session_id: session_id.clone(),
            entry: user_entry.clone(),
        });

        // Session switch: idle app-servers of other sessions are stopped.
        self.close_other_harness_sessions(Some(&session_id)).await;
        let executor = self.executor(backend);
        let started = self
            .start_turn(
                &home,
                &session_id,
                &prompt_user_text,
                tools,
                prepared,
                &sink,
            )
            .await
            .and_then(|(round, binding, continuity)| {
                let stream = executor.events(&round).map_err(|error| error.to_string())?;
                Ok((round, binding, continuity, stream))
            });
        let (round, binding, continuity, mut stream) = match started {
            Ok(started) => started,
            Err(error) => {
                self.release(&session_id, None);
                let message = format!("未能启动本机 {}：{error}", backend_name(backend));
                let note = message.clone();
                let (sid, tid) = (session_id.clone(), thread.clone());
                if let Ok(entry) = with_home(&home, move |home| {
                    home.append_thread_transcript(
                        &sid,
                        &tid,
                        TranscriptRole::System,
                        &note,
                        None,
                        Some("failed"),
                    )
                })
                .await
                {
                    sink(DialogueUpdate::Transcript {
                        sandbox_session_id: session_id.clone(),
                        entry,
                    });
                }
                return Err(message);
            }
        };
        self.inner
            .round_backends
            .lock()
            .unwrap()
            .insert(round.clone(), backend);
        let round_workspace = workspace.clone();
        harness_record["harness_thread"] = json!({
            "backend": backend.as_str(),
            "thread_id": binding.thread_id,
            "continuity": continuity,
        });
        // Register round ↔ investigation in the session copy's store so the
        // MCP server can verify the adapter-supplied investigation id.
        {
            let (root, rid, inv) = (
                round_workspace,
                round.as_str().to_owned(),
                mathscape_mcp::investigation::sandbox_investigation_id(&session_id),
            );
            let bound = tauri::async_runtime::spawn_blocking(move || {
                mathscape_store_sqlite::SqliteArtifactStore::open(&root.join("mathscape.sqlite3"))
                    .and_then(|mut store| {
                        store.bind_round_investigation(
                            &rid,
                            &inv,
                            "mathscape-desktop",
                            &format!(
                                "unix:{}",
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs()
                            ),
                        )
                    })
                    .map_err(|error| error.to_string())
            })
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result);
            if let Err(error) = bound {
                eprintln!("mathscape: failed to bind round investigation: {error}");
            }
        }
        self.active()
            .insert(session_id.clone(), Some(round.clone()));
        harness_record["round_id"] = json!(round.as_str());
        {
            let sid = session_id.clone();
            if let Err(error) = with_home(&home, move |home| {
                home.append_harness_context(&sid, &harness_record)
            })
            .await
            {
                eprintln!("mathscape: failed to record harness context: {error}");
            }
        }

        let this = self.clone();
        let pump_round = round.clone();
        let pump_session = session_id.clone();
        let pump_thread = thread.clone();
        let pump_binding = binding;
        let pump_workspace = workspace;
        tauri::async_runtime::spawn(async move {
            let (session_id, round) = (pump_session, pump_round);
            let mut recording = true;
            let mut last_error: Option<String> = None;
            let mut closing: Option<(TranscriptRole, String, &'static str)> = None;
            while let Some(event) = stream.next().await {
                let value = serde_json::to_value(&event).unwrap_or(Value::Null);
                if recording {
                    let (sid, line) = (session_id.clone(), value.clone());
                    // Refused once the session is discarded: stop writing.
                    recording = with_home(&home, move |home| home.append_event(&sid, &line))
                        .await
                        .is_ok();
                }
                sink(DialogueUpdate::AgentEvent {
                    sandbox_session_id: session_id.clone(),
                    event: value,
                });
                match &event.payload {
                    EventPayload::Error(error) => last_error = Some(error.message.clone()),
                    EventPayload::TurnEnd(end) => {
                        closing = Some(match end.outcome {
                            TurnOutcome::Completed => (
                                TranscriptRole::Agent,
                                executor
                                    .final_text(&round)
                                    .filter(|text| !text.trim().is_empty())
                                    .unwrap_or_else(|| "（本轮没有可见回复文本）".into()),
                                "completed",
                            ),
                            TurnOutcome::Cancelled => {
                                (TranscriptRole::System, "本轮已取消。".into(), "cancelled")
                            }
                            TurnOutcome::Failed => (
                                TranscriptRole::System,
                                format!(
                                    "本轮失败：{}",
                                    last_error.clone().unwrap_or_else(|| "未知错误".into())
                                ),
                                "failed",
                            ),
                        });
                        break;
                    }
                    _ => {}
                }
            }
            let (role, text, outcome) = closing.unwrap_or_else(|| {
                (
                    TranscriptRole::System,
                    "事件流在 turn_end 之前中断。".into(),
                    "failed",
                )
            });
            if recording {
                let (sid, rid) = (session_id.clone(), round.as_str().to_owned());
                let tid = pump_thread.clone();
                if let Ok(entry) = with_home(&home, move |home| {
                    home.append_thread_transcript(
                        &sid,
                        &tid,
                        role,
                        &text,
                        Some(&rid),
                        Some(outcome),
                    )
                })
                .await
                {
                    sink(DialogueUpdate::Transcript {
                        sandbox_session_id: session_id.clone(),
                        entry,
                    });
                }
            }
            if recording {
                this.persist_thread_state(&home, &session_id, &pump_thread, &pump_binding)
                    .await;
                let (sid, rid, ws) = (
                    session_id.clone(),
                    round.as_str().to_owned(),
                    pump_workspace,
                );
                if let Ok((pending_questions, ink_snapshots)) = with_home(&home, move |home| {
                    if outcome == "cancelled"
                        && let Ok(Some(mut store)) = open_or_upgrade_session_store(&ws)
                    {
                        let inv = mathscape_mcp::investigation::sandbox_investigation_id(&sid);
                        if let Ok(open_qs) = store.list_pending_questions(&inv, true) {
                            for q in open_qs.into_iter().filter(|q| q.round_id == rid) {
                                let _ = store.cancel_pending_question(
                                    &inv,
                                    &q.question_id,
                                    &mathscape_sandbox::now_rfc3339(),
                                );
                            }
                        }
                    }
                    let threads = home.dialogue_threads(&sid)?;
                    let transcript = home.read_transcript(&sid)?;
                    Ok(read_session_questions_and_ink(
                        home,
                        &sid,
                        &transcript,
                        &threads.active_thread_id,
                    ))
                })
                .await
                {
                    sink(DialogueUpdate::PendingQuestions {
                        sandbox_session_id: session_id.clone(),
                        pending_questions,
                        ink_snapshots,
                    });
                }
            }
            this.release(&session_id, Some(&round));
            sink(DialogueUpdate::RoundFinished {
                sandbox_session_id: session_id,
                round_id: round.as_str().to_owned(),
            });
        });

        Ok(SendOutcome {
            round_id: round.as_str().to_owned(),
            user_entry,
            active_thread_id: thread,
            pending_questions,
            ink_snapshots,
        })
    }

    /// Open the harness session, continue (or start) the dialogue thread's
    /// harness thread, record the binding and run the prompt.
    async fn start_turn(
        &self,
        home: &Arc<Mutex<SandboxHome>>,
        session_id: &str,
        text: &str,
        tools: Vec<String>,
        prepared: Prepared,
        sink: &Sink,
    ) -> Result<(RoundId, HarnessThreadBinding, ThreadContinuity), String> {
        let backend = prepared.backend;
        let executor = self.executor(backend);
        // Skill or tool schema changed since the thread's last turn: restart
        // the app-server (and its MCP) so the new tools are listed; the
        // thread is resumed by id below and told what changed.
        let stale_tools = prepared
            .dialogue_thread
            .as_ref()
            .and_then(|thread| thread.harness.as_ref())
            .zip(prepared.skill.as_ref())
            .is_some_and(|(binding, skill)| {
                binding.skill_sha256.as_deref() != Some(skill.sha256.as_str())
                    || binding.tool_schema_version != skill.tool_schema_version
            });
        if stale_tools && self.opened().contains(session_id) {
            self.close_harness_session(session_id).await;
        }
        executor
            .open_session(SessionSpec {
                backend,
                session_key: session_id.to_owned(),
                workspace: prepared.workspace.clone(),
                tools_allow: tools,
                harness: prepared.harness.clone(),
                round_file: Some(prepared.round_file.clone()),
                ephemeral: false,
            })
            .await
            .map_err(|error| error.to_string())?;
        self.opened().insert(session_id.to_owned());

        let previous = prepared
            .dialogue_thread
            .as_ref()
            .and_then(|thread| thread.harness.clone())
            .filter(|binding| binding.backend == backend.as_str());
        // A dialogue that already has replies but no harness thread predates
        // persistent threads.
        let earlier_replies = prepared
            .history
            .iter()
            .any(|entry| entry.role == TranscriptRole::Agent);
        let switched = prepared
            .dialogue_thread
            .as_ref()
            .and_then(|thread| thread.harness.as_ref())
            .is_some_and(|binding| binding.backend != backend.as_str());
        let (bound, degraded): (ThreadBinding, Option<String>) = match &previous {
            Some(previous) => {
                let state_copy = previous
                    .rollout_file
                    .as_ref()
                    .map(|name| prepared.state_dir.join(name));
                match executor
                    .load_thread(
                        session_id,
                        LoadThread {
                            thread_id: previous.thread_id.clone(),
                            state_copy,
                        },
                    )
                    .await
                {
                    Ok(bound) => (bound, None),
                    Err(error) => (
                        executor
                            .new_thread(session_id)
                            .await
                            .map_err(|error| error.to_string())?,
                        Some(format!(
                            "{} 无法续接原线程（{error}）",
                            backend_name(backend)
                        )),
                    ),
                }
            }
            None => (
                executor
                    .new_thread(session_id)
                    .await
                    .map_err(|error| error.to_string())?,
                // A dialogue from before harness threads were persisted.
                (switched || earlier_replies)
                    .then(|| format!("已切换执行器或没有可续接的 {} 线程", backend_name(backend))),
            ),
        };
        let rebuilt = bound.continuity == ThreadContinuity::Started && degraded.is_some();
        if let Some(reason) = &degraded {
            let note = format!(
                "上下文已降级：{reason}。本轮起改用新线程，并从对话记录重建最近的上下文；之前轮次的工具结果不会回放。"
            );
            let (sid, tid) = (session_id.to_owned(), prepared.thread.clone());
            if let Ok(entry) = with_home(home, move |home| {
                home.append_thread_transcript(
                    &sid,
                    &tid,
                    TranscriptRole::System,
                    &note,
                    None,
                    Some("degraded"),
                )
            })
            .await
            {
                sink(DialogueUpdate::Transcript {
                    sandbox_session_id: session_id.to_owned(),
                    entry,
                });
            }
        }
        let same_thread = previous
            .as_ref()
            .filter(|previous| previous.thread_id == bound.thread_id);
        let mut notes = context_notes(&NoteInputs {
            continuity: bound.continuity,
            previous: same_thread,
            workspace: &prepared.workspace,
            skill: prepared.skill.as_ref(),
            rebuilt,
            later_thread: prepared.later_thread,
            before: same_thread
                .and(prepared.before.as_ref())
                .or_else(|| rebuilt.then_some(prepared.before.as_ref()).flatten()),
            now: prepared.now.as_ref(),
        });
        notes.extend(prepared.extra_notes);
        let prompt_text = if rebuilt {
            build_dialogue_prompt(&prepared.history, text, prepared.later_thread)
        } else {
            text.to_owned()
        };
        let binding = HarnessThreadBinding {
            backend: backend.as_str().to_owned(),
            thread_id: bound.thread_id.clone(),
            rollout_file: same_thread.and_then(|previous| previous.rollout_file.clone()),
            workspace: prepared.workspace.clone(),
            tool_schema_version: prepared
                .skill
                .as_ref()
                .and_then(|skill| skill.tool_schema_version.clone()),
            skill_version: prepared
                .skill
                .as_ref()
                .and_then(|skill| skill.version.clone()),
            skill_sha256: prepared.skill.as_ref().map(|skill| skill.sha256.clone()),
            last_turn_at: same_thread.and_then(|previous| previous.last_turn_at.clone()),
        };
        let round = executor
            .prompt(
                session_id,
                Prompt {
                    thread_id: bound.thread_id.clone(),
                    text: prompt_text,
                    context: notes,
                },
            )
            .await
            .map_err(|error| error.to_string())?;
        // Bind before the turn ends so a crash mid-turn still resumes.
        let (sid, tid, record) = (
            session_id.to_owned(),
            prepared.thread.clone(),
            binding.clone(),
        );
        if let Err(error) = with_home(home, move |home| {
            home.set_thread_harness(&sid, &tid, Some(record))
                .map(|_| ())
        })
        .await
        {
            eprintln!("mathscape: failed to record harness thread: {error}");
        }
        Ok((round, binding, bound.continuity))
    }

    /// After a turn: keep a rollout copy in the session dir, snapshot the
    /// workspace for the next 「自上轮以来工作区变化」, stamp the binding.
    async fn persist_thread_state(
        &self,
        home: &Arc<Mutex<SandboxHome>>,
        session_id: &str,
        thread: &str,
        binding: &HarnessThreadBinding,
    ) {
        let executor =
            self.executor(parse_backend(&binding.backend).unwrap_or(AgentBackend::Codex));
        let (sid, harness_thread) = (session_id.to_owned(), binding.thread_id.clone());
        let rollout = tauri::async_runtime::spawn_blocking(move || {
            executor.thread_state_path(&sid, &harness_thread)
        })
        .await
        .ok()
        .flatten();
        let (sid, tid, mut binding) = (session_id.to_owned(), thread.to_owned(), binding.clone());
        let result = with_home(home, move |home| {
            let state_dir = home.ensure_harness_state_dir(&sid, &binding.backend)?;
            if let Some(rollout) = rollout {
                match keep_rollout_copy(&rollout, &state_dir) {
                    Ok(name) => binding.rollout_file = Some(name),
                    Err(error) => eprintln!(
                        "mathscape: failed to copy rollout {}: {error}",
                        rollout.display()
                    ),
                }
            }
            let investigation_id = mathscape_mcp::investigation::sandbox_investigation_id(&sid);
            match digest_workspace(&binding.workspace, &investigation_id) {
                Ok(digest) => {
                    if let Err(error) = save_digest(&state_dir, &tid, &digest) {
                        eprintln!("mathscape: failed to save workspace digest: {error}");
                    }
                }
                Err(error) => eprintln!("mathscape: workspace digest failed: {error}"),
            }
            binding.last_turn_at = Some(mathscape_sandbox::now_rfc3339());
            home.set_thread_harness(&sid, &tid, Some(binding))
                .map(|_| ())
        })
        .await;
        if let Err(error) = result {
            eprintln!("mathscape: failed to persist harness thread state: {error}");
        }
    }

    /// 「新对话」: open a fresh thread on the same isolated copy. Refused
    /// while a round streams; the round slot is held for the duration so a
    /// concurrent send cannot start on the old thread mid-switch.
    async fn new_thread(
        &self,
        home: &Arc<Mutex<SandboxHome>>,
        session_id: String,
    ) -> Result<SessionDialogue, String> {
        {
            let mut active = self.active();
            if active.contains_key(&session_id) {
                return Err("当前一轮还没结束，请等待或先取消再开新对话".into());
            }
            active.insert(session_id.clone(), None);
        }
        let sid = session_id.clone();
        let opened = with_home(home, move |home| home.start_dialogue_thread(&sid)).await;
        self.release(&session_id, None);
        opened?;
        self.dialogue(home, session_id).await
    }

    /// Switch the session's active dialogue thread back to an existing thread
    /// (`d1`..`dN`). Refused while a round is streaming so two threads never
    /// race on the same workspace copy.
    async fn activate_thread(
        &self,
        home: &Arc<Mutex<SandboxHome>>,
        session_id: String,
        thread_id: String,
    ) -> Result<SessionDialogue, String> {
        {
            let mut active = self.active();
            if active.contains_key(&session_id) {
                return Err("当前一轮还没结束，请等待或先取消再切换对话".into());
            }
            active.insert(session_id.clone(), None);
        }
        let sid = session_id.clone();
        let switched = with_home(home, move |home| {
            home.activate_dialogue_thread(&sid, &thread_id)
        })
        .await;
        self.release(&session_id, None);
        switched?;
        self.dialogue(home, session_id).await
    }

    async fn cancel(&self, session_id: &str) -> Result<bool, String> {
        match self.active_round(session_id) {
            None => Ok(false),
            Some(round) => match self.round_executor(&round) {
                Some(executor) => executor
                    .cancel(&round)
                    .await
                    .map(|()| true)
                    .map_err(|error| error.to_string()),
                None => Ok(false),
            },
        }
    }
}

#[tauri::command]
pub async fn sandbox_agent_status(
    dialogue: State<'_, SandboxDialogueState>,
) -> Result<AgentStatus, String> {
    Ok(dialogue.status().await)
}

#[tauri::command]
pub async fn sandbox_set_backend(
    dialogue: State<'_, SandboxDialogueState>,
    backend: String,
) -> Result<AgentStatus, String> {
    dialogue.set_backend(backend).await
}

/// Persist the selected executor's reasoning effort for subsequent rounds.
#[tauri::command]
pub async fn sandbox_set_effort(
    dialogue: State<'_, SandboxDialogueState>,
    effort: String,
) -> Result<AgentStatus, String> {
    dialogue.set_effort(effort).await
}

/// Persist the selected executor's model selection for subsequent rounds.
#[tauri::command]
pub async fn sandbox_set_model(
    dialogue: State<'_, SandboxDialogueState>,
    model_id: String,
) -> Result<AgentStatus, String> {
    dialogue.set_model(model_id).await
}

#[tauri::command]
pub async fn sandbox_session_dialogue(
    sandbox: State<'_, SandboxState>,
    dialogue: State<'_, SandboxDialogueState>,
    sandbox_session_id: String,
) -> Result<SessionDialogue, String> {
    dialogue.dialogue(&sandbox.home(), sandbox_session_id).await
}

/// Opens a new dialogue thread (fresh agent context, same isolated copy).
#[tauri::command]
pub async fn sandbox_new_dialogue_thread(
    sandbox: State<'_, SandboxState>,
    dialogue: State<'_, SandboxDialogueState>,
    sandbox_session_id: String,
) -> Result<SessionDialogue, String> {
    dialogue
        .new_thread(&sandbox.home(), sandbox_session_id)
        .await
}

/// Switches the active dialogue thread to an existing thread on the same copy.
#[tauri::command]
pub async fn sandbox_activate_dialogue_thread(
    sandbox: State<'_, SandboxState>,
    dialogue: State<'_, SandboxDialogueState>,
    sandbox_session_id: String,
    thread_id: String,
) -> Result<SessionDialogue, String> {
    dialogue
        .activate_thread(&sandbox.home(), sandbox_session_id, thread_id)
        .await
}

/// Starts one round; the reply arrives on [`DIALOGUE_EVENT`].
#[tauri::command]
pub async fn sandbox_send_message(
    app: AppHandle,
    sandbox: State<'_, SandboxState>,
    dialogue: State<'_, SandboxDialogueState>,
    sandbox_session_id: String,
    text: String,
) -> Result<SendOutcome, String> {
    let sink: Sink = Arc::new(move |update| {
        let _ = app.emit(DIALOGUE_EVENT, update);
    });
    dialogue
        .send(sandbox.home(), sandbox_session_id, text, sink)
        .await
}

/// Answers an open `user.ask` pending question on the session copy and starts
/// the next Agent turn with structured `<mathscape_turn_context>` delivery.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn sandbox_answer_question(
    app: AppHandle,
    sandbox: State<'_, SandboxState>,
    dialogue: State<'_, SandboxDialogueState>,
    sandbox_session_id: String,
    question_id: String,
    text: Option<String>,
    selected_option: Option<String>,
    ink_snapshot_ref: Option<String>,
) -> Result<SendOutcome, String> {
    let sink: Sink = Arc::new(move |update| {
        let _ = app.emit(DIALOGUE_EVENT, update);
    });
    dialogue
        .answer_question(
            sandbox.home(),
            sandbox_session_id,
            question_id,
            text,
            selected_option,
            ink_snapshot_ref,
            sink,
        )
        .await
}

/// Cancels (`state = 'cancelled'`) an open `user.ask` pending question without
/// starting a new turn.
#[tauri::command]
pub async fn sandbox_dismiss_question(
    sandbox: State<'_, SandboxState>,
    dialogue: State<'_, SandboxDialogueState>,
    sandbox_session_id: String,
    question_id: String,
) -> Result<SessionDialogue, String> {
    dialogue
        .dismiss_question(&sandbox.home(), sandbox_session_id, question_id)
        .await
}

/// Returns `false` when no round was running.
#[tauri::command]
pub async fn sandbox_cancel_round(
    dialogue: State<'_, SandboxDialogueState>,
    sandbox_session_id: String,
) -> Result<bool, String> {
    dialogue.cancel(&sandbox_session_id).await
}

#[cfg(test)]
mod tests {
    use std::{
        path::Path,
        process::Command,
        sync::{OnceLock, mpsc},
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use mathscape_sandbox::{EVENTS_FILE, fingerprint_workspace};

    use super::*;
    use crate::DesktopService;

    /// `mathscape-codex-fixture` (dry-run Codex app-server: no login, no
    /// model; echoes the user text as the agent message).
    fn fixture_program() -> PathBuf {
        static FIXTURE: OnceLock<PathBuf> = OnceLock::new();
        FIXTURE
            .get_or_init(|| {
                // CARGO_BIN_EXE is only available to integration tests in the binary's package.
                let output = Command::new(env!("CARGO"))
                    .current_dir(env!("CARGO_MANIFEST_DIR"))
                    .args([
                        "build",
                        "-p",
                        "mathscape-agent",
                        "--bin",
                        "mathscape-codex-fixture",
                        "--message-format=json-render-diagnostics",
                    ])
                    .output()
                    .expect("build codex fixture");
                assert!(
                    output.status.success(),
                    "building mathscape-codex-fixture failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                    .find_map(|message| {
                        if message["reason"] == "compiler-artifact"
                            && message["target"]["name"] == "mathscape-codex-fixture"
                        {
                            message["executable"].as_str().map(PathBuf::from)
                        } else {
                            None
                        }
                    })
                    .expect("Cargo must report the codex fixture executable")
            })
            .clone()
    }

    struct Fixture {
        root: PathBuf,
        checkpoint: PathBuf,
        home: Arc<Mutex<SandboxHome>>,
        session_id: String,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn fixture(tag: &str) -> Fixture {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            env::temp_dir().join(format!("mathscape-s1-{tag}-{}-{nanos}", std::process::id()));
        let checkpoint = root.join("mother");
        DesktopService::open_with_credential_store(
            &checkpoint,
            Arc::new(mathscape_llm::MemoryCredentialResolver::default()),
        )
        .unwrap();
        let home = SandboxHome::open(root.join("sandbox")).unwrap();
        let record = home.register_checkpoint(&checkpoint, Some("s1")).unwrap();
        let session = home.start_session(&record.checkpoint_id).unwrap();
        Fixture {
            root,
            checkpoint,
            home: Arc::new(Mutex::new(home)),
            session_id: session.sandbox_session_id,
        }
    }

    /// Dialogue state on the fixture with its own fake Codex home under the
    /// fixture root (`codex_home` name lets one test simulate two machines)
    /// and a request log at `<root>/<codex_home>.log`.
    fn dialogue_in(fx: &Fixture, codex_home: &str, args: &[&str]) -> SandboxDialogueState {
        dialogue_with_sink(fx, codex_home, args, mathscape_resident::null_sink())
    }

    fn dialogue_with_sink(
        fx: &Fixture,
        codex_home: &str,
        args: &[&str],
        sink: mathscape_resident::EventSink,
    ) -> SandboxDialogueState {
        let home = fx.root.join(codex_home);
        std::fs::create_dir_all(&home).unwrap();
        let mut all: Vec<String> = vec![
            "--log".into(),
            fx.root
                .join(format!("{codex_home}.log"))
                .display()
                .to_string(),
        ];
        all.extend(args.iter().map(|arg| (*arg).to_owned()));
        SandboxDialogueState::with_config_and_sink(
            CodexConfig {
                program: fixture_program(),
                args: all,
                mcp_program: PathBuf::from("mathscape"),
                home: Some(home),
                ..CodexConfig::default()
            },
            sink,
        )
    }

    fn dialogue(fx: &Fixture, args: &[&str]) -> SandboxDialogueState {
        dialogue_in(fx, "codex-home", args)
    }

    fn acp_program() -> PathBuf {
        static FIXTURE: OnceLock<PathBuf> = OnceLock::new();
        FIXTURE
            .get_or_init(|| {
                let output = Command::new(env!("CARGO"))
                    .current_dir(env!("CARGO_MANIFEST_DIR"))
                    .args([
                        "build",
                        "-p",
                        "mathscape-agent",
                        "--bin",
                        "mathscape-acp-fixture",
                        "--message-format=json-render-diagnostics",
                    ])
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                    .find_map(|message| {
                        (message["target"]["name"] == "mathscape-acp-fixture")
                            .then(|| message["executable"].as_str().map(PathBuf::from))
                            .flatten()
                    })
                    .unwrap()
            })
            .clone()
    }

    fn dual_dialogue(
        fx: &Fixture,
        mode: &str,
        backend_override: Option<AgentBackend>,
    ) -> SandboxDialogueState {
        let home = fx.root.join("codex-home");
        std::fs::create_dir_all(&home).unwrap();
        SandboxDialogueState::with_configs(
            CodexConfig {
                program: fixture_program(),
                home: Some(home),
                args: vec![
                    "--log".into(),
                    fx.root.join("codex-home.log").display().to_string(),
                ],
                ..CodexConfig::default()
            },
            OpenCodeConfig {
                program: acp_program(),
                args: vec![
                    "--mode".into(),
                    mode.into(),
                    "--log".into(),
                    fx.root.join("acp.log").display().to_string(),
                ],
                state_dir: fx.root.join("opencode"),
                ..OpenCodeConfig::default()
            },
            fx.root.join("agent-backend"),
            backend_override,
            mathscape_resident::null_sink(),
        )
    }

    #[test]
    fn opencode_status_selection_and_round_are_backend_aware() {
        let fx = fixture("opencode");
        let state = dual_dialogue(&fx, "normal", None);
        let status = tauri::async_runtime::block_on(state.status());
        assert_eq!(status.backend, "codex");
        assert_eq!(
            status
                .backends
                .iter()
                .map(|b| b.backend)
                .collect::<Vec<_>>(),
            ["codex", "opencode"]
        );
        assert!(status.backends.iter().all(|b| b.available));
        assert!(tauri::async_runtime::block_on(state.set_backend("unknown".into())).is_err());
        let status = tauri::async_runtime::block_on(state.set_backend("opencode".into())).unwrap();
        assert_eq!(status.program, acp_program());
        assert!(!status.effort_selection);
        assert_eq!(status.models.len(), 2);
        tauri::async_runtime::block_on(state.set_model("fixture/b".into())).unwrap();
        assert_eq!(
            tauri::async_runtime::block_on(state.status())
                .model
                .as_deref(),
            Some("fixture/b")
        );
        let (_, seen) = send_and_wait(&state, &fx, "hello OpenCode");
        assert!(
            seen.iter()
                .filter_map(|update| match update {
                    DialogueUpdate::AgentEvent { event, .. } => Some(event),
                    _ => None,
                })
                .all(|event| event["backend"] == "opencode")
        );
        let home = fx.home.lock().unwrap();
        assert!(
            home.read_transcript(&fx.session_id)
                .unwrap()
                .iter()
                .any(|line| line.text == "fixture reply")
        );
        assert_eq!(home.session(&fx.session_id).unwrap().backend, "opencode");
        assert!(
            home.ensure_harness_state_dir(&fx.session_id, "opencode")
                .unwrap()
                .is_dir()
        );
        drop(home);
        assert_eq!(
            std::fs::read_to_string(fx.root.join("agent-backend")).unwrap(),
            "opencode"
        );
        let restored = dual_dialogue(&fx, "normal", None);
        assert_eq!(restored.backend(), AgentBackend::OpenCode);
        state.close_all();
        restored.close_all();
    }

    #[test]
    fn switching_harness_rebuilds_transcript_and_preserves_backend_bindings() {
        let fx = fixture("switch-backend");
        let state = dual_dialogue(&fx, "normal", None);
        send_and_wait(&state, &fx, "codex context");
        tauri::async_runtime::block_on(state.set_backend("opencode".into())).unwrap();
        send_and_wait(&state, &fx, "opencode question");
        {
            let home = fx.home.lock().unwrap();
            let thread = &home.dialogue_threads(&fx.session_id).unwrap().threads[0];
            assert_eq!(thread.harness.as_ref().unwrap().backend, "opencode");
            assert_eq!(thread.harnesses.len(), 2);
            assert!(
                home.read_transcript(&fx.session_id)
                    .unwrap()
                    .iter()
                    .any(|line| line.text.contains("上下文已降级"))
            );
        }
        let calls = logged(&fx, "acp", "session/prompt");
        assert!(
            calls[0]["params"]["prompt"]
                .to_string()
                .contains("codex context")
        );
        tauri::async_runtime::block_on(state.set_backend("codex".into())).unwrap();
        send_and_wait(&state, &fx, "back to codex");
        let calls = logged(&fx, "codex-home", "turn/start");
        assert!(calls.last().unwrap().to_string().contains("fixture reply"));
        let home = fx.home.lock().unwrap();
        let thread = &home.dialogue_threads(&fx.session_id).unwrap().threads[0];
        assert_eq!(thread.harness.as_ref().unwrap().backend, "codex");
        assert_eq!(thread.harnesses.len(), 2);
        drop(home);
        state.close_all();
    }

    #[test]
    fn cancel_after_backend_switch_reaches_original_executor() {
        let fx = fixture("switch-cancel");
        let state = dual_dialogue(&fx, "hold", None);
        tauri::async_runtime::block_on(state.set_backend("opencode".into())).unwrap();
        let (sink, updates) = channel_sink();
        tauri::async_runtime::block_on(state.send(
            Arc::clone(&fx.home),
            fx.session_id.clone(),
            "wait".into(),
            sink,
        ))
        .unwrap();
        tauri::async_runtime::block_on(state.set_backend("codex".into())).unwrap();
        assert!(tauri::async_runtime::block_on(state.cancel(&fx.session_id)).unwrap());
        let seen = wait_for(&updates, finished);
        assert!(seen.iter().any(|update| matches!(update, DialogueUpdate::AgentEvent { event, .. } if event["type"] == "turn_end" && event["payload"]["outcome"] == "cancelled")));
        assert_eq!(logged(&fx, "acp", "session/cancel").len(), 1);
        tauri::async_runtime::block_on(state.close_harness_session(&fx.session_id));
        assert!(state.opened().is_empty());
        state.close_all();
    }

    #[test]
    fn unavailable_backend_and_environment_override_cannot_be_selected_around() {
        let fx = fixture("backend-override");
        let state = dual_dialogue(&fx, "normal", Some(AgentBackend::OpenCode));
        let status = tauri::async_runtime::block_on(state.status());
        assert_eq!(status.backend, "opencode");
        assert!(!status.backend_selection);
        assert!(tauri::async_runtime::block_on(state.set_backend("codex".into())).is_err());
        let unavailable = SandboxDialogueState::with_configs(
            CodexConfig {
                program: fixture_program(),
                ..CodexConfig::default()
            },
            OpenCodeConfig {
                program: fx.root.join("not-installed"),
                ..OpenCodeConfig::default()
            },
            fx.root.join("missing-choice"),
            None,
            mathscape_resident::null_sink(),
        );
        let status = tauri::async_runtime::block_on(unavailable.status());
        assert!(!status.backends[1].available);
        assert!(
            tauri::async_runtime::block_on(unavailable.set_backend("opencode".into())).is_err()
        );
        assert!(!fx.root.join("missing-choice").exists());
        state.close_all();
        unavailable.close_all();
    }

    /// Inbound requests the fixture logged with `method`.
    fn logged(fx: &Fixture, codex_home: &str, method: &str) -> Vec<Value> {
        std::fs::read_to_string(fx.root.join(format!("{codex_home}.log")))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|line| line["method"] == method)
            .collect()
    }

    fn send_and_wait(
        state: &SandboxDialogueState,
        fx: &Fixture,
        text: &str,
    ) -> (SendOutcome, Vec<DialogueUpdate>) {
        let (sink, updates) = channel_sink();
        let sent = tauri::async_runtime::block_on(state.send(
            Arc::clone(&fx.home),
            fx.session_id.clone(),
            text.into(),
            sink,
        ))
        .unwrap();
        let seen = wait_for(&updates, finished);
        (sent, seen)
    }

    fn channel_sink() -> (Sink, mpsc::Receiver<DialogueUpdate>) {
        let (sender, receiver) = mpsc::channel();
        let sender = Mutex::new(sender);
        let sink: Sink = Arc::new(move |update| {
            let _ = sender.lock().unwrap().send(update);
        });
        (sink, receiver)
    }

    fn wait_for(
        receiver: &mpsc::Receiver<DialogueUpdate>,
        mut done: impl FnMut(&DialogueUpdate) -> bool,
    ) -> Vec<DialogueUpdate> {
        let mut seen = Vec::new();
        loop {
            let update = receiver
                .recv_timeout(Duration::from_secs(10))
                .unwrap_or_else(|_| panic!("timed out; saw {seen:?}"));
            let stop = done(&update);
            seen.push(update);
            if stop {
                return seen;
            }
        }
    }

    fn finished(update: &DialogueUpdate) -> bool {
        matches!(update, DialogueUpdate::RoundFinished { .. })
    }

    fn event_types(events: &[Value]) -> Vec<String> {
        events
            .iter()
            .map(|event| event["type"].as_str().unwrap_or_default().to_owned())
            .collect()
    }

    #[test]
    fn round_harness_grants_lean_root_skill_and_scratch() {
        let dir = tempfile::tempdir().unwrap();
        let checkpoint = dir.path().join("mother");
        let profiles = checkpoint.join(mathscape_tool_lean::WORKSPACE_PROFILE_DIR);
        std::fs::create_dir_all(&profiles).unwrap();
        let scratch = dir.path().join("scratch");
        let tools = vec!["lean.check".to_owned()];
        let (harness, record) = round_harness(Some(&checkpoint), scratch.clone(), &tools);
        if std::env::var_os(LEAN_PROFILE_ROOT_ENV).is_none() {
            let root = profiles.canonicalize().unwrap();
            assert_eq!(harness.read_roots, vec![root.clone()]);
            assert!(
                harness
                    .env
                    .iter()
                    .any(|(k, v)| k == LEAN_PROFILE_ROOT_ENV && Path::new(v) == root)
            );
            assert_eq!(record["lean_profile_root"]["source"], "workspace");
        }
        assert_eq!(harness.scratch_dir.as_deref(), Some(scratch.as_path()));
        assert!(harness.uses_permission_profile());
        assert_eq!(harness.skill_roots.len(), 1, "repo skill pack should load");
        assert!(
            harness
                .instructions
                .as_deref()
                .is_some_and(|text| text.contains("mathscape-worknet"))
        );
        assert_eq!(record["schema"], "mathscape.sandbox_harness.v0");
        assert_eq!(record["skill"]["name"], "mathscape-worknet");
        assert_eq!(record["network"], false);
        assert_eq!(record["tools"], json!(["lean.check"]));
    }

    #[test]
    fn round_harness_without_profiles_grants_no_lean_root() {
        let dir = tempfile::tempdir().unwrap();
        if std::env::var_os(LEAN_PROFILE_ROOT_ENV).is_some() {
            return;
        }
        let (harness, record) = round_harness(Some(dir.path()), dir.path().join("s"), &[]);
        // A platform data root may exist on a dev machine; it must be a dir.
        for root in &harness.read_roots {
            assert!(root.is_dir());
        }
        if harness.read_roots.is_empty() {
            assert!(record["lean_profile_root"].is_null());
        }
    }

    #[test]
    fn round_streams_events_and_records_reply_inside_session() {
        let fx = fixture("round");
        let before = fingerprint_workspace(&fx.checkpoint).unwrap();
        let state = dialogue(&fx, &["app-server"]);
        let (sink, updates) = channel_sink();

        let sent = tauri::async_runtime::block_on(state.send(
            Arc::clone(&fx.home),
            fx.session_id.clone(),
            "  解释紧集  ".into(),
            Arc::clone(&sink),
        ))
        .unwrap();
        assert_eq!(sent.user_entry.text, "解释紧集");
        let seen = wait_for(&updates, finished);
        assert!(seen.iter().any(|u| matches!(
            u,
            DialogueUpdate::AgentEvent { event, .. } if event["type"] == "tool_call"
        )));

        let view = tauri::async_runtime::block_on(state.dialogue(&fx.home, fx.session_id.clone()))
            .unwrap();
        assert_eq!(view.active_round_id, None);
        assert_eq!(
            event_types(&view.events),
            [
                "turn_start",
                "tool_call",
                "tool_result",
                "tool_call",
                "tool_result",
                "tool_call",
                "tool_result",
                "usage",
                "turn_end"
            ]
        );
        assert_eq!(view.transcript.len(), 2);
        assert_eq!(view.transcript[0].role, TranscriptRole::User);
        let reply = &view.transcript[1];
        assert_eq!(reply.role, TranscriptRole::Agent);
        assert_eq!(reply.outcome.as_deref(), Some("completed"));
        assert_eq!(reply.round_id.as_deref(), Some(sent.round_id.as_str()));
        assert!(reply.text.ends_with("解释紧集"), "{}", reply.text);

        // Second round continues the same Codex thread: no transcript
        // replay, just the user text plus a turn-context item.
        tauri::async_runtime::block_on(state.send(
            Arc::clone(&fx.home),
            fx.session_id.clone(),
            "再举个例子".into(),
            sink,
        ))
        .unwrap();
        wait_for(&updates, finished);
        assert_eq!(logged(&fx, "codex-home", "initialize").len(), 1);
        assert_eq!(logged(&fx, "codex-home", "thread/start").len(), 1);
        let turns = logged(&fx, "codex-home", "turn/start");
        assert_eq!(turns.len(), 2);
        let first = turns[0]["params"]["input"].as_array().unwrap();
        let first_context = first[0]["text"].as_str().unwrap();
        assert!(first_context.contains("<mathscape_turn_context>"));
        assert!(first_context.contains("沙箱探索会话"));
        assert!(first_context.contains("SKILL.md"), "{first_context}");
        assert_eq!(first[1]["text"], "解释紧集");
        let second = turns[1]["params"]["input"].as_array().unwrap();
        let second_context = second[0]["text"].as_str().unwrap();
        assert!(
            second_context.contains("工作区没有变化"),
            "{second_context}"
        );
        assert!(
            !second_context.contains("SKILL.md"),
            "skill read only on turn 1"
        );
        assert_eq!(second[1]["text"], "再举个例子");
        let home = fx.home.lock().unwrap();
        let transcript = home.read_transcript(&fx.session_id).unwrap();
        assert_eq!(transcript.len(), 4);
        assert_eq!(transcript[3].text, "再举个例子");
        let binding = home.dialogue_threads(&fx.session_id).unwrap().threads[0]
            .harness
            .clone()
            .expect("harness binding");
        assert_eq!(binding.backend, "codex");
        assert_eq!(binding.thread_id, "thr_fixture");
        assert!(binding.last_turn_at.is_some());
        let copy = home
            .session_dir(&fx.session_id)
            .join("harness/codex")
            .join(binding.rollout_file.as_deref().expect("rollout copy"));
        assert!(copy.is_file(), "{}", copy.display());
        assert_eq!(
            std::fs::read_to_string(home.current_round_path(&fx.session_id))
                .unwrap()
                .trim(),
            transcript[3].round_id.as_deref().unwrap()
        );
        assert!(home.events_path(&fx.session_id).is_file());
        let record = home.session(&fx.session_id).unwrap();
        assert_eq!(record.backend, "codex");
        assert_eq!(record.tool_allowlist, sandbox_tool_allowlist());
        assert_eq!(record.tool_allowlist.len(), 15);
        let export = home.export_session(&fx.session_id, None, None).unwrap();
        assert!(export.export_dir.join(EVENTS_FILE).is_file());
        drop(home);
        // Nothing reached the checkpoint (= mother workspace here).
        assert!(
            fingerprint_workspace(&fx.checkpoint)
                .unwrap()
                .same_content(&before)
        );
        assert!(!fx.checkpoint.join(EVENTS_FILE).exists());
    }

    #[test]
    fn new_thread_starts_empty_context_on_same_copy() {
        let fx = fixture("thread");
        let state = dialogue(&fx, &["app-server"]);
        let (sink, updates) = channel_sink();
        tauri::async_runtime::block_on(state.send(
            Arc::clone(&fx.home),
            fx.session_id.clone(),
            "调试旧功能".into(),
            Arc::clone(&sink),
        ))
        .unwrap();
        wait_for(&updates, finished);

        let view =
            tauri::async_runtime::block_on(state.new_thread(&fx.home, fx.session_id.clone()))
                .unwrap();
        assert_eq!(view.active_thread_id, "d2");
        assert_eq!(view.threads.len(), 2);
        // Full history is still returned for read-only browsing.
        assert_eq!(view.transcript.len(), 2);
        let workspace = fx.home.lock().unwrap().session_workspace(&fx.session_id);

        tauri::async_runtime::block_on(state.send(
            Arc::clone(&fx.home),
            fx.session_id.clone(),
            "新问题".into(),
            sink,
        ))
        .unwrap();
        wait_for(&updates, finished);
        let home = fx.home.lock().unwrap();
        let d2 = home.read_thread_transcript(&fx.session_id, "d2").unwrap();
        assert_eq!(d2.len(), 2, "{d2:?}");
        // A new Codex thread: old thread content is not replayed, and the
        // agent is told the copy may hold earlier state.
        assert_eq!(d2[1].text, "新问题");
        let starts = logged(&fx, "codex-home", "thread/start");
        assert_eq!(starts.len(), 2);
        let turns = logged(&fx, "codex-home", "turn/start");
        let last = turns.last().unwrap();
        assert_eq!(last["params"]["threadId"], "thr_fixture-2");
        let context = last["params"]["input"][0]["text"].as_str().unwrap();
        assert!(context.contains("新开的一条对话"), "{context}");
        assert!(!last.to_string().contains("调试旧功能"));
        let threads = home.dialogue_threads(&fx.session_id).unwrap().threads;
        assert_eq!(
            threads[0].harness.as_ref().unwrap().thread_id,
            "thr_fixture"
        );
        assert_eq!(
            threads[1].harness.as_ref().unwrap().thread_id,
            "thr_fixture-2"
        );
        assert_eq!(home.session_workspace(&fx.session_id), workspace);
        assert_eq!(
            home.read_thread_transcript(&fx.session_id, "d1")
                .unwrap()
                .len(),
            2
        );
        drop(home);

        // Mutate workspace during d2 (create an ink note), then switch back to d1:
        // d1 resumes its own Codex thread (`thr_fixture`) and sees the workspace diff!
        {
            let mut document: mathscape_domain::InkLabDocument =
                serde_json::from_str(include_str!("../../../testdata/ink/p0/trace.json")).unwrap();
            document.operations.clear();
            DesktopService::open_with_credential_store(
                &workspace,
                Arc::new(mathscape_llm::MemoryCredentialResolver::default()),
            )
            .unwrap()
            .create_ink_document(crate::ink::CreateInkDocumentRequest {
                document,
                session_id: "sess-d2".into(),
                writer_id: "tablet".into(),
                device: json!({}),
                request_id: None,
                title_mode: None,
            })
            .unwrap();
        }
        let switched = tauri::async_runtime::block_on(state.activate_thread(
            &fx.home,
            fx.session_id.clone(),
            "d1".into(),
        ))
        .unwrap();
        assert_eq!(switched.active_thread_id, "d1");

        let (sink2, updates2) = channel_sink();
        tauri::async_runtime::block_on(state.send(
            Arc::clone(&fx.home),
            fx.session_id.clone(),
            "回到对话1继续追问".into(),
            sink2,
        ))
        .unwrap();
        wait_for(&updates2, finished);

        let home = fx.home.lock().unwrap();
        let d1 = home.read_thread_transcript(&fx.session_id, "d1").unwrap();
        assert_eq!(d1.len(), 4, "{d1:?}");
        assert_eq!(d1[2].text, "回到对话1继续追问");
        assert_eq!(
            home.read_thread_transcript(&fx.session_id, "d2")
                .unwrap()
                .len(),
            2
        );
        let turns = logged(&fx, "codex-home", "turn/start");
        let resumed_d1_turn = turns.last().unwrap();
        assert_eq!(resumed_d1_turn["params"]["threadId"], "thr_fixture");
        let d1_ctx = resumed_d1_turn["params"]["input"][0]["text"]
            .as_str()
            .unwrap();
        assert!(d1_ctx.contains("自上轮以来工作区变化"), "{d1_ctx}");
        assert!(d1_ctx.contains("手稿数：0 → 1"), "{d1_ctx}");
    }

    #[test]
    fn cancel_ends_round_with_system_line_and_blocks_overlap() {
        let fx = fixture("cancel");
        let state = dialogue(&fx, &["--hold", "app-server"]);
        let (sink, updates) = channel_sink();
        tauri::async_runtime::block_on(state.send(
            Arc::clone(&fx.home),
            fx.session_id.clone(),
            "慢慢想".into(),
            Arc::clone(&sink),
        ))
        .unwrap();
        wait_for(
            &updates,
            |u| matches!(u, DialogueUpdate::AgentEvent { event, .. } if event["type"] == "tool_call"),
        );
        let overlap = tauri::async_runtime::block_on(state.send(
            Arc::clone(&fx.home),
            fx.session_id.clone(),
            "插队".into(),
            sink,
        ));
        assert!(overlap.unwrap_err().contains("上一轮还没结束"));
        // No thread switch while a round streams.
        let switch =
            tauri::async_runtime::block_on(state.new_thread(&fx.home, fx.session_id.clone()));
        assert!(switch.unwrap_err().contains("当前一轮还没结束"));
        let reactivate = tauri::async_runtime::block_on(state.activate_thread(
            &fx.home,
            fx.session_id.clone(),
            "d1".into(),
        ));
        assert!(reactivate.unwrap_err().contains("当前一轮还没结束"));
        assert!(tauri::async_runtime::block_on(state.cancel(&fx.session_id)).unwrap());
        wait_for(&updates, finished);
        assert!(!tauri::async_runtime::block_on(state.cancel(&fx.session_id)).unwrap());

        let home = fx.home.lock().unwrap();
        let transcript = home.read_transcript(&fx.session_id).unwrap();
        assert_eq!(transcript.len(), 2, "{transcript:?}");
        assert_eq!(transcript[1].role, TranscriptRole::System);
        assert_eq!(transcript[1].outcome.as_deref(), Some("cancelled"));
        let types = event_types(&home.read_events(&fx.session_id).unwrap());
        assert!(types.contains(&"cancel".to_owned()));
        assert_eq!(types.last().map(String::as_str), Some("turn_end"));
    }

    #[test]
    fn harness_failure_is_a_system_line_not_a_fake_reply() {
        let fx = fixture("auth");
        let state = dialogue(&fx, &["--reject-auth", "app-server"]);
        let (sink, updates) = channel_sink();
        let sent = tauri::async_runtime::block_on(state.send(
            Arc::clone(&fx.home),
            fx.session_id.clone(),
            "你好".into(),
            sink,
        ));
        if sent.is_ok() {
            wait_for(&updates, finished);
        }
        let home = fx.home.lock().unwrap();
        let transcript = home.read_transcript(&fx.session_id).unwrap();
        assert_eq!(transcript.len(), 2, "{transcript:?}");
        assert_eq!(transcript[1].role, TranscriptRole::System);
        assert_eq!(transcript[1].outcome.as_deref(), Some("failed"));
        assert!(!transcript.iter().any(|e| e.role == TranscriptRole::Agent));
        drop(home);
        assert!(state.active_round(&fx.session_id).is_none());
        assert!(state.active().is_empty());
    }

    #[test]
    fn discard_mid_round_leaves_no_session_bytes() {
        let fx = fixture("discard");
        let state = dialogue(&fx, &["--hold", "app-server"]);
        let (sink, updates) = channel_sink();
        tauri::async_runtime::block_on(state.send(
            Arc::clone(&fx.home),
            fx.session_id.clone(),
            "长任务".into(),
            sink,
        ))
        .unwrap();
        wait_for(
            &updates,
            |u| matches!(u, DialogueUpdate::AgentEvent { event, .. } if event["type"] == "tool_call"),
        );
        tauri::async_runtime::block_on(state.cancel_session_round(&fx.session_id));
        tauri::async_runtime::block_on(state.close_harness_session(&fx.session_id));
        let session_dir = {
            let home = fx.home.lock().unwrap();
            home.discard_session(&fx.session_id).unwrap();
            home.session_dir(&fx.session_id)
        };
        wait_for(&updates, finished);
        assert!(!session_dir.exists());
        let refused =
            tauri::async_runtime::block_on(state.dialogue(&fx.home, fx.session_id.clone()));
        assert!(refused.is_err());
    }

    #[test]
    fn rejects_empty_and_oversized_messages() {
        let fx = fixture("limits");
        let state = dialogue(&fx, &["app-server"]);
        let (sink, _updates) = channel_sink();
        for text in ["   ".to_owned(), "长".repeat(MAX_MESSAGE_CHARS + 1)] {
            let result = tauri::async_runtime::block_on(state.send(
                Arc::clone(&fx.home),
                fx.session_id.clone(),
                text,
                Arc::clone(&sink),
            ));
            assert!(result.is_err());
        }
        assert!(state.active().is_empty());
        let home = fx.home.lock().unwrap();
        assert!(home.read_transcript(&fx.session_id).unwrap().is_empty());
    }

    #[test]
    fn app_server_lifecycle_goes_to_the_resident_process_log() {
        let fx = fixture("resident-log");
        let log = crate::resident_log::log_file(Some(&fx.root.join("logs"))).unwrap();
        let state = dialogue_with_sink(
            &fx,
            "codex-home",
            &["app-server"],
            crate::resident_log::jsonl_sink(Some(log.clone())),
        );
        send_and_wait(&state, &fx, "你好");
        tauri::async_runtime::block_on(state.close_harness_session(&fx.session_id));
        let lines: Vec<Value> = std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let instance = format!("codex-app-server:{}", fx.session_id);
        let kinds: Vec<&str> = lines
            .iter()
            .inspect(|line| {
                assert_eq!(line["schema"], "mathscape.resident_event.v0");
                assert_eq!(line["event"]["instance"], instance.as_str());
            })
            .map(|line| line["event"]["kind"].as_str().unwrap())
            .collect();
        for kind in ["spawn", "ready", "shutdown", "exit"] {
            assert!(kinds.contains(&kind), "{kind} missing in {kinds:?}");
        }
        let spawn = lines
            .iter()
            .find(|l| l["event"]["kind"] == "spawn")
            .unwrap();
        assert!(spawn["event"]["pid"].as_u64().is_some());
        // The key names the session (Debug-formatted, like Loogle's).
        assert!(
            spawn["event"]["key"]
                .as_str()
                .unwrap()
                .contains(&format!("{}#", fx.session_id))
        );
        state.close_all();
    }

    #[test]
    fn skill_change_restarts_the_app_server_and_tells_the_agent() {
        let fx = fixture("skill-change");
        let state = dialogue(&fx, &["app-server"]);
        send_and_wait(&state, &fx, "记住 X");
        if load_worknet_skill(&locate_skills_root().unwrap()).is_err() {
            state.close_all();
            return;
        }
        // Pretend the last turn ran with an older skill.
        {
            let home = fx.home.lock().unwrap();
            let threads = home.dialogue_threads(&fx.session_id).unwrap();
            let active = threads
                .threads
                .iter()
                .find(|t| t.thread_id == threads.active_thread_id)
                .unwrap();
            let mut binding = active.harness.clone().expect("bound after a turn");
            binding.skill_sha256 = Some("old".into());
            binding.skill_version = Some("0.0.1".into());
            home.set_thread_harness(&fx.session_id, &active.thread_id, Some(binding))
                .unwrap();
        }
        send_and_wait(&state, &fx, "X 是什么");
        // A fresh app-server (new handshake) resumed the same thread.
        assert_eq!(logged(&fx, "codex-home", "initialize").len(), 2);
        let resume = logged(&fx, "codex-home", "thread/resume");
        assert_eq!(resume.len(), 1);
        assert_eq!(resume[0]["params"]["threadId"], "thr_fixture");
        let turns = logged(&fx, "codex-home", "turn/start");
        let input = turns.last().unwrap()["params"]["input"].to_string();
        assert!(input.contains("工具/skill 已更新到"), "{input}");
        assert!(input.contains("0.0.1"), "{input}");
        state.close_all();
    }

    #[test]
    fn closed_session_resumes_the_thread_and_restores_on_another_machine() {
        let fx = fixture("resume");
        let state = dialogue(&fx, &["app-server"]);
        send_and_wait(&state, &fx, "记住 X");
        // Discard-free close (session switch / app exit).
        tauri::async_runtime::block_on(state.close_harness_session(&fx.session_id));
        let (_, seen) = send_and_wait(&state, &fx, "X 是什么");
        assert!(!seen.iter().any(|u| matches!(
            u,
            DialogueUpdate::Transcript { entry, .. } if entry.text.contains("上下文已降级")
        )));
        let resume = logged(&fx, "codex-home", "thread/resume");
        assert_eq!(resume.len(), 1);
        assert_eq!(resume[0]["params"]["threadId"], "thr_fixture");
        assert_eq!(logged(&fx, "codex-home", "thread/start").len(), 1);
        state.close_all();

        // Another machine: empty Codex home; the session dir's rollout copy
        // is restored and the same thread continues.
        let other = dialogue_in(&fx, "other-home", &["app-server"]);
        send_and_wait(&other, &fx, "继续");
        assert_eq!(logged(&fx, "other-home", "thread/start").len(), 0);
        // First by id (not in this home), then after restoring the copy.
        assert_eq!(logged(&fx, "other-home", "thread/resume").len(), 2);
        let restored = fx.root.join("other-home/sessions/mathscape");
        assert!(
            std::fs::read_dir(&restored).unwrap().next().is_some(),
            "rollout restored under {}",
            restored.display()
        );
        let home = fx.home.lock().unwrap();
        let transcript = home.read_transcript(&fx.session_id).unwrap();
        assert!(!transcript.iter().any(|e| e.text.contains("上下文已降级")));
        other.close_all();
    }

    #[test]
    fn lost_harness_thread_degrades_to_a_rebuilt_prompt_with_a_notice() {
        let fx = fixture("degraded");
        let state = dialogue(&fx, &["app-server"]);
        send_and_wait(&state, &fx, "第一问");
        {
            // The Codex thread is gone and no copy exists.
            let home = fx.home.lock().unwrap();
            let mut binding = home.dialogue_threads(&fx.session_id).unwrap().threads[0]
                .harness
                .clone()
                .unwrap();
            binding.thread_id = "thr_missing".into();
            binding.rollout_file = None;
            home.set_thread_harness(&fx.session_id, "d1", Some(binding))
                .unwrap();
        }
        tauri::async_runtime::block_on(state.close_harness_session(&fx.session_id));
        send_and_wait(&state, &fx, "第二问");
        let home = fx.home.lock().unwrap();
        let transcript = home.read_transcript(&fx.session_id).unwrap();
        let notice = transcript
            .iter()
            .find(|entry| entry.text.starts_with("上下文已降级"))
            .expect("degraded notice");
        assert_eq!(notice.role, TranscriptRole::System);
        assert_eq!(notice.outcome.as_deref(), Some("degraded"));
        // The fixture echoes the rebuilt prompt: it carries the history.
        let reply = transcript.last().unwrap();
        assert_eq!(reply.role, TranscriptRole::Agent);
        assert!(reply.text.contains("[用户] 第一问"), "{}", reply.text);
        assert!(reply.text.ends_with("第二问"));
        let binding = home.dialogue_threads(&fx.session_id).unwrap().threads[0]
            .harness
            .clone()
            .unwrap();
        assert_ne!(binding.thread_id, "thr_missing");
        drop(home);
        state.close_all();
    }

    #[test]
    fn legacy_dialogue_without_binding_is_rebuilt_once() {
        let fx = fixture("legacy");
        {
            let home = fx.home.lock().unwrap();
            home.append_thread_transcript(
                &fx.session_id,
                "d1",
                TranscriptRole::User,
                "旧问题",
                None,
                None,
            )
            .unwrap();
            home.append_thread_transcript(
                &fx.session_id,
                "d1",
                TranscriptRole::Agent,
                "旧回答",
                Some("old-round"),
                Some("completed"),
            )
            .unwrap();
        }
        let state = dialogue(&fx, &["app-server"]);
        send_and_wait(&state, &fx, "新问题");
        send_and_wait(&state, &fx, "再问");
        let home = fx.home.lock().unwrap();
        let transcript = home.read_transcript(&fx.session_id).unwrap();
        let notices = transcript
            .iter()
            .filter(|entry| entry.text.starts_with("上下文已降级"))
            .count();
        assert_eq!(notices, 1, "{transcript:?}");
        let last = transcript.last().unwrap();
        assert_eq!(last.text, "再问", "second turn continues the new thread");
        drop(home);
        state.close_all();
    }

    #[test]
    fn user_ask_answer_dismiss_and_auto_settle_deliver_turn_context() {
        use mathscape_store_sqlite::{CreatePendingQuestionInput, SqliteArtifactStore};

        let fx = fixture("user-ask");
        let state = dialogue(&fx, &["app-server"]);
        let (first_sent, _) = send_and_wait(&state, &fx, "帮我整理习题 8");

        let workspace = fx.home.lock().unwrap().session_workspace(&fx.session_id);
        let inv = mathscape_mcp::investigation::sandbox_investigation_id(&fx.session_id);
        {
            let mut store =
                SqliteArtifactStore::prepare(&workspace.join("mathscape.sqlite3")).unwrap();
            store
                .create_pending_question(&CreatePendingQuestionInput {
                    question_id: "mathscape:question/q-d1".into(),
                    investigation_id: inv.clone(),
                    round_id: first_sent.round_id.clone(),
                    origin: "mcp_user_ask".into(),
                    harness_ref: None,
                    question: "定理 4 指哪一章？".into(),
                    options: Some(vec!["第一章 · 定理 4".into(), "第二章 · 定理 4".into()]),
                    ink_requested: false,
                    created_at: "2026-11-24T00:00:00Z".into(),
                })
                .unwrap();
        }

        let snap = tauri::async_runtime::block_on(state.dialogue(&fx.home, fx.session_id.clone()))
            .unwrap();
        assert_eq!(snap.pending_questions.len(), 1);
        assert_eq!(snap.pending_questions[0].thread_id, "d1");

        // 1. Structured answer via `answer_question` settles the row, appends clean user bubble,
        //    and injects `mathscape.user_answer` into `<mathscape_turn_context>`.
        let (sink, updates) = channel_sink();
        let answered_out = tauri::async_runtime::block_on(state.answer_question(
            Arc::clone(&fx.home),
            fx.session_id.clone(),
            "mathscape:question/q-d1".into(),
            Some("按第一章母函数一节处理".into()),
            Some("第一章 · 定理 4".into()),
            None,
            sink,
        ))
        .unwrap();
        let _ = wait_for(&updates, finished);
        assert!(answered_out.pending_questions.is_empty());
        assert!(
            answered_out
                .user_entry
                .text
                .contains("【确认：第一章 · 定理 4】")
        );
        assert!(
            answered_out
                .user_entry
                .text
                .contains("按第一章母函数一节处理")
        );

        let turns = logged(&fx, "codex-home", "turn/start");
        let last_turn = turns.last().expect("answer turn/start").to_string();
        assert!(last_turn.contains("mathscape:question/q-d1"), "{last_turn}");
        assert!(last_turn.contains("第一章 · 定理 4"), "{last_turn}");
        assert!(last_turn.contains("按第一章母函数一节处理"), "{last_turn}");

        // 2. Plain `send` on the active thread auto-settles an open pending question,
        //    and `dismiss_question` cancels another round's pending question.
        {
            let mut store =
                SqliteArtifactStore::open(&workspace.join("mathscape.sqlite3")).unwrap();
            store
                .create_pending_question(&CreatePendingQuestionInput {
                    question_id: "mathscape:question/q-auto".into(),
                    investigation_id: inv.clone(),
                    round_id: answered_out.round_id.clone(),
                    origin: "mcp_user_ask".into(),
                    harness_ref: None,
                    question: "是否升格独立题型？".into(),
                    options: None,
                    ink_requested: false,
                    created_at: "2026-11-24T00:01:00Z".into(),
                })
                .unwrap();
        }
        let (auto_sent, _) = send_and_wait(&state, &fx, "不用升格，保持来源层即可");
        assert!(auto_sent.pending_questions.is_empty());
        {
            let mut store =
                SqliteArtifactStore::open(&workspace.join("mathscape.sqlite3")).unwrap();
            let all = store.list_pending_questions(&inv, false).unwrap();
            assert_eq!(all.len(), 2);
            assert!(all.iter().all(|q| q.state == "answered"));
            store
                .create_pending_question(&CreatePendingQuestionInput {
                    question_id: "mathscape:question/q-dismiss".into(),
                    investigation_id: inv.clone(),
                    round_id: auto_sent.round_id.clone(),
                    origin: "mcp_user_ask".into(),
                    harness_ref: None,
                    question: "顺便检查第 9 题吗？".into(),
                    options: None,
                    ink_requested: false,
                    created_at: "2026-11-24T00:02:00Z".into(),
                })
                .unwrap();
        }
        let after_dismiss = tauri::async_runtime::block_on(state.dismiss_question(
            &fx.home,
            fx.session_id.clone(),
            "mathscape:question/q-dismiss".into(),
        ))
        .unwrap();
        assert!(after_dismiss.pending_questions.is_empty());
        let store = SqliteArtifactStore::open(&workspace.join("mathscape.sqlite3")).unwrap();
        let dismissed = store
            .list_pending_questions(&inv, false)
            .unwrap()
            .into_iter()
            .find(|q| q.question_id == "mathscape:question/q-dismiss")
            .unwrap();
        assert_eq!(dismissed.state, "cancelled");
        drop(store);
        state.close_all();
    }
}
