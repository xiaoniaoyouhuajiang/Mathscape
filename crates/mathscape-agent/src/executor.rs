//! `AgentExecutor`: the only execution interface Desktop talks to (§5).
//!
//! Phase0 ships the trait, [`crate::FakeBackend`] and an
//! [`UnimplementedExecutor`] placeholder. [`crate::CodexExecutor`] is the
//! Phase1b Codex app-server adapter. Claude Code, optional hmz, and Custom
//! flows on `mathscape-orchestrator` still use [`UnimplementedExecutor`].
//! [`crate::OpenCodeExecutor`] implements OpenCode via ACP.
//! Each adapter owns session management, registering `mathscape mcp` with the
//! harness, mapping native events to `agent_events v0`, cancellation and
//! `round_id` injection. hmz is never required.
//!
//! # 会话 → 线程 → 轮次（ACP 形状）
//!
//! 2026-11 起接口按 ACP（Agent Client Protocol）的形状分三层，harness 无关
//! （docs/codex-session-persistence.md §2.1）：
//!
//! | 本接口 | ACP |
//! | --- | --- |
//! | [`AgentExecutor::open_session`]：一个 harness 进程，工具白名单与授权在此固定 | `initialize` |
//! | [`AgentExecutor::new_thread`] | `session/new` |
//! | [`AgentExecutor::load_thread`] | `session/load` |
//! | [`AgentExecutor::prompt`] + [`AgentExecutor::events`] | `session/prompt` + `session/update` |
//! | [`AgentExecutor::cancel`] | `session/cancel` |
//! | [`ApprovalHandler`] | `session/request_permission` |
//!
//! [`AgentExecutor::start_round`] 保留为一次性便捷方法：临时会话 + 新线程 +
//! 一轮，轮末关闭进程。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::{future::BoxFuture, stream::BoxStream};
use serde::{Deserialize, Serialize};

use crate::{
    AgentBackend, AgentEvent, CapabilityProbe, ErrorCode, RoundId, SystemCommandRunner,
    probe_capability,
};

/// One selectable model exposed by an adapter that supports model selection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentModel {
    /// Stable id the adapter accepts in [`AgentExecutor::set_model`] (e.g. Codex slug).
    pub id: String,
    /// Optional human label; UI may fall back to [`Self::id`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Reasoning efforts this model advertises, in catalog order. Empty when
    /// the catalog does not say (the adapter then accepts no explicit effort
    /// check for this model).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasoning_efforts: Vec<AgentEffort>,
    /// Catalog default effort for this model, if advertised.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_reasoning_effort: Option<String>,
}

/// One reasoning-effort level a model advertises (e.g. Codex `low` … `ultra`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEffort {
    /// Value the adapter accepts in [`AgentExecutor::set_effort`].
    pub id: String,
    /// Optional catalog description (shown as a hint).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// `start_round(backend, workspace, input_ref, tools_allow)`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartRoundRequest {
    pub backend: AgentBackend,
    pub workspace: PathBuf,
    /// Hash or reference of the input bundle for the first turn.
    pub input_ref: String,
    /// Tool names the harness may call, e.g. `workspace.search`.
    pub tools_allow: Vec<String>,
    /// What the harness's own file/shell tools may touch, plus skills.
    /// [`HarnessContext::default`] keeps the legacy read-only sandbox.
    pub harness: HarnessContext,
}

/// Harness-side context for one round.
///
/// Mathscape MCP tools are governed by `tools_allow`; this struct governs the
/// harness itself (Codex's shell / file tools). When any filesystem grant is
/// present the adapter replaces the coarse `read-only` sandbox with a
/// per-session permission profile: `:minimal` system paths, the workspace,
/// `read_roots` and `skill_roots` are readable, only `scratch_dir` is
/// writable, and the network stays off. Formal map changes still go through
/// MCP proposals that the user accepts.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HarnessContext {
    /// Skill directories registered with the harness (read-only).
    pub skill_roots: Vec<PathBuf>,
    /// Extra read-only roots, e.g. the managed Lean profile root.
    pub read_roots: Vec<PathBuf>,
    /// The only harness-writable directory (drafts, object cards, notes).
    pub scratch_dir: Option<PathBuf>,
    /// Non-secret variables set on the harness process and forwarded to the
    /// Mathscape MCP child, e.g. `MATHSCAPE_LEAN_PROFILE_ROOT`.
    pub env: Vec<(String, String)>,
    /// Extra developer instructions (skill index, stable paths).
    pub instructions: Option<String>,
}

impl HarnessContext {
    /// `true` when the round should use a permission profile instead of the
    /// legacy `read-only` sandbox.
    pub fn uses_permission_profile(&self) -> bool {
        self.scratch_dir.is_some() || !self.skill_roots.is_empty() || !self.read_roots.is_empty()
    }
}

/// One harness session: one long-lived harness process (one Codex
/// app-server) that can hold several threads. The tool allow-list, harness
/// grants and skills are fixed per session; opening the same key with a
/// different spec makes the adapter restart the process on next use.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionSpec {
    pub backend: AgentBackend,
    /// Stable caller key, e.g. the sandbox session id.
    pub session_key: String,
    pub workspace: PathBuf,
    /// Fixed for the session (not per round).
    pub tools_allow: Vec<String>,
    pub harness: HarnessContext,
    /// File the adapter writes the current `round_id` into before each turn;
    /// the long-lived Mathscape MCP process reads it (`--round-file`).
    /// `None` → an adapter-managed temporary file.
    pub round_file: Option<PathBuf>,
    /// Threads are not persisted and the process closes after the first
    /// turn ends ([`AgentExecutor::start_round`]).
    pub ephemeral: bool,
}

/// `session/load`: continue a harness thread.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadThread {
    /// Harness-native thread id.
    pub thread_id: String,
    /// A copy of the thread's persisted state kept by the caller (for Codex:
    /// the rollout file in the session dir). Used when the harness itself no
    /// longer has the thread.
    pub state_copy: Option<PathBuf>,
}

/// How a thread became usable for the next prompt.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadContinuity {
    /// New thread, empty context.
    Started,
    /// Already loaded in the running harness process.
    Live,
    /// Reloaded from the harness's own store.
    Resumed,
    /// Reloaded after restoring the caller's state copy into the harness store.
    Restored,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThreadBinding {
    pub thread_id: String,
    pub continuity: ThreadContinuity,
    /// Where the harness persists the thread, when known (Codex rollout).
    pub state_path: Option<PathBuf>,
}

/// Per-turn context placed before the user message. Not part of the
/// thread's stable prefix (developer instructions).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextNote {
    /// Short source key, e.g. `mathscape.workspace_delta`.
    pub key: String,
    pub text: String,
}

/// `session/prompt`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Prompt {
    pub thread_id: String,
    /// The user message (or a rebuilt prompt after a context fallback).
    pub text: String,
    pub context: Vec<ContextNote>,
}

/// Approval request forwarded to an [`ApprovalHandler`].
#[derive(Clone, Copy, Debug)]
pub struct ApprovalRequest<'a> {
    pub round_id: &'a RoundId,
    /// Harness-native method, e.g. `item/commandExecution/requestApproval`.
    pub method: &'a str,
    pub params: &'a serde_json::Value,
    pub tools_allow: &'a [String],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalDecision {
    /// Apply the adapter's built-in policy (read-only allow-list; shell and
    /// writes declined; unclassified requests held, then timed out).
    Policy,
    /// Decline now.
    Decline,
}

/// Host-side approval callback (ACP `session/request_permission`). The
/// default keeps the built-in policy; interactive approval cards would
/// implement this.
pub trait ApprovalHandler: Send + Sync {
    fn decide(&self, request: &ApprovalRequest<'_>) -> ApprovalDecision {
        let _ = request;
        ApprovalDecision::Policy
    }
}

/// [`ApprovalHandler`] that always applies the built-in policy.
#[derive(Clone, Copy, Debug, Default)]
pub struct PolicyApprovals;

impl ApprovalHandler for PolicyApprovals {}

static ONE_SHOT: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum AgentError {
    #[error("the {0} backend adapter is not implemented yet")]
    NotImplemented(AgentBackend),
    #[error("executor serves {served}, but the request asked for {requested}")]
    BackendMismatch {
        served: AgentBackend,
        requested: AgentBackend,
    },
    #[error("unknown round {0}")]
    UnknownRound(RoundId),
    #[error("unknown harness session {0}")]
    UnknownSession(String),
    #[error("required capability {} is unavailable", .0.capability)]
    CapabilityUnavailable(CapabilityProbe),
    /// Adapter does not support listing or changing models (e.g. Claude Code today).
    #[error("model selection unsupported for {backend:?}")]
    ModelSelectionUnsupported { backend: AgentBackend },
    /// Adapter does not support listing or changing reasoning effort.
    #[error("reasoning effort selection unsupported for {backend:?}")]
    EffortSelectionUnsupported { backend: AgentBackend },
    /// The harness process or its handshake failed. `code` matches §3 `error`.
    /// Mathscape does not turn an `auth` failure into a login or token proxy.
    #[error("{backend} adapter ({code}): {message}")]
    Harness {
        backend: AgentBackend,
        code: ErrorCode,
        message: String,
    },
}

/// Object-safe so Desktop can hold `Box<dyn AgentExecutor>` per backend.
pub trait AgentExecutor: Send + Sync {
    fn backend(&self) -> AgentBackend;

    /// Declare (or update) a harness session. Cheap: the process starts
    /// lazily on the first thread operation.
    fn open_session(&self, spec: SessionSpec) -> BoxFuture<'_, Result<(), AgentError>>;

    /// New harness thread with an empty context.
    fn new_thread(&self, session_key: &str) -> BoxFuture<'_, Result<ThreadBinding, AgentError>>;

    /// Continue an existing harness thread. Errors when the harness cannot
    /// reload it; the caller then rebuilds context itself.
    fn load_thread(
        &self,
        session_key: &str,
        load: LoadThread,
    ) -> BoxFuture<'_, Result<ThreadBinding, AgentError>>;

    /// Run one turn on a thread; returns the adapter-assigned `round_id`.
    fn prompt(
        &self,
        session_key: &str,
        prompt: Prompt,
    ) -> BoxFuture<'_, Result<RoundId, AgentError>>;

    /// Where the harness persists `thread_id` right now, if known.
    fn thread_state_path(&self, session_key: &str, thread_id: &str) -> Option<PathBuf> {
        let _ = (session_key, thread_id);
        None
    }

    /// Stop the session's harness process. Idempotent.
    fn close_session(&self, session_key: &str) -> BoxFuture<'_, Result<(), AgentError>>;

    /// Stop every harness process (App exit). Blocking.
    fn close_all(&self) {}

    /// One-shot round: temporary session + new thread + one turn; the
    /// process closes when the turn ends.
    fn start_round(
        &self,
        request: StartRoundRequest,
    ) -> BoxFuture<'_, Result<RoundId, AgentError>> {
        Box::pin(async move {
            let key = format!(
                "one-shot-{}-{}",
                std::process::id(),
                ONE_SHOT.fetch_add(1, Ordering::Relaxed) + 1
            );
            self.open_session(SessionSpec {
                backend: request.backend,
                session_key: key.clone(),
                workspace: request.workspace,
                tools_allow: request.tools_allow,
                harness: request.harness,
                round_file: None,
                ephemeral: true,
            })
            .await?;
            let started = async {
                let thread = self.new_thread(&key).await?;
                self.prompt(
                    &key,
                    Prompt {
                        thread_id: thread.thread_id,
                        text: request.input_ref,
                        context: Vec::new(),
                    },
                )
                .await
            }
            .await;
            if started.is_err() {
                let _ = self.close_session(&key).await;
            }
            started
        })
    }

    /// Event stream for a round, in `seq` order (`agent_events v0`).
    fn events(&self, round_id: &RoundId) -> Result<BoxStream<'static, AgentEvent>, AgentError>;

    /// Requests cancellation; the stream reports `cancel(ack=false)` then
    /// `cancel(ack=true)` and `turn_end(cancelled)`.
    fn cancel(&self, round_id: &RoundId) -> BoxFuture<'_, Result<(), AgentError>>;

    /// Resolves a round's `turn_end.final_text_ref` to the agent's visible
    /// reply text, once the round has ended. Adapters that do not keep the
    /// text return `None` (the default), so this is additive to §5.
    fn final_text(&self, _round_id: &RoundId) -> Option<String> {
        None
    }

    /// Currently selected model id for this adapter, if known.
    ///
    /// Default: `None`. Codex reads `~/.codex/config.toml`.
    fn current_model(&self) -> Option<String> {
        let _ = self;
        None
    }

    /// Models the user may pick for subsequent rounds.
    ///
    /// Default: [`AgentError::ModelSelectionUnsupported`]. Codex lists
    /// `~/.codex/models_cache.json` (plus the current default if missing).
    fn list_models(&self) -> Result<Vec<AgentModel>, AgentError> {
        Err(AgentError::ModelSelectionUnsupported {
            backend: self.backend(),
        })
    }

    /// Persist a model selection for subsequent rounds.
    ///
    /// Default: [`AgentError::ModelSelectionUnsupported`]. Codex writes the
    /// top-level `model` key in `~/.codex/config.toml`.
    fn set_model(&self, id: &str) -> Result<(), AgentError> {
        let _ = id;
        Err(AgentError::ModelSelectionUnsupported {
            backend: self.backend(),
        })
    }

    /// Reasoning effort the next round will use, if known.
    ///
    /// Default: `None`. Codex resolves `model_reasoning_effort` from
    /// `~/.codex/config.toml` against the current model's catalog entry
    /// (falling back to the model's `default_reasoning_level`).
    fn current_effort(&self) -> Option<String> {
        let _ = self;
        None
    }

    /// Efforts selectable for `model` (or the current model when `None`).
    ///
    /// Default: [`AgentError::EffortSelectionUnsupported`]. Codex reads the
    /// model's `supported_reasoning_levels` from `models_cache.json`.
    fn list_efforts(&self, model: Option<&str>) -> Result<Vec<AgentEffort>, AgentError> {
        let _ = model;
        Err(AgentError::EffortSelectionUnsupported {
            backend: self.backend(),
        })
    }

    /// Persist a reasoning effort for subsequent rounds.
    ///
    /// Default: [`AgentError::EffortSelectionUnsupported`]. Codex writes the
    /// top-level `model_reasoning_effort` key in `~/.codex/config.toml` and
    /// forwards it as `effort` on `turn/start`.
    fn set_effort(&self, effort: &str) -> Result<(), AgentError> {
        let _ = effort;
        Err(AgentError::EffortSelectionUnsupported {
            backend: self.backend(),
        })
    }

    /// Backend self-reported availability. Blocking (may spawn
    /// `--version` probes); call from a blocking context.
    fn probe(&self) -> Vec<CapabilityProbe>;
}

/// Placeholder for a backend whose adapter has not been written yet.
/// Every round operation returns [`AgentError::NotImplemented`]; `probe`
/// still reports the backend's required capability.
#[derive(Clone, Copy, Debug)]
pub struct UnimplementedExecutor {
    backend: AgentBackend,
}

impl UnimplementedExecutor {
    pub const fn new(backend: AgentBackend) -> Self {
        Self { backend }
    }
}

impl AgentExecutor for UnimplementedExecutor {
    fn backend(&self) -> AgentBackend {
        self.backend
    }

    fn open_session(&self, _spec: SessionSpec) -> BoxFuture<'_, Result<(), AgentError>> {
        let backend = self.backend;
        Box::pin(async move { Err(AgentError::NotImplemented(backend)) })
    }

    fn new_thread(&self, _session_key: &str) -> BoxFuture<'_, Result<ThreadBinding, AgentError>> {
        let backend = self.backend;
        Box::pin(async move { Err(AgentError::NotImplemented(backend)) })
    }

    fn load_thread(
        &self,
        _session_key: &str,
        _load: LoadThread,
    ) -> BoxFuture<'_, Result<ThreadBinding, AgentError>> {
        let backend = self.backend;
        Box::pin(async move { Err(AgentError::NotImplemented(backend)) })
    }

    fn prompt(
        &self,
        _session_key: &str,
        _prompt: Prompt,
    ) -> BoxFuture<'_, Result<RoundId, AgentError>> {
        let backend = self.backend;
        Box::pin(async move { Err(AgentError::NotImplemented(backend)) })
    }

    fn close_session(&self, _session_key: &str) -> BoxFuture<'_, Result<(), AgentError>> {
        Box::pin(async move { Ok(()) })
    }

    fn events(&self, _round_id: &RoundId) -> Result<BoxStream<'static, AgentEvent>, AgentError> {
        Err(AgentError::NotImplemented(self.backend))
    }

    fn cancel(&self, _round_id: &RoundId) -> BoxFuture<'_, Result<(), AgentError>> {
        let backend = self.backend;
        Box::pin(async move { Err(AgentError::NotImplemented(backend)) })
    }

    fn probe(&self) -> Vec<CapabilityProbe> {
        self.backend
            .required_capability()
            .map(|capability| probe_capability(&SystemCommandRunner::default(), capability))
            .into_iter()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;

    use super::*;

    #[test]
    fn unimplemented_backends_refuse_rounds() {
        for backend in AgentBackend::ALL {
            let executor: Box<dyn AgentExecutor> = Box::new(UnimplementedExecutor::new(backend));
            let request = StartRoundRequest {
                backend,
                workspace: PathBuf::from("/tmp/ws"),
                input_ref: "sha256:x".into(),
                tools_allow: vec![],
                harness: HarnessContext::default(),
            };
            assert_eq!(
                block_on(executor.start_round(request)),
                Err(AgentError::NotImplemented(backend))
            );
            let round = RoundId::parse("r").unwrap();
            assert!(executor.events(&round).is_err());
            assert!(block_on(executor.cancel(&round)).is_err());
            let probes = executor.probe();
            assert_eq!(probes.len(), usize::from(backend != AgentBackend::Custom));
        }
    }
}
