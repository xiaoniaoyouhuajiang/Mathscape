use std::sync::{Arc, Mutex};

use mathscape_domain::{
    ContentHash, DiscoveryAdmission, DiscoveryCoverageProposal, DiscoveryIssueProposal,
    DiscoveryProposal, DiscoveryReferenceProposal, DiscoveryRelationProposal,
    DiscoverySelectionRequest, DiscoveryUnitProposal,
};
use mathscape_llm::{
    GenaiBackend, GenaiBackendOptions, LlmError, LlmErrorKind, LlmFinishReason, LlmProfile,
};
use mathscape_orchestrator::{
    DiscoveryDiagnostic, DiscoveryProducer, DiscoveryProductionOutcome, DiscoveryProposalRejection,
    DiscoveryRunSummary, DiscoveryValidationCode, PreparedDiscoveryRequest, SelectorResolutionCode,
    SelectorResolutionDiagnostic, ValidatedDiscoveryProposal,
};
use serde::{Deserialize, Serialize};
use tauri::State;
use thiserror::Error;

use crate::{
    DesktopService, DesktopState,
    llm_settings::LlmSettingsError,
    workspace_settings::{EffectiveDiscoveryPolicy, WorkspaceSettingsError},
};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunDiscoveryRequest {
    selection: DiscoverySelectionRequest,
    profile_id: Option<String>,
}

impl RunDiscoveryRequest {
    pub fn new(selection: DiscoverySelectionRequest, profile_id: Option<String>) -> Self {
        Self {
            selection,
            profile_id,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ActiveDiscoveryTask {
    artifact_id: String,
    input_fingerprint: ContentHash,
    profile_id: String,
    request_timeout_seconds: u64,
}

impl ActiveDiscoveryTask {
    pub fn artifact_id(&self) -> &str {
        &self.artifact_id
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum DesktopDiscoveryOutcome {
    Valid {
        proposal: DesktopDiscoveryProposal,
        run: DesktopDiscoveryRun,
    },
    InvalidProposal {
        proposal: Option<DiscoveryProposal>,
        rejection_kind: &'static str,
        message: String,
        diagnostics: Vec<DesktopDiscoveryDiagnostic>,
        run: DesktopDiscoveryRun,
    },
    Failed {
        error_kind: &'static str,
        message: String,
        run: DesktopDiscoveryRun,
    },
}

#[derive(Debug, Serialize)]
pub struct DesktopDiscoveryProposal {
    status: &'static str,
    input_fingerprint: ContentHash,
    units: Vec<DesktopDiscoveryUnit>,
    relations: Vec<DiscoveryRelationProposal>,
    references: Vec<DiscoveryReferenceProposal>,
    issues: Vec<DiscoveryIssueProposal>,
    coverage: Vec<DiscoveryCoverageProposal>,
}

#[derive(Debug, Serialize)]
pub struct DesktopDiscoveryUnit {
    #[serde(flatten)]
    proposal: DiscoveryUnitProposal,
    source_excerpt: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DesktopDiscoveryRun {
    task_kind: String,
    task_version: String,
    input_fingerprint: ContentHash,
    prompt_fingerprint: ContentHash,
    profile_id: String,
    provider: String,
    requested_model: String,
    admission: DiscoveryAdmission,
    request_timeout_seconds: u64,
    connect_timeout_seconds: u64,
    response: Option<DesktopDiscoveryResponse>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DesktopDiscoveryResponse {
    provider_model: String,
    finish_reason: Option<&'static str>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
    response_id: Option<String>,
    elapsed_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DesktopDiscoveryDiagnostic {
    code: &'static str,
    path: String,
    message: String,
}

#[derive(Debug, Error)]
pub enum DesktopDiscoveryError {
    #[error("已有内容发现任务正在运行，请等待完成后再提交")]
    AlreadyRunning,
    #[error("内容发现输入构建失败：{0}")]
    Input(String),
    #[error(transparent)]
    Settings(#[from] LlmSettingsError),
    #[error(transparent)]
    WorkspaceSettings(#[from] WorkspaceSettingsError),
    #[error("LLM backend 初始化失败：{0}")]
    Backend(#[from] LlmError),
    #[error("内容发现准备线程失败：{0}")]
    Worker(String),
}

impl DesktopService {
    pub fn active_discovery(&self) -> Option<ActiveDiscoveryTask> {
        self.active_discovery
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub async fn run_discovery(
        &self,
        request: RunDiscoveryRequest,
    ) -> Result<DesktopDiscoveryOutcome, DesktopDiscoveryError> {
        self.run_discovery_observed(request, None).await
    }

    async fn run_discovery_observed(
        &self,
        request: RunDiscoveryRequest,
        observer: Option<Arc<dyn Fn(&str, &str) + Send + Sync>>,
    ) -> Result<DesktopDiscoveryOutcome, DesktopDiscoveryError> {
        let service = self.clone();
        let (prepared, profile, policy) = tauri::async_runtime::spawn_blocking(move || {
            service.prepare_discovery_execution(request)
        })
        .await
        .map_err(|error| DesktopDiscoveryError::Worker(error.to_string()))??;

        let task = ActiveDiscoveryTask {
            artifact_id: prepared.input().snapshot.artifact_id.to_string(),
            input_fingerprint: prepared.input().fingerprint.clone(),
            profile_id: profile.id.clone(),
            request_timeout_seconds: policy.request_timeout.as_secs(),
        };
        let _permit = self.begin_discovery(task)?;
        if let Some(observer) = &observer {
            observer(
                "started",
                &format!(
                    "{} · {}",
                    profile.display_name.as_deref().unwrap_or(&profile.id),
                    profile.model
                ),
            );
        }
        let credentials: Arc<dyn mathscape_llm::CredentialResolver> = self.credentials.clone();
        let mut backend = GenaiBackend::new(
            profile,
            credentials,
            GenaiBackendOptions {
                timeout: policy.request_timeout,
                connect_timeout: policy.connect_timeout,
            },
        )?;
        if let Some(observer) = observer {
            backend = backend.with_stream_observer(observer);
        }
        let outcome = DiscoveryProducer::new(&backend).produce(prepared).await;
        Ok(desktop_outcome(outcome, policy))
    }

    fn prepare_discovery_execution(
        &self,
        request: RunDiscoveryRequest,
    ) -> Result<
        (
            PreparedDiscoveryRequest,
            LlmProfile,
            EffectiveDiscoveryPolicy,
        ),
        DesktopDiscoveryError,
    > {
        let settings = self.workspace_settings()?;
        let admission = match settings.discovery_admission() {
            crate::DiscoveryAdmissionPreference::ReviewBeforePublish => {
                DiscoveryAdmission::ReviewBeforePublish
            }
            crate::DiscoveryAdmissionPreference::AutomaticCandidate => {
                DiscoveryAdmission::AutomaticCandidate
            }
            crate::DiscoveryAdmissionPreference::AskEachRun => request.selection.admission,
        };
        let profile = self.selected_llm_profile(request.profile_id.as_deref())?;
        let policy = settings.discovery_policy(admission);
        let prepared = self
            .prepare_discovery_request(request.selection)
            .map_err(DesktopDiscoveryError::Input)?;
        Ok((prepared, profile, policy))
    }

    fn begin_discovery(
        &self,
        task: ActiveDiscoveryTask,
    ) -> Result<DiscoveryPermit, DesktopDiscoveryError> {
        let mut active = self
            .active_discovery
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if active.is_some() {
            return Err(DesktopDiscoveryError::AlreadyRunning);
        }
        *active = Some(task);
        Ok(DiscoveryPermit(self.active_discovery.clone()))
    }
}

struct DiscoveryPermit(Arc<Mutex<Option<ActiveDiscoveryTask>>>);

impl Drop for DiscoveryPermit {
    fn drop(&mut self) {
        *self.0.lock().unwrap_or_else(|error| error.into_inner()) = None;
    }
}

fn desktop_outcome(
    outcome: DiscoveryProductionOutcome,
    policy: EffectiveDiscoveryPolicy,
) -> DesktopDiscoveryOutcome {
    match outcome {
        DiscoveryProductionOutcome::Valid { proposal, run } => DesktopDiscoveryOutcome::Valid {
            proposal: DesktopDiscoveryProposal::from(&proposal),
            run: DesktopDiscoveryRun::from_summary(run, policy),
        },
        DiscoveryProductionOutcome::InvalidProposal {
            proposal,
            rejection,
            run,
        } => {
            let message = rejection.to_string();
            let rejection_kind = rejection_kind(&rejection);
            let diagnostics = match &rejection {
                DiscoveryProposalRejection::Validation(report) => report
                    .diagnostics()
                    .iter()
                    .map(DesktopDiscoveryDiagnostic::from)
                    .collect(),
                DiscoveryProposalRejection::SelectorResolution(report) => report
                    .diagnostics()
                    .iter()
                    .map(DesktopDiscoveryDiagnostic::from)
                    .collect(),
                _ => vec![],
            };
            DesktopDiscoveryOutcome::InvalidProposal {
                proposal,
                rejection_kind,
                message,
                diagnostics,
                run: DesktopDiscoveryRun::from_summary(run, policy),
            }
        }
        DiscoveryProductionOutcome::Failed { error, run } => {
            let message = if error.kind() == LlmErrorKind::Timeout {
                format!(
                    "LLM 请求在当前超时策略下未完成：连接超时 {} 秒，请求总超时 {} 秒",
                    policy.connect_timeout.as_secs(),
                    policy.request_timeout.as_secs()
                )
            } else {
                error.to_string()
            };
            DesktopDiscoveryOutcome::Failed {
                error_kind: llm_error_kind(error.kind()),
                message,
                run: DesktopDiscoveryRun::from_summary(run, policy),
            }
        }
    }
}

impl From<&ValidatedDiscoveryProposal> for DesktopDiscoveryProposal {
    fn from(proposal: &ValidatedDiscoveryProposal) -> Self {
        Self {
            status: match proposal.status() {
                mathscape_orchestrator::ValidatedProposalStatus::Complete => "complete",
                mathscape_orchestrator::ValidatedProposalStatus::Partial => "partial",
            },
            input_fingerprint: proposal.input_fingerprint().clone(),
            units: proposal
                .units()
                .iter()
                .map(|unit| DesktopDiscoveryUnit {
                    proposal: unit.proposal().clone(),
                    source_excerpt: bounded_excerpt(
                        unit.selectors()
                            .iter()
                            .map(|selector| selector.text())
                            .collect::<Vec<_>>()
                            .join("\n"),
                    ),
                })
                .collect(),
            relations: proposal
                .relations()
                .iter()
                .map(|relation| relation.proposal().clone())
                .collect(),
            references: proposal
                .references()
                .iter()
                .map(|reference| reference.proposal().clone())
                .collect(),
            issues: proposal
                .issues()
                .iter()
                .map(|issue| issue.proposal().clone())
                .collect(),
            coverage: proposal
                .coverage()
                .iter()
                .map(|coverage| coverage.proposal().clone())
                .collect(),
        }
    }
}

impl DesktopDiscoveryRun {
    fn from_summary(run: DiscoveryRunSummary, policy: EffectiveDiscoveryPolicy) -> Self {
        Self {
            task_kind: run.task_kind,
            task_version: run.task_version,
            input_fingerprint: run.input_fingerprint,
            prompt_fingerprint: run.prompt_fingerprint,
            profile_id: run.profile_id,
            provider: run.provider,
            requested_model: run.requested_model,
            admission: policy.admission,
            request_timeout_seconds: policy.request_timeout.as_secs(),
            connect_timeout_seconds: policy.connect_timeout.as_secs(),
            response: run.response.map(|response| DesktopDiscoveryResponse {
                provider_model: response.target.provider_model,
                finish_reason: response.finish_reason.as_ref().map(finish_reason),
                input_tokens: response.usage.input_tokens,
                output_tokens: response.usage.output_tokens,
                total_tokens: response.usage.total_tokens,
                response_id: response.response_id,
                elapsed_ms: response.elapsed_ms,
            }),
        }
    }
}

impl From<&DiscoveryDiagnostic> for DesktopDiscoveryDiagnostic {
    fn from(diagnostic: &DiscoveryDiagnostic) -> Self {
        Self {
            code: validation_code(diagnostic.code()),
            path: diagnostic.path().to_owned(),
            message: diagnostic.message().to_owned(),
        }
    }
}

impl From<&SelectorResolutionDiagnostic> for DesktopDiscoveryDiagnostic {
    fn from(diagnostic: &SelectorResolutionDiagnostic) -> Self {
        Self {
            code: selector_resolution_code(diagnostic.code()),
            path: diagnostic.path().to_owned(),
            message: diagnostic.message().to_owned(),
        }
    }
}

fn rejection_kind(rejection: &DiscoveryProposalRejection) -> &'static str {
    match rejection {
        DiscoveryProposalRejection::MaxOutputTokens => "max_output_tokens",
        DiscoveryProposalRejection::ContentFiltered => "content_filtered",
        DiscoveryProposalRejection::UnusableFinishReason(_) => "unusable_finish_reason",
        DiscoveryProposalRejection::UnexpectedToolCall => "unexpected_tool_call",
        DiscoveryProposalRejection::MissingText => "missing_text",
        DiscoveryProposalRejection::ResponseTooLarge { .. } => "response_too_large",
        DiscoveryProposalRejection::InvalidJson { .. } => "invalid_json",
        DiscoveryProposalRejection::NonCanonicalJson => "non_canonical_json",
        DiscoveryProposalRejection::SelectorResolution(_) => "selector_resolution",
        DiscoveryProposalRejection::Validation(_) => "deterministic_validation",
    }
}

fn selector_resolution_code(code: SelectorResolutionCode) -> &'static str {
    match code {
        SelectorResolutionCode::UnknownBlockRef => "unknown_block_ref",
        SelectorResolutionCode::QuoteNotFound => "quote_not_found",
        SelectorResolutionCode::AmbiguousQuote => "ambiguous_quote",
        SelectorResolutionCode::InvalidDisambiguator => "invalid_disambiguator",
    }
}

fn llm_error_kind(kind: LlmErrorKind) -> &'static str {
    match kind {
        LlmErrorKind::Configuration => "configuration",
        LlmErrorKind::InvalidRequest => "invalid_request",
        LlmErrorKind::Credential => "credential",
        LlmErrorKind::Authentication => "authentication",
        LlmErrorKind::RateLimited => "rate_limited",
        LlmErrorKind::Timeout => "timeout",
        LlmErrorKind::Transport => "transport",
        LlmErrorKind::Provider => "provider",
        LlmErrorKind::InvalidResponse => "invalid_response",
        LlmErrorKind::UnsupportedCapability => "unsupported_capability",
        LlmErrorKind::Exhausted => "exhausted",
    }
}

fn finish_reason(reason: &LlmFinishReason) -> &'static str {
    match reason {
        LlmFinishReason::Completed { .. } => "completed",
        LlmFinishReason::MaxTokens { .. } => "max_tokens",
        LlmFinishReason::ToolCall { .. } => "tool_call",
        LlmFinishReason::ContentFilter { .. } => "content_filter",
        LlmFinishReason::StopSequence { .. } => "stop_sequence",
        LlmFinishReason::Other { .. } => "other",
    }
}

fn validation_code(code: DiscoveryValidationCode) -> &'static str {
    match code {
        DiscoveryValidationCode::InputVersion => "input_version",
        DiscoveryValidationCode::InputIntegrity => "input_integrity",
        DiscoveryValidationCode::FingerprintMismatch => "fingerprint_mismatch",
        DiscoveryValidationCode::LimitExceeded => "limit_exceeded",
        DiscoveryValidationCode::InvalidValue => "invalid_value",
        DiscoveryValidationCode::DuplicateId => "duplicate_id",
        DiscoveryValidationCode::UnknownUnit => "unknown_unit",
        DiscoveryValidationCode::ParentCycle => "parent_cycle",
        DiscoveryValidationCode::InvalidSelector => "invalid_selector",
        DiscoveryValidationCode::InvalidRelation => "invalid_relation",
        DiscoveryValidationCode::InvalidReference => "invalid_reference",
        DiscoveryValidationCode::InvalidEvidence => "invalid_evidence",
        DiscoveryValidationCode::InvalidCompleteness => "invalid_completeness",
        DiscoveryValidationCode::OverlappingUnits => "overlapping_units",
        DiscoveryValidationCode::InvalidCoverage => "invalid_coverage",
        DiscoveryValidationCode::CoverageGap => "coverage_gap",
    }
}

fn bounded_excerpt(value: String) -> String {
    const MAX_CHARACTERS: usize = 480;
    if value.chars().count() <= MAX_CHARACTERS {
        return value;
    }
    let mut excerpt = value.chars().take(MAX_CHARACTERS).collect::<String>();
    excerpt.push('…');
    excerpt
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn timeout_failure_reports_and_records_the_effective_policy() {
        let hash = ContentHash::parse(format!("sha256:{}", "1".repeat(64))).unwrap();
        let run = DiscoveryRunSummary {
            task_kind: "source_discovery".into(),
            task_version: "1".into(),
            input_fingerprint: hash.clone(),
            prompt_fingerprint: hash,
            profile_id: "desktop-default".into(),
            provider: "test".into(),
            requested_model: "test-model".into(),
            response: None,
        };
        let policy = EffectiveDiscoveryPolicy {
            admission: DiscoveryAdmission::ReviewBeforePublish,
            request_timeout: Duration::from_secs(600),
            connect_timeout: Duration::from_secs(15),
        };

        let outcome = desktop_outcome(
            DiscoveryProductionOutcome::Failed {
                error: LlmError::Timeout,
                run,
            },
            policy,
        );
        let value = serde_json::to_value(outcome).unwrap();
        assert_eq!(value["outcome"], "failed");
        assert_eq!(value["error_kind"], "timeout");
        assert!(
            value["message"]
                .as_str()
                .unwrap()
                .contains("请求总超时 600 秒")
        );
        assert_eq!(value["run"]["request_timeout_seconds"], 600);
        assert_eq!(value["run"]["connect_timeout_seconds"], 15);
        assert_eq!(value["run"]["admission"], "review_before_publish");
    }
}

#[tauri::command]
pub(crate) fn get_active_discovery(state: State<'_, DesktopState>) -> Option<ActiveDiscoveryTask> {
    state.service.active_discovery()
}

#[tauri::command]
pub(crate) async fn run_discovery(
    state: State<'_, DesktopState>,
    request: RunDiscoveryRequest,
    on_progress: tauri::ipc::Channel<serde_json::Value>,
) -> Result<DesktopDiscoveryOutcome, String> {
    state
        .service
        .run_discovery_observed(
            request,
            Some(Arc::new(move |kind: &str, text: &str| {
                let _ = on_progress.send(serde_json::json!({"kind": kind, "text": text}));
            })),
        )
        .await
        .map_err(|error| error.to_string())
}
