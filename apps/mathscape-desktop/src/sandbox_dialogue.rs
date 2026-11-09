//! S1 dialogue shell over a sandbox session (docs/sandbox-flow.md §4.2, §5 S1).
//!
//! One user message = one `agent_events v0` round on the local Codex
//! app-server ([`CodexExecutor`]), with the ten read-only Mathscape MCP
//! tools mounted against the session's *isolated copy* (never the checkpoint,
//! never the mother workspace). Workflow is "diluted" to chat: there is no
//! planner and no propose/accept; Mathscape MCP tools stay read-only (T2).
//!
//! The harness itself runs under a per-session permission profile
//! ([`round_harness`]): it may read the workspace copy, the versioned skill
//! pack and the managed Lean profile, and write only the session's
//! `scratch/` (drafts). Network stays off. Each round's harness context
//! (skill version + hash, grants, Lean profile root) is appended to the
//! session's `harness.jsonl`.
//!
//! Codex rounds are single-turn and ephemeral, so continuity comes from
//! replaying recent transcript lines **of the active dialogue thread** into
//! each new round ([`build_dialogue_prompt`]). 「新对话」 opens another thread
//! on the same isolated copy: the agent context starts empty, workspace state
//! carries over (docs/sandbox-flow.md §3.5). Everything the round produces stays inside the
//! session directory: `transcript.jsonl` (user / agent / system lines) and
//! `events.jsonl` (the raw event stream). The UI receives the same events
//! live on [`DIALOGUE_EVENT`]; it never shows a reply that did not come from
//! the event stream (`turn_end` + [`AgentExecutor::final_text`]).

use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use futures::StreamExt;
use mathscape_agent::{
    AgentBackend, AgentEffort, AgentExecutor, AgentModel, CapabilityProbe, CodexConfig,
    CodexExecutor, EventPayload, HarnessContext, RoundId, StartRoundRequest, TurnOutcome,
};
use mathscape_sandbox::{
    DialogueThread, MAX_MESSAGE_CHARS, SandboxError, SandboxHome, TranscriptEntry, TranscriptRole,
    build_dialogue_prompt,
};
use mathscape_tool_lean::{LEAN_PROFILE_ROOT_ENV, discover_profile_root};
use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, State};

use crate::sandbox::SandboxState;
use crate::skill_pack::{load_worknet_skill, locate_skills_root};

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
/// Backend label written into `session.json` / export manifests.
const BACKEND_LABEL: &str = "codex";

/// Live update for the dialogue view.
/// Blocking-side result of [`SandboxDialogueState::status`].
#[derive(Default)]
struct StatusSnapshot {
    probes: Vec<CapabilityProbe>,
    model: Option<String>,
    models: Vec<AgentModel>,
    model_selection: bool,
    effort: Option<String>,
    efforts: Vec<AgentEffort>,
    effort_selection: bool,
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
    pub mcp_program: PathBuf,
    pub tools: Vec<String>,
    pub probes: Vec<CapabilityProbe>,
    pub available: bool,
    /// Current model id from the adapter (`AgentExecutor::current_model`).
    pub model: Option<String>,
    /// Selectable models from the adapter (`list_models`). Empty when unsupported.
    pub models: Vec<AgentModel>,
    /// True when the adapter implements model selection (Codex).
    pub model_selection: bool,
    /// Reasoning effort the next round will use (`AgentExecutor::current_effort`).
    pub effort: Option<String>,
    /// Efforts the current model advertises (`list_efforts(None)`). Empty when
    /// the catalog says nothing for this model.
    pub efforts: Vec<AgentEffort>,
    /// True when the adapter implements effort selection (Codex).
    pub effort_selection: bool,
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
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendOutcome {
    pub round_id: String,
    pub user_entry: TranscriptEntry,
}

#[derive(Clone)]
pub(crate) struct SandboxDialogueState {
    inner: Arc<Inner>,
}

struct Inner {
    executor: Arc<dyn AgentExecutor>,
    codex_program: PathBuf,
    mcp_program: PathBuf,
    /// One round at a time per session. `None` = reserved, round starting.
    active: Mutex<HashMap<String, Option<RoundId>>>,
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
    let home = Arc::clone(home);
    tauri::async_runtime::spawn_blocking(move || {
        let home = home
            .lock()
            .map_err(|_| "沙箱状态锁已损坏，请重启应用".to_owned())?;
        work(&home).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("sandbox dialogue worker failed: {error}"))?
}

impl SandboxDialogueState {
    pub(crate) fn from_environment() -> Self {
        let config = CodexConfig {
            program: env_path(CODEX_PROGRAM_ENVIRONMENT_VARIABLE)
                .unwrap_or_else(|| PathBuf::from("codex")),
            mcp_program: env_path(MCP_PROGRAM_ENVIRONMENT_VARIABLE)
                .unwrap_or_else(default_mcp_program),
            ..CodexConfig::default()
        };
        Self::with_config(config)
    }

    fn with_config(config: CodexConfig) -> Self {
        Self {
            inner: Arc::new(Inner {
                codex_program: config.program.clone(),
                mcp_program: config.mcp_program.clone(),
                executor: Arc::new(CodexExecutor::new(config)),
                active: Mutex::new(HashMap::new()),
            }),
        }
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
        }
    }

    /// Best-effort cancel of the session's running round (used by discard).
    pub(crate) async fn cancel_session_round(&self, session_id: &str) {
        if let Some(round) = self.active_round(session_id) {
            let _ = self.inner.executor.cancel(&round).await;
        }
    }

    async fn status(&self) -> AgentStatus {
        let executor = Arc::clone(&self.inner.executor);
        let snapshot = tauri::async_runtime::spawn_blocking(move || {
            let probes = executor.probe();
            let model = executor.current_model();
            let (models, model_selection) = match executor.list_models() {
                Ok(models) => (models, true),
                Err(_) => (Vec::new(), false),
            };
            let effort = executor.current_effort();
            // Efforts come from the selected model's catalog entry. Reuse the
            // catalog fetched above instead of asking the adapter again (which
            // may spawn an app-server when the cache is missing).
            let efforts = if model_selection {
                Ok(model
                    .as_deref()
                    .and_then(|id| models.iter().find(|m| m.id == id))
                    .map(|m| m.reasoning_efforts.clone())
                    .unwrap_or_default())
            } else {
                executor.list_efforts(None)
            };
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
            backend: BACKEND_LABEL,
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
        let executor = Arc::clone(&self.inner.executor);
        tauri::async_runtime::spawn_blocking(move || executor.set_effort(&effort))
            .await
            .map_err(|error| format!("set_effort worker failed: {error}"))?
            .map_err(|error| error.to_string())?;
        Ok(self.status().await)
    }

    async fn set_model(&self, id: String) -> Result<AgentStatus, String> {
        let executor = Arc::clone(&self.inner.executor);
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
            Ok(SessionDialogue {
                transcript: home.read_transcript(&session_id)?,
                threads: threads.threads,
                active_thread_id: threads.active_thread_id,
                events: home.read_events(&session_id)?,
                sandbox_session_id: session_id,
                active_round_id,
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
        let text = text.trim().to_owned();
        if text.is_empty() {
            return Err("消息为空".into());
        }
        if text.chars().count() > MAX_MESSAGE_CHARS {
            return Err(format!("消息过长（上限 {MAX_MESSAGE_CHARS} 字符）"));
        }
        {
            let mut active = self.active();
            if active.contains_key(&session_id) {
                return Err("上一轮还没结束，请等待或先取消".into());
            }
            active.insert(session_id.clone(), None);
        }

        let tools = sandbox_tool_allowlist();
        let prepared = {
            let (session_id, text, tools) = (session_id.clone(), text.clone(), tools.clone());
            with_home(&home, move |home| {
                let view = home.live_session(&session_id)?;
                let threads = home.dialogue_threads(&session_id)?;
                let thread = threads.active_thread_id.clone();
                let continues_session = threads
                    .threads
                    .first()
                    .is_some_and(|first| first.thread_id != thread);
                let history = home.read_thread_transcript(&session_id, &thread)?;
                let entry = home.append_thread_transcript(
                    &session_id,
                    &thread,
                    TranscriptRole::User,
                    &text,
                    None,
                    None,
                )?;
                home.record_backend(&session_id, BACKEND_LABEL, &tools)?;
                let checkpoint_root = home.checkpoint(&view.checkpoint_id).ok().map(|c| c.root);
                let scratch = home.ensure_session_scratch(&session_id)?;
                let investigation_id = format!("sandbox:{session_id}:{thread}");
                let (harness, harness_record) = round_harness_for_investigation(
                    checkpoint_root.as_deref(),
                    scratch,
                    &tools,
                    Some(&investigation_id),
                );
                Ok((
                    build_dialogue_prompt(&history, &text, continues_session),
                    view.workspace_root
                        .unwrap_or_else(|| home.session_workspace(&session_id)),
                    entry,
                    thread,
                    harness,
                    harness_record,
                ))
            })
            .await
        };
        let (prompt, workspace, user_entry, thread, harness, mut harness_record) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                self.release(&session_id, None);
                return Err(error);
            }
        };
        sink(DialogueUpdate::Transcript {
            sandbox_session_id: session_id.clone(),
            entry: user_entry.clone(),
        });

        let executor = Arc::clone(&self.inner.executor);
        let started = executor
            .start_round(StartRoundRequest {
                backend: AgentBackend::Codex,
                workspace,
                input_ref: prompt,
                tools_allow: tools,
                harness,
            })
            .await
            .map_err(|error| error.to_string())
            .and_then(|round| {
                let stream = executor.events(&round).map_err(|error| error.to_string())?;
                Ok((round, stream))
            });
        let (round, mut stream) = match started {
            Ok(started) => started,
            Err(error) => {
                self.release(&session_id, None);
                let message = format!("未能启动本机 Codex：{error}");
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
        let pump_thread = thread;
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
                                this.inner
                                    .executor
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
            this.release(&session_id, Some(&round));
            sink(DialogueUpdate::RoundFinished {
                sandbox_session_id: session_id,
                round_id: round.as_str().to_owned(),
            });
        });

        Ok(SendOutcome {
            round_id: round.as_str().to_owned(),
            user_entry,
        })
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

    async fn cancel(&self, session_id: &str) -> Result<bool, String> {
        match self.active_round(session_id) {
            None => Ok(false),
            Some(round) => self
                .inner
                .executor
                .cancel(&round)
                .await
                .map(|()| true)
                .map_err(|error| error.to_string()),
        }
    }
}

#[tauri::command]
pub async fn sandbox_agent_status(
    dialogue: State<'_, SandboxDialogueState>,
) -> Result<AgentStatus, String> {
    Ok(dialogue.status().await)
}

/// Persist a Codex reasoning effort for subsequent sandbox rounds.
#[tauri::command]
pub async fn sandbox_set_effort(
    dialogue: State<'_, SandboxDialogueState>,
    effort: String,
) -> Result<AgentStatus, String> {
    dialogue.set_effort(effort).await
}

/// Persist a Codex model selection for subsequent sandbox rounds.
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
        sync::mpsc,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use mathscape_sandbox::{EVENTS_FILE, fingerprint_workspace};

    use super::*;
    use crate::DesktopService;

    /// `mathscape-codex-fixture` (dry-run Codex app-server: no login, no
    /// model; echoes the user text as the agent message).
    fn fixture_program() -> PathBuf {
        let exe = env::current_exe().unwrap();
        let profile_dir = exe.parent().and_then(Path::parent).unwrap().to_path_buf();
        let candidate = profile_dir.join(format!(
            "mathscape-codex-fixture{}",
            env::consts::EXE_SUFFIX
        ));
        if !candidate.is_file() {
            let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
            let status = Command::new(cargo)
                .args([
                    "build",
                    "-p",
                    "mathscape-agent",
                    "--bin",
                    "mathscape-codex-fixture",
                ])
                .status()
                .expect("build codex fixture");
            assert!(status.success(), "building mathscape-codex-fixture failed");
        }
        assert!(
            candidate.is_file(),
            "fixture missing at {}",
            candidate.display()
        );
        candidate
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

    fn dialogue(args: &[&str]) -> SandboxDialogueState {
        SandboxDialogueState::with_config(CodexConfig {
            program: fixture_program(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            mcp_program: PathBuf::from("mathscape"),
            ..CodexConfig::default()
        })
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
        let state = dialogue(&["app-server"]);
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

        // Second round replays the first exchange into its prompt.
        tauri::async_runtime::block_on(state.send(
            Arc::clone(&fx.home),
            fx.session_id.clone(),
            "再举个例子".into(),
            sink,
        ))
        .unwrap();
        wait_for(&updates, finished);
        let home = fx.home.lock().unwrap();
        let transcript = home.read_transcript(&fx.session_id).unwrap();
        assert_eq!(transcript.len(), 4);
        assert!(
            transcript[3].text.contains("[用户] 解释紧集"),
            "{}",
            transcript[3].text
        );
        assert!(home.events_path(&fx.session_id).is_file());
        let record = home.session(&fx.session_id).unwrap();
        assert_eq!(record.backend, "codex");
        assert_eq!(record.tool_allowlist, sandbox_tool_allowlist());
        assert_eq!(record.tool_allowlist.len(), 10);
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
        let state = dialogue(&["app-server"]);
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
        // The fixture echoes the prompt: old thread content is not replayed,
        // and the agent is told the copy may hold earlier state.
        assert!(!d2[1].text.contains("调试旧功能"), "{}", d2[1].text);
        assert!(d2[1].text.contains("新开的一条对话"), "{}", d2[1].text);
        assert_eq!(home.session_workspace(&fx.session_id), workspace);
        assert_eq!(
            home.read_thread_transcript(&fx.session_id, "d1")
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn cancel_ends_round_with_system_line_and_blocks_overlap() {
        let fx = fixture("cancel");
        let state = dialogue(&["--hold", "app-server"]);
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
        let state = dialogue(&["--reject-auth", "app-server"]);
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
        let state = dialogue(&["--hold", "app-server"]);
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
        let state = dialogue(&["app-server"]);
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
}
