use std::{
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use crate::{LlmBackend, LlmError, LlmFuture, LlmProfile, LlmRequest, LlmResponse};

/// Deterministic in-process backend for producer and orchestration tests.
pub struct ScriptedLlmBackend {
    profile: LlmProfile,
    responses: Mutex<VecDeque<Result<LlmResponse, LlmError>>>,
    requests: Mutex<Vec<LlmRequest>>,
    streaming_requests: AtomicUsize,
}

impl ScriptedLlmBackend {
    pub fn new(
        profile: LlmProfile,
        responses: impl IntoIterator<Item = Result<LlmResponse, LlmError>>,
    ) -> Self {
        Self {
            profile,
            responses: Mutex::new(responses.into_iter().collect()),
            requests: Mutex::new(Vec::new()),
            streaming_requests: AtomicUsize::new(0),
        }
    }

    pub fn requests(&self) -> Vec<LlmRequest> {
        self.requests
            .lock()
            .expect("scripted backend request lock poisoned")
            .clone()
    }

    pub fn streaming_request_count(&self) -> usize {
        self.streaming_requests.load(Ordering::Relaxed)
    }

    fn take_response(&self, request: LlmRequest, streaming: bool) -> Result<LlmResponse, LlmError> {
        request
            .validate()
            .map_err(|error| LlmError::InvalidRequest(error.to_string()))?;
        self.requests
            .lock()
            .map_err(|_| LlmError::Configuration("scripted request lock poisoned".to_string()))?
            .push(request);
        if streaming {
            self.streaming_requests.fetch_add(1, Ordering::Relaxed);
        }
        self.responses
            .lock()
            .map_err(|_| LlmError::Configuration("scripted response lock poisoned".to_string()))?
            .pop_front()
            .unwrap_or(Err(LlmError::Exhausted))
    }
}

impl LlmBackend for ScriptedLlmBackend {
    fn profile(&self) -> &LlmProfile {
        &self.profile
    }

    fn execute<'a>(&'a self, request: LlmRequest) -> LlmFuture<'a> {
        Box::pin(async move { self.take_response(request, false) })
    }

    fn execute_stream<'a>(&'a self, request: LlmRequest) -> LlmFuture<'a> {
        Box::pin(async move { self.take_response(request, true) })
    }
}
