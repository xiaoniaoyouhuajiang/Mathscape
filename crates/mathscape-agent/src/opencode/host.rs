use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{
    Arc, Mutex,
    mpsc::{self, Receiver, Sender},
};
use std::time::{Duration, Instant};

use futures::channel::oneshot;
use mathscape_resident::EventSink;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{
    OpenCodeConfig, approval, config,
    session::{Catalog, error, selected_model},
};
use crate::acp::{
    map::Mapper,
    transport::{Transport, response},
};
use crate::codex::{
    approval::approval_event_raw,
    session::{EventBus, turn_context_text},
};
use crate::{
    AgentBackend, AgentError, ApprovalHandler, ApprovalRequest, CancelRequester, LoadThread,
    Prompt, RoundId, SessionSpec, ThreadBinding, ThreadContinuity,
};

type Reply<T> = oneshot::Sender<Result<T, AgentError>>;
static NEXT_HOST: AtomicU64 = AtomicU64::new(0);

pub(crate) enum Command {
    Update(Box<SessionSpec>),
    New(Reply<ThreadBinding>),
    Load(LoadThread, Reply<ThreadBinding>),
    Prompt(Box<PromptJob>),
    Cancel(RoundId),
    Model(String, Sender<Result<(), AgentError>>),
    Close(Option<oneshot::Sender<()>>),
}
pub(crate) struct Handle {
    pub(crate) tx: Sender<Command>,
    pub(crate) spec: SessionSpec,
    pub(crate) alive: Arc<AtomicBool>,
}
pub(crate) struct PromptJob {
    pub(crate) prompt: Prompt,
    pub(crate) round: RoundId,
    pub(crate) bus: Arc<EventBus>,
    pub(crate) reply: Reply<()>,
}
struct Active {
    round: RoundId,
    id: u64,
    session: String,
    bus: Arc<EventBus>,
    mapper: Mapper,
    progress: Instant,
    cancel: Option<Instant>,
    failure: Option<String>,
}

pub(crate) fn spawn(
    config: OpenCodeConfig,
    spec: SessionSpec,
    approvals: Arc<dyn ApprovalHandler>,
    sink: EventSink,
    alive: Arc<AtomicBool>,
    catalog: Arc<Mutex<Catalog>>,
) -> Result<Sender<Command>, AgentError> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name(format!("opencode-{}", spec.session_key))
        .spawn(move || {
            let mut driver = Driver::new(config, spec, approvals, sink, rx, catalog);
            driver.run();
            alive.store(false, Ordering::Release);
        })
        .map_err(|e| error(e.to_string()))?;
    Ok(tx)
}

struct Driver {
    config: OpenCodeConfig,
    spec: SessionSpec,
    approvals: Arc<dyn ApprovalHandler>,
    rpc: Transport,
    commands: Receiver<Command>,
    deferred: VecDeque<Command>,
    catalog: Arc<Mutex<Catalog>>,
    loaded: HashSet<String>,
    dir: PathBuf,
    active: Option<Active>,
    finished: bool,
}

impl Driver {
    fn new(
        config: OpenCodeConfig,
        spec: SessionSpec,
        approvals: Arc<dyn ApprovalHandler>,
        sink: EventSink,
        commands: Receiver<Command>,
        catalog: Arc<Mutex<Catalog>>,
    ) -> Self {
        let n = NEXT_HOST.fetch_add(1, Ordering::Relaxed);
        let digest = format!("{:x}", Sha256::digest(spec.session_key.as_bytes()));
        let dir = config.state_dir.join("sessions").join(format!(
            "{}-{}-{n}",
            &digest[..16],
            std::process::id()
        ));
        let rpc = Transport::new(
            format!("opencode:{}", spec.session_key),
            config.handshake_timeout,
            config.interrupt_grace,
            config.idle_timeout,
            sink,
        );
        Self {
            config,
            spec,
            approvals,
            rpc,
            commands,
            deferred: VecDeque::new(),
            catalog,
            loaded: HashSet::new(),
            dir,
            active: None,
            finished: false,
        }
    }
    fn round_file(&self) -> PathBuf {
        self.spec
            .round_file
            .clone()
            .unwrap_or_else(|| self.dir.join("current_round"))
    }
    fn ensure(&mut self) -> Result<(), String> {
        let model = selected_model(&self.config);
        let process = config::process(&self.config, &self.spec, &self.dir, model.as_deref())?;
        // Model changes are also applied to loaded ACP sessions, without restarting them.
        let key = format!(
            "{:x}",
            Sha256::digest(format!("{:?}", self.spec).as_bytes())
        );
        if self.rpc.ensure(&key, process)? {
            self.loaded.clear();
        }
        let round_file = self.round_file();
        if let Some(parent) = round_file.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        if !round_file.exists() {
            std::fs::write(&round_file, "").map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    fn set_model(&mut self, id: &str) -> Result<(), String> {
        for session in self.loaded.clone() {
            let result = self.rpc.request(
                "session/set_config_option",
                json!({"sessionId":session,"configId":"model","value":id}),
                self.config.handshake_timeout,
            )?;
            self.catalog
                .lock()
                .expect("OpenCode catalog")
                .update(&result);
        }
        Ok(())
    }
    fn bind(&mut self, load: Option<&LoadThread>) -> Result<ThreadBinding, String> {
        self.ensure()?;
        if let Some(load) = load
            && self.loaded.contains(&load.thread_id)
        {
            return Ok(ThreadBinding {
                thread_id: load.thread_id.clone(),
                continuity: ThreadContinuity::Live,
                state_path: None,
            });
        }
        let mut params = config::session_params(&self.config, &self.spec, &self.round_file());
        let method = if let Some(load) = load {
            params["sessionId"] = json!(load.thread_id);
            "session/load"
        } else {
            "session/new"
        };
        let result = self
            .rpc
            .request(method, params, self.config.handshake_timeout)?;
        self.catalog
            .lock()
            .expect("OpenCode catalog")
            .update(&result);
        let id = if let Some(load) = load {
            load.thread_id.clone()
        } else {
            result["sessionId"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or("ACP session/new omitted sessionId")?
                .to_owned()
        };
        if let Some(model) = selected_model(&self.config) {
            let switched = self.rpc.request(
                "session/set_config_option",
                json!({"sessionId":id,"configId":"model","value":model}),
                self.config.handshake_timeout,
            )?;
            self.catalog
                .lock()
                .expect("OpenCode catalog")
                .update(&switched);
        }
        self.loaded.insert(id.clone());
        Ok(ThreadBinding {
            thread_id: id,
            continuity: if load.is_some() {
                ThreadContinuity::Resumed
            } else {
                ThreadContinuity::Started
            },
            state_path: None,
        })
    }
    fn prompt(&mut self, job: PromptJob) {
        let result: Result<(), String> = (|| {
            self.bind(Some(&LoadThread {
                thread_id: job.prompt.thread_id.clone(),
                state_copy: None,
            }))?;
            std::fs::write(self.round_file(), job.round.as_str()).map_err(|e| e.to_string())?;
            let mut blocks = Vec::new();
            if let Some(text) = turn_context_text(&job.prompt.context) {
                blocks.push(json!({"type":"text","text":text}));
            }
            blocks.push(json!({"type":"text","text":job.prompt.text}));
            let id = self.rpc.start(
                "session/prompt",
                json!({"sessionId":job.prompt.thread_id,"prompt":blocks}),
            )?;
            let mut mapper = Mapper::new(
                job.round.clone(),
                AgentBackend::OpenCode,
                "opencode",
                approval::tool_names(&self.spec.tools_allow),
            );
            job.bus.publish(mapper.start(job.prompt.text.clone()));
            self.active = Some(Active {
                round: job.round.clone(),
                id,
                session: job.prompt.thread_id.clone(),
                bus: Arc::clone(&job.bus),
                mapper,
                progress: Instant::now(),
                cancel: None,
                failure: None,
            });
            Ok(())
        })();
        let _ = job.reply.send(result.map_err(error));
    }
    fn cancel(&mut self, by: CancelRequester, failure: Option<String>) {
        let Some(active) = &mut self.active else {
            return;
        };
        if active.cancel.is_some() {
            return;
        }
        active.bus.publish_all(active.mapper.cancel(by));
        active.failure = failure;
        active.cancel = Some(Instant::now());
        if let Err(error)=self.rpc.send(&json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":active.session}})) {
            self.end(None,Some(error),true);
        }
    }
    fn end(&mut self, result: Option<Value>, failure: Option<String>, shutdown: bool) {
        self.rpc.touch();
        if shutdown || self.spec.ephemeral {
            self.rpc.shutdown();
            self.loaded.clear();
        }
        if let Some(mut active) = self.active.take() {
            let events = active
                .mapper
                .finish(result.as_ref(), active.failure.or(failure));
            active
                .bus
                .set_final_text((!active.mapper.text.is_empty()).then_some(active.mapper.text));
            active.bus.publish_all(events);
            active.bus.finish();
            self.finished = true;
        }
    }
    fn incoming(&mut self, message: Value) -> Result<(), String> {
        let Some(active) = &mut self.active else {
            return self.rpc.reject(&message);
        };
        if message.get("method").is_none() && message["id"] == active.id {
            match response(&message) {
                Ok(result) => self.end(Some(result), None, false),
                Err(error) => self.end(None, Some(error), false),
            }
            return Ok(());
        }
        let method = message["method"].as_str().unwrap_or("");
        let params = &message["params"];
        if params["sessionId"].as_str() != Some(active.session.as_str()) {
            return self.rpc.reject(&message);
        }
        if method == "session/update" {
            let update = &params["update"];
            let progress = match update["sessionUpdate"].as_str() {
                Some("agent_message_chunk" | "agent_thought_chunk") => update
                    .pointer("/content/text")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.is_empty()),
                Some("tool_call" | "tool_call_update") => update["toolCallId"].as_str().is_some(),
                _ => false,
            };
            if progress {
                active.progress = Instant::now();
                self.rpc.touch();
            }
            if update["sessionUpdate"] == "config_option_update" {
                self.catalog
                    .lock()
                    .expect("OpenCode catalog")
                    .update(update);
            }
            active.bus.publish_all(active.mapper.update(update));
        } else if method == "session/request_permission"
            && let Some(id) = message.get("id")
        {
            active.progress = Instant::now();
            let decision = self.approvals.decide(&ApprovalRequest {
                round_id: &active.round,
                method,
                params,
                tools_allow: &self.spec.tools_allow,
            });
            let allow =
                active.cancel.is_none() && approval::permitted(&self.spec, params, decision);
            let reply = approval::reply(params, allow);
            if !allow
                || reply.pointer("/outcome/outcome").and_then(Value::as_str) == Some("cancelled")
            {
                let mut note_params = params.clone();
                note_params["itemId"] = params["toolCall"]["toolCallId"].clone();
                note_params["reason"] = params["toolCall"]["title"].clone();
                let raw = approval_event_raw("denied", method, id, &note_params);
                active.bus.publish(active.mapper.note("OpenCode 审批策略拒绝了该工具调用（仅允许授权读取和 scratch 内写入；shell 与网络工具禁用）".into(),raw));
            }
            self.rpc
                .send(&json!({"jsonrpc":"2.0","id":id,"result":reply}))?;
        } else {
            self.rpc.reject(&message)?;
        }
        Ok(())
    }
    fn command(&mut self, command: Command) -> bool {
        match command {
            Command::Close(ack) => {
                self.cancel(CancelRequester::System, None);
                self.rpc.shutdown();
                self.end(None, None, false);
                if let Some(ack) = ack {
                    let _ = ack.send(());
                }
                return false;
            }
            Command::Cancel(round) => {
                if self.active.as_ref().is_some_and(|a| a.round == round) {
                    self.cancel(CancelRequester::User, None);
                }
            }
            Command::New(reply) => {
                if self.active.is_some() {
                    let _ = reply.send(Err(error("OpenCode session is busy")));
                } else {
                    let result = self.bind(None).map_err(error);
                    let _ = reply.send(result);
                }
            }
            Command::Load(load, reply) => {
                if self.active.is_some() {
                    let _ = reply.send(Err(error("OpenCode session is busy")));
                } else {
                    let result = self.bind(Some(&load)).map_err(error);
                    let _ = reply.send(result);
                }
            }
            Command::Prompt(job) => {
                if self.active.is_some() {
                    let _ = job.reply.send(Err(error("OpenCode session is busy")));
                } else {
                    self.prompt(*job);
                }
            }
            command if self.active.is_some() => self.deferred.push_back(command),
            Command::Update(spec) => self.spec = *spec,
            Command::Model(model, reply) => {
                let result = self.set_model(&model).map_err(error);
                let _ = reply.send(result);
            }
        }
        true
    }
    fn run(&mut self) {
        loop {
            let command = if self.active.is_none() {
                self.deferred
                    .pop_front()
                    .or_else(|| self.commands.recv_timeout(Duration::from_millis(50)).ok())
            } else {
                self.commands.try_recv().ok()
            };
            if let Some(command) = command
                && !self.command(command)
            {
                break;
            }
            if self.active.is_some() {
                match self.rpc.poll(Duration::from_millis(20)) {
                    Ok(Some(message)) => {
                        if let Err(error) = self.incoming(message) {
                            self.end(None, Some(error), true);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => self.end(None, Some(error), true),
                }
                if let Some(active) = &self.active {
                    if active
                        .cancel
                        .is_some_and(|at| at.elapsed() >= self.config.interrupt_grace)
                    {
                        self.end(None, None, true);
                    } else if active.cancel.is_none()
                        && active.progress.elapsed() >= self.config.no_progress_timeout
                    {
                        self.cancel(CancelRequester::System,Some("OpenCode 长时间没有进展，已取消。可能是模型额度或认证问题，请检查 OpenCode。".into()));
                    }
                }
            } else if self.rpc.reap_idle() {
                self.loaded.clear();
            }
            if self.spec.ephemeral && self.finished {
                break;
            }
        }
        self.rpc.shutdown();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub(crate) fn probe_models(config: &OpenCodeConfig, sink: EventSink) -> Result<Value, String> {
    let n = NEXT_HOST.fetch_add(1, Ordering::Relaxed);
    let dir = config
        .state_dir
        .join("probes")
        .join(format!("{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let spec = SessionSpec {
            backend: AgentBackend::OpenCode,
            session_key: "model-probe".into(),
            workspace: dir.clone(),
            tools_allow: vec![],
            harness: Default::default(),
            round_file: None,
            ephemeral: true,
        };
        let process = config::process(config, &spec, &dir, selected_model(config).as_deref())?;
        let timeout = config.handshake_timeout.min(Duration::from_secs(8));
        let mut rpc = Transport::new(
            "opencode:model-probe".into(),
            timeout,
            config.interrupt_grace,
            None,
            sink,
        );
        rpc.ensure("probe", process)?;
        let result = rpc.request("session/new", json!({"cwd":dir,"mcpServers":[]}), timeout);
        rpc.shutdown();
        result
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}
