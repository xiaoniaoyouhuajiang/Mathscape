//! `AgentExecutor`: the only execution interface Desktop talks to (§5).
//!
//! Phase0 ships the trait, [`crate::FakeBackend`] and an
//! [`UnimplementedExecutor`] placeholder. [`crate::CodexExecutor`] is the
//! Phase1b Codex app-server adapter. Claude Code, optional hmz, and Custom
//! flows on `mathscape-orchestrator` still use [`UnimplementedExecutor`].
//! Each adapter owns session management, registering `mathscape mcp` with the
//! harness, mapping native events to `agent_events v0`, cancellation and
//! `round_id` injection. hmz is never required.

use std::path::PathBuf;

use futures::{future::BoxFuture, stream::BoxStream};

use crate::{
    AgentBackend, AgentEvent, CapabilityProbe, ErrorCode, RoundId, SystemCommandRunner,
    probe_capability,
};

/// `start_round(backend, workspace, input_ref, tools_allow)`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartRoundRequest {
    pub backend: AgentBackend,
    pub workspace: PathBuf,
    /// Hash or reference of the input bundle for the first turn.
    pub input_ref: String,
    /// Tool names the harness may call, e.g. `workspace.search`.
    pub tools_allow: Vec<String>,
}

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
    #[error("required capability {} is unavailable", .0.capability)]
    CapabilityUnavailable(CapabilityProbe),
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

    /// Starts a round and returns the adapter-assigned `round_id`.
    fn start_round(&self, request: StartRoundRequest)
    -> BoxFuture<'_, Result<RoundId, AgentError>>;

    /// Event stream for a round, in `seq` order (`agent_events v0`).
    fn events(&self, round_id: &RoundId) -> Result<BoxStream<'static, AgentEvent>, AgentError>;

    /// Requests cancellation; the stream reports `cancel(ack=false)` then
    /// `cancel(ack=true)` and `turn_end(cancelled)`.
    fn cancel(&self, round_id: &RoundId) -> BoxFuture<'_, Result<(), AgentError>>;

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

    fn start_round(
        &self,
        _request: StartRoundRequest,
    ) -> BoxFuture<'_, Result<RoundId, AgentError>> {
        let backend = self.backend;
        Box::pin(async move { Err(AgentError::NotImplemented(backend)) })
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
