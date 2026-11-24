use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures::{channel::oneshot, future::BoxFuture, stream::BoxStream};
use mathscape_resident::{EventSink, null_sink};
use serde_json::Value;

use super::{
    OpenCodeConfig,
    host::{Command, Handle, PromptJob, probe_models, spawn},
};
use crate::codex::session::{EventBus, absolute_path};
use crate::{
    AgentBackend, AgentError, AgentEvent, AgentExecutor, AgentModel, ApprovalHandler, Capability,
    CapabilityProbe, CommandRunner, ErrorCode, LoadThread, PolicyApprovals, Prompt, RoundId,
    SessionSpec, SystemCommandRunner, ThreadBinding, probe_program,
};

pub(crate) fn error(message: impl Into<String>) -> AgentError {
    AgentError::Harness {
        backend: AgentBackend::OpenCode,
        code: ErrorCode::Protocol,
        message: message.into(),
    }
}

#[derive(Default)]
pub(crate) struct Catalog {
    pub(crate) models: Option<Vec<AgentModel>>,
    pub(crate) current: Option<String>,
}

impl Catalog {
    pub(crate) fn update(&mut self, value: &Value) {
        let Some(option) = value["configOptions"]
            .as_array()
            .and_then(|options| options.iter().find(|v| v["id"] == "model"))
        else {
            return;
        };
        self.current = option["currentValue"].as_str().map(str::to_owned);
        self.models = Some(
            option["options"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|option| {
                    Some(AgentModel {
                        id: option["value"].as_str()?.into(),
                        display_name: option["name"].as_str().map(str::to_owned),
                        ..Default::default()
                    })
                })
                .collect(),
        );
    }
}

struct RoundSlot {
    bus: Arc<EventBus>,
    key: String,
}

struct Shared {
    config: OpenCodeConfig,
    approvals: Arc<dyn ApprovalHandler>,
    runner: Arc<dyn CommandRunner>,
    sink: EventSink,
    sessions: Mutex<HashMap<String, Handle>>,
    rounds: Mutex<HashMap<RoundId, RoundSlot>>,
    catalog: Arc<Mutex<Catalog>>,
    next: AtomicU64,
}

/// One lazy, resident ACP process per session key; caller-owned thread ids.
pub struct OpenCodeExecutor {
    shared: Arc<Shared>,
}

impl OpenCodeExecutor {
    pub fn new(config: OpenCodeConfig) -> Self {
        Self::with_runner(config, Arc::new(SystemCommandRunner::default()))
    }
    pub fn with_runner(mut config: OpenCodeConfig, runner: Arc<dyn CommandRunner>) -> Self {
        config.state_dir = absolute_path(&config.state_dir);
        Self {
            shared: Arc::new(Shared {
                config,
                approvals: Arc::new(PolicyApprovals),
                runner,
                sink: null_sink(),
                sessions: Mutex::new(HashMap::new()),
                rounds: Mutex::new(HashMap::new()),
                catalog: Arc::new(Mutex::new(Catalog::default())),
                next: AtomicU64::new(0),
            }),
        }
    }
    #[must_use]
    pub fn with_approvals(mut self, approvals: Arc<dyn ApprovalHandler>) -> Self {
        Arc::get_mut(&mut self.shared)
            .expect("configure before sharing")
            .approvals = approvals;
        self
    }
    #[must_use]
    pub fn with_event_sink(mut self, sink: EventSink) -> Self {
        Arc::get_mut(&mut self.shared)
            .expect("configure before sharing")
            .sink = sink;
        self
    }
    pub fn live_sessions(&self) -> Vec<String> {
        let mut keys: Vec<_> = self
            .shared
            .sessions
            .lock()
            .expect("opencode sessions")
            .iter()
            .filter(|(_, h)| h.alive.load(Ordering::Acquire))
            .map(|(key, _)| key.clone())
            .collect();
        keys.sort();
        keys
    }
    /// Latest ACP `currentValue`, which may differ from a persisted user choice
    /// if OpenCode rejected a live switch. `current_model` reads the choice.
    pub fn advertised_model(&self) -> Option<String> {
        self.shared
            .catalog
            .lock()
            .expect("OpenCode catalog")
            .current
            .clone()
    }
    fn handle(&self, key: &str) -> Result<mpsc::Sender<Command>, AgentError> {
        self.shared
            .sessions
            .lock()
            .expect("opencode sessions")
            .get(key)
            .filter(|h| h.alive.load(Ordering::Acquire))
            .map(|h| h.tx.clone())
            .ok_or_else(|| AgentError::UnknownSession(key.into()))
    }
}

impl Drop for OpenCodeExecutor {
    fn drop(&mut self) {
        self.close_all();
    }
}

pub(crate) fn selected_model(config: &OpenCodeConfig) -> Option<String> {
    std::fs::read_to_string(config.state_dir.join("model"))
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

impl AgentExecutor for OpenCodeExecutor {
    fn backend(&self) -> AgentBackend {
        AgentBackend::OpenCode
    }
    fn open_session(&self, mut spec: SessionSpec) -> BoxFuture<'_, Result<(), AgentError>> {
        let result = (|| {
            if spec.backend != self.backend() {
                return Err(AgentError::BackendMismatch {
                    served: self.backend(),
                    requested: spec.backend,
                });
            }
            spec.workspace = absolute_path(&spec.workspace);
            for root in spec
                .harness
                .read_roots
                .iter_mut()
                .chain(spec.harness.skill_roots.iter_mut())
                .chain(spec.harness.scratch_dir.iter_mut())
            {
                if root.is_relative() {
                    *root = spec.workspace.join(&*root);
                }
            }
            if let Some(path) = &mut spec.round_file {
                *path = absolute_path(path);
            }
            let mut sessions = self.shared.sessions.lock().expect("opencode sessions");
            if let Some(handle) = sessions.get_mut(&spec.session_key)
                && handle.alive.load(Ordering::Acquire)
            {
                if handle.spec != spec {
                    handle
                        .tx
                        .send(Command::Update(Box::new(spec.clone())))
                        .map_err(|_| error("OpenCode host ended"))?;
                    handle.spec = spec;
                }
                return Ok(());
            }
            let probe = probe_program(
                self.shared.runner.as_ref(),
                Capability::OpenCode,
                &self.shared.config.program,
            );
            if !probe.is_available() {
                return Err(AgentError::CapabilityUnavailable(probe));
            }
            let alive = Arc::new(AtomicBool::new(true));
            let tx = spawn(
                self.shared.config.clone(),
                spec.clone(),
                Arc::clone(&self.shared.approvals),
                self.shared.sink.clone(),
                Arc::clone(&alive),
                Arc::clone(&self.shared.catalog),
            )?;
            sessions.insert(spec.session_key.clone(), Handle { tx, spec, alive });
            Ok(())
        })();
        Box::pin(async move { result })
    }
    fn new_thread(&self, key: &str) -> BoxFuture<'_, Result<ThreadBinding, AgentError>> {
        let tx = self.handle(key);
        Box::pin(async move {
            let (tx_reply, rx) = oneshot::channel();
            tx?.send(Command::New(tx_reply))
                .map_err(|_| error("OpenCode host ended"))?;
            rx.await.map_err(|_| error("OpenCode host ended"))?
        })
    }
    fn load_thread(
        &self,
        key: &str,
        load: LoadThread,
    ) -> BoxFuture<'_, Result<ThreadBinding, AgentError>> {
        let tx = self.handle(key);
        Box::pin(async move {
            let (reply, rx) = oneshot::channel();
            tx?.send(Command::Load(load, reply))
                .map_err(|_| error("OpenCode host ended"))?;
            rx.await.map_err(|_| error("OpenCode host ended"))?
        })
    }
    fn prompt(&self, key: &str, prompt: Prompt) -> BoxFuture<'_, Result<RoundId, AgentError>> {
        let key = key.to_owned();
        Box::pin(async move {
            let tx = self.handle(&key)?;
            let n = self.shared.next.fetch_add(1, Ordering::Relaxed);
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let round = RoundId::parse(format!("opencode-{now}-{n}")).expect("valid round id");
            let bus = Arc::new(EventBus::new());
            self.shared.rounds.lock().expect("opencode rounds").insert(
                round.clone(),
                RoundSlot {
                    bus: Arc::clone(&bus),
                    key,
                },
            );
            let (reply, rx) = oneshot::channel();
            let result = if tx
                .send(Command::Prompt(Box::new(PromptJob {
                    prompt,
                    round: round.clone(),
                    bus,
                    reply,
                })))
                .is_err()
            {
                Err(error("OpenCode host ended"))
            } else {
                rx.await
                    .unwrap_or_else(|_| Err(error("OpenCode host ended")))
            };
            if let Err(error) = result {
                self.shared
                    .rounds
                    .lock()
                    .expect("opencode rounds")
                    .remove(&round);
                return Err(error);
            }
            Ok(round)
        })
    }
    fn close_session(&self, key: &str) -> BoxFuture<'_, Result<(), AgentError>> {
        let handle = self
            .shared
            .sessions
            .lock()
            .expect("opencode sessions")
            .remove(key);
        Box::pin(async move {
            if let Some(handle) = handle {
                let (reply, rx) = oneshot::channel();
                if handle.tx.send(Command::Close(Some(reply))).is_ok() {
                    let _ = rx.await;
                }
            }
            Ok(())
        })
    }
    fn close_all(&self) {
        let handles: Vec<_> = self
            .shared
            .sessions
            .lock()
            .expect("opencode sessions")
            .drain()
            .map(|(_, h)| h)
            .collect();
        let waits: Vec<_> = handles
            .into_iter()
            .filter_map(|h| {
                let (tx, rx) = oneshot::channel();
                h.tx.send(Command::Close(Some(tx))).ok().map(|()| rx)
            })
            .collect();
        let deadline = Instant::now()
            + self.shared.config.handshake_timeout
            + self.shared.config.interrupt_grace
            + Duration::from_secs(2);
        for mut rx in waits {
            while Instant::now() < deadline {
                match rx.try_recv() {
                    Ok(Some(())) | Err(_) => break,
                    Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
        }
    }
    fn events(&self, round: &RoundId) -> Result<BoxStream<'static, AgentEvent>, AgentError> {
        self.shared
            .rounds
            .lock()
            .expect("opencode rounds")
            .get(round)
            .map(|s| s.bus.subscribe())
            .ok_or_else(|| AgentError::UnknownRound(round.clone()))
    }
    fn cancel(&self, round: &RoundId) -> BoxFuture<'_, Result<(), AgentError>> {
        let result = (|| {
            let rounds = self.shared.rounds.lock().expect("opencode rounds");
            let slot = rounds
                .get(round)
                .ok_or_else(|| AgentError::UnknownRound(round.clone()))?;
            if let Ok(tx) = self.handle(&slot.key) {
                let _ = tx.send(Command::Cancel(round.clone()));
            }
            Ok(())
        })();
        Box::pin(async move { result })
    }
    fn final_text(&self, round: &RoundId) -> Option<String> {
        self.shared
            .rounds
            .lock()
            .expect("opencode rounds")
            .get(round)
            .and_then(|s| s.bus.final_text())
    }
    fn current_model(&self) -> Option<String> {
        selected_model(&self.shared.config)
    }
    fn list_models(&self) -> Result<Vec<AgentModel>, AgentError> {
        let mut catalog = self.shared.catalog.lock().expect("OpenCode catalog");
        if catalog.models.is_none() {
            catalog.update(
                &probe_models(&self.shared.config, self.shared.sink.clone()).map_err(error)?,
            );
        }
        catalog
            .models
            .clone()
            .ok_or_else(|| error("OpenCode did not expose model configOptions"))
    }
    fn set_model(&self, id: &str) -> Result<(), AgentError> {
        let id = id.trim();
        if id.is_empty() || id.chars().any(char::is_control) {
            return Err(error("invalid OpenCode model id"));
        }
        let catalog = self.list_models()?;
        if !catalog.iter().any(|m| m.id == id) {
            return Err(error(format!("unknown OpenCode model: {id}")));
        }
        std::fs::create_dir_all(&self.shared.config.state_dir).map_err(|e| error(e.to_string()))?;
        std::fs::write(self.shared.config.state_dir.join("model"), id)
            .map_err(|e| error(e.to_string()))?;
        let handles: Vec<_> = self
            .shared
            .sessions
            .lock()
            .expect("opencode sessions")
            .values()
            .filter(|h| h.alive.load(Ordering::Acquire))
            .map(|h| h.tx.clone())
            .collect();
        let mut waits = Vec::new();
        for tx in handles {
            let (reply, rx) = mpsc::channel();
            tx.send(Command::Model(id.into(), reply))
                .map_err(|_| error("OpenCode host ended"))?;
            waits.push(rx);
        }
        for rx in waits {
            rx.recv().map_err(|_| error("OpenCode host ended"))??;
        }
        Ok(())
    }
    fn probe(&self) -> Vec<CapabilityProbe> {
        vec![probe_program(
            self.shared.runner.as_ref(),
            Capability::OpenCode,
            &self.shared.config.program,
        )]
    }
}
