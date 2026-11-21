//! [`CodexExecutor`]: the ACP-shaped [`AgentExecutor`] over long-lived
//! `codex app-server` processes (one per [`SessionSpec::session_key`],
//! driven by [`super::host`]), plus the JSON-RPC message builders.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::StreamExt;
use futures::channel::mpsc as futures_mpsc;
use futures::channel::oneshot;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use mathscape_resident::{EventSink, null_sink};
use serde_json::{Value, json};

use super::approval::{APPROVAL_TIMEOUT, SANDBOX_APPROVAL_POLICY, enabled_tools};
use super::host::{HostCommand, HostHandle, PromptJob, spawn_host};
use crate::{
    AgentBackend, AgentError, AgentEvent, AgentExecutor, ApprovalHandler, Capability,
    CapabilityProbe, CommandRunner, ContextNote, ErrorCode, HarnessContext, LoadThread,
    PolicyApprovals, Prompt, RoundId, SessionSpec, SystemCommandRunner, ThreadBinding,
    probe_program,
};

/// `serviceName` / `threadSource` on threads Mathscape starts.
pub(crate) const SERVICE_NAME: &str = "mathscape";
/// How long `close_all` waits for each session to stop.
const CLOSE_WAIT: Duration = Duration::from_secs(6);

/// How [`CodexExecutor`] locates the Codex CLI and the Mathscape MCP binary.
///
/// The child inherits the process environment so the CLI can use the user's
/// existing vendor login. This config has no field for an API key or ChatGPT
/// token; Mathscape does not read or inject one.
#[derive(Clone, Debug)]
pub struct CodexConfig {
    /// Codex CLI. Default `codex` on `PATH`.
    pub program: PathBuf,
    /// Arguments. Default `["app-server"]` (stdio JSON-RPC).
    pub args: Vec<String>,
    /// `mathscape` binary registered as MCP server `mathscape`.
    pub mcp_program: PathBuf,
    /// Bound for `initialize`, `thread/start|resume` and `turn/start`
    /// acceptance. The model run itself is not capped here.
    pub handshake_timeout: Duration,
    /// After `turn/interrupt`, how long to wait before killing the child.
    pub interrupt_grace: Duration,
    /// Unclassified approval requests are declined after this long.
    /// Default [`APPROVAL_TIMEOUT`] (60s). The dialogue shows 「等待审批已超时」.
    pub approval_timeout: Duration,
    /// A session's app-server is stopped after this long without a turn
    /// (default 15 min; `None` keeps it until closed).
    pub idle_timeout: Option<Duration>,
    /// Override for Codex home (`config.toml` / `models_cache.json` /
    /// `sessions/`). `None` → `$CODEX_HOME` or `~/.codex` and the child's
    /// environment is left alone. Tests pass a temp dir; then the child gets
    /// `CODEX_HOME` set to it.
    pub home: Option<PathBuf>,
}

impl Default for CodexConfig {
    fn default() -> Self {
        Self {
            program: PathBuf::from("codex"),
            args: vec!["app-server".into()],
            mcp_program: PathBuf::from("mathscape"),
            handshake_timeout: Duration::from_secs(20),
            interrupt_grace: Duration::from_secs(5),
            approval_timeout: APPROVAL_TIMEOUT,
            idle_timeout: Some(Duration::from_secs(15 * 60)),
            home: None,
        }
    }
}

struct Shared {
    config: CodexConfig,
    runner: Arc<dyn CommandRunner>,
    approvals: Arc<dyn ApprovalHandler>,
    sink: EventSink,
    sessions: Mutex<HashMap<String, HostHandle>>,
    rounds: Mutex<HashMap<RoundId, RoundSlot>>,
    next_round: AtomicU64,
}

struct RoundSlot {
    bus: Arc<EventBus>,
    session_key: String,
}

/// Local Codex backend: one long-lived app-server per harness session.
pub struct CodexExecutor {
    shared: Arc<Shared>,
}

impl CodexExecutor {
    pub fn new(config: CodexConfig) -> Self {
        Self::with_runner(config, Arc::new(SystemCommandRunner::default()))
    }

    pub fn with_runner(config: CodexConfig, runner: Arc<dyn CommandRunner>) -> Self {
        Self {
            shared: Arc::new(Shared {
                config,
                runner,
                approvals: Arc::new(PolicyApprovals),
                sink: null_sink(),
                sessions: Mutex::new(HashMap::new()),
                rounds: Mutex::new(HashMap::new()),
                next_round: AtomicU64::new(0),
            }),
        }
    }

    fn shared_mut(&mut self) -> &mut Shared {
        Arc::get_mut(&mut self.shared).expect("configure CodexExecutor before sharing it")
    }

    /// Host approval callback (ACP `session/request_permission`).
    #[must_use]
    pub fn with_approvals(mut self, approvals: Arc<dyn ApprovalHandler>) -> Self {
        self.shared_mut().approvals = approvals;
        self
    }

    /// Lifecycle events of the app-server processes (spawn / ready /
    /// idle_reclaim / exit …), e.g. into `resident-processes.jsonl`.
    #[must_use]
    pub fn with_event_sink(mut self, sink: EventSink) -> Self {
        self.shared_mut().sink = sink;
        self
    }

    /// Process ids of running app-servers, by session key (diagnostics/tests).
    pub fn live_sessions(&self) -> Vec<String> {
        let mut keys: Vec<String> = self
            .shared
            .sessions
            .lock()
            .expect("codex sessions")
            .iter()
            .filter(|(_, handle)| handle.alive.load(Ordering::Acquire))
            .map(|(key, _)| key.clone())
            .collect();
        keys.sort();
        keys
    }
}

impl Drop for CodexExecutor {
    fn drop(&mut self) {
        if Arc::strong_count(&self.shared) == 1 {
            self.close_all();
        }
    }
}

fn session_ended(key: &str) -> AgentError {
    harness(
        ErrorCode::Internal,
        format!("codex session {key} ended before answering"),
    )
}

impl Shared {
    fn handle(&self, key: &str) -> Result<mpsc::Sender<HostCommand>, AgentError> {
        let sessions = self.sessions.lock().expect("codex sessions");
        match sessions.get(key) {
            Some(handle) if handle.alive.load(Ordering::Acquire) => Ok(handle.tx.clone()),
            _ => Err(AgentError::UnknownSession(key.to_owned())),
        }
    }

    fn open(&self, spec: SessionSpec) -> Result<(), AgentError> {
        if spec.backend != AgentBackend::Codex {
            return Err(AgentError::BackendMismatch {
                served: AgentBackend::Codex,
                requested: spec.backend,
            });
        }
        let mut sessions = self.sessions.lock().expect("codex sessions");
        if let Some(handle) = sessions.get_mut(&spec.session_key)
            && handle.alive.load(Ordering::Acquire)
        {
            if handle.spec != spec {
                handle.spec = spec.clone();
                if handle
                    .tx
                    .send(HostCommand::Update(Box::new(spec.clone())))
                    .is_ok()
                {
                    return Ok(());
                }
            } else {
                return Ok(());
            }
        }
        let probe = probe_program(
            self.runner.as_ref(),
            Capability::Codex,
            &self.config.program,
        );
        if !probe.is_available() {
            return Err(AgentError::CapabilityUnavailable(probe));
        }
        let alive = Arc::new(AtomicBool::new(true));
        let tx = spawn_host(
            self.config.clone(),
            spec.clone(),
            Arc::clone(&self.approvals),
            self.sink.clone(),
            Arc::clone(&alive),
        )?;
        sessions.insert(spec.session_key.clone(), HostHandle { tx, spec, alive });
        Ok(())
    }

    fn next_round_id(&self) -> RoundId {
        let n = self.next_round.fetch_add(1, Ordering::Relaxed) + 1;
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0);
        RoundId::parse(format!("codex-{millis}-{n}")).expect("round id has no whitespace")
    }
}

impl AgentExecutor for CodexExecutor {
    fn backend(&self) -> AgentBackend {
        AgentBackend::Codex
    }

    fn open_session(&self, spec: SessionSpec) -> BoxFuture<'_, Result<(), AgentError>> {
        let result = self.shared.open(spec);
        Box::pin(async move { result })
    }

    fn new_thread(&self, session_key: &str) -> BoxFuture<'_, Result<ThreadBinding, AgentError>> {
        let key = session_key.to_owned();
        Box::pin(async move {
            let tx = self.shared.handle(&key)?;
            let (reply, rx) = oneshot::channel();
            tx.send(HostCommand::NewThread(reply))
                .map_err(|_| session_ended(&key))?;
            rx.await.map_err(|_| session_ended(&key))?
        })
    }

    fn load_thread(
        &self,
        session_key: &str,
        load: LoadThread,
    ) -> BoxFuture<'_, Result<ThreadBinding, AgentError>> {
        let key = session_key.to_owned();
        Box::pin(async move {
            let tx = self.shared.handle(&key)?;
            let (reply, rx) = oneshot::channel();
            tx.send(HostCommand::LoadThread(load, reply))
                .map_err(|_| session_ended(&key))?;
            rx.await.map_err(|_| session_ended(&key))?
        })
    }

    fn prompt(
        &self,
        session_key: &str,
        prompt: Prompt,
    ) -> BoxFuture<'_, Result<RoundId, AgentError>> {
        let key = session_key.to_owned();
        Box::pin(async move {
            let tx = self.shared.handle(&key)?;
            let round_id = self.shared.next_round_id();
            let bus = Arc::new(EventBus::new());
            self.shared.rounds.lock().expect("codex rounds").insert(
                round_id.clone(),
                RoundSlot {
                    bus: Arc::clone(&bus),
                    session_key: key.clone(),
                },
            );
            let (reply, rx) = oneshot::channel();
            let sent = tx.send(HostCommand::Prompt(Box::new(PromptJob {
                prompt,
                round_id: round_id.clone(),
                bus,
                reply,
            })));
            let result = match sent {
                Ok(()) => rx.await.unwrap_or_else(|_| Err(session_ended(&key))),
                Err(_) => Err(session_ended(&key)),
            };
            match result {
                Ok(()) => Ok(round_id),
                Err(error) => {
                    self.shared
                        .rounds
                        .lock()
                        .expect("codex rounds")
                        .remove(&round_id);
                    Err(error)
                }
            }
        })
    }

    fn thread_state_path(&self, session_key: &str, thread_id: &str) -> Option<PathBuf> {
        if let Ok(tx) = self.shared.handle(session_key) {
            let (reply, rx) = mpsc::channel();
            if tx
                .send(HostCommand::StatePath(thread_id.to_owned(), reply))
                .is_ok()
                && let Ok(path) = rx.recv_timeout(Duration::from_secs(5))
            {
                return path;
            }
        }
        let home = super::model::codex_home(self.shared.config.home.as_deref())?;
        super::rollout::find_rollout(&home, thread_id)
    }

    fn close_session(&self, session_key: &str) -> BoxFuture<'_, Result<(), AgentError>> {
        let handle = self
            .shared
            .sessions
            .lock()
            .expect("codex sessions")
            .remove(session_key);
        Box::pin(async move {
            if let Some(handle) = handle {
                let (ack, rx) = oneshot::channel();
                if handle.tx.send(HostCommand::Close(Some(ack))).is_ok() {
                    let _ = rx.await;
                }
            }
            Ok(())
        })
    }

    fn close_all(&self) {
        let handles: Vec<HostHandle> = self
            .shared
            .sessions
            .lock()
            .expect("codex sessions")
            .drain()
            .map(|(_, handle)| handle)
            .collect();
        let mut waits = Vec::new();
        for handle in handles {
            let (ack, rx) = oneshot::channel();
            if handle.tx.send(HostCommand::Close(Some(ack))).is_ok() {
                waits.push(rx);
            }
        }
        // Sessions shut down in parallel on their own threads.
        let deadline = std::time::Instant::now() + CLOSE_WAIT;
        for mut rx in waits {
            while std::time::Instant::now() < deadline {
                match rx.try_recv() {
                    Ok(Some(())) | Err(_) => break,
                    Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                }
            }
        }
    }

    fn events(&self, round_id: &RoundId) -> Result<BoxStream<'static, AgentEvent>, AgentError> {
        let rounds = self.shared.rounds.lock().expect("codex rounds");
        let slot = rounds
            .get(round_id)
            .ok_or_else(|| AgentError::UnknownRound(round_id.clone()))?;
        Ok(slot.bus.subscribe())
    }

    fn cancel(&self, round_id: &RoundId) -> BoxFuture<'_, Result<(), AgentError>> {
        let result = {
            let rounds = self.shared.rounds.lock().expect("codex rounds");
            match rounds.get(round_id) {
                None => Err(AgentError::UnknownRound(round_id.clone())),
                Some(slot) => {
                    if let Ok(tx) = self.shared.handle(&slot.session_key) {
                        let _ = tx.send(HostCommand::Cancel(round_id.clone()));
                    }
                    Ok(())
                }
            }
        };
        Box::pin(async move { result })
    }

    fn final_text(&self, round_id: &RoundId) -> Option<String> {
        let rounds = self.shared.rounds.lock().expect("codex rounds");
        rounds.get(round_id).and_then(|slot| slot.bus.final_text())
    }

    fn current_model(&self) -> Option<String> {
        super::model::current_model(self.shared.config.home.as_deref())
    }

    fn list_models(&self) -> Result<Vec<crate::AgentModel>, AgentError> {
        let config = &self.shared.config;
        super::model::list_models(
            config.home.as_deref(),
            Some(super::model::AppServerProbe {
                program: &config.program,
                args: &config.args,
                timeout: config
                    .handshake_timeout
                    .min(std::time::Duration::from_secs(8)),
            }),
        )
    }

    fn set_model(&self, id: &str) -> Result<(), AgentError> {
        let catalog = self.list_models().unwrap_or_default();
        super::model::set_model(self.shared.config.home.as_deref(), id, &catalog)
    }

    fn current_effort(&self) -> Option<String> {
        let home = self.shared.config.home.as_deref();
        super::model::effective_effort(
            super::model::configured_effort(home).as_deref(),
            super::model::current_model(home).as_deref(),
            &super::model::cached_catalog(home),
        )
    }

    fn list_efforts(&self, model: Option<&str>) -> Result<Vec<crate::AgentEffort>, AgentError> {
        let model = match model {
            Some(model) => Some(model.to_owned()),
            None => self.current_model(),
        };
        let Some(model) = model else {
            return Ok(Vec::new());
        };
        Ok(super::model::efforts_for(&model, &self.list_models()?))
    }

    fn set_effort(&self, effort: &str) -> Result<(), AgentError> {
        let catalog = self.list_models().unwrap_or_default();
        super::model::set_effort(self.shared.config.home.as_deref(), effort, &catalog)
    }

    fn probe(&self) -> Vec<CapabilityProbe> {
        vec![probe_program(
            self.shared.runner.as_ref(),
            Capability::Codex,
            &self.shared.config.program,
        )]
    }
}

const MCP_FORWARDED_ENV_VARS: [&str; 4] = [
    "MATHSCAPE_MARKER_EXECUTABLE",
    "MATHSCAPE_MARKER_VENV",
    "XDG_DATA_HOME",
    "MATHSCAPE_LEAN_PROFILE_ROOT",
];

/// Name of the per-round Codex permission profile (`[permissions.<name>]`).
pub(crate) const SESSION_PERMISSION_PROFILE: &str = "mathscape-session";

fn forwarded_env_vars(harness: &HarnessContext) -> Vec<String> {
    let mut names: Vec<String> = MCP_FORWARDED_ENV_VARS
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    for (name, _) in &harness.env {
        if !names.contains(name) {
            names.push(name.clone());
        }
    }
    names
}

/// `[permissions.mathscape-session.filesystem]`: read `:minimal`, the
/// workspace, extra read roots and skills; write only the scratch dir. No
/// `:root`, so everything else (home, `~/.codex`, other projects) stays
/// unreadable to harness tools. Network is off by default in a profile.
fn session_filesystem_permissions(workspace: &str, harness: &HarnessContext) -> Value {
    let mut grants = serde_json::Map::new();
    grants.insert(":minimal".to_owned(), json!("read"));
    grants.insert(workspace.to_owned(), json!("read"));
    for root in harness.read_roots.iter().chain(&harness.skill_roots) {
        grants.insert(absolute_path(root).display().to_string(), json!("read"));
    }
    if let Some(scratch) = &harness.scratch_dir {
        grants.insert(absolute_path(scratch).display().to_string(), json!("write"));
    }
    Value::Object(grants)
}

pub(crate) fn skill_roots_message(id: u64, harness: &HarnessContext) -> Option<Value> {
    if harness.skill_roots.is_empty() {
        return None;
    }
    let roots: Vec<String> = harness
        .skill_roots
        .iter()
        .map(|root| absolute_path(root).display().to_string())
        .collect();
    Some(json!({
        "method": "skills/extraRoots/set",
        "id": id,
        "params": { "extraRoots": roots }
    }))
}

/// Per-call MCP tool timeout. `material.read` on a scanned PDF waits for
/// Marker synchronously, bounded by the ensure `sync_timeout` (15 min in
/// `mathscape-mcp`); this leaves one minute of margin so the tool, not Codex,
/// reports the timeout and clears its lease.
const MCP_TOOL_TIMEOUT_SEC: u64 = 16 * 60;

/// `config` overrides shared by `thread/start` and `thread/resume`: the
/// mathscape MCP registration and (with harness grants) the session
/// permission profile. On resume Codex honours them and restarts the
/// thread's MCP server, so a resumed thread gets the same tools.
fn thread_base_params(spec: &SessionSpec, mcp_program: &Path, round_file: &Path) -> Value {
    let workspace = absolute_path(&spec.workspace);
    let workspace = workspace.display().to_string();
    let mut params = json!({
        "cwd": workspace,
        "approvalPolicy": SANDBOX_APPROVAL_POLICY,
        "config": {
            "mcp_servers.mathscape.command": mcp_program.display().to_string(),
            "mcp_servers.mathscape.args": [
                "mcp",
                "--workspace",
                workspace,
                "--round-file",
                absolute_path(round_file).display().to_string()
            ],
            "mcp_servers.mathscape.enabled": true,
            "mcp_servers.mathscape.default_tools_approval_mode": "approve",
            "mcp_servers.mathscape.enabled_tools": enabled_tools(&spec.tools_allow),
            // material.read may run a bounded Marker ensure on a scanned PDF:
            // forward only the Marker lookup variables (no secrets) and allow
            // the synchronous ensure wait plus margin.
            "mcp_servers.mathscape.env_vars": forwarded_env_vars(&spec.harness),
            "mcp_servers.mathscape.tool_timeout_sec": MCP_TOOL_TIMEOUT_SEC
        }
    });
    if spec.harness.uses_permission_profile() {
        // Do not also send `sandbox`: a legacy sandbox mode overrides the
        // profile (thread/start then reports `activePermissionProfile: null`).
        let grants = session_filesystem_permissions(&workspace, &spec.harness);
        params["config"]["default_permissions"] = json!(SESSION_PERMISSION_PROFILE);
        params["config"][format!("permissions.{SESSION_PERMISSION_PROFILE}.filesystem")] = grants;
    } else {
        params["sandbox"] = json!("read-only");
    }
    params
}

fn with_model(params: &mut Value, model: Option<&str>) {
    if let Some(model) = model.map(str::trim).filter(|id| !id.is_empty()) {
        params["model"] = json!(model);
    }
}

/// `thread/start`. Persistent sessions start non-ephemeral threads (Codex
/// writes a rollout under `$CODEX_HOME/sessions`); `threadSource` tags them
/// as Mathscape's in the user's Codex history.
pub(crate) fn thread_start_message(
    id: u64,
    spec: &SessionSpec,
    mcp_program: &Path,
    round_file: &Path,
    model: Option<&str>,
) -> Value {
    let mut params = thread_base_params(spec, mcp_program, round_file);
    params["ephemeral"] = json!(spec.ephemeral);
    params["serviceName"] = json!(SERVICE_NAME);
    params["threadSource"] = json!(SERVICE_NAME);
    params["developerInstructions"] =
        json!(developer_instructions(&spec.tools_allow, &spec.harness));
    with_model(&mut params, model);
    json!({
        "method": "thread/start",
        "id": id,
        "params": params
    })
}

/// `thread/resume` by id. Codex ignores `developerInstructions` on resume
/// (the thread keeps its original prefix), so none are sent; `cwd` and the
/// `config` overrides are honoured.
pub(crate) fn thread_resume_message(
    id: u64,
    thread_id: &str,
    spec: &SessionSpec,
    mcp_program: &Path,
    round_file: &Path,
    model: Option<&str>,
) -> Value {
    let mut params = thread_base_params(spec, mcp_program, round_file);
    params["threadId"] = json!(thread_id);
    params["excludeTurns"] = json!(true);
    with_model(&mut params, model);
    json!({
        "method": "thread/resume",
        "id": id,
        "params": params
    })
}

/// Opening tag of the per-turn context item (docs/codex-session-persistence.md §2.5).
pub(crate) const TURN_CONTEXT_OPEN: &str = "<mathscape_turn_context>";
pub(crate) const TURN_CONTEXT_CLOSE: &str = "</mathscape_turn_context>";

/// Per-turn context rendered as its own `input` text item placed before the
/// user text. `turn/start.additionalContext` needs the experimental API, so
/// the notes travel as ordinary user input; they are appended at the end of
/// the conversation and never touch the cached prefix.
pub(crate) fn turn_context_text(notes: &[ContextNote]) -> Option<String> {
    let notes: Vec<&ContextNote> = notes
        .iter()
        .filter(|note| !note.text.trim().is_empty())
        .collect();
    if notes.is_empty() {
        return None;
    }
    let mut text = String::from(TURN_CONTEXT_OPEN);
    text.push('\n');
    text.push_str("Mathscape-supplied context for this turn (not written by the user):\n");
    for note in notes {
        text.push_str(&format!("\n[{}]\n{}\n", note.key, note.text.trim()));
    }
    text.push_str(TURN_CONTEXT_CLOSE);
    Some(text)
}

/// `effort` maps to app-server v2 `TurnStartParams.effort` ("Override the
/// reasoning effort for this turn and subsequent turns"). `inputs` are text
/// items in order; the user text is last.
pub(crate) fn turn_start_message(
    id: u64,
    thread_id: &str,
    inputs: &[String],
    cwd: &str,
    model: Option<&str>,
    effort: Option<&str>,
) -> Value {
    let input: Vec<Value> = inputs
        .iter()
        .map(|text| json!({ "type": "text", "text": text }))
        .collect();
    let mut params = json!({
        "threadId": thread_id,
        "cwd": cwd,
        "input": input
    });
    with_model(&mut params, model);
    if let Some(effort) = effort.map(str::trim).filter(|e| !e.is_empty()) {
        params["effort"] = json!(effort);
    }
    json!({
        "method": "turn/start",
        "id": id,
        "params": params
    })
}

pub(crate) fn initialize_message(id: u64) -> Value {
    json!({
        "method": "initialize",
        "id": id,
        "params": {
            "clientInfo": {
                "name": "mathscape",
                "title": "Mathscape",
                "version": env!("CARGO_PKG_VERSION")
            }
        }
    })
}

/// Stable per-thread prefix: no round id or other per-turn values (those
/// go into the turn context item), so the prompt cache survives turns.
fn developer_instructions(tools_allow: &[String], harness: &HarnessContext) -> String {
    let tools = if tools_allow.is_empty() {
        "The mathscape MCP server exposes a fixed read-only tool list; call those tools when they help.".to_owned()
    } else {
        format!(
            "Call only these mathscape tools in this session: {}.",
            tools_allow.join(", ")
        )
    };
    let mut text = format!(
        "This is a Mathscape session. {tools} Mathscape does not provide model credentials; use this Codex process's existing local login. Do not ask the user for an API key."
    );
    if let Some(scratch) = &harness.scratch_dir {
        text.push_str(&format!(
            " Your own file and shell tools may write only under {}; every other path is read-only and the network is off. Files there are drafts, not map changes: map changes go through mathscape proposals that the user accepts.",
            absolute_path(scratch).display()
        ));
    }
    if let Some(extra) = harness
        .instructions
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        text.push_str("\n\n");
        text.push_str(extra);
    }
    text
}

pub(crate) fn absolute_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }
    if path.is_absolute() {
        return path.to_path_buf();
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

pub(crate) enum Incoming {
    Empty,
    Foreign,
    BadJson(String),
    Notification(Value),
    Response(Value),
    Request {
        id: Value,
        method: String,
        params: Value,
    },
}

pub(crate) fn parse_incoming(line: &str) -> Incoming {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Incoming::Empty;
    }
    if !trimmed.starts_with('{') {
        return Incoming::Foreign;
    }
    let value: Value = match serde_json::from_str(trimmed) {
        Ok(value) => value,
        Err(error) => return Incoming::BadJson(error.to_string()),
    };
    let method = value
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let has_id = value.get("id").is_some();
    match (method, has_id) {
        (Some(method), true) => Incoming::Request {
            id: value.get("id").cloned().unwrap_or(Value::Null),
            method,
            params: value.get("params").cloned().unwrap_or(Value::Null),
        },
        (Some(_), false) => Incoming::Notification(value),
        (None, true) => Incoming::Response(value),
        (None, false) => Incoming::Foreign,
    }
}

pub(crate) fn response_id(value: &Value) -> Option<u64> {
    value.get("id").and_then(|id| {
        id.as_u64()
            .or_else(|| id.as_i64().filter(|n| *n >= 0).map(|n| n as u64))
    })
}

pub(crate) fn rpc_error_message(value: &Value) -> Option<String> {
    let error = value.get("error")?;
    Some(
        error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("codex request failed")
            .to_owned(),
    )
}

pub(crate) fn harness(code: ErrorCode, message: impl Into<String>) -> AgentError {
    AgentError::Harness {
        backend: AgentBackend::Codex,
        code,
        message: message.into(),
    }
}

pub(crate) fn error_text(error: &AgentError) -> String {
    error.to_string()
}

pub(crate) fn error_code(error: &AgentError) -> ErrorCode {
    match error {
        AgentError::Harness { code, .. } => *code,
        AgentError::CapabilityUnavailable(_) => ErrorCode::BackendUnavailable,
        _ => ErrorCode::Internal,
    }
}

struct BusInner {
    events: Vec<AgentEvent>,
    final_text: Option<String>,
    senders: Vec<futures_mpsc::UnboundedSender<AgentEvent>>,
    finished: bool,
}

pub(crate) struct EventBus {
    inner: Mutex<BusInner>,
}

impl EventBus {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(BusInner {
                events: Vec::new(),
                final_text: None,
                senders: Vec::new(),
                finished: false,
            }),
        }
    }

    pub(crate) fn publish_all(&self, events: Vec<AgentEvent>) {
        for event in events {
            self.publish(event);
        }
    }

    pub(crate) fn publish(&self, event: AgentEvent) {
        let mut guard = self.inner.lock().expect("event bus");
        if guard.finished {
            return;
        }
        guard.events.push(event.clone());
        guard
            .senders
            .retain(|sender| sender.unbounded_send(event.clone()).is_ok());
    }

    pub(crate) fn set_final_text(&self, text: Option<String>) {
        self.inner.lock().expect("event bus").final_text = text;
    }

    pub(crate) fn final_text(&self) -> Option<String> {
        self.inner.lock().expect("event bus").final_text.clone()
    }

    pub(crate) fn finish(&self) {
        let mut guard = self.inner.lock().expect("event bus");
        guard.finished = true;
        guard.senders.clear();
    }

    pub(crate) fn subscribe(&self) -> BoxStream<'static, AgentEvent> {
        let (sender, receiver) = futures_mpsc::unbounded();
        let snapshot = {
            let mut guard = self.inner.lock().expect("event bus");
            if !guard.finished {
                guard.senders.push(sender);
            }
            guard.events.clone()
        };
        Box::pin(futures::stream::iter(snapshot).chain(receiver))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use futures::executor::block_on;

    use super::*;
    use crate::{CapabilityState, CommandOutput};

    fn fixture_path(parts: &[&str]) -> PathBuf {
        parts
            .iter()
            .fold(PathBuf::from(env!("CARGO_MANIFEST_DIR")), |path, part| {
                path.join(part)
            })
    }

    fn resolved(path: &Path) -> String {
        absolute_path(path).display().to_string()
    }

    fn round_file() -> PathBuf {
        fixture_path(&["session", "current_round"])
    }

    fn request(backend: AgentBackend) -> crate::StartRoundRequest {
        crate::StartRoundRequest {
            backend,
            workspace: fixture_path(&["test-workspace"]),
            input_ref: "explain 紧集".into(),
            tools_allow: vec!["workspace.search".into(), "material.read".into()],
            harness: HarnessContext::default(),
        }
    }

    fn spec(ephemeral: bool) -> SessionSpec {
        SessionSpec {
            backend: AgentBackend::Codex,
            session_key: "s1".into(),
            workspace: fixture_path(&["test-workspace"]),
            tools_allow: vec!["workspace.search".into(), "material.read".into()],
            harness: HarnessContext::default(),
            round_file: None,
            ephemeral,
        }
    }

    #[test]
    fn thread_start_registers_mcp_without_credentials() {
        let message =
            thread_start_message(2, &spec(true), Path::new("mathscape"), &round_file(), None);
        let params = &message["params"];
        assert_eq!(params["cwd"], resolved(&spec(true).workspace));
        assert_eq!(params["sandbox"], "read-only");
        assert_eq!(params["approvalPolicy"], "on-request");
        assert_eq!(params["ephemeral"], true);
        assert_eq!(params["serviceName"], "mathscape");
        assert_eq!(params["threadSource"], "mathscape");
        assert!(message.get("params").unwrap().get("capabilities").is_none());
        let config = &params["config"];
        assert_eq!(config["mcp_servers.mathscape.command"], "mathscape");
        assert_eq!(config["mcp_servers.mathscape.enabled"], true);
        assert_eq!(
            config["mcp_servers.mathscape.default_tools_approval_mode"],
            "approve"
        );
        assert_eq!(
            config["mcp_servers.mathscape.enabled_tools"],
            json!(["workspace.search", "material.read"])
        );
        assert_eq!(
            config["mcp_servers.mathscape.env_vars"],
            json!([
                "MATHSCAPE_MARKER_EXECUTABLE",
                "MATHSCAPE_MARKER_VENV",
                "XDG_DATA_HOME",
                "MATHSCAPE_LEAN_PROFILE_ROOT"
            ])
        );
        assert!(config.get("default_permissions").is_none());
        assert_eq!(config["mcp_servers.mathscape.tool_timeout_sec"], 960);
        assert_eq!(
            config["mcp_servers.mathscape.args"],
            json!([
                "mcp",
                "--workspace",
                resolved(&spec(true).workspace),
                "--round-file",
                resolved(&round_file())
            ])
        );
        let text = message.to_string().to_ascii_lowercase();
        assert!(!text.contains("apikey"));
        assert!(!text.contains("authorization"));
        assert!(!text.contains("account/login"));
        assert!(!text.contains("bearer"));
        let instructions = params["developerInstructions"].as_str().unwrap();
        assert!(instructions.contains("workspace.search"));
        assert!(instructions.contains("material.read"));
        assert!(instructions.contains("existing local login"));
        // Stable prefix: no per-round values in developer instructions.
        assert!(!instructions.contains("round"));
    }

    #[test]
    fn persistent_threads_are_not_ephemeral_and_resume_keeps_the_config() {
        let start = thread_start_message(
            2,
            &spec(false),
            Path::new("mathscape"),
            &round_file(),
            Some("gpt-test"),
        );
        assert_eq!(start["params"]["ephemeral"], false);
        let resume = thread_resume_message(
            7,
            "thr_1",
            &spec(false),
            Path::new("mathscape"),
            &round_file(),
            Some("gpt-test"),
        );
        assert_eq!(resume["method"], "thread/resume");
        let params = &resume["params"];
        assert_eq!(params["threadId"], "thr_1");
        assert_eq!(params["cwd"], resolved(&spec(false).workspace));
        assert_eq!(params["approvalPolicy"], "on-request");
        assert_eq!(params["excludeTurns"], true);
        assert_eq!(params["model"], "gpt-test");
        // Same MCP registration as thread/start (Codex restarts the server).
        assert_eq!(params["config"], start["params"]["config"]);
        assert_eq!(params["sandbox"], start["params"]["sandbox"]);
        // Ignored by Codex on resume; not sent.
        assert!(params.get("developerInstructions").is_none());
        assert!(params.get("path").is_none());
    }

    #[test]
    fn turn_context_is_a_separate_input_item_before_the_user_text() {
        assert!(turn_context_text(&[]).is_none());
        assert!(
            turn_context_text(&[ContextNote {
                key: "k".into(),
                text: "  ".into()
            }])
            .is_none()
        );
        let context = turn_context_text(&[
            ContextNote {
                key: "mathscape.workspace_delta".into(),
                text: "图谱新增 2 个节点".into(),
            },
            ContextNote {
                key: "mathscape.round".into(),
                text: "round_id: codex-9".into(),
            },
        ])
        .unwrap();
        assert!(context.starts_with(TURN_CONTEXT_OPEN));
        assert!(context.ends_with(TURN_CONTEXT_CLOSE));
        assert!(context.contains("[mathscape.workspace_delta]\n图谱新增 2 个节点"));
        let message = turn_start_message(
            3,
            "thr_1",
            &[context.clone(), "下一步？".into()],
            "/tmp/ws",
            None,
            None,
        );
        let input = message["params"]["input"].as_array().unwrap();
        assert_eq!(input.len(), 2);
        assert_eq!(input[0]["text"], context);
        assert_eq!(input[1]["text"], "下一步？");
        assert!(message["params"].get("additionalContext").is_none());
    }

    #[test]
    fn harness_grants_switch_to_session_permission_profile() {
        let mut request = request(AgentBackend::Codex);
        request.harness = HarnessContext {
            skill_roots: vec![fixture_path(&["repo", "skills"])],
            read_roots: vec![fixture_path(&["lean", "profiles"])],
            scratch_dir: Some(fixture_path(&["session", "scratch"])),
            env: vec![(
                "MATHSCAPE_LEAN_PROFILE_ROOT".into(),
                fixture_path(&["lean", "profiles"]).display().to_string(),
            )],
            instructions: Some("Skill index: mathscape-worknet v0.6".into()),
        };
        let mut spec = spec(true);
        spec.harness = request.harness.clone();
        let message = thread_start_message(2, &spec, Path::new("mathscape"), &round_file(), None);
        let params = &message["params"];
        // A legacy sandbox mode would override the profile.
        assert!(params.get("sandbox").is_none());
        let config = &params["config"];
        assert_eq!(config["default_permissions"], SESSION_PERMISSION_PROFILE);
        let grants = &config["permissions.mathscape-session.filesystem"];
        assert_eq!(grants[":minimal"], "read");
        assert_eq!(grants[resolved(&spec.workspace)], "read");
        assert_eq!(grants[resolved(&request.harness.skill_roots[0])], "read");
        assert_eq!(grants[resolved(&request.harness.read_roots[0])], "read");
        assert_eq!(
            grants[resolved(request.harness.scratch_dir.as_ref().unwrap())],
            "write"
        );
        assert!(grants.get(":root").is_none());
        assert_eq!(
            grants
                .as_object()
                .unwrap()
                .values()
                .filter(|v| *v == "write")
                .count(),
            1
        );
        let env_vars = config["mcp_servers.mathscape.env_vars"].as_array().unwrap();
        assert_eq!(
            env_vars
                .iter()
                .filter(|name| *name == "MATHSCAPE_LEAN_PROFILE_ROOT")
                .count(),
            1
        );
        let instructions = params["developerInstructions"].as_str().unwrap();
        assert!(instructions.contains(&resolved(request.harness.scratch_dir.as_ref().unwrap())));
        assert!(instructions.contains("mathscape-worknet v0.6"));
    }

    #[test]
    fn skill_roots_are_registered_only_when_present() {
        assert!(skill_roots_message(4, &HarnessContext::default()).is_none());
        let harness = HarnessContext {
            skill_roots: vec![fixture_path(&["repo", "skills"])],
            ..HarnessContext::default()
        };
        let message = skill_roots_message(4, &harness).unwrap();
        assert_eq!(message["method"], "skills/extraRoots/set");
        assert_eq!(
            message["params"]["extraRoots"],
            json!([resolved(&harness.skill_roots[0])])
        );
    }

    #[test]
    fn empty_tools_allow_does_not_invent_a_restriction_list() {
        let mut spec = spec(true);
        spec.tools_allow.clear();
        let message = thread_start_message(2, &spec, Path::new("mathscape"), &round_file(), None);
        let instructions = message["params"]["developerInstructions"].as_str().unwrap();
        assert!(instructions.contains("fixed read-only tool list"));
        assert!(!instructions.contains("Call only these"));
        let tools = &message["params"]["config"]["mcp_servers.mathscape.enabled_tools"];
        assert_eq!(tools.as_array().map(Vec::len), Some(11));
        assert!(
            tools
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool == "pdf.page_image")
        );
        assert!(
            tools
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool == "ink.page_image")
        );
        assert!(tools.as_array().unwrap().iter().all(|tool| tool != "shell"));
    }

    #[test]
    fn turn_start_forwards_input_ref_as_user_text() {
        let message = turn_start_message(
            3,
            "thr_1",
            &["explain 紧集".into()],
            "/tmp/mathscape-ws",
            None,
            None,
        );
        assert_eq!(message["params"]["threadId"], "thr_1");
        assert_eq!(message["params"]["input"][0]["text"], "explain 紧集");
        assert!(message["params"].get("model").is_none());
        assert!(message["params"].get("effort").is_none());
    }

    #[test]
    fn turn_start_forwards_selected_effort() {
        let hi = ["hi".to_owned()];
        let message = turn_start_message(3, "thr_1", &hi, "/tmp/ws", Some("m"), Some("xhigh"));
        assert_eq!(message["params"]["effort"], "xhigh");
        assert_eq!(message["params"]["model"], "m");
        let blank = turn_start_message(3, "thr_1", &hi, "/tmp/ws", None, Some("  "));
        assert!(blank["params"].get("effort").is_none());
    }

    #[test]
    fn thread_start_forwards_selected_model() {
        let message = thread_start_message(
            2,
            &spec(true),
            Path::new("mathscape"),
            &round_file(),
            Some("gpt-test"),
        );
        assert_eq!(message["params"]["model"], "gpt-test");
    }

    #[test]
    fn initialize_does_not_opt_into_experimental_auth() {
        let message = initialize_message(1);
        assert!(message["params"].get("capabilities").is_none());
        assert_eq!(message["params"]["clientInfo"]["name"], "mathscape");
    }

    struct RecordingRunner {
        calls: Mutex<Vec<(PathBuf, Vec<String>)>>,
        output: Mutex<Option<CommandOutput>>,
    }

    impl CommandRunner for RecordingRunner {
        fn run(&self, program: &Path, args: &[&str]) -> std::io::Result<CommandOutput> {
            self.calls.lock().expect("calls").push((
                program.to_path_buf(),
                args.iter().map(|arg| (*arg).to_owned()).collect(),
            ));
            match self.output.lock().expect("output").clone() {
                Some(output) => Ok(output),
                None => Err(std::io::Error::from(std::io::ErrorKind::NotFound)),
            }
        }
        fn env_var(&self, _key: &str) -> Option<std::ffi::OsString> {
            None
        }
        fn is_file(&self, _path: &Path) -> bool {
            false
        }
    }

    #[test]
    fn probe_is_version_only_and_missing_binary_does_not_start_a_round() {
        let runner = Arc::new(RecordingRunner {
            calls: Mutex::new(Vec::new()),
            output: Mutex::new(Some(CommandOutput {
                success: true,
                stdout: "codex-cli 0.46.0\n".into(),
                stderr: String::new(),
            })),
        });
        let executor = CodexExecutor::with_runner(
            CodexConfig {
                program: PathBuf::from("codex"),
                ..CodexConfig::default()
            },
            runner.clone(),
        );
        let probes = executor.probe();
        assert_eq!(probes.len(), 1);
        assert_eq!(probes[0].capability, Capability::Codex);
        assert_eq!(probes[0].state, CapabilityState::Available);
        assert_eq!(probes[0].found_version.as_deref(), Some("0.46.0"));
        assert_eq!(
            runner.calls.lock().expect("calls").as_slice(),
            &[(PathBuf::from("codex"), vec!["--version".into()])]
        );

        let missing = Arc::new(RecordingRunner {
            calls: Mutex::new(Vec::new()),
            output: Mutex::new(None),
        });
        let executor = CodexExecutor::with_runner(
            CodexConfig {
                program: PathBuf::from("/mathscape/not-codex"),
                handshake_timeout: Duration::from_millis(200),
                ..CodexConfig::default()
            },
            missing,
        );
        let error = block_on(executor.start_round(request(AgentBackend::Codex))).unwrap_err();
        assert!(
            matches!(error, AgentError::CapabilityUnavailable(ref probe) if probe.state == CapabilityState::Missing),
            "{error:?}"
        );
        assert!(matches!(
            block_on(executor.start_round(request(AgentBackend::Hmz))),
            Err(AgentError::BackendMismatch { .. })
        ));
    }
}
