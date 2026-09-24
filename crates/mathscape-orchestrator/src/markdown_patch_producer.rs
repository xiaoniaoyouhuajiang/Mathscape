//! One bounded LLM call for Markdown patch suggestions. This producer never
//! applies bytes or grants source-map authority.

use mathscape_domain::{MAX_PATCH_OUTPUT_BYTES, MarkdownPatchResponse};
use mathscape_llm::{
    LlmBackend, LlmError, LlmFinishReason, LlmResponse, LlmResponsePart, LlmUsage,
};
use serde_json::Value;
use thiserror::Error;

use crate::markdown_patch_prompt::{PreparedMarkdownPatchRequest, markdown_patch_prompt_manifest};

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct MarkdownPatchProductionSummary {
    pub task_kind: String,
    pub task_version: String,
    pub input_fingerprint: mathscape_domain::ContentHash,
    pub prompt_fingerprint: mathscape_domain::ContentHash,
    pub profile_id: String,
    pub provider: String,
    pub requested_model: String,
    pub response: Option<MarkdownPatchResponseSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct MarkdownPatchResponseSummary {
    pub finish_reason: Option<LlmFinishReason>,
    pub usage: LlmUsage,
    pub response_id: Option<String>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Error)]
pub enum MarkdownPatchRejection {
    #[error("patch response ended because the output token limit was reached")]
    MaxOutputTokens,
    #[error("patch response was stopped by a content filter")]
    ContentFiltered,
    #[error("patch response has an unusable finish reason: {0:?}")]
    UnusableFinishReason(Option<LlmFinishReason>),
    #[error("patch response unexpectedly contains a tool call")]
    UnexpectedToolCall,
    #[error("patch response does not contain JSON text")]
    MissingText,
    #[error("patch response is {actual} bytes, exceeding the {limit} byte limit")]
    ResponseTooLarge { limit: usize, actual: usize },
    #[error("patch response is not one JSON value: {message}")]
    InvalidJson { message: String },
    #[error("patch response has a non-canonical JSON shape")]
    NonCanonicalJson,
}

#[derive(Debug)]
pub enum MarkdownPatchProductionOutcome {
    Valid {
        response: MarkdownPatchResponse,
        run: MarkdownPatchProductionSummary,
    },
    Invalid {
        response: Option<MarkdownPatchResponse>,
        rejection: MarkdownPatchRejection,
        run: MarkdownPatchProductionSummary,
    },
    Failed {
        error: LlmError,
        run: MarkdownPatchProductionSummary,
    },
}

pub struct MarkdownPatchProducer<'a> {
    backend: &'a dyn LlmBackend,
    max_response_bytes: usize,
}

impl<'a> MarkdownPatchProducer<'a> {
    pub fn new(backend: &'a dyn LlmBackend) -> Self {
        Self {
            backend,
            max_response_bytes: MAX_PATCH_OUTPUT_BYTES,
        }
    }

    pub fn with_max_response_bytes(mut self, limit: usize) -> Self {
        self.max_response_bytes = limit;
        self
    }

    fn pending(&self, prepared: &PreparedMarkdownPatchRequest) -> MarkdownPatchProductionSummary {
        let profile = self.backend.profile();
        MarkdownPatchProductionSummary {
            task_kind: prepared.request().task.kind.clone(),
            task_version: prepared.request().task.version.clone(),
            input_fingerprint: prepared.input().input_fingerprint.clone(),
            prompt_fingerprint: prepared.prompt_fingerprint().clone(),
            profile_id: profile.id.clone(),
            provider: profile.provider.clone(),
            requested_model: profile.model.clone(),
            response: None,
        }
    }

    pub async fn produce(
        &self,
        prepared: PreparedMarkdownPatchRequest,
    ) -> MarkdownPatchProductionOutcome {
        let mut run = self.pending(&prepared);
        let response = match self
            .backend
            .execute_stream(prepared.request().clone())
            .await
        {
            Ok(response) => response,
            Err(error) => return MarkdownPatchProductionOutcome::Failed { error, run },
        };
        run.response = Some(MarkdownPatchResponseSummary {
            finish_reason: response.finish_reason.clone(),
            usage: response.usage.clone(),
            response_id: response.response_id.clone(),
            elapsed_ms: response.elapsed_ms,
        });
        if let Some(rejection) = patch_finish_rejection(response.finish_reason.as_ref()) {
            return MarkdownPatchProductionOutcome::Invalid {
                response: None,
                rejection,
                run,
            };
        }
        let text = match collect_text(
            &response,
            self.max_response_bytes
                .min(prepared.manifest().max_response_bytes),
        ) {
            Ok(text) => text,
            Err(rejection) => {
                return MarkdownPatchProductionOutcome::Invalid {
                    response: None,
                    rejection,
                    run,
                };
            }
        };
        let value: Value = match serde_json::from_str(&text) {
            Ok(value) => value,
            Err(error) => {
                return MarkdownPatchProductionOutcome::Invalid {
                    response: None,
                    rejection: MarkdownPatchRejection::InvalidJson {
                        message: error.to_string(),
                    },
                    run,
                };
            }
        };
        let response_value: MarkdownPatchResponse = match serde_json::from_value(value.clone()) {
            Ok(response) => response,
            Err(error) => {
                return MarkdownPatchProductionOutcome::Invalid {
                    response: None,
                    rejection: MarkdownPatchRejection::InvalidJson {
                        message: error.to_string(),
                    },
                    run,
                };
            }
        };
        if serde_json::to_value(&response_value).expect("patch response serializes") != value {
            return MarkdownPatchProductionOutcome::Invalid {
                response: Some(response_value),
                rejection: MarkdownPatchRejection::NonCanonicalJson,
                run,
            };
        }
        MarkdownPatchProductionOutcome::Valid {
            response: response_value,
            run,
        }
    }
}

fn patch_finish_rejection(reason: Option<&LlmFinishReason>) -> Option<MarkdownPatchRejection> {
    match reason {
        None | Some(LlmFinishReason::Completed { .. } | LlmFinishReason::StopSequence { .. }) => {
            None
        }
        Some(LlmFinishReason::MaxTokens { .. }) => Some(MarkdownPatchRejection::MaxOutputTokens),
        Some(LlmFinishReason::ContentFilter { .. }) => {
            Some(MarkdownPatchRejection::ContentFiltered)
        }
        Some(reason) => Some(MarkdownPatchRejection::UnusableFinishReason(Some(
            reason.clone(),
        ))),
    }
}

fn collect_text(response: &LlmResponse, limit: usize) -> Result<String, MarkdownPatchRejection> {
    if response
        .parts
        .iter()
        .any(|part| matches!(part, LlmResponsePart::ToolCall { .. }))
    {
        return Err(MarkdownPatchRejection::UnexpectedToolCall);
    }
    let actual = response
        .parts
        .iter()
        .map(|part| match part {
            LlmResponsePart::Text { text } => text.len(),
            LlmResponsePart::ToolCall { .. } => 0,
        })
        .sum::<usize>();
    if actual > limit {
        return Err(MarkdownPatchRejection::ResponseTooLarge { limit, actual });
    }
    let mut text = String::with_capacity(actual);
    for part in &response.parts {
        if let LlmResponsePart::Text { text: part } = part {
            text.push_str(part);
        }
    }
    if text.is_empty() {
        return Err(MarkdownPatchRejection::MissingText);
    }
    Ok(text)
}

#[allow(dead_code)]
fn _manifest_is_loaded() {
    let _ = markdown_patch_prompt_manifest();
}
