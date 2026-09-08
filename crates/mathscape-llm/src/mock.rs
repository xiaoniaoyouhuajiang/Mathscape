use std::{collections::VecDeque, sync::Mutex};

use crate::{LlmBackend, LlmError, LlmFuture, LlmProfile, LlmRequest, LlmResponse};

/// Deterministic in-process backend for producer and orchestration tests.
pub struct ScriptedLlmBackend {
    profile: LlmProfile,
    responses: Mutex<VecDeque<Result<LlmResponse, LlmError>>>,
    requests: Mutex<Vec<LlmRequest>>,
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
        }
    }

    pub fn requests(&self) -> Vec<LlmRequest> {
        self.requests
            .lock()
            .expect("scripted backend request lock poisoned")
            .clone()
    }
}

impl LlmBackend for ScriptedLlmBackend {
    fn profile(&self) -> &LlmProfile {
        &self.profile
    }

    fn execute<'a>(&'a self, request: LlmRequest) -> LlmFuture<'a> {
        Box::pin(async move {
            request
                .validate()
                .map_err(|error| LlmError::InvalidRequest(error.to_string()))?;
            self.requests
                .lock()
                .map_err(|_| LlmError::Configuration("scripted request lock poisoned".to_string()))?
                .push(request);
            self.responses
                .lock()
                .map_err(|_| {
                    LlmError::Configuration("scripted response lock poisoned".to_string())
                })?
                .pop_front()
                .unwrap_or(Err(LlmError::Exhausted))
        })
    }
}
