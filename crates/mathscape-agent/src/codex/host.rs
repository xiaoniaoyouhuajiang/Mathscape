//! One long-lived `codex app-server` per harness session
//! (docs/codex-session-persistence.md §2.2).
//!
//! Each session has a driver thread that owns a
//! [`mathscape_resident::Supervisor`] (lazy start, handshake timeout, idle
//! reclaim, whole-process-group kill) and multiplexes JSON-RPC over the
//! child's stdio: requests from [`super::session::CodexExecutor`] arrive as
//! [`HostCommand`]s, notifications are routed to the active turn's
//! [`CodexEventMapper`]. One turn runs at a time per session; threads are
//! loaded on demand and survive between turns while the process lives.
//!
//! The Mathscape MCP server is started by Codex per loaded thread and reads
//! the current round from `--round-file`, which the driver rewrites before
//! every `turn/start`.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use futures::channel::oneshot;
use mathscape_resident::{
    AcquireError, Concurrency, EventSink, ProcessSpec, ReadLineError, ResidentChild,
    ResidentPolicy, Supervisor, SystemClock,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::approval::{
    ApprovalPlan, ApprovalReply, approval_event_raw, approval_plan, timeout_plan,
};
use super::map::{
    CodexEventMapper, classify_codex_failure, mcp_ready_thread, mcp_status_list_message,
};
use super::rollout::{find_rollout, restore_rollout};
use super::session::{
    CodexConfig, EventBus, Incoming, absolute_path, error_code, error_text, harness,
    initialize_message, parse_incoming, response_id, rpc_error_message, skill_roots_message,
    thread_resume_message, thread_start_message, turn_context_text, turn_start_message,
};
use crate::{
    AgentError, AgentEvent, ApprovalDecision, ApprovalHandler, ApprovalRequest, ContextNote,
    ErrorCode, LoadThread, Prompt, RoundId, SessionSpec, ThreadBinding, ThreadContinuity,
};

const INITIALIZE_ID: u64 = 1;
const SKILL_ROOTS_ID: u64 = 2;
const FIRST_REQUEST_ID: u64 = 10;
/// Upper bound between SIGTERM and SIGKILL for the app-server process group.
const TERM_GRACE: Duration = Duration::from_secs(2);
/// Poll interval while a turn runs / while idle.
const ACTIVE_POLL: Duration = Duration::from_millis(20);
const IDLE_POLL: Duration = Duration::from_millis(100);

type Reply<T> = oneshot::Sender<Result<T, AgentError>>;

pub(crate) enum HostCommand {
    /// The caller re-opened the session with a different spec; the next
    /// thread operation restarts the process if its identity changed.
    Update(Box<SessionSpec>),
    NewThread(Reply<ThreadBinding>),
    LoadThread(LoadThread, Reply<ThreadBinding>),
    Prompt(Box<PromptJob>),
    Cancel(RoundId),
    StatePath(String, Sender<Option<PathBuf>>),
    Close(Option<oneshot::Sender<()>>),
}

pub(crate) struct PromptJob {
    pub(crate) prompt: Prompt,
    pub(crate) round_id: RoundId,
    pub(crate) bus: Arc<EventBus>,
    pub(crate) reply: Reply<()>,
}

pub(crate) struct HostHandle {
    pub(crate) tx: Sender<HostCommand>,
    pub(crate) spec: SessionSpec,
    pub(crate) alive: Arc<AtomicBool>,
}

pub(crate) fn spawn_host(
    config: CodexConfig,
    spec: SessionSpec,
    approvals: Arc<dyn ApprovalHandler>,
    sink: EventSink,
    alive: Arc<AtomicBool>,
) -> Result<Sender<HostCommand>, AgentError> {
    let (tx, rx) = mpsc::channel();
    let name = format!("codex-session-{}", spec.session_key);
    let alive_thread = Arc::clone(&alive);
    thread::Builder::new()
        .name(name)
        .spawn(move || {
            let mut driver = Driver::new(config, spec, approvals, sink, rx);
            driver.run();
            alive_thread.store(false, Ordering::Release);
            driver.shutdown();
        })
        .map_err(|error| {
            alive.store(false, Ordering::Release);
            harness(
                ErrorCode::Internal,
                format!("failed to spawn codex session thread: {error}"),
            )
        })?;
    Ok(tx)
}

struct PendingApproval {
    id: Value,
    method: String,
    params: Value,
    deadline: Instant,
}

#[derive(Default)]
struct LoadedThread {
    /// `mcpServer/startupStatus/updated` seen before the thread's first turn.
    mcp_notes: Vec<Value>,
    /// `mcpServerStatus/list` response seen before the thread's first turn.
    inventory: Option<Value>,
    /// The first turn on this thread has replayed the MCP startup.
    mcp_shown: bool,
    list_requested: bool,
}

/// State tied to one running process; reset when the pid changes.
#[derive(Default)]
struct Conn {
    pid: Option<u32>,
    next_id: u64,
    threads: HashMap<String, LoadedThread>,
    inventory_rpc: HashMap<u64, String>,
}

struct ActiveTurn {
    round_id: RoundId,
    thread_id: String,
    turn_id: Option<String>,
    mapper: CodexEventMapper,
    bus: Arc<EventBus>,
    pending: Vec<PendingApproval>,
    interrupt_rpc: Option<u64>,
    interrupt_deadline: Option<Instant>,
}

struct Driver {
    config: CodexConfig,
    spec: SessionSpec,
    approvals: Arc<dyn ApprovalHandler>,
    sup: Supervisor<String>,
    commands: Receiver<HostCommand>,
    conn: Conn,
    known_paths: HashMap<String, PathBuf>,
    active: Option<ActiveTurn>,
    /// Adapter-managed round file when the spec has none.
    temp_round_file: PathBuf,
    closing: bool,
    close_acks: Vec<oneshot::Sender<()>>,
}

impl Driver {
    fn new(
        config: CodexConfig,
        spec: SessionSpec,
        approvals: Arc<dyn ApprovalHandler>,
        sink: EventSink,
        commands: Receiver<HostCommand>,
    ) -> Self {
        let policy = ResidentPolicy {
            startup_timeout: config.handshake_timeout,
            request_timeout: config.handshake_timeout,
            idle_timeout: config.idle_timeout,
            // A failed start is reported to the caller; the next request
            // retries. The app-server never degrades to a one-shot path.
            max_consecutive_failures: u32::MAX,
            backoff_initial: Duration::from_millis(200),
            backoff_max: Duration::from_secs(2),
            shutdown_grace: TERM_GRACE.min(config.interrupt_grace),
            concurrency: Concurrency::Multiplexed,
            farewell: None,
        };
        let sup = Supervisor::new(
            format!("codex-app-server:{}", spec.session_key),
            policy,
            Arc::new(SystemClock),
            sink,
        );
        let temp_round_file = std::env::temp_dir().join(format!(
            "mathscape-codex-{}-{}.round",
            std::process::id(),
            sanitize(&spec.session_key)
        ));
        Self {
            config,
            spec,
            approvals,
            sup,
            commands,
            conn: Conn::default(),
            known_paths: HashMap::new(),
            active: None,
            temp_round_file,
            closing: false,
            close_acks: Vec::new(),
        }
    }

    fn round_file(&self) -> PathBuf {
        self.spec
            .round_file
            .clone()
            .unwrap_or_else(|| self.temp_round_file.clone())
    }

    /// Identity of the process: anything that is fixed at spawn or in the
    /// thread config. A change restarts the app-server.
    fn key(&self) -> String {
        let spec = &self.spec;
        let fingerprint = format!(
            "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
            self.config.program,
            self.config.args,
            self.config.home,
            self.config.mcp_program,
            spec.workspace,
            spec.tools_allow,
            spec.harness,
            self.round_file(),
        );
        let digest = Sha256::digest(fingerprint.as_bytes());
        let hex: String = digest.iter().take(6).map(|b| format!("{b:02x}")).collect();
        format!("{}#{hex}", spec.session_key)
    }

    fn run(&mut self) {
        loop {
            if self.closing && self.active.is_none() {
                return;
            }
            let poll = if self.active.is_some() {
                ACTIVE_POLL
            } else {
                IDLE_POLL
            };
            match self.commands.recv_timeout(poll) {
                Ok(command) => self.handle_command(command),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    if self.active.is_none() {
                        return;
                    }
                    // Executor dropped mid-turn: stop now.
                    self.abort_active("interrupted", "Mathscape closed the session");
                    return;
                }
            }
            while let Ok(command) = self.commands.try_recv() {
                self.handle_command(command);
            }
            self.pump(Duration::ZERO);
            self.tick_active();
            if self.active.is_none() && self.sup.reap_idle() {
                self.conn = Conn::default();
            }
        }
    }

    fn shutdown(&mut self) {
        if self.active.is_some() {
            self.abort_active("interrupted", "Mathscape closed the session");
        }
        self.sup.shutdown();
        self.conn = Conn::default();
        if self.spec.round_file.is_none() {
            let _ = fs::remove_file(&self.temp_round_file);
        }
        // Answer anything still queued so callers never hang.
        while let Ok(command) = self.commands.try_recv() {
            self.refuse(command);
        }
        for ack in self.close_acks.drain(..) {
            let _ = ack.send(());
        }
    }

    fn refuse(&mut self, command: HostCommand) {
        let closed = || harness(ErrorCode::Internal, "codex session is closed");
        match command {
            HostCommand::NewThread(reply) | HostCommand::LoadThread(_, reply) => {
                let _ = reply.send(Err(closed()));
            }
            HostCommand::Prompt(job) => {
                job.bus.finish();
                let _ = job.reply.send(Err(closed()));
            }
            HostCommand::StatePath(thread, reply) => {
                let _ = reply.send(self.state_path(&thread));
            }
            HostCommand::Close(Some(ack)) => self.close_acks.push(ack),
            HostCommand::Close(None) | HostCommand::Update(_) | HostCommand::Cancel(_) => {}
        }
    }

    fn handle_command(&mut self, command: HostCommand) {
        if self.closing {
            self.refuse(command);
            return;
        }
        match command {
            HostCommand::Update(spec) => {
                self.spec = *spec;
            }
            HostCommand::NewThread(reply) => {
                let result = self.busy_guard().and_then(|()| self.new_thread());
                let _ = reply.send(result);
            }
            HostCommand::LoadThread(load, reply) => {
                let result = self.busy_guard().and_then(|()| self.load_thread(load));
                let _ = reply.send(result);
            }
            HostCommand::Prompt(job) => self.start_prompt(*job),
            HostCommand::Cancel(round_id) => self.cancel(&round_id),
            HostCommand::StatePath(thread, reply) => {
                let _ = reply.send(self.state_path(&thread));
            }
            HostCommand::Close(ack) => {
                self.closing = true;
                if let Some(ack) = ack {
                    self.close_acks.push(ack);
                }
                if self.active.is_some() {
                    self.abort_active("interrupted", "Mathscape closed the session");
                }
            }
        }
    }

    fn busy_guard(&self) -> Result<(), AgentError> {
        if self.active.is_some() {
            return Err(harness(
                ErrorCode::Internal,
                "a turn is still running in this codex session",
            ));
        }
        Ok(())
    }

    fn state_path(&self, thread_id: &str) -> Option<PathBuf> {
        if let Some(path) = self.known_paths.get(thread_id)
            && path.is_file()
        {
            return Some(path.clone());
        }
        let home = super::model::codex_home(self.config.home.as_deref())?;
        find_rollout(&home, thread_id)
    }

    // ---- process ---------------------------------------------------------

    fn ensure_process(&mut self) -> Result<(), AgentError> {
        let key = self.key();
        let config = &self.config;
        let spec = &self.spec;
        let skills = skill_roots_message(SKILL_ROOTS_ID, &spec.harness);
        let process = {
            let mut process = ProcessSpec::new(config.program.clone());
            for arg in &config.args {
                process = process.arg(arg);
            }
            for (name, value) in &spec.harness.env {
                process = process.env(name, value);
            }
            if let Some(home) = &config.home {
                process = process.env("CODEX_HOME", home.as_os_str());
            }
            if spec.workspace.is_dir() {
                process = process.cwd(spec.workspace.clone());
            }
            process
        };
        let acquired = self.sup.acquire(
            &key,
            move || Ok(process),
            move |child, timeout| handshake(child, timeout, skills),
        );
        let pid = match acquired {
            Ok(child) => child.pid(),
            Err(AcquireError::Startup { reason, stderr }) => {
                let message = if stderr.trim().is_empty() {
                    reason
                } else {
                    format!("{reason}: {}", stderr.trim())
                };
                self.conn = Conn::default();
                return Err(harness(classify_codex_failure(&message), message));
            }
            Err(other) => {
                self.conn = Conn::default();
                return Err(harness(ErrorCode::BackendUnavailable, other.to_string()));
            }
        };
        if self.conn.pid != Some(pid) {
            self.conn = Conn {
                pid: Some(pid),
                next_id: FIRST_REQUEST_ID,
                ..Conn::default()
            };
        }
        Ok(())
    }

    fn stderr_suffix(&mut self) -> String {
        let tail = self
            .sup
            .child_mut()
            .map(|child| child.stderr_tail())
            .unwrap_or_default();
        let tail = tail.trim();
        if tail.is_empty() {
            String::new()
        } else {
            format!(": {tail}")
        }
    }

    /// The process failed (stream closed, write error, timeout): kill the
    /// group, reset per-process state.
    fn process_failed(&mut self, reason: &str) {
        self.sup.report_failure(reason.to_owned());
        self.conn = Conn::default();
    }

    fn write(&mut self, value: &Value) -> Result<(), AgentError> {
        let Some(child) = self.sup.child_mut() else {
            return Err(harness(
                ErrorCode::BackendUnavailable,
                "codex app-server is not running",
            ));
        };
        if let Err(error) = child.write_line(&value.to_string()) {
            let message = format!("failed to write to codex app-server: {error}");
            self.process_failed(&message);
            return Err(harness(ErrorCode::BackendUnavailable, message));
        }
        Ok(())
    }

    fn next_id(&mut self) -> u64 {
        let id = self.conn.next_id.max(FIRST_REQUEST_ID);
        self.conn.next_id = id + 1;
        id
    }

    /// Synchronous request; other lines that arrive meanwhile are dispatched.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, AgentError> {
        let id = self.next_id();
        self.write(&json!({ "method": method, "id": id, "params": params }))?;
        let deadline = Instant::now() + self.config.handshake_timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                let message = format!(
                    "timed out waiting for codex app-server ({method}){}",
                    self.stderr_suffix()
                );
                self.process_failed(&message);
                return Err(harness(ErrorCode::BackendUnavailable, message));
            }
            let Some(child) = self.sup.child_mut() else {
                return Err(harness(
                    ErrorCode::BackendUnavailable,
                    format!("codex app-server stopped during {method}"),
                ));
            };
            match child.read_line(remaining) {
                Ok(line) => match parse_incoming(&line) {
                    Incoming::Response(message) if response_id(&message) == Some(id) => {
                        if let Some(text) = rpc_error_message(&message) {
                            return Err(harness(
                                classify_codex_failure(&text),
                                format!("{method}: {text}"),
                            ));
                        }
                        return Ok(message.get("result").cloned().unwrap_or(Value::Null));
                    }
                    incoming => self.dispatch(incoming),
                },
                Err(ReadLineError::Timeout) => {}
                Err(error) => {
                    let message = format!(
                        "codex app-server closed the event stream during {method} ({error}){}",
                        self.stderr_suffix()
                    );
                    self.process_failed(&message);
                    return Err(harness(classify_codex_failure(&message), message));
                }
            }
        }
    }

    /// Dispatch every line that is already waiting (or within `wait`).
    fn pump(&mut self, wait: Duration) {
        let mut wait = wait;
        loop {
            let Some(child) = self.sup.child_mut() else {
                return;
            };
            match child.read_line(wait) {
                Ok(line) => {
                    let incoming = parse_incoming(&line);
                    self.dispatch(incoming);
                    wait = Duration::ZERO;
                }
                Err(ReadLineError::Timeout) => return,
                Err(error) => {
                    let message = format!(
                        "codex app-server closed the event stream ({error}){}",
                        self.stderr_suffix()
                    );
                    if self.active.is_some() {
                        self.abort_active_with(
                            "failed",
                            &message,
                            classify_codex_failure(&message),
                        );
                    }
                    self.process_failed(&message);
                    return;
                }
            }
        }
    }

    // ---- routing ---------------------------------------------------------

    fn dispatch(&mut self, incoming: Incoming) {
        match incoming {
            Incoming::Empty | Incoming::Foreign => {}
            Incoming::BadJson(message) => {
                if let Some(active) = self.active.as_mut() {
                    let events = active
                        .mapper
                        .note_protocol(format!("ignored invalid app-server json: {message}"));
                    self.publish(events);
                }
            }
            Incoming::Notification(message) => self.on_notification(message),
            Incoming::Request { id, method, params } => self.on_request(id, &method, params),
            Incoming::Response(message) => self.on_response(message),
        }
    }

    fn on_notification(&mut self, message: Value) {
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let thread = message
            .pointer("/params/threadId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Some(thread_id) = mcp_ready_thread(&message) {
            self.request_inventory(&thread_id);
        }
        let to_active = self.active.as_ref().is_some_and(|active| {
            thread
                .as_deref()
                .is_none_or(|thread| thread == active.thread_id)
        });
        if method == "mcpServer/startupStatus/updated" {
            let Some(thread) = thread else {
                return;
            };
            let entry = self.conn.threads.entry(thread).or_default();
            if to_active && entry.mcp_shown {
                // fall through to the active mapper
            } else if !entry.mcp_shown {
                entry.mcp_notes.push(message);
                return;
            } else {
                return;
            }
        }
        if !to_active {
            return;
        }
        if let Some(active) = self.active.as_mut() {
            let events = active.mapper.on_notification(&message);
            self.publish(events);
        }
    }

    fn request_inventory(&mut self, thread_id: &str) {
        let entry = self.conn.threads.entry(thread_id.to_owned()).or_default();
        if entry.list_requested {
            return;
        }
        entry.list_requested = true;
        let id = self.next_id();
        self.conn.inventory_rpc.insert(id, thread_id.to_owned());
        if self.write(&mcp_status_list_message(id, thread_id)).is_err() {
            self.conn.inventory_rpc.remove(&id);
            self.deliver_inventory(thread_id, Value::Null);
        }
    }

    fn deliver_inventory(&mut self, thread_id: &str, message: Value) {
        let shown = self
            .conn
            .threads
            .get(thread_id)
            .is_some_and(|thread| thread.mcp_shown);
        let active_here = self
            .active
            .as_ref()
            .is_some_and(|active| active.thread_id == thread_id);
        if shown && active_here {
            let tools = self.spec.tools_allow.clone();
            if let Some(active) = self.active.as_mut() {
                let events = active.mapper.on_mcp_inventory(&message, &tools);
                self.publish(events);
            }
        } else if !shown {
            self.conn
                .threads
                .entry(thread_id.to_owned())
                .or_default()
                .inventory = Some(message);
        }
    }

    fn on_response(&mut self, message: Value) {
        let Some(id) = response_id(&message) else {
            return;
        };
        if let Some(thread) = self.conn.inventory_rpc.remove(&id) {
            self.deliver_inventory(&thread, message);
            return;
        }
        let interrupt_failed = self
            .active
            .as_ref()
            .is_some_and(|active| active.interrupt_rpc == Some(id));
        if interrupt_failed && let Some(text) = rpc_error_message(&message) {
            self.abort_active_with(
                "interrupted",
                &format!("turn/interrupt: {text}"),
                classify_codex_failure(&text),
            );
        }
    }

    fn on_request(&mut self, id: Value, method: &str, params: Value) {
        let Some(active) = self.active.as_ref() else {
            // No turn to attribute it to: decline.
            let reply = timeout_plan(method);
            let _ = self.write(&json!({ "id": id, "result": reply.result }));
            return;
        };
        let decision = self.approvals.decide(&ApprovalRequest {
            round_id: &active.round_id,
            method,
            params: &params,
            tools_allow: &self.spec.tools_allow,
        });
        let plan = match decision {
            ApprovalDecision::Decline => ApprovalPlan::Respond(timeout_plan(method)),
            ApprovalDecision::Policy => approval_plan(method, &params, &self.spec.tools_allow),
        };
        match plan {
            ApprovalPlan::Respond(reply) => self.send_reply(&id, method, &params, reply),
            ApprovalPlan::Hold { note } => {
                let deadline = Instant::now() + self.config.approval_timeout;
                if let Some(active) = self.active.as_mut() {
                    let events = active.mapper.note_approval(
                        note.message,
                        approval_event_raw(note.phase, method, &id, &params),
                    );
                    active.pending.push(PendingApproval {
                        id,
                        method: method.to_owned(),
                        params,
                        deadline,
                    });
                    self.publish(events);
                }
            }
        }
    }

    fn send_reply(&mut self, id: &Value, method: &str, params: &Value, reply: ApprovalReply) {
        if let Some(note) = &reply.note
            && let Some(active) = self.active.as_mut()
        {
            let events = active.mapper.note_approval(
                note.message.clone(),
                approval_event_raw(note.phase, method, id, params),
            );
            self.publish(events);
        }
        if let Err(error) = self.write(&json!({ "id": id, "result": reply.result })) {
            self.abort_active_with("failed", &error_text(&error), error_code(&error));
        }
    }

    // ---- turns -----------------------------------------------------------

    /// Publishes mapped events. Before a `turn_end` goes out the agent's
    /// final text is recorded on the bus so `final_text_ref` resolves; after
    /// the turn closes the bus is finished.
    fn publish(&mut self, events: Vec<AgentEvent>) {
        let Some(active) = self.active.as_ref() else {
            return;
        };
        if events
            .iter()
            .any(|event| matches!(event.payload, crate::EventPayload::TurnEnd(_)))
        {
            active
                .bus
                .set_final_text(active.mapper.agent_text().map(str::to_owned));
        }
        active.bus.publish_all(events);
        if active.mapper.turn_closed() {
            self.finish_active();
        }
    }

    fn finish_active(&mut self) {
        let Some(active) = self.active.take() else {
            return;
        };
        active.bus.finish();
        // Unanswered approvals of a finished turn are declined.
        for pending in active.pending {
            let reply = timeout_plan(&pending.method);
            let _ = self.write(&json!({ "id": pending.id, "result": reply.result }));
        }
        self.sup.report_success();
        if self.spec.ephemeral {
            self.closing = true;
        }
    }

    fn abort_active(&mut self, status: &str, message: &str) {
        self.abort_active_with(status, message, ErrorCode::Internal);
    }

    fn abort_active_with(&mut self, status: &str, message: &str, code: ErrorCode) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        let events = active.mapper.force_stop(status, message, code);
        self.publish(events);
        // force_stop always closes the turn; be defensive anyway.
        self.finish_active();
    }

    fn tick_active(&mut self) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        let now = Instant::now();
        let mut due = Vec::new();
        let mut waiting = Vec::new();
        for pending in active.pending.drain(..) {
            if now >= pending.deadline {
                due.push(pending);
            } else {
                waiting.push(pending);
            }
        }
        active.pending = waiting;
        let interrupt_expired = active
            .interrupt_deadline
            .is_some_and(|deadline| now >= deadline);
        for pending in due {
            let reply = timeout_plan(&pending.method);
            self.send_reply(&pending.id, &pending.method, &pending.params, reply);
        }
        if interrupt_expired && self.active.is_some() {
            self.abort_active(
                "interrupted",
                "turn/interrupt was not acknowledged before the grace period",
            );
            // The turn may still be running inside Codex: stop the process.
            self.process_failed("turn/interrupt timed out");
        }
        if self.active.is_some() {
            self.sup.touch();
        }
    }

    fn cancel(&mut self, round_id: &RoundId) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        if &active.round_id != round_id || active.mapper.turn_closed() {
            return;
        }
        let events = active.mapper.on_cancel_request();
        let thread_id = active.thread_id.clone();
        let turn_id = active.turn_id.clone().unwrap_or_else(|| "turn-1".into());
        let already = active.interrupt_rpc.is_some();
        self.publish(events);
        if already {
            return;
        }
        let id = self.next_id();
        let grace = self.config.interrupt_grace;
        if let Some(active) = self.active.as_mut() {
            active.interrupt_rpc = Some(id);
            active.interrupt_deadline = Some(Instant::now() + grace);
        }
        if let Err(error) = self.write(&json!({
            "method": "turn/interrupt",
            "id": id,
            "params": { "threadId": thread_id, "turnId": turn_id }
        })) {
            self.abort_active_with("interrupted", &error_text(&error), ErrorCode::Internal);
        }
    }

    fn start_prompt(&mut self, job: PromptJob) {
        let PromptJob {
            prompt,
            round_id,
            bus,
            reply,
        } = job;
        if let Err(error) = self.busy_guard() {
            bus.finish();
            let _ = reply.send(Err(error));
            return;
        }
        if let Err(error) = self.prepare_thread(&prompt.thread_id) {
            bus.finish();
            let _ = reply.send(Err(error));
            return;
        }
        let round_file = self.round_file();
        if let Err(error) = write_round_file(&round_file, &round_id) {
            bus.finish();
            let _ = reply.send(Err(harness(
                ErrorCode::Internal,
                format!("failed to write {}: {error}", round_file.display()),
            )));
            return;
        }
        let mut context = prompt.context.clone();
        if !self.spec.ephemeral {
            context.push(ContextNote {
                key: "mathscape.round".into(),
                text: format!("round_id: {round_id}"),
            });
        }
        let mut inputs = Vec::new();
        if let Some(text) = turn_context_text(&context) {
            inputs.push(text);
        }
        inputs.push(prompt.text.clone());
        let mut mapper = CodexEventMapper::new(round_id.clone(), prompt.text.clone());
        mapper.note_thread_id(&prompt.thread_id);
        self.active = Some(ActiveTurn {
            round_id: round_id.clone(),
            thread_id: prompt.thread_id.clone(),
            turn_id: None,
            mapper,
            bus: Arc::clone(&bus),
            pending: Vec::new(),
            interrupt_rpc: None,
            interrupt_deadline: None,
        });
        self.replay_mcp(&prompt.thread_id);
        let home = self.config.home.clone();
        let model = super::model::current_model(home.as_deref());
        let effort = super::model::effective_effort(
            super::model::configured_effort(home.as_deref()).as_deref(),
            model.as_deref(),
            &super::model::cached_catalog(home.as_deref()),
        );
        let cwd = absolute_path(&self.spec.workspace).display().to_string();
        let params = turn_start_message(
            0,
            &prompt.thread_id,
            &inputs,
            &cwd,
            model.as_deref(),
            effort.as_deref(),
        )["params"]
            .clone();
        match self.call("turn/start", params) {
            Ok(result) => {
                let turn_id = result
                    .pointer("/turn/id")
                    .and_then(Value::as_str)
                    .unwrap_or("turn-1")
                    .to_owned();
                if let Some(active) = self.active.as_mut() {
                    active.turn_id = Some(turn_id.clone());
                    let events = active.mapper.accept_turn(&turn_id, &result);
                    self.publish(events);
                }
                let _ = reply.send(Ok(()));
            }
            Err(error) => {
                // Nothing reached the caller yet: drop the turn silently and
                // report the error from `prompt`.
                if let Some(active) = self.active.take() {
                    active.bus.finish();
                }
                let _ = reply.send(Err(error));
                if self.spec.ephemeral {
                    self.closing = true;
                }
            }
        }
    }

    /// Feed MCP startup seen before the thread's first turn into this
    /// turn's mapper (the step shows "MCP 工具 n/n" once per thread).
    fn replay_mcp(&mut self, thread_id: &str) {
        let Some(entry) = self.conn.threads.get_mut(thread_id) else {
            return;
        };
        if entry.mcp_shown {
            return;
        }
        entry.mcp_shown = true;
        let notes = std::mem::take(&mut entry.mcp_notes);
        let inventory = entry.inventory.take();
        let tools = self.spec.tools_allow.clone();
        let mut events = Vec::new();
        if let Some(active) = self.active.as_mut() {
            for note in &notes {
                events.extend(active.mapper.on_notification(note));
            }
            if let Some(inventory) = &inventory {
                events.extend(active.mapper.on_mcp_inventory(inventory, &tools));
            }
        }
        self.publish(events);
    }

    // ---- threads ---------------------------------------------------------

    /// Make `thread_id` loaded in the running process (resume by id if the
    /// process restarted since).
    fn prepare_thread(&mut self, thread_id: &str) -> Result<(), AgentError> {
        self.ensure_process()?;
        if self.conn.threads.contains_key(thread_id) {
            return Ok(());
        }
        self.resume(thread_id).map(|_| ())
    }

    fn new_thread(&mut self) -> Result<ThreadBinding, AgentError> {
        self.ensure_process()?;
        let model = super::model::current_model(self.config.home.as_deref());
        let params = thread_start_message(
            0,
            &self.spec,
            &self.config.mcp_program,
            &self.round_file(),
            model.as_deref(),
        )["params"]
            .clone();
        let result = self.call("thread/start", params)?;
        let thread_id = result
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .ok_or_else(|| harness(ErrorCode::Protocol, "thread/start result has no thread.id"))?
            .to_owned();
        self.conn.threads.entry(thread_id.clone()).or_default();
        let state_path = thread_path(&result);
        if let Some(path) = &state_path {
            self.known_paths.insert(thread_id.clone(), path.clone());
        }
        Ok(ThreadBinding {
            thread_id,
            continuity: ThreadContinuity::Started,
            state_path,
        })
    }

    fn resume(&mut self, thread_id: &str) -> Result<Option<PathBuf>, AgentError> {
        let model = super::model::current_model(self.config.home.as_deref());
        let params = thread_resume_message(
            0,
            thread_id,
            &self.spec,
            &self.config.mcp_program,
            &self.round_file(),
            model.as_deref(),
        )["params"]
            .clone();
        let result = self.call("thread/resume", params)?;
        self.conn.threads.entry(thread_id.to_owned()).or_default();
        let path = thread_path(&result);
        if let Some(path) = &path {
            self.known_paths.insert(thread_id.to_owned(), path.clone());
        }
        Ok(path)
    }

    /// Live → resume by id → restore the caller's rollout copy into
    /// `$CODEX_HOME/sessions/mathscape/` and resume again.
    fn load_thread(&mut self, load: LoadThread) -> Result<ThreadBinding, AgentError> {
        self.ensure_process()?;
        let thread_id = load.thread_id;
        if self.conn.threads.contains_key(&thread_id) {
            return Ok(ThreadBinding {
                state_path: self.state_path(&thread_id),
                thread_id,
                continuity: ThreadContinuity::Live,
            });
        }
        let first = match self.resume(&thread_id) {
            Ok(path) => {
                return Ok(ThreadBinding {
                    state_path: path.or_else(|| self.state_path(&thread_id)),
                    thread_id,
                    continuity: ThreadContinuity::Resumed,
                });
            }
            Err(error) => error,
        };
        let Some(copy) = load.state_copy.filter(|copy| copy.is_file()) else {
            return Err(first);
        };
        let Some(home) = super::model::codex_home(self.config.home.as_deref()) else {
            return Err(first);
        };
        if find_rollout(&home, &thread_id).is_some() {
            // Codex has the rollout and still refused: restoring changes nothing.
            return Err(first);
        }
        restore_rollout(&home, &thread_id, &copy).map_err(|error| {
            harness(
                ErrorCode::Internal,
                format!("failed to restore rollout {}: {error}", copy.display()),
            )
        })?;
        self.ensure_process()?;
        let path = self.resume(&thread_id)?;
        Ok(ThreadBinding {
            state_path: path.or_else(|| self.state_path(&thread_id)),
            thread_id,
            continuity: ThreadContinuity::Restored,
        })
    }
}

fn thread_path(result: &Value) -> Option<PathBuf> {
    result
        .pointer("/thread/path")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

fn write_round_file(path: &std::path::Path, round_id: &RoundId) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let partial = path.with_extension("round-partial");
    fs::write(&partial, format!("{round_id}\n"))?;
    fs::rename(&partial, path)
}

fn sanitize(key: &str) -> String {
    key.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect()
}

/// `initialize` / `initialized` (+ `skills/extraRoots/set`). Never opts into
/// the experimental API.
fn handshake(
    child: &mut ResidentChild,
    timeout: Duration,
    skills: Option<Value>,
) -> Result<(), String> {
    let send = |child: &mut ResidentChild, value: &Value| {
        child
            .write_line(&value.to_string())
            .map_err(|error| format!("failed to write to codex app-server: {error}"))
    };
    send(child, &initialize_message(INITIALIZE_ID))?;
    send(child, &json!({ "method": "initialized", "params": {} }))?;
    let mut wanted = vec![INITIALIZE_ID];
    if let Some(message) = &skills {
        send(child, message)?;
        wanted.push(SKILL_ROOTS_ID);
    }
    let got: Mutex<HashMap<u64, Value>> = Mutex::new(HashMap::new());
    child
        .read_until(timeout, |line| {
            if let Incoming::Response(message) = parse_incoming(line)
                && let Some(id) = response_id(&message)
                && wanted.contains(&id)
            {
                let mut got = got.lock().expect("handshake");
                got.insert(id, message);
                return got.len() == wanted.len();
            }
            false
        })
        .map_err(|error| match error {
            ReadLineError::Timeout => "timed out waiting for codex app-server".to_owned(),
            other => format!("codex app-server closed stdout before the turn started ({other})"),
        })?;
    let got = got.into_inner().expect("handshake");
    if let Some(message) = got.get(&INITIALIZE_ID).and_then(rpc_error_message) {
        return Err(format!("initialize: {message}"));
    }
    // Not fatal: developerInstructions still name the skill's stable path.
    if let Some(message) = got.get(&SKILL_ROOTS_ID).and_then(rpc_error_message) {
        eprintln!("mathscape: skills/extraRoots/set failed: {message}");
    }
    Ok(())
}
