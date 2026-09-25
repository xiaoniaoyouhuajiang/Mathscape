use mathscape_domain::{ContentHash, DiscoveryProposal};
use mathscape_llm::{
    LlmBackend, LlmError, LlmFinishReason, LlmResolvedTarget, LlmResponse, LlmResponsePart,
    LlmUsage,
};
use serde_json::Value;
use thiserror::Error;

use crate::{
    DiscoveryValidationLimits, DiscoveryValidationReport, PreparedDiscoveryRequest,
    SelectorResolutionReport, ValidatedDiscoveryProposal, validate_discovery_proposal,
};

pub const DEFAULT_DISCOVERY_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiscoveryProducerLimits {
    max_response_bytes: usize,
    validation: DiscoveryValidationLimits,
}

impl DiscoveryProducerLimits {
    pub fn new(max_response_bytes: usize, validation: DiscoveryValidationLimits) -> Option<Self> {
        (max_response_bytes > 0).then_some(Self {
            max_response_bytes,
            validation,
        })
    }

    pub fn max_response_bytes(self) -> usize {
        self.max_response_bytes
    }

    pub fn validation(self) -> DiscoveryValidationLimits {
        self.validation
    }
}

impl Default for DiscoveryProducerLimits {
    fn default() -> Self {
        Self {
            max_response_bytes: DEFAULT_DISCOVERY_RESPONSE_BYTES,
            validation: DiscoveryValidationLimits::default(),
        }
    }
}

/// Facts available only after a provider returned. Raw content and reasoning
/// are deliberately excluded from this summary.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DiscoveryResponseSummary {
    pub target: LlmResolvedTarget,
    pub finish_reason: Option<LlmFinishReason>,
    pub usage: LlmUsage,
    pub response_id: Option<String>,
    pub elapsed_ms: u64,
}

/// Persistence-neutral observation of one producer attempt. This is not a
/// trust record and does not imply that a proposal was accepted or registered.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DiscoveryRunSummary {
    pub task_kind: String,
    pub task_version: String,
    pub input_fingerprint: ContentHash,
    pub prompt_fingerprint: ContentHash,
    pub profile_id: String,
    pub provider: String,
    pub requested_model: String,
    pub response: Option<DiscoveryResponseSummary>,
}

impl DiscoveryRunSummary {
    fn pending(prepared: &PreparedDiscoveryRequest, backend: &dyn LlmBackend) -> Self {
        let profile = backend.profile();
        Self {
            task_kind: prepared.request().task.kind.clone(),
            task_version: prepared.request().task.version.clone(),
            input_fingerprint: prepared.input().fingerprint.clone(),
            prompt_fingerprint: prepared.prompt_fingerprint().clone(),
            profile_id: profile.id.clone(),
            provider: profile.provider.clone(),
            requested_model: profile.model.clone(),
            response: None,
        }
    }

    pub(crate) fn observe(&mut self, response: &LlmResponse) {
        self.response = Some(DiscoveryResponseSummary {
            target: response.target.clone(),
            finish_reason: response.finish_reason.clone(),
            usage: response.usage.clone(),
            response_id: response.response_id.clone(),
            elapsed_ms: response.elapsed_ms,
        });
    }
}

#[derive(Debug, Error)]
pub enum DiscoveryProposalRejection {
    #[error("LLM response ended because the output token limit was reached")]
    MaxOutputTokens,
    #[error("LLM response was stopped by a content filter")]
    ContentFiltered,
    #[error("LLM response has an unusable finish reason: {0:?}")]
    UnusableFinishReason(Option<LlmFinishReason>),
    #[error("Discovery response unexpectedly contains a tool call")]
    UnexpectedToolCall,
    #[error("Discovery response does not contain JSON text")]
    MissingText,
    #[error("Discovery response is {actual} bytes, exceeding the {limit} byte limit")]
    ResponseTooLarge { limit: usize, actual: usize },
    #[error("Discovery response is not one JSON value: {message}")]
    InvalidJson { message: String },
    #[error("Discovery response does not use the canonical producer representation")]
    NonCanonicalJson,
    #[error(transparent)]
    SelectorResolution(#[from] SelectorResolutionReport),
    #[error(transparent)]
    Validation(#[from] DiscoveryValidationReport),
}

#[derive(Debug)]
pub enum DiscoveryProductionOutcome {
    Valid {
        proposal: ValidatedDiscoveryProposal,
        run: DiscoveryRunSummary,
    },
    InvalidProposal {
        proposal: Option<DiscoveryProposal>,
        rejection: DiscoveryProposalRejection,
        run: DiscoveryRunSummary,
    },
    Failed {
        error: LlmError,
        run: DiscoveryRunSummary,
    },
}

/// Performs exactly one provider call. It does not retry, repair, persist,
/// accept candidates, register source entities, or mutate the map.
pub struct DiscoveryProducer<'a> {
    backend: &'a dyn LlmBackend,
    limits: DiscoveryProducerLimits,
}

impl<'a> DiscoveryProducer<'a> {
    pub fn new(backend: &'a dyn LlmBackend) -> Self {
        Self {
            backend,
            limits: DiscoveryProducerLimits::default(),
        }
    }

    pub fn with_limits(backend: &'a dyn LlmBackend, limits: DiscoveryProducerLimits) -> Self {
        Self { backend, limits }
    }

    pub async fn produce(&self, prepared: PreparedDiscoveryRequest) -> DiscoveryProductionOutcome {
        let mut run = DiscoveryRunSummary::pending(&prepared, self.backend);
        if let Err(error) = prepared.request().validate() {
            return DiscoveryProductionOutcome::Failed {
                error: LlmError::InvalidRequest(error.to_string()),
                run,
            };
        }
        if prepared.request().task.input_fingerprint.as_ref() != Some(&prepared.input().fingerprint)
        {
            return DiscoveryProductionOutcome::Failed {
                error: LlmError::InvalidRequest(
                    "Discovery request fingerprint does not match its input".into(),
                ),
                run,
            };
        }

        let (input, request, manifest, _, selector_resolver) = prepared.into_parts();
        // Discovery responses can be large structured documents. Use the
        // streaming boundary so provider SSE traffic keeps intermediary
        // gateways alive; validation still begins only after the final JSON
        // document has been collected.
        let response = match self.backend.execute_stream(request).await {
            Ok(response) => response,
            Err(error) => return DiscoveryProductionOutcome::Failed { error, run },
        };
        run.observe(&response);

        if let Some(rejection) = reject_finish_reason(response.finish_reason.as_ref()) {
            return invalid(None, rejection, run);
        }

        let response_limit = self
            .limits
            .max_response_bytes
            .min(manifest.max_response_bytes);
        let text = match collect_text(&response, response_limit) {
            Ok(text) => text,
            Err(rejection) => return invalid(None, rejection, run),
        };
        let value: Value = match serde_json::from_str(&text) {
            Ok(value) => value,
            Err(error) => {
                return invalid(
                    None,
                    DiscoveryProposalRejection::InvalidJson {
                        message: error.to_string(),
                    },
                    run,
                );
            }
        };
        if !value.is_object() {
            return invalid(
                None,
                DiscoveryProposalRejection::InvalidJson {
                    message: "top-level value must be an object".into(),
                },
                run,
            );
        }
        let proposal: DiscoveryProposal = match serde_json::from_value(value.clone()) {
            Ok(proposal) => proposal,
            Err(error) => {
                return invalid(
                    None,
                    DiscoveryProposalRejection::InvalidJson {
                        message: error.to_string(),
                    },
                    run,
                );
            }
        };

        // Deserialization accepts historical omission of optional fields. A
        // current producer must emit their explicit nulls as required by the
        // published serialization schema.
        if serde_json::to_value(&proposal).expect("DiscoveryProposal is serializable") != value {
            return invalid(
                Some(proposal),
                DiscoveryProposalRejection::NonCanonicalJson,
                run,
            );
        }

        let rejected_proposal = proposal.clone();
        let resolved = match selector_resolver.resolve(proposal) {
            Ok(resolved) => resolved.into_proposal(),
            Err(report) => {
                return invalid(
                    Some(rejected_proposal),
                    DiscoveryProposalRejection::SelectorResolution(report),
                    run,
                );
            }
        };
        match validate_discovery_proposal(&input, resolved, self.limits.validation) {
            Ok(proposal) => DiscoveryProductionOutcome::Valid { proposal, run },
            Err(report) => invalid(
                Some(rejected_proposal),
                DiscoveryProposalRejection::Validation(report),
                run,
            ),
        }
    }
}

pub(crate) fn reject_finish_reason(
    reason: Option<&LlmFinishReason>,
) -> Option<DiscoveryProposalRejection> {
    match reason {
        None | Some(LlmFinishReason::Completed { .. } | LlmFinishReason::StopSequence { .. }) => {
            None
        }
        Some(LlmFinishReason::MaxTokens { .. }) => {
            Some(DiscoveryProposalRejection::MaxOutputTokens)
        }
        Some(LlmFinishReason::ContentFilter { .. }) => {
            Some(DiscoveryProposalRejection::ContentFiltered)
        }
        Some(reason @ (LlmFinishReason::ToolCall { .. } | LlmFinishReason::Other { .. })) => Some(
            DiscoveryProposalRejection::UnusableFinishReason(Some(reason.clone())),
        ),
    }
}

pub(crate) fn collect_text(
    response: &LlmResponse,
    limit: usize,
) -> Result<String, DiscoveryProposalRejection> {
    if response
        .parts
        .iter()
        .any(|part| matches!(part, LlmResponsePart::ToolCall { .. }))
    {
        return Err(DiscoveryProposalRejection::UnexpectedToolCall);
    }
    let actual = response.parts.iter().fold(0usize, |total, part| {
        let bytes = match part {
            LlmResponsePart::Text { text } => text.len(),
            LlmResponsePart::ToolCall { .. } => 0,
        };
        total.saturating_add(bytes)
    });
    if actual > limit {
        return Err(DiscoveryProposalRejection::ResponseTooLarge { limit, actual });
    }
    let mut text = String::with_capacity(actual);
    for part in &response.parts {
        if let LlmResponsePart::Text { text: part } = part {
            text.push_str(part);
        }
    }
    if text.is_empty() {
        return Err(DiscoveryProposalRejection::MissingText);
    }
    Ok(text)
}

fn invalid(
    proposal: Option<DiscoveryProposal>,
    rejection: DiscoveryProposalRejection,
    run: DiscoveryRunSummary,
) -> DiscoveryProductionOutcome {
    DiscoveryProductionOutcome::InvalidProposal {
        proposal,
        rejection,
        run,
    }
}
