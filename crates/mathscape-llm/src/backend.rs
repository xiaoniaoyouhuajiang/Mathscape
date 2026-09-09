use std::{future::Future, pin::Pin};

use crate::{CredentialError, LlmCapabilities, LlmProfile, LlmRequest, LlmResponse};

pub type LlmFuture<'a> = Pin<Box<dyn Future<Output = Result<LlmResponse, LlmError>> + Send + 'a>>;

/// Object-safe boundary used by producers and future agent runtimes.
pub trait LlmBackend: Send + Sync {
    fn profile(&self) -> &LlmProfile;

    fn capabilities(&self) -> &LlmCapabilities {
        &self.profile().capabilities
    }

    fn execute<'a>(&'a self, request: LlmRequest) -> LlmFuture<'a>;

    /// Executes a request over a streaming transport and collects its events
    /// into the same provider-neutral final response used by `execute`.
    ///
    /// Backends without a native streaming implementation retain a safe
    /// full-response fallback. Callers therefore choose streaming for latency
    /// and gateway liveness without coupling orchestration to provider events.
    fn execute_stream<'a>(&'a self, request: LlmRequest) -> LlmFuture<'a> {
        self.execute(request)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LlmErrorKind {
    Configuration,
    InvalidRequest,
    Credential,
    Authentication,
    RateLimited,
    Timeout,
    Transport,
    Provider,
    InvalidResponse,
    UnsupportedCapability,
    Exhausted,
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum LlmError {
    #[error("LLM configuration error: {0}")]
    Configuration(String),
    #[error("invalid LLM request: {0}")]
    InvalidRequest(String),
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error("LLM authentication failed: {0}")]
    Authentication(String),
    #[error("LLM provider rate limited the request")]
    RateLimited { retry_after_ms: Option<u64> },
    #[error("LLM request timed out")]
    Timeout,
    #[error("LLM transport failed: {0}")]
    Transport(String),
    #[error("LLM provider returned an error: status={status:?}, message={message}")]
    Provider {
        status: Option<u16>,
        message: String,
    },
    #[error("LLM response is invalid: {0}")]
    InvalidResponse(String),
    #[error("LLM target does not support capability '{0}'")]
    UnsupportedCapability(String),
    #[error("scripted LLM backend has no remaining responses")]
    Exhausted,
}

impl LlmError {
    pub const fn kind(&self) -> LlmErrorKind {
        match self {
            Self::Configuration(_) => LlmErrorKind::Configuration,
            Self::InvalidRequest(_) => LlmErrorKind::InvalidRequest,
            Self::Credential(_) => LlmErrorKind::Credential,
            Self::Authentication(_) => LlmErrorKind::Authentication,
            Self::RateLimited { .. } => LlmErrorKind::RateLimited,
            Self::Timeout => LlmErrorKind::Timeout,
            Self::Transport(_) => LlmErrorKind::Transport,
            Self::Provider { .. } => LlmErrorKind::Provider,
            Self::InvalidResponse(_) => LlmErrorKind::InvalidResponse,
            Self::UnsupportedCapability(_) => LlmErrorKind::UnsupportedCapability,
            Self::Exhausted => LlmErrorKind::Exhausted,
        }
    }
}
