//! Spawn `codex app-server` and drive one round over stdio JSON-RPC.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures::StreamExt;
use futures::channel::mpsc as futures_mpsc;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use serde_json::{Value, json};

use super::map::{CodexEventMapper, classify_codex_failure};
use crate::{
    AgentBackend, AgentError, AgentEvent, AgentExecutor, Capability, CapabilityProbe,
    CommandRunner, ErrorCode, RoundId, StartRoundRequest, SystemCommandRunner, probe_program,
};

const INITIALIZE_ID: u64 = 1;
const THREAD_START_ID: u64 = 2;
const TURN_START_ID: u64 = 3;
const STDERR_TAIL: usize = 4 * 1024;
/// Upper bound between SIGTERM and SIGKILL for the app-server process group.
const TERM_GRACE: Duration = Duration::from_secs(2);
/// Upper bound for joining the stderr reader after the group is dead. A
/// process that escaped the group (e.g. `setsid`) may still hold the pipe;
/// the reader is then detached instead of blocking the session thread.
const READER_JOIN: Duration = Duration::from_secs(2);

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
    /// Bound for `initialize` + `thread/start` + `turn/start` acceptance.
    /// The model run itself is not capped here.
    pub handshake_timeout: Duration,
    /// After `turn/interrupt`, how long to wait before killing the child.
    pub interrupt_grace: Duration,
    /// Override for Codex home (`config.toml` / `models_cache.json`).
    /// `None` → `$CODEX_HOME` or `~/.codex`. Tests pass a temp dir.
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
            home: None,
        }
    }
}

struct Shared {
    config: CodexConfig,
    runner: Arc<dyn CommandRunner>,
    rounds: Mutex<HashMap<RoundId, RoundSlot>>,
    next_round: AtomicU64,
}

struct RoundSlot {
    bus: Arc<EventBus>,
    cancel: Sender<SessionCommand>,
}

enum SessionCommand {
    Cancel,
}

/// Local Codex backend. One app-server process per round.
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
                rounds: Mutex::new(HashMap::new()),
                next_round: AtomicU64::new(0),
            }),
        }
    }
}

impl AgentExecutor for CodexExecutor {
    fn backend(&self) -> AgentBackend {
        AgentBackend::Codex
    }

    fn start_round(
        &self,
        request: StartRoundRequest,
    ) -> BoxFuture<'_, Result<RoundId, AgentError>> {
        let shared = Arc::clone(&self.shared);
        Box::pin(async move { start_round(shared, request).await })
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
                    let _ = slot.cancel.send(SessionCommand::Cancel);
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
        super::model::set_model(self.shared.config.home.as_deref(), id)
    }

    fn probe(&self) -> Vec<CapabilityProbe> {
        vec![probe_program(
            self.shared.runner.as_ref(),
            Capability::Codex,
            &self.shared.config.program,
        )]
    }
}

async fn start_round(
    shared: Arc<Shared>,
    request: StartRoundRequest,
) -> Result<RoundId, AgentError> {
    if request.backend != AgentBackend::Codex {
        return Err(AgentError::BackendMismatch {
            served: AgentBackend::Codex,
            requested: request.backend,
        });
    }
    let probe = probe_program(
        shared.runner.as_ref(),
        Capability::Codex,
        &shared.config.program,
    );
    if !probe.is_available() {
        return Err(AgentError::CapabilityUnavailable(probe));
    }
    let n = shared.next_round.fetch_add(1, Ordering::Relaxed) + 1;
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let round_id =
        RoundId::parse(format!("codex-{millis}-{n}")).expect("round id has no whitespace");
    let bus = Arc::new(EventBus::new());
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (ready_tx, ready_rx) = futures::channel::oneshot::channel();
    let config = shared.config.clone();
    let bus_thread = Arc::clone(&bus);
    let round_for_thread = round_id.clone();
    thread::Builder::new()
        .name("codex-app-server".into())
        .spawn(move || {
            drive_child(
                config,
                request,
                round_for_thread,
                bus_thread,
                cmd_rx,
                Ready::new(ready_tx),
            );
        })
        .map_err(|error| {
            harness(
                ErrorCode::Internal,
                format!("failed to spawn session thread: {error}"),
            )
        })?;
    match ready_rx.await {
        Ok(Ok(())) => {
            shared.rounds.lock().expect("codex rounds").insert(
                round_id.clone(),
                RoundSlot {
                    bus,
                    cancel: cmd_tx,
                },
            );
            Ok(round_id)
        }
        Ok(Err(error)) => Err(error),
        Err(_) => Err(harness(
            ErrorCode::Internal,
            "codex app-server session ended before the turn started",
        )),
    }
}

fn drive_child(
    config: CodexConfig,
    request: StartRoundRequest,
    round_id: RoundId,
    bus: Arc<EventBus>,
    commands: Receiver<SessionCommand>,
    mut ready: Ready,
) {
    let _finish = FinishBus(Arc::clone(&bus));
    let mut child = match spawn_child(&config, &request.workspace) {
        Ok(child) => child,
        Err(error) => {
            ready.fail(error);
            return;
        }
    };
    let grace = TERM_GRACE.min(config.interrupt_grace);
    let Some(stdin) = child.stdin.take() else {
        terminate_group(&mut child, grace);
        ready.fail(harness(ErrorCode::Internal, "codex stdin was not piped"));
        return;
    };
    let Some(stdout) = child.stdout.take() else {
        drop(stdin);
        terminate_group(&mut child, grace);
        ready.fail(harness(ErrorCode::Internal, "codex stdout was not piped"));
        return;
    };
    let Some(stderr) = child.stderr.take() else {
        drop(stdin);
        terminate_group(&mut child, grace);
        ready.fail(harness(ErrorCode::Internal, "codex stderr was not piped"));
        return;
    };
    let stderr_tail = Arc::new(Mutex::new(String::new()));
    let tail_for_thread = Arc::clone(&stderr_tail);
    let (stderr_done_tx, stderr_done) = mpsc::channel::<()>();
    thread::spawn(move || {
        collect_stderr(stderr, tail_for_thread);
        let _ = stderr_done_tx.send(());
    });
    let (line_tx, line_rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(line) => {
                    if line_tx.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    let mut session = Session {
        writer: Some(Box::new(stdin)),
        lines: line_rx,
        commands,
        mapper: CodexEventMapper::new(round_id.clone(), request.input_ref.clone()),
        bus,
        handshake_timeout: config.handshake_timeout,
        interrupt_grace: config.interrupt_grace,
        stderr_tail,
        next_id: TURN_START_ID,
        thread_id: None,
        turn_id: None,
        interrupt_rpc: None,
        child,
        term_grace: grace,
        reaped: false,
        mcp_program: config.mcp_program.clone(),
        model: super::model::current_model(config.home.as_deref()),
    };
    if let Err(error) = session.handshake(&request, &round_id, &ready) {
        // Kill the whole process group first, then unblock `start_round`.
        // Joining the stderr reader must never gate `ready`.
        session.shutdown();
        ready.fail(error);
        let _ = stderr_done.recv_timeout(READER_JOIN);
        return;
    }
    ready.succeed();
    session.run_until_closed();
    session.shutdown();
    // Pipes close once the process group is dead; bounded in case a process
    // escaped the group and still holds stderr.
    let _ = stderr_done.recv_timeout(READER_JOIN);
}

struct Session {
    /// `None` after [`Session::shutdown`]: dropping it gives the app-server
    /// (and any grandchild that inherited the pipe) EOF on stdin.
    writer: Option<Box<dyn Write + Send>>,
    lines: Receiver<String>,
    commands: Receiver<SessionCommand>,
    mapper: CodexEventMapper,
    bus: Arc<EventBus>,
    handshake_timeout: Duration,
    interrupt_grace: Duration,
    stderr_tail: Arc<Mutex<String>>,
    next_id: u64,
    thread_id: Option<String>,
    turn_id: Option<String>,
    interrupt_rpc: Option<u64>,
    /// Process-group leader (`process_group(0)` on Unix).
    child: Child,
    term_grace: Duration,
    reaped: bool,
    mcp_program: PathBuf,
    /// Top-level Codex default at handshake time (forwarded on thread/start).
    model: Option<String>,
}

impl Session {
    /// Publishes mapped events. Before a `turn_end` goes out, the agent's
    /// final message text is recorded on the bus so a consumer that sees
    /// `turn_end.final_text_ref` can immediately resolve it with
    /// [`AgentExecutor::final_text`].
    fn publish(&self, events: Vec<AgentEvent>) {
        if events
            .iter()
            .any(|event| matches!(event.payload, crate::EventPayload::TurnEnd(_)))
        {
            self.bus
                .set_final_text(self.mapper.agent_text().map(str::to_owned));
        }
        self.bus.publish_all(events);
    }

    fn handshake(
        &mut self,
        request: &StartRoundRequest,
        round_id: &RoundId,
        ready: &Ready,
    ) -> Result<(), AgentError> {
        let deadline = Instant::now() + self.handshake_timeout;
        self.write_value(&initialize_message())?;
        self.write_value(&json!({"method": "initialized", "params": {}}))?;
        self.write_value(&thread_start_message(
            THREAD_START_ID,
            request,
            round_id,
            &self.mcp_program,
            self.model.as_deref(),
        ))?;
        let opened = self.collect_responses(&[INITIALIZE_ID, THREAD_START_ID], deadline, ready)?;
        if let Some(message) = rpc_error_message(&opened[&INITIALIZE_ID]) {
            return Err(harness(
                classify_codex_failure(&message),
                format!("initialize: {message}"),
            ));
        }
        if let Some(message) = rpc_error_message(&opened[&THREAD_START_ID]) {
            return Err(harness(
                classify_codex_failure(&message),
                format!("thread/start: {message}"),
            ));
        }
        let thread_result = opened[&THREAD_START_ID]
            .get("result")
            .cloned()
            .unwrap_or(Value::Null);
        let thread_id = thread_result
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .ok_or_else(|| harness(ErrorCode::Protocol, "thread/start result has no thread.id"))?;
        self.thread_id = Some(thread_id.to_owned());
        self.mapper.note_thread_id(thread_id);
        let cwd = absolute_path(&request.workspace);
        self.write_value(&turn_start_message(
            TURN_START_ID,
            thread_id,
            &request.input_ref,
            &cwd.display().to_string(),
        ))?;
        let turn = self.collect_responses(&[TURN_START_ID], deadline, ready)?;
        let turn = &turn[&TURN_START_ID];
        if let Some(message) = rpc_error_message(turn) {
            return Err(harness(
                classify_codex_failure(&message),
                format!("turn/start: {message}"),
            ));
        }
        let result = turn.get("result").cloned().unwrap_or(Value::Null);
        let turn_id = result
            .pointer("/turn/id")
            .and_then(Value::as_str)
            .unwrap_or("turn-1")
            .to_owned();
        self.turn_id = Some(turn_id.clone());
        let events = self.mapper.accept_turn(&turn_id, &result);
        self.publish(events);
        Ok(())
    }

    fn run_until_closed(&mut self) {
        let mut interrupt_deadline: Option<Instant> = None;
        while !self.mapper.turn_closed() {
            if interrupt_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                let events = self.mapper.force_stop(
                    "interrupted",
                    "turn/interrupt was not acknowledged before the grace period",
                    ErrorCode::Internal,
                );
                self.publish(events);
                break;
            }
            match self.lines.recv_timeout(Duration::from_millis(50)) {
                Ok(line) => {
                    if let Err(error) = self.handle_runtime_line(&line) {
                        let events = self.mapper.force_stop(
                            "failed",
                            &error_text(&error),
                            error_code(&error),
                        );
                        self.publish(events);
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    let message = format!(
                        "codex app-server closed the event stream{}",
                        self.tail_suffix()
                    );
                    let events = self.mapper.force_stop(
                        "failed",
                        &message,
                        classify_codex_failure(&message),
                    );
                    self.publish(events);
                    break;
                }
            }
            while let Ok(command) = self.commands.try_recv() {
                if !matches!(command, SessionCommand::Cancel) || self.mapper.turn_closed() {
                    continue;
                }
                let events = self.mapper.on_cancel_request();
                self.publish(events);
                if let Err(error) = self.write_interrupt() {
                    let events = self.mapper.force_stop(
                        "interrupted",
                        &error_text(&error),
                        ErrorCode::Internal,
                    );
                    self.publish(events);
                    break;
                }
                interrupt_deadline.get_or_insert_with(|| Instant::now() + self.interrupt_grace);
            }
        }
    }

    fn handle_runtime_line(&mut self, line: &str) -> Result<(), AgentError> {
        match parse_incoming(line) {
            Incoming::Empty | Incoming::Foreign => Ok(()),
            Incoming::BadJson(message) => {
                let events = self
                    .mapper
                    .note_protocol(format!("ignored invalid app-server json: {message}"));
                self.publish(events);
                Ok(())
            }
            Incoming::Notification(message) => {
                let events = self.mapper.on_notification(&message);
                self.publish(events);
                Ok(())
            }
            Incoming::Request { id, method, params } => self.answer(&id, &method, &params),
            Incoming::Response(message) => {
                let id = response_id(&message);
                if id == self.interrupt_rpc
                    && let Some(text) = rpc_error_message(&message)
                {
                    let events = self.mapper.force_stop(
                        "interrupted",
                        &format!("turn/interrupt: {text}"),
                        classify_codex_failure(&text),
                    );
                    self.publish(events);
                }
                Ok(())
            }
        }
    }

    fn collect_responses(
        &mut self,
        wanted: &[u64],
        deadline: Instant,
        ready: &Ready,
    ) -> Result<HashMap<u64, Value>, AgentError> {
        let mut got = HashMap::new();
        while got.len() < wanted.len() {
            if ready.caller_gone() {
                return Err(harness(
                    ErrorCode::Internal,
                    "start_round dropped before the turn started",
                ));
            }
            match self.next_incoming(deadline)? {
                Incoming::Empty | Incoming::Foreign => {}
                Incoming::BadJson(message) => {
                    return Err(harness(
                        ErrorCode::Protocol,
                        format!("codex app-server sent invalid json: {message}"),
                    ));
                }
                Incoming::Notification(message) => {
                    let events = self.mapper.on_notification(&message);
                    self.publish(events);
                }
                Incoming::Request { id, method, params } => self.answer(&id, &method, &params)?,
                Incoming::Response(message) => {
                    if let Some(id) = response_id(&message)
                        && wanted.contains(&id)
                    {
                        got.insert(id, message);
                    }
                }
            }
        }
        Ok(got)
    }

    fn next_incoming(&mut self, deadline: Instant) -> Result<Incoming, AgentError> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(self.timed_out());
        }
        match self.lines.recv_timeout(remaining) {
            Ok(line) => Ok(parse_incoming(&line)),
            Err(RecvTimeoutError::Timeout) => Err(self.timed_out()),
            Err(RecvTimeoutError::Disconnected) => Err(self.stream_closed()),
        }
    }

    fn answer(&mut self, id: &Value, method: &str, params: &Value) -> Result<(), AgentError> {
        if is_user_input(method) {
            let events = self.mapper.note_declined_user_input(params.clone());
            self.publish(events);
        }
        self.write_value(&json!({
            "id": id,
            "result": server_request_result(method),
        }))
    }

    fn write_interrupt(&mut self) -> Result<(), AgentError> {
        let Some(thread_id) = self.thread_id.clone() else {
            return Err(harness(ErrorCode::Protocol, "no codex thread to interrupt"));
        };
        let turn_id = self.turn_id.clone().unwrap_or_else(|| "turn-1".into());
        self.next_id += 1;
        let id = self.next_id;
        self.interrupt_rpc = Some(id);
        self.write_value(&json!({
            "method": "turn/interrupt",
            "id": id,
            "params": { "threadId": thread_id, "turnId": turn_id }
        }))
    }

    fn write_value(&mut self, value: &Value) -> Result<(), AgentError> {
        let Some(writer) = self.writer.as_mut() else {
            return Err(harness(
                ErrorCode::Internal,
                "codex app-server stdin is already closed",
            ));
        };
        serde_json::to_writer(&mut *writer, value).map_err(|error| {
            harness(
                ErrorCode::Protocol,
                format!("failed to write app-server message: {error}"),
            )
        })?;
        writer.write_all(b"\n").map_err(|error| {
            harness(
                ErrorCode::Internal,
                format!("failed to write app-server newline: {error}"),
            )
        })?;
        writer.flush().map_err(|error| {
            harness(
                ErrorCode::Internal,
                format!("failed to flush app-server: {error}"),
            )
        })
    }

    /// Idempotent. Drop stdin first, then SIGTERM the process group, wait up
    /// to `term_grace` for the leader, SIGKILL the group, and reap the leader.
    fn shutdown(&mut self) {
        if self.reaped {
            return;
        }
        drop(self.writer.take());
        terminate_group(&mut self.child, self.term_grace);
        self.reaped = true;
    }

    fn timed_out(&self) -> AgentError {
        harness(
            ErrorCode::BackendUnavailable,
            format!(
                "timed out waiting for codex app-server{}",
                self.tail_suffix()
            ),
        )
    }

    fn stream_closed(&self) -> AgentError {
        harness(
            ErrorCode::BackendUnavailable,
            format!(
                "codex app-server closed stdout before the turn started{}",
                self.tail_suffix()
            ),
        )
    }

    fn tail_suffix(&self) -> String {
        let tail = self
            .stderr_tail
            .lock()
            .expect("stderr tail")
            .trim()
            .to_owned();
        if tail.is_empty() {
            String::new()
        } else {
            format!(": {tail}")
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Terminate `child` and every process in its process group.
///
/// npm's `codex` is a node wrapper that spawns the native `codex app-server`;
/// killing only the direct child would orphan the native process (and its
/// `mathscape mcp` grandchild) while it still holds our pipes. The workspace
/// forbids `unsafe`, so signals go through the system `kill` utility.
fn terminate_group(child: &mut Child, grace: Duration) {
    #[cfg(unix)]
    {
        let group = format!("-{}", child.id());
        signal_group("TERM", &group);
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(Some(_))) {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        // Always KILL the group: grandchildren may ignore TERM or outlive the
        // leader. A no-op when the group is already gone.
        signal_group("KILL", &group);
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(unix)]
fn signal_group(signal: &str, group: &str) {
    let _ = Command::new("kill")
        .args(["-s", signal, "--", group])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn spawn_child(config: &CodexConfig, workspace: &Path) -> Result<Child, AgentError> {
    let mut command = Command::new(&config.program);
    command
        .args(&config.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if workspace.is_dir() {
        command.current_dir(workspace);
    }
    // Own process group so shutdown can signal the wrapper and everything it
    // spawned (see `terminate_group`).
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    // Inherit the environment unchanged. The Codex CLI reads its own login;
    // Mathscape must not add OPENAI_API_KEY / ChatGPT tokens, and must not
    // call account/login.
    command.spawn().map_err(|error| {
        let code = if error.kind() == std::io::ErrorKind::NotFound {
            ErrorCode::BackendUnavailable
        } else {
            ErrorCode::Internal
        };
        harness(
            code,
            format!(
                "failed to start {} {}: {error}",
                config.program.display(),
                config.args.join(" ")
            ),
        )
    })
}

pub(crate) fn thread_start_message(
    id: u64,
    request: &StartRoundRequest,
    round_id: &RoundId,
    mcp_program: &Path,
    model: Option<&str>,
) -> Value {
    let workspace = absolute_path(&request.workspace);
    let workspace = workspace.display().to_string();
    let mut params = json!({
        "cwd": workspace,
        "approvalPolicy": "never",
        "sandbox": "read-only",
        "ephemeral": true,
        "serviceName": "mathscape",
        "developerInstructions": developer_instructions(round_id, &request.tools_allow),
        "config": {
            "mcp_servers.mathscape.command": mcp_program.display().to_string(),
            "mcp_servers.mathscape.args": [
                "mcp",
                "--workspace",
                workspace,
                "--round",
                round_id.as_str()
            ],
            "mcp_servers.mathscape.enabled": true
        }
    });
    if let Some(model) = model.map(str::trim).filter(|id| !id.is_empty()) {
        params["model"] = json!(model);
    }
    json!({
        "method": "thread/start",
        "id": id,
        "params": params
    })
}

pub(crate) fn turn_start_message(id: u64, thread_id: &str, input_ref: &str, cwd: &str) -> Value {
    json!({
        "method": "turn/start",
        "id": id,
        "params": {
            "threadId": thread_id,
            "cwd": cwd,
            "input": [{ "type": "text", "text": input_ref }]
        }
    })
}

fn initialize_message() -> Value {
    json!({
        "method": "initialize",
        "id": INITIALIZE_ID,
        "params": {
            "clientInfo": {
                "name": "mathscape",
                "title": "Mathscape",
                "version": env!("CARGO_PKG_VERSION")
            }
        }
    })
}

fn developer_instructions(round_id: &RoundId, tools_allow: &[String]) -> String {
    let tools = if tools_allow.is_empty() {
        "The mathscape MCP server exposes a fixed read-only tool list; call those tools when they help.".to_owned()
    } else {
        format!(
            "Call only these mathscape tools in this round: {}.",
            tools_allow.join(", ")
        )
    };
    format!(
        "This is Mathscape round {round_id}. {tools} Mathscape does not provide model credentials; use this Codex process's existing local login. Do not ask the user for an API key."
    )
}

pub(crate) fn server_request_result(method: &str) -> Value {
    match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            json!({ "decision": "decline" })
        }
        "item/permissions/requestApproval" => json!({ "permissions": {} }),
        "mcpServer/elicitation/request" => json!({ "action": "decline", "content": null }),
        // Codex 0.163 schema: `{ "answers": { <questionId>: { "answers": [..] } } }`.
        // `answers` is an object and there is no `action` field. An empty map
        // declines every question without implementing user.ask.
        "item/tool/requestUserInput" | "tool/requestUserInput" => {
            json!({ "answers": {} })
        }
        "item/tool/call" => json!({
            "contentItems": [{
                "type": "inputText",
                "text": "Mathscape Phase1b does not execute Codex dynamic tools; use the mathscape MCP server."
            }],
            "success": false
        }),
        _ => json!({}),
    }
}

fn is_user_input(method: &str) -> bool {
    matches!(
        method,
        "item/tool/requestUserInput" | "tool/requestUserInput"
    )
}

fn absolute_path(path: &Path) -> PathBuf {
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

enum Incoming {
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

fn parse_incoming(line: &str) -> Incoming {
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

fn response_id(value: &Value) -> Option<u64> {
    value.get("id").and_then(|id| {
        id.as_u64()
            .or_else(|| id.as_i64().filter(|n| *n >= 0).map(|n| n as u64))
    })
}

fn rpc_error_message(value: &Value) -> Option<String> {
    let error = value.get("error")?;
    Some(
        error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("codex request failed")
            .to_owned(),
    )
}

fn collect_stderr(stderr: impl std::io::Read, tail: Arc<Mutex<String>>) {
    let mut reader = BufReader::new(stderr);
    let mut chunk = String::new();
    while reader.read_line(&mut chunk).unwrap_or(0) > 0 {
        let mut buffer = tail.lock().expect("stderr tail");
        buffer.push_str(&chunk);
        if buffer.len() > STDERR_TAIL {
            let mut cut = buffer.len() - STDERR_TAIL;
            while cut < buffer.len() && !buffer.is_char_boundary(cut) {
                cut += 1;
            }
            *buffer = buffer[cut..].to_owned();
        }
        chunk.clear();
    }
}

fn harness(code: ErrorCode, message: impl Into<String>) -> AgentError {
    AgentError::Harness {
        backend: AgentBackend::Codex,
        code,
        message: message.into(),
    }
}

fn error_text(error: &AgentError) -> String {
    error.to_string()
}

fn error_code(error: &AgentError) -> ErrorCode {
    match error {
        AgentError::Harness { code, .. } => *code,
        AgentError::CapabilityUnavailable(_) => ErrorCode::BackendUnavailable,
        _ => ErrorCode::Internal,
    }
}

struct Ready {
    tx: Option<futures::channel::oneshot::Sender<Result<(), AgentError>>>,
}

impl Ready {
    fn new(tx: futures::channel::oneshot::Sender<Result<(), AgentError>>) -> Self {
        Self { tx: Some(tx) }
    }

    fn succeed(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Ok(()));
        }
    }

    fn fail(&mut self, error: AgentError) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Err(error));
        }
    }

    fn caller_gone(&self) -> bool {
        self.tx
            .as_ref()
            .is_some_and(futures::channel::oneshot::Sender::is_canceled)
    }
}

impl Drop for Ready {
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Err(harness(
                ErrorCode::Internal,
                "codex app-server session ended before the turn started",
            )));
        }
    }
}

struct FinishBus(Arc<EventBus>);

impl Drop for FinishBus {
    fn drop(&mut self) {
        self.0.finish();
    }
}

struct BusInner {
    events: Vec<AgentEvent>,
    final_text: Option<String>,
    senders: Vec<futures_mpsc::UnboundedSender<AgentEvent>>,
    finished: bool,
}

struct EventBus {
    inner: Mutex<BusInner>,
}

impl EventBus {
    fn new() -> Self {
        Self {
            inner: Mutex::new(BusInner {
                events: Vec::new(),
                final_text: None,
                senders: Vec::new(),
                finished: false,
            }),
        }
    }

    fn publish_all(&self, events: Vec<AgentEvent>) {
        for event in events {
            self.publish(event);
        }
    }

    fn publish(&self, event: AgentEvent) {
        let mut guard = self.inner.lock().expect("event bus");
        if guard.finished {
            return;
        }
        guard.events.push(event.clone());
        guard
            .senders
            .retain(|sender| sender.unbounded_send(event.clone()).is_ok());
    }

    fn set_final_text(&self, text: Option<String>) {
        self.inner.lock().expect("event bus").final_text = text;
    }

    fn final_text(&self) -> Option<String> {
        self.inner.lock().expect("event bus").final_text.clone()
    }

    fn finish(&self) {
        let mut guard = self.inner.lock().expect("event bus");
        guard.finished = true;
        guard.senders.clear();
    }

    fn subscribe(&self) -> BoxStream<'static, AgentEvent> {
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

    fn request(backend: AgentBackend) -> StartRoundRequest {
        StartRoundRequest {
            backend,
            workspace: PathBuf::from("/tmp/mathscape-ws"),
            input_ref: "explain 紧集".into(),
            tools_allow: vec!["workspace.search".into(), "material.read".into()],
        }
    }

    #[test]
    fn thread_start_registers_mcp_without_credentials() {
        let round = RoundId::parse("codex-1").unwrap();
        let message = thread_start_message(
            2,
            &request(AgentBackend::Codex),
            &round,
            Path::new("mathscape"),
            None,
        );
        let params = &message["params"];
        assert_eq!(params["cwd"], "/tmp/mathscape-ws");
        assert_eq!(params["sandbox"], "read-only");
        assert_eq!(params["approvalPolicy"], "never");
        assert_eq!(params["ephemeral"], true);
        assert_eq!(params["serviceName"], "mathscape");
        assert!(message.get("params").unwrap().get("capabilities").is_none());
        let config = &params["config"];
        assert_eq!(config["mcp_servers.mathscape.command"], "mathscape");
        assert_eq!(config["mcp_servers.mathscape.enabled"], true);
        assert_eq!(
            config["mcp_servers.mathscape.args"],
            json!([
                "mcp",
                "--workspace",
                "/tmp/mathscape-ws",
                "--round",
                "codex-1"
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
    }

    #[test]
    fn empty_tools_allow_does_not_invent_a_restriction_list() {
        let mut request = request(AgentBackend::Codex);
        request.tools_allow.clear();
        let message = thread_start_message(
            2,
            &request,
            &RoundId::parse("codex-1").unwrap(),
            Path::new("mathscape"),
            None,
        );
        let instructions = message["params"]["developerInstructions"].as_str().unwrap();
        assert!(instructions.contains("fixed read-only tool list"));
        assert!(!instructions.contains("Call only these"));
    }

    #[test]
    fn turn_start_forwards_input_ref_as_user_text() {
        let message = turn_start_message(3, "thr_1", "explain 紧集", "/tmp/mathscape-ws");
        assert_eq!(message["params"]["threadId"], "thr_1");
        assert_eq!(message["params"]["input"][0]["text"], "explain 紧集");
        assert!(message["params"].get("model").is_none());
    }

    #[test]
    fn thread_start_forwards_selected_model() {
        let message = thread_start_message(
            2,
            &request(AgentBackend::Codex),
            &RoundId::parse("codex-1").unwrap(),
            Path::new("mathscape"),
            Some("gpt-test"),
        );
        assert_eq!(message["params"]["model"], "gpt-test");
    }

    #[test]
    fn initialize_does_not_opt_into_experimental_auth() {
        let message = initialize_message();
        assert!(message["params"].get("capabilities").is_none());
        assert_eq!(message["params"]["clientInfo"]["name"], "mathscape");
    }

    #[test]
    fn approval_requests_are_declined() {
        assert_eq!(
            server_request_result("item/commandExecution/requestApproval")["decision"],
            "decline"
        );
        assert_eq!(
            server_request_result("item/fileChange/requestApproval")["decision"],
            "decline"
        );
        assert_eq!(
            server_request_result("mcpServer/elicitation/request")["action"],
            "decline"
        );
        assert_eq!(server_request_result("item/tool/call")["success"], false);
        assert!(
            server_request_result("item/permissions/requestApproval")["permissions"].is_object()
        );
        let user_input = server_request_result("item/tool/requestUserInput");
        assert_eq!(user_input, json!({ "answers": {} }));
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
