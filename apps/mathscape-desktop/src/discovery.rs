use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use mathscape_domain::{
    ActorRef, AnchoredMarkdown, ContentHash, DISCOVERY_PIPELINE_VERSION, DiscoveryAdmission,
    DiscoveryInput, DiscoverySelectionRequest, FactTaskProposal, MarkdownPatchSet,
    MarkdownPatchStatus, SourceReviewAcceptanceState, StableId, Timestamp, UnitDiscoveryCheckpoint,
};
use mathscape_llm::{
    GenaiBackend, GenaiBackendOptions, LLM_PROFILE_VERSION, LlmError, LlmProfile, LlmResponse,
    ScriptedLlmBackend, WireProtocol,
};
use mathscape_orchestrator::{
    AcceptanceAuthority, AcceptanceSelection, DiscoveryAcceptanceDecision, DiscoveryRunSummary,
    DiscoveryWorkflow, DiscoveryWorkflowEvent, DiscoveryWorkflowLimits, MarkdownPatchGate,
    MarkdownPatchWorkflowState, PatchReviewCheckpoint, PatchReviewDecision,
    PreparedUnitDiscoveryRequest, SourceRegistrationPlan, WorkflowAttempt, WorkflowStatus,
    acceptance_decision_id, discovery_result_fingerprint, materialize_source_registration,
    plan_source_registration, prepare_unit_discovery_request_from_input, project_source_review,
    rebind_discovery_input, revalidate_discovery_checkpoint,
};
use mathscape_store_sqlite::{
    AcceptedRegistrationPlanRecord, DiscoveryAttemptCheckpoint, DiscoveryCheckpointStatus,
    DiscoveryDiagnosticCheckpoint, DiscoveryFactCheckpointRecord, DiscoveryRunCheckpoint,
    DiscoveryRunDispositionReceipt, DiscoveryUnitCheckpointRecord, DiscoveryValidationCheckpoint,
    PendingDiscoveryTask, SourceRegistrationReceipt, SqliteArtifactStore, SqliteStoreError,
    StoredAcceptanceAuthority, StoredDiscoveryRunSummary,
};
use serde::{Deserialize, Serialize};
use tauri::State;
use thiserror::Error;
use tokio::sync::watch;

use crate::{
    DesktopService, DesktopState,
    discovery_view::DesktopDiscoveryView,
    llm_settings::LlmSettingsError,
    workspace_settings::{EffectiveDiscoveryPolicy, WorkspaceSettingsError},
};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunDiscoveryRequest {
    selection: DiscoverySelectionRequest,
    profile_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptDiscoveryRequest {
    run_id: String,
    expected_result_fingerprint: ContentHash,
    selection: AcceptanceSelection,
}

#[derive(Clone, Debug, Serialize)]
pub struct DesktopDiscoveryAcceptance {
    state: String,
    decision_id: String,
    run_id: String,
    result_fingerprint: ContentHash,
    plan_fingerprint: ContentHash,
    selected_units: Vec<String>,
    partial: bool,
    span_count: usize,
    unit_count: usize,
    reference_count: usize,
    relation_count: usize,
    pending_task_count: usize,
    already_accepted: bool,
    commit_id: String,
    created_object_count: usize,
    reused_object_count: usize,
    already_registered: bool,
    registration_plan: Option<mathscape_domain::SourceRegistrationCommitPlan>,
    reactivated_object_count: usize,
    revised_object_count: usize,
}

impl RunDiscoveryRequest {
    pub fn new(selection: DiscoverySelectionRequest, profile_id: Option<String>) -> Self {
        Self {
            selection,
            profile_id,
        }
    }
}

/// Continues a run paused for Markdown patch review. The reviewer decision
/// must already be recorded by `submit_markdown_patch_decision`; the run keeps
/// its profile, admission and spent call/token budget.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResumeDiscoveryRequest {
    run_id: String,
}

impl ResumeDiscoveryRequest {
    pub fn new(run_id: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActiveDiscoveryPhase {
    Starting,
    DiscoveringUnits,
    ValidatingUnits,
    ReviewingPatches,
    AwaitingPatchReview,
    ApplyingPatches,
    RebuildingUnits,
    PlanningFacts,
    DiscoveringFacts,
    ValidatingFacts,
    Repairing,
    Assembling,
    Cancelling,
    Finishing,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ActiveDiscoveryTask {
    artifact_id: String,
    input_fingerprint: ContentHash,
    profile_id: String,
    request_timeout_seconds: u64,
    phase: ActiveDiscoveryPhase,
    attempt: usize,
    batch_index: Option<usize>,
    batch_total: Option<usize>,
    unit_count: usize,
    completed_tasks: usize,
    total_tasks: usize,
    cancellation_requested: bool,
}

impl ActiveDiscoveryTask {
    pub fn artifact_id(&self) -> &str {
        &self.artifact_id
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ActiveDiscoveryControl {
    task: ActiveDiscoveryTask,
    cancel: watch::Sender<bool>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DesktopDiscoveryProgress {
    Started {
        profile: String,
        model: String,
    },
    WorkflowEvent {
        event: DiscoveryWorkflowEvent,
    },
    StreamDelta {
        attempt: usize,
        stage: String,
        batch_index: Option<usize>,
        batch_total: Option<usize>,
        stream: String,
        text: String,
    },
    ResponseCompleted {
        attempt: usize,
        stage: String,
        batch_index: Option<usize>,
        batch_total: Option<usize>,
    },
}

pub type DesktopDiscoveryObserver = Arc<dyn Fn(DesktopDiscoveryProgress) + Send + Sync>;

/// Stable Desktop envelope. Its view is a read-only projection and deliberately
/// excludes prompts, authorized source text and raw provider responses.
#[derive(Debug, Serialize)]
pub struct DesktopDiscoveryOutcome {
    outcome: WorkflowStatus,
    view: DesktopDiscoveryView,
    review: Option<mathscape_domain::SourceReviewProjection>,
    run: DesktopDiscoveryRun,
}

#[derive(Debug, Serialize)]
pub struct DesktopDiscoveryRunSummary {
    run_id: String,
    artifact_id: String,
    artifact_name: Option<String>,
    conversion_id: Option<String>,
    input_fingerprint: ContentHash,
    result_fingerprint: Option<ContentHash>,
    pipeline_version: String,
    status: DiscoveryCheckpointStatus,
    review_status: String,
    registrations: Vec<mathscape_domain::RegistrationSummary>,
    disposition: Option<DiscoveryRunDispositionReceipt>,
    profile_id: String,
    unit_count: usize,
    fact_count: usize,
    pending_task_count: usize,
    unresolved_reference_count: usize,
    relation_count: usize,
    created_at: String,
    updated_at: String,
}

/// Safe, replay-independent diagnostics for a run that cannot be opened as a
/// source-review projection. Raw prompts and provider responses are omitted;
/// the persisted checkpoint remains the audit source of truth.
#[derive(Debug, Serialize)]
pub struct DesktopDiscoveryAttemptDiagnostic {
    ordinal: usize,
    stage: String,
    repair: bool,
    status: DiscoveryCheckpointStatus,
    error: Option<String>,
    response_received: bool,
    elapsed_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct DesktopDiscoveryValidationDiagnostic {
    report: usize,
    code: String,
    path: String,
    message: String,
    checked: bool,
}

#[derive(Debug, Serialize)]
pub struct DesktopDiscoveryRunDiagnostics {
    run_id: String,
    status: DiscoveryCheckpointStatus,
    review_status: String,
    result_fingerprint: Option<ContentHash>,
    message: String,
    attempts: Vec<DesktopDiscoveryAttemptDiagnostic>,
    diagnostics: Vec<DesktopDiscoveryValidationDiagnostic>,
    events: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct DesktopDiscoveryReviewEnvelope {
    run_id: String,
    review_status: String,
    created_at: String,
    updated_at: String,
    outcome: DesktopDiscoveryOutcome,
    registrations: Vec<mathscape_domain::RegistrationSummary>,
    disposition: Option<DiscoveryRunDispositionReceipt>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListDiscoveryRunsRequest {
    artifact_id: Option<String>,
}

pub type DiscardDiscoveryRunRequest = mathscape_domain::DiscoveryDispositionRequest;
pub type RestoreDiscoveryReviewRequest = mathscape_domain::DiscoveryDispositionRequest;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DesktopDiscoveryRun {
    run_id: String,
    profile_id: String,
    provider: String,
    requested_model: String,
    admission: DiscoveryAdmission,
    request_timeout_seconds: Option<u64>,
    connect_timeout_seconds: Option<u64>,
    attempts: usize,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
    elapsed_ms: u64,
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
    #[error("内容发现检查点保存失败：{0}")]
    Persistence(#[from] SqliteStoreError),
    #[error("内容发现领域值无效：{0}")]
    Domain(#[from] mathscape_domain::DomainError),
    #[error("内容发现检查点编码失败：{0}")]
    Json(#[from] serde_json::Error),
    #[error("来源审核接受失败：{0}")]
    Acceptance(String),
    #[error("内容发现检查点无效：{0}")]
    Checkpoint(String),
    #[error("Markdown 修订后继续内容发现失败：{0}")]
    Resume(String),
}

enum DiscoveryStart {
    Fresh(PreparedUnitDiscoveryRequest),
    Resume(Box<PatchResumePlan>),
}

struct PatchResumePlan {
    prepared: PreparedUnitDiscoveryRequest,
    checkpoint: PatchReviewCheckpoint,
    decision: PatchResumeDecision,
}

enum PatchResumeDecision {
    RejectAll,
    /// Accepted patches were written as Markdown errata on the unchanged
    /// physical snapshot; B continues on the original input.
    AppliedAsErrata {
        accepted_patch_refs: Vec<String>,
    },
    /// Legacy replay: an approval that promoted a separate patched artifact.
    Apply {
        base_document: AnchoredMarkdown,
        selected: MarkdownPatchSet,
        /// The promoted artifact as read back from the Artifact Pool.
        patched_document: AnchoredMarkdown,
    },
}

struct DiscoveryExecution {
    run_id: StableId,
    start: DiscoveryStart,
    profile: LlmProfile,
    policy: EffectiveDiscoveryPolicy,
}

impl DiscoveryExecution {
    fn base_input(&self) -> &DiscoveryInput {
        match &self.start {
            DiscoveryStart::Fresh(prepared) => prepared.input(),
            DiscoveryStart::Resume(plan) => plan.prepared.input(),
        }
    }
}

#[derive(Clone, Debug, Default)]
struct StreamContext {
    attempt: usize,
    stage: String,
    batch_index: Option<usize>,
    batch_total: Option<usize>,
}

impl DesktopService {
    pub fn list_discovery_runs(
        &self,
        artifact_id: Option<&str>,
    ) -> Result<Vec<DesktopDiscoveryRunSummary>, DesktopDiscoveryError> {
        let artifact_id = artifact_id
            .map(StableId::parse)
            .transpose()
            .map_err(DesktopDiscoveryError::Domain)?;
        let store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))?;
        store
            .list_discovery_run_summaries(artifact_id.as_ref())
            .map(|summaries| summaries.into_iter().map(Into::into).collect())
            .map_err(Into::into)
    }

    pub fn get_discovery_diagnostics(
        &self,
        run_id_text: &str,
    ) -> Result<DesktopDiscoveryRunDiagnostics, DesktopDiscoveryError> {
        let run_id = StableId::parse(run_id_text)?;
        let store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))?;
        let stored = store
            .discovery_checkpoint(&run_id)?
            .ok_or_else(|| DesktopDiscoveryError::Checkpoint("内容发现运行不存在".into()))?;
        let checkpoint = stored.checkpoint;
        let summary = store
            .list_discovery_run_summaries(Some(&checkpoint.artifact_id))?
            .into_iter()
            .find(|summary| summary.run_id == run_id);
        let review_status = summary
            .as_ref()
            .map(|summary| summary.review_status.clone())
            .unwrap_or_else(|| "diagnostic".into());
        let attempts = checkpoint
            .attempts
            .iter()
            .enumerate()
            .map(|(index, attempt)| DesktopDiscoveryAttemptDiagnostic {
                ordinal: index + 1,
                stage: attempt.stage.clone(),
                repair: attempt.repair,
                status: attempt.status,
                error: attempt.error.clone(),
                response_received: attempt
                    .run
                    .get("response")
                    .is_some_and(|response| !response.is_null()),
                elapsed_ms: attempt
                    .run
                    .pointer("/response/elapsed_ms")
                    .and_then(serde_json::Value::as_u64),
            })
            .collect();
        let diagnostics = checkpoint
            .reports
            .iter()
            .enumerate()
            .flat_map(|(report_index, report)| {
                report.diagnostics.iter().map(move |diagnostic| {
                    DesktopDiscoveryValidationDiagnostic {
                        report: report_index + 1,
                        code: diagnostic.code.clone(),
                        path: diagnostic.path.clone(),
                        message: diagnostic.message.clone(),
                        checked: diagnostic.checked,
                    }
                })
            })
            .collect();
        let events = checkpoint
            .snapshot
            .get("events")
            .and_then(serde_json::Value::as_array)
            .map(|events| {
                events
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let message = match checkpoint.status {
            DiscoveryCheckpointStatus::Running => {
                "该运行仍在进行中，以下是最近一次已保存的检查点。".into()
            }
            DiscoveryCheckpointStatus::AwaitingPatchReview => {
                "该运行正在等待 Markdown 修订审核，请从修订审核入口继续。".into()
            }
            DiscoveryCheckpointStatus::NeedsAttention => {
                "该运行需要处理，但尚未形成可恢复的来源审核结果。".into()
            }
            DiscoveryCheckpointStatus::Failed => "该运行失败，未形成可恢复的来源审核结果。".into(),
            DiscoveryCheckpointStatus::Cancelled => {
                "该运行已取消，未形成可恢复的来源审核结果。".into()
            }
            DiscoveryCheckpointStatus::Completed | DiscoveryCheckpointStatus::Partial => {
                "该运行的来源审核结果当前不可恢复，请查看诊断并重新运行。".into()
            }
        };
        Ok(DesktopDiscoveryRunDiagnostics {
            run_id: run_id.to_string(),
            status: checkpoint.status,
            review_status,
            result_fingerprint: checkpoint.result_fingerprint,
            message,
            attempts,
            diagnostics,
            events,
        })
    }

    pub fn get_discovery_review(
        &self,
        run_id_text: &str,
    ) -> Result<DesktopDiscoveryReviewEnvelope, DesktopDiscoveryError> {
        let run_id = StableId::parse(run_id_text)?;
        let store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))?;
        let stored = store
            .discovery_checkpoint(&run_id)?
            .ok_or_else(|| DesktopDiscoveryError::Checkpoint("内容发现运行不存在".into()))?;
        let checkpoint = stored.checkpoint;
        if !matches!(
            checkpoint.status,
            DiscoveryCheckpointStatus::Completed
                | DiscoveryCheckpointStatus::Partial
                | DiscoveryCheckpointStatus::NeedsAttention
        ) || checkpoint.result_fingerprint.is_none()
        {
            return Err(DesktopDiscoveryError::Checkpoint(
                "该运行尚未形成可恢复的来源审核结果；等待 Markdown 修订的运行请使用修订审核入口"
                    .into(),
            ));
        }
        if checkpoint.pipeline_version != DISCOVERY_PIPELINE_VERSION {
            return Err(DesktopDiscoveryError::Checkpoint(format!(
                "内容发现运行使用了不兼容的流水线版本 {}",
                checkpoint.pipeline_version
            )));
        }
        let source: DiscoveryInput = checkpoint_value(&checkpoint.snapshot, "source")?;
        let document = self
            .markdown_document(
                source.snapshot.artifact_id.as_str(),
                source.snapshot.conversion_id.as_ref().map(StableId::as_str),
            )
            .map_err(DesktopDiscoveryError::Checkpoint)?;
        rebind_discovery_input(&document, &source, Default::default())
            .map_err(|error| DesktopDiscoveryError::Checkpoint(error.to_string()))?;
        let prepared = prepare_unit_discovery_request_from_input(source);
        let fact_proposals = checkpoint
            .facts
            .iter()
            .map(|fact| {
                fact.payload
                    .get("original")
                    .cloned()
                    .ok_or_else(|| {
                        DesktopDiscoveryError::Checkpoint(format!(
                            "事实任务 {} 缺少原始提案",
                            fact.task_ref
                        ))
                    })
                    .and_then(|value| {
                        serde_json::from_value::<FactTaskProposal>(value)
                            .map_err(DesktopDiscoveryError::Json)
                    })
            })
            .collect::<Result<Vec<_>, DesktopDiscoveryError>>()?;
        let restored = revalidate_discovery_checkpoint(
            &prepared,
            &checkpoint_value(&checkpoint.snapshot, "units")?,
            fact_proposals,
            DiscoveryWorkflowLimits::default().max_tasks_per_batch,
        )
        .map_err(|error| DesktopDiscoveryError::Checkpoint(error.to_string()))?;
        let reproduced = discovery_result_fingerprint(restored.result());
        if checkpoint.result_fingerprint.as_ref() != Some(&reproduced) {
            return Err(DesktopDiscoveryError::Checkpoint(
                "当前验证器无法重现持久化的内容发现结果".into(),
            ));
        }

        let attempts = checkpoint
            .attempts
            .iter()
            .map(|attempt| {
                Ok(WorkflowAttempt {
                    stage: attempt.stage.clone(),
                    batch: attempt.batch_fingerprint.clone(),
                    repair: attempt.repair,
                    request: serde_json::from_value(attempt.request.clone())?,
                    raw_text: attempt.raw_text.clone(),
                    run: serde_json::from_value(attempt.run.clone())?,
                    status: workflow_status(attempt.status),
                    error: attempt.error.clone(),
                })
            })
            .collect::<Result<Vec<_>, serde_json::Error>>()?;
        let diagnostics = checkpoint
            .reports
            .iter()
            .map(|report| serde_json::from_value(report.report.clone()))
            .collect::<Result<Vec<_>, serde_json::Error>>()?;
        let events: Vec<String> = checkpoint_value(&checkpoint.snapshot, "events")?;
        let patch: MarkdownPatchWorkflowState = checkpoint_value(&checkpoint.snapshot, "patch")?;
        // Recovery is a read-only projection: the saved LLM profile may have
        // been edited or removed since this run. Build a non-executing backend
        // from the attempt's historical identity instead of requiring current
        // credentials or configuration.
        let first_run = attempts
            .first()
            .map(|attempt| &attempt.run)
            .ok_or_else(|| DesktopDiscoveryError::Checkpoint("检查点缺少 LLM 调用记录".into()))?;
        let profile = LlmProfile {
            version: LLM_PROFILE_VERSION.into(),
            id: checkpoint.profile_id.clone(),
            display_name: None,
            provider: first_run.provider.clone(),
            wire_protocol: WireProtocol::OpenAiChatCompletions,
            model: first_run.requested_model.clone(),
            endpoint: None,
            credential: None,
            capabilities: Default::default(),
        };
        let backend = ScriptedLlmBackend::new(
            profile.clone(),
            std::iter::empty::<Result<LlmResponse, LlmError>>(),
        );
        // Historical timeout settings were not persisted by older runs. Use
        // placeholders internally and serialize them as unknown below.
        let policy = EffectiveDiscoveryPolicy {
            admission: checkpoint.admission,
            request_timeout: Duration::ZERO,
            connect_timeout: Duration::ZERO,
        };
        let workflow = DiscoveryWorkflow::restore_from_revalidated(
            &backend,
            DiscoveryWorkflowLimits {
                timeout: policy.request_timeout,
                ..Default::default()
            },
            run_id.clone(),
            &prepared,
            restored,
            attempts,
            diagnostics,
            events,
            patch,
            workflow_status(checkpoint.status),
        )
        .map_err(DesktopDiscoveryError::Checkpoint)?;
        let view = DesktopDiscoveryView::project(&workflow);
        let summary = store
            .list_discovery_run_summaries(Some(&checkpoint.artifact_id))?
            .into_iter()
            .find(|summary| summary.run_id == run_id)
            .ok_or_else(|| DesktopDiscoveryError::Checkpoint("运行摘要不存在".into()))?;
        let mut review = project_source_review(&workflow, run_id.clone());
        if let Some(review) = &mut review {
            match summary.review_status.as_str() {
                "accepted" => review.acceptance.state = SourceReviewAcceptanceState::Accepted,
                "registered" => review.acceptance.state = SourceReviewAcceptanceState::Registered,
                "withdrawn" => {
                    review.acceptance.state = SourceReviewAcceptanceState::Blocked;
                    review
                        .units
                        .iter_mut()
                        .for_each(|unit| unit.acceptance = SourceReviewAcceptanceState::Blocked);
                }
                "discarded" => {
                    review.acceptance.state = SourceReviewAcceptanceState::Blocked;
                    review.acceptance.blockers = review
                        .units
                        .iter()
                        .map(|unit| unit.unit_ref.clone())
                        .collect();
                    review.units.iter_mut().for_each(|unit| {
                        unit.acceptance = SourceReviewAcceptanceState::Blocked;
                    });
                }
                _ => {}
            }
        }
        let mut outcome = DesktopDiscoveryOutcome::from_workflow(
            view,
            review,
            workflow.attempts(),
            run_id.clone(),
            &profile,
            policy,
        );
        outcome.run.request_timeout_seconds = None;
        outcome.run.connect_timeout_seconds = None;
        Ok(DesktopDiscoveryReviewEnvelope {
            run_id: run_id.to_string(),
            review_status: summary.review_status,
            created_at: summary.created_at,
            updated_at: summary.updated_at,
            outcome,
            registrations: summary.registrations,
            disposition: summary.disposition,
        })
    }

    pub fn discard_discovery_run(
        &self,
        request: DiscardDiscoveryRunRequest,
    ) -> Result<DiscoveryRunDispositionReceipt, DesktopDiscoveryError> {
        let actor = ActorRef::parse("mathscape-desktop-user")?;
        let mut store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))?;
        store
            .discard_discovery_run_checked(&request, &actor, &timestamp_now()?)
            .map_err(Into::into)
    }

    pub fn restore_discovery_review(
        &self,
        request: RestoreDiscoveryReviewRequest,
    ) -> Result<DiscoveryRunDispositionReceipt, DesktopDiscoveryError> {
        let actor = ActorRef::parse("mathscape-desktop-user")?;
        let mut store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))?;
        if let Some(receipt) = store.replay_discovery_restore(&request, &actor)? {
            return Ok(receipt);
        }
        let checkpoint = store
            .discovery_checkpoint(&request.run_id)?
            .ok_or_else(|| DesktopDiscoveryError::Checkpoint("内容发现运行不存在".into()))?
            .checkpoint;
        if checkpoint.result_fingerprint.as_ref() != Some(&request.expected_result_fingerprint) {
            return Err(DesktopDiscoveryError::Checkpoint(
                "结果已变化，请刷新记录".into(),
            ));
        }
        // Read-only deterministic replay; no LLM, credentials or map writes.
        // The writer rechecks this complete checkpoint under its lock.
        self.get_discovery_review(request.run_id.as_str())?;
        store
            .restore_discovery_review(&request, &actor, &timestamp_now()?, &checkpoint)
            .map_err(Into::into)
    }

    pub fn active_discovery(&self) -> Option<ActiveDiscoveryTask> {
        self.active_discovery
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .map(|control| control.task.clone())
    }

    pub fn cancel_discovery(&self) -> bool {
        let mut active = self
            .active_discovery
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(control) = active.as_mut() else {
            return false;
        };
        if control.task.phase == ActiveDiscoveryPhase::Finishing {
            return false;
        }
        control.task.phase = ActiveDiscoveryPhase::Cancelling;
        control.task.cancellation_requested = true;
        let _ = control.cancel.send(true);
        true
    }

    pub async fn run_discovery(
        &self,
        request: RunDiscoveryRequest,
    ) -> Result<DesktopDiscoveryOutcome, DesktopDiscoveryError> {
        self.run_discovery_observed(request, None).await
    }

    pub async fn resume_discovery_after_patch(
        &self,
        request: ResumeDiscoveryRequest,
    ) -> Result<DesktopDiscoveryOutcome, DesktopDiscoveryError> {
        self.resume_discovery_after_patch_observed(request, None)
            .await
    }

    pub fn accept_discovery_result(
        &self,
        request: AcceptDiscoveryRequest,
    ) -> Result<DesktopDiscoveryAcceptance, DesktopDiscoveryError> {
        let run_id = StableId::parse(&request.run_id)?;
        let actor = ActorRef::parse("mathscape-desktop-user")?;
        let mut store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))?;
        let stored = store
            .discovery_checkpoint(&run_id)?
            .ok_or_else(|| DesktopDiscoveryError::Acceptance("内容发现运行不存在".into()))?;
        let checkpoint = stored.checkpoint;
        if store.discovery_run_is_discarded(&run_id)? {
            return Err(DesktopDiscoveryError::Acceptance(
                "该内容发现结果已被放弃，不能再次接受；请重新运行内容发现".into(),
            ));
        }
        // A replay of an already registered decision is historical, not re-adoption.
        // It must remain readable even if a newer validator cannot replay the run.
        let replay_id = acceptance_decision_id(
            &run_id,
            &request.expected_result_fingerprint,
            &actor,
            &request.selection,
        )
        .map_err(|e| DesktopDiscoveryError::Acceptance(e.to_string()))?;
        if let (Some(record), Some(receipt)) = (
            store.registration_plan(&replay_id)?,
            store.source_registration_receipt(&replay_id)?,
        ) {
            let withdrawn = store
                .discovery_registration_reversal(replay_id.as_str())?
                .is_some();
            let mut result =
                DesktopDiscoveryAcceptance::from_registration(record, receipt, true, true)?;
            if withdrawn {
                result.state = "withdrawn".into();
            }
            return Ok(result);
        }
        if checkpoint.pipeline_version != DISCOVERY_PIPELINE_VERSION {
            return Err(DesktopDiscoveryError::Acceptance(format!(
                "内容发现运行使用了不兼容的流水线版本 {}",
                checkpoint.pipeline_version
            )));
        }
        if checkpoint.result_fingerprint.as_ref() != Some(&request.expected_result_fingerprint) {
            return Err(DesktopDiscoveryError::Acceptance(
                "审核页面已经过期，请重新打开最新结果".into(),
            ));
        }

        let source: DiscoveryInput = checkpoint_value(&checkpoint.snapshot, "source")?;
        let unit_checkpoint: UnitDiscoveryCheckpoint =
            checkpoint_value(&checkpoint.snapshot, "units")?;
        if source.snapshot.artifact_id != checkpoint.artifact_id
            || source.snapshot.conversion_id != checkpoint.conversion_id
            || source.fingerprint != checkpoint.input_fingerprint
            || unit_checkpoint.input_fingerprint != checkpoint.input_fingerprint
            || checkpoint.unit_set_fingerprint.as_ref()
                != Some(&unit_checkpoint.unit_set_fingerprint)
        {
            return Err(DesktopDiscoveryError::Acceptance(
                "持久化检查点的来源身份不一致".into(),
            ));
        }
        let document = self
            .markdown_document(
                source.snapshot.artifact_id.as_str(),
                source.snapshot.conversion_id.as_ref().map(StableId::as_str),
            )
            .map_err(DesktopDiscoveryError::Acceptance)?;
        rebind_discovery_input(&document, &source, Default::default())
            .map_err(|error| DesktopDiscoveryError::Acceptance(error.to_string()))?;
        let prepared = prepare_unit_discovery_request_from_input(source);
        let fact_proposals = checkpoint
            .facts
            .iter()
            .map(|fact| {
                let value = fact.payload.get("original").cloned().ok_or_else(|| {
                    DesktopDiscoveryError::Acceptance(format!(
                        "事实任务 {} 缺少原始提案",
                        fact.task_ref
                    ))
                })?;
                serde_json::from_value::<FactTaskProposal>(value).map_err(Into::into)
            })
            .collect::<Result<Vec<_>, DesktopDiscoveryError>>()?;
        let revalidated = revalidate_discovery_checkpoint(
            &prepared,
            &unit_checkpoint,
            fact_proposals,
            DiscoveryWorkflowLimits::default().max_tasks_per_batch,
        )
        .map_err(|error| DesktopDiscoveryError::Acceptance(error.to_string()))?;
        let reproduced = discovery_result_fingerprint(revalidated.result());
        if reproduced != request.expected_result_fingerprint {
            return Err(DesktopDiscoveryError::Acceptance(
                "当前验证器无法重现待审核结果，请重新运行内容发现".into(),
            ));
        }

        let decision_id = acceptance_decision_id(&run_id, &reproduced, &actor, &request.selection)
            .map_err(|error| DesktopDiscoveryError::Acceptance(error.to_string()))?;
        let (record, already_accepted) = match store.registration_plan(&decision_id)? {
            Some(existing) => (existing, true),
            None => {
                let decision = DiscoveryAcceptanceDecision {
                    decision_id: decision_id.clone(),
                    discovery_run_id: run_id.clone(),
                    actor: actor.clone(),
                    decided_at: timestamp_now()?,
                    authority: AcceptanceAuthority::HumanReview,
                    expected_result_fingerprint: reproduced.clone(),
                    selection: request.selection,
                };
                let plan = plan_source_registration(
                    prepared.input(),
                    revalidated.units(),
                    revalidated.result(),
                    decision,
                )
                .map_err(|error| DesktopDiscoveryError::Acceptance(error.to_string()))?;
                let record = AcceptedRegistrationPlanRecord {
                    decision_id,
                    run_id,
                    actor: actor.clone(),
                    decided_at: plan.decision.decided_at.clone(),
                    authority: StoredAcceptanceAuthority::HumanReview,
                    expected_result_fingerprint: reproduced,
                    selection: serde_json::to_value(&plan.decision.selection)?,
                    plan_fingerprint: plan.plan_fingerprint.clone(),
                    plan: serde_json::to_value(&plan)?,
                };
                store.save_registration_plan(&record)?;
                (record, false)
            }
        };
        let plan: SourceRegistrationPlan = serde_json::from_value(record.plan.clone())?;
        let patch = materialize_source_registration(&plan)
            .map_err(|error| DesktopDiscoveryError::Acceptance(error.to_string()))?;
        let already_registered = store
            .source_registration_receipt(&record.decision_id)?
            .is_some();
        let bound_request = mathscape_domain::SourceRegistrationCommitRequest {
            request_id: format!(
                "mathscape:source-registration/{}",
                timestamp_now()?.as_str()
            ),
            decision_id: record.decision_id.to_string(),
            acceptance_plan_fingerprint: record.plan_fingerprint.to_string(),
            base_commit: store.map_edit_snapshot()?.base_commit,
            replacement_approvals: vec![],
        };
        let bound = mathscape_orchestrator::registration::plan_registration_commit(
            &store.registration_snapshot()?,
            &bound_request,
            &patch,
            &actor,
        );
        if bound.plan.requires_confirmation
            || bound.plan.impact.status != mathscape_domain::ImpactPlanStatus::Ready
        {
            return DesktopDiscoveryAcceptance::from_pending(record, bound.plan, already_accepted);
        }
        let receipt = store.commit_bound_source_registration(
            &bound_request,
            &bound.plan.impact.plan_fingerprint,
            &actor,
            timestamp_now()?,
            |snapshot| {
                let e = mathscape_orchestrator::registration::plan_registration_commit(
                    snapshot,
                    &bound_request,
                    &patch,
                    &actor,
                );
                (e.plan, e.patch)
            },
        )?;
        DesktopDiscoveryAcceptance::from_registration(
            record,
            receipt,
            already_accepted,
            already_registered,
        )
    }

    async fn run_discovery_observed(
        &self,
        request: RunDiscoveryRequest,
        observer: Option<DesktopDiscoveryObserver>,
    ) -> Result<DesktopDiscoveryOutcome, DesktopDiscoveryError> {
        let service = self.clone();
        let execution = tauri::async_runtime::spawn_blocking(move || {
            service.prepare_discovery_execution(request)
        })
        .await
        .map_err(|error| DesktopDiscoveryError::Worker(error.to_string()))??;
        self.execute_discovery(execution, observer).await
    }

    async fn resume_discovery_after_patch_observed(
        &self,
        request: ResumeDiscoveryRequest,
        observer: Option<DesktopDiscoveryObserver>,
    ) -> Result<DesktopDiscoveryOutcome, DesktopDiscoveryError> {
        let service = self.clone();
        let execution =
            tauri::async_runtime::spawn_blocking(move || service.prepare_patch_resume(request))
                .await
                .map_err(|error| DesktopDiscoveryError::Worker(error.to_string()))??;
        self.execute_discovery(execution, observer).await
    }

    async fn execute_discovery(
        &self,
        execution: DiscoveryExecution,
        observer: Option<DesktopDiscoveryObserver>,
    ) -> Result<DesktopDiscoveryOutcome, DesktopDiscoveryError> {
        let (cancel, mut cancelled) = watch::channel(false);
        let task = ActiveDiscoveryTask {
            artifact_id: execution.base_input().snapshot.artifact_id.to_string(),
            input_fingerprint: execution.base_input().fingerprint.clone(),
            profile_id: execution.profile.id.clone(),
            request_timeout_seconds: execution.policy.request_timeout.as_secs(),
            phase: match execution.start {
                DiscoveryStart::Fresh(_) => ActiveDiscoveryPhase::Starting,
                DiscoveryStart::Resume(_) => ActiveDiscoveryPhase::ApplyingPatches,
            },
            attempt: 0,
            batch_index: None,
            batch_total: None,
            unit_count: 0,
            completed_tasks: 0,
            total_tasks: 0,
            cancellation_requested: false,
        };
        let _permit = self.begin_discovery(task, cancel)?;
        let DiscoveryExecution {
            run_id,
            start,
            profile,
            policy,
        } = execution;

        if let Some(observer) = &observer {
            observer(DesktopDiscoveryProgress::Started {
                profile: profile
                    .display_name
                    .clone()
                    .unwrap_or_else(|| profile.id.clone()),
                model: profile.model.clone(),
            });
        }

        let stream_context = Arc::new(Mutex::new(StreamContext::default()));
        let credentials: Arc<dyn mathscape_llm::CredentialResolver> = self.credentials.clone();
        let mut backend = GenaiBackend::new(
            profile.clone(),
            credentials,
            GenaiBackendOptions {
                timeout: policy.request_timeout,
                connect_timeout: policy.connect_timeout,
            },
        )?;
        if let Some(progress) = observer.clone() {
            let context = stream_context.clone();
            backend = backend.with_stream_observer(Arc::new(move |kind: &str, text: &str| {
                let current = context
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .clone();
                if kind == "validating" {
                    progress(DesktopDiscoveryProgress::ResponseCompleted {
                        attempt: current.attempt,
                        stage: current.stage,
                        batch_index: current.batch_index,
                        batch_total: current.batch_total,
                    });
                } else {
                    progress(DesktopDiscoveryProgress::StreamDelta {
                        attempt: current.attempt,
                        stage: current.stage,
                        batch_index: current.batch_index,
                        batch_total: current.batch_total,
                        stream: kind.to_owned(),
                        text: text.to_owned(),
                    });
                }
            }));
        }

        let active = self.active_discovery.clone();
        let context = stream_context;
        let progress = observer;
        let workflow_observer = Arc::new(move |event: DiscoveryWorkflowEvent| {
            if let DiscoveryWorkflowEvent::AttemptStarted {
                attempt,
                stage,
                batch_index,
                batch_total,
                ..
            } = &event
            {
                *context.lock().unwrap_or_else(|error| error.into_inner()) = StreamContext {
                    attempt: *attempt,
                    stage: stage.clone(),
                    batch_index: *batch_index,
                    batch_total: *batch_total,
                };
            }
            update_active_task(&active, &event);
            if let Some(progress) = &progress {
                progress(DesktopDiscoveryProgress::WorkflowEvent {
                    event: event.clone(),
                });
            }
        });

        let limits = DiscoveryWorkflowLimits {
            timeout: policy.request_timeout,
            ..Default::default()
        };
        let excluded = {
            let store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))?;
            let artifact_id = match &start {
                DiscoveryStart::Fresh(prepared) => prepared.input().snapshot.artifact_id.clone(),
                DiscoveryStart::Resume(plan) => plan.prepared.input().snapshot.artifact_id.clone(),
            };
            crate::markdown_patch::errata_excluded_ranges(&store, &artifact_id)?
        };
        let mut workflow = DiscoveryWorkflow::new(&backend, limits)
            .with_observer(workflow_observer)
            .with_patch_gate(MarkdownPatchGate::ReviewBeforeApply)
            .with_patch_excluded_ranges(excluded)
            .with_run_id(run_id.clone());
        let prepared = match start {
            DiscoveryStart::Fresh(prepared) => prepared,
            DiscoveryStart::Resume(plan) => {
                let PatchResumePlan {
                    prepared,
                    checkpoint,
                    decision,
                } = *plan;
                let resume = |error: mathscape_orchestrator::PatchReviewError| {
                    DesktopDiscoveryError::Resume(error.to_string())
                };
                workflow = workflow
                    .continue_from(&prepared, checkpoint)
                    .map_err(resume)?;
                match decision {
                    PatchResumeDecision::RejectAll => {
                        workflow
                            .resolve_patch_review(PatchReviewDecision::RejectAll)
                            .map_err(resume)?;
                        prepared
                    }
                    PatchResumeDecision::AppliedAsErrata {
                        accepted_patch_refs,
                    } => {
                        workflow
                            .resolve_patch_review(PatchReviewDecision::AppliedAsErrata {
                                accepted_patch_refs,
                            })
                            .map_err(resume)?;
                        prepared
                    }
                    PatchResumeDecision::Apply {
                        base_document,
                        selected,
                        patched_document,
                    } => {
                        let continuation = workflow
                            .prepare_patch_continuation(&base_document, &selected)
                            .map_err(resume)?;
                        if continuation.document().snapshot != patched_document.snapshot {
                            return Err(DesktopDiscoveryError::Resume(
                                "已登记的修订 Markdown 与审核通过的 patch 不一致".into(),
                            ));
                        }
                        let next = continuation.prepared();
                        workflow
                            .resolve_patch_review(PatchReviewDecision::Apply(Box::new(
                                continuation,
                            )))
                            .map_err(resume)?;
                        next
                    }
                }
            }
        };
        let completed = tokio::select! {
            status = workflow.run(prepared) => Some(status),
            changed = cancelled.changed() => {
                debug_assert!(changed.is_ok(), "active cancellation sender outlives the workflow");
                None
            }
        };
        if completed.is_none() {
            workflow.cancel();
        }
        let checkpoint = persistence_checkpoint(&workflow, &run_id, &profile, policy.admission)?;
        let mut store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))?;
        if *workflow.status() == WorkflowStatus::AwaitingPatchReview
            && let Some(pending) = workflow.pending_patch()
        {
            let patch_set_id = crate::markdown_patch::patch_set_id(pending)?;
            store.save_discovery_checkpoint_with_patch_set(
                &checkpoint,
                &patch_set_id,
                pending,
                MarkdownPatchStatus::Proposed,
                &timestamp_now()?,
            )?;
        } else {
            store.save_discovery_checkpoint(&checkpoint)?;
        }
        let view = DesktopDiscoveryView::project(&workflow);
        let review = project_source_review(&workflow, run_id.clone());
        Ok(DesktopDiscoveryOutcome::from_workflow(
            view,
            review,
            workflow.attempts(),
            run_id,
            &profile,
            policy,
        ))
    }

    fn prepare_discovery_execution(
        &self,
        request: RunDiscoveryRequest,
    ) -> Result<DiscoveryExecution, DesktopDiscoveryError> {
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
            .prepare_unit_discovery_request(request.selection)
            .map_err(DesktopDiscoveryError::Input)?;
        Ok(DiscoveryExecution {
            run_id: new_discovery_run_id()?,
            start: DiscoveryStart::Fresh(prepared),
            profile,
            policy,
        })
    }

    fn prepare_patch_resume(
        &self,
        request: ResumeDiscoveryRequest,
    ) -> Result<DiscoveryExecution, DesktopDiscoveryError> {
        let resume = DesktopDiscoveryError::Resume;
        let run_id = StableId::parse(&request.run_id)?;
        let store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))?;
        let checkpoint = store
            .discovery_checkpoint(&run_id)?
            .ok_or_else(|| resume("内容发现运行不存在".into()))?
            .checkpoint;
        if checkpoint.status != DiscoveryCheckpointStatus::AwaitingPatchReview {
            return Err(resume("该内容发现运行没有在等待 Markdown 修订审核".into()));
        }
        if checkpoint.pipeline_version != DISCOVERY_PIPELINE_VERSION {
            return Err(resume(format!(
                "内容发现运行使用了不兼容的流水线版本 {}",
                checkpoint.pipeline_version
            )));
        }
        let snapshot = &checkpoint.snapshot;
        let source: DiscoveryInput = checkpoint_value(snapshot, "source")?;
        if source.snapshot.artifact_id != checkpoint.artifact_id
            || source.snapshot.conversion_id != checkpoint.conversion_id
            || source.fingerprint != checkpoint.input_fingerprint
        {
            return Err(resume("持久化检查点的来源身份不一致".into()));
        }
        let patch: MarkdownPatchWorkflowState = checkpoint_value(snapshot, "patch")?;
        let pending = patch
            .pending
            .ok_or_else(|| resume("检查点缺少待审核的 patch set".into()))?;
        let review = PatchReviewCheckpoint {
            units: checkpoint_value(snapshot, "units")?,
            pending,
            attempts: checkpoint_value(snapshot, "attempts")?,
            diagnostics: checkpoint_value(snapshot, "diagnostics")?,
            events: checkpoint_value(snapshot, "events")?,
        };

        let base_document = self
            .markdown_document(
                source.snapshot.artifact_id.as_str(),
                source.snapshot.conversion_id.as_ref().map(StableId::as_str),
            )
            .map_err(resume)?;
        rebind_discovery_input(&base_document, &source, Default::default())
            .map_err(|error| resume(error.to_string()))?;

        let (patch_set_id, value, status) = store
            .latest_markdown_patch_set_json_for_run(&run_id)?
            .ok_or_else(|| resume("尚未提交 Markdown 修订决定".into()))?;
        let decision = match status.as_str() {
            "rejected" => {
                if patch_set_id != crate::markdown_patch::patch_set_id(&review.pending)? {
                    return Err(resume("被拒绝的 patch set 不是本次暂停的审核对象".into()));
                }
                PatchResumeDecision::RejectAll
            }
            "applied" => {
                let selected: MarkdownPatchSet = serde_json::from_value(value)?;
                let application = store
                    .latest_markdown_patch_application(&patch_set_id)?
                    .filter(|application| application.status == "applied")
                    .ok_or_else(|| resume("修订决定缺少应用回执".into()))?;
                if application.patched_artifact_id.as_deref()
                    == Some(source.snapshot.artifact_id.as_str())
                {
                    // Errata mode: the base snapshot is unchanged, the
                    // accepted operations now live in markdown_block_errata.
                    PatchResumeDecision::AppliedAsErrata {
                        accepted_patch_refs: selected
                            .patches
                            .iter()
                            .map(|patch| patch.patch_ref.clone())
                            .collect(),
                    }
                } else {
                    let (Some(patched_artifact_id), Some(conversion_id)) =
                        (application.patched_artifact_id, application.conversion_id)
                    else {
                        return Err(resume("修订应用回执缺少修订后的 Markdown".into()));
                    };
                    let patched_document = self
                        .markdown_document(&patched_artifact_id, Some(&conversion_id))
                        .map_err(resume)?;
                    PatchResumeDecision::Apply {
                        base_document,
                        selected,
                        patched_document,
                    }
                }
            }
            _ => return Err(resume("请先接受或拒绝 Markdown 修订建议".into())),
        };

        let profile = self.selected_llm_profile(Some(&checkpoint.profile_id))?;
        let policy = self
            .workspace_settings()?
            .discovery_policy(checkpoint.admission);
        Ok(DiscoveryExecution {
            run_id,
            start: DiscoveryStart::Resume(Box::new(PatchResumePlan {
                prepared: prepare_unit_discovery_request_from_input(source),
                checkpoint: review,
                decision,
            })),
            profile,
            policy,
        })
    }

    fn begin_discovery(
        &self,
        task: ActiveDiscoveryTask,
        cancel: watch::Sender<bool>,
    ) -> Result<DiscoveryPermit, DesktopDiscoveryError> {
        let mut active = self
            .active_discovery
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if active.is_some() {
            return Err(DesktopDiscoveryError::AlreadyRunning);
        }
        *active = Some(ActiveDiscoveryControl { task, cancel });
        Ok(DiscoveryPermit(self.active_discovery.clone()))
    }
}

fn update_active_task(
    active: &Arc<Mutex<Option<ActiveDiscoveryControl>>>,
    event: &DiscoveryWorkflowEvent,
) {
    let mut active = active.lock().unwrap_or_else(|error| error.into_inner());
    let Some(task) = active.as_mut().map(|control| &mut control.task) else {
        return;
    };
    match event {
        DiscoveryWorkflowEvent::WorkflowStarted => {
            if task.phase == ActiveDiscoveryPhase::Starting {
                task.phase = ActiveDiscoveryPhase::DiscoveringUnits;
            }
        }
        DiscoveryWorkflowEvent::AttemptStarted {
            attempt,
            stage,
            batch_index,
            batch_total,
            repair,
        } => {
            task.attempt = *attempt;
            task.batch_index = *batch_index;
            task.batch_total = *batch_total;
            task.phase = if *repair {
                ActiveDiscoveryPhase::Repairing
            } else if stage == "markdown-patch" {
                ActiveDiscoveryPhase::ReviewingPatches
            } else if stage == "unit-discovery" {
                if task.phase == ActiveDiscoveryPhase::RebuildingUnits {
                    ActiveDiscoveryPhase::RebuildingUnits
                } else {
                    ActiveDiscoveryPhase::DiscoveringUnits
                }
            } else {
                ActiveDiscoveryPhase::DiscoveringFacts
            };
        }
        DiscoveryWorkflowEvent::ValidationStarted { stage, .. } => {
            task.phase = match stage.as_str() {
                "unit-discovery" => ActiveDiscoveryPhase::ValidatingUnits,
                "markdown-patch" => ActiveDiscoveryPhase::ReviewingPatches,
                _ => ActiveDiscoveryPhase::ValidatingFacts,
            };
        }
        DiscoveryWorkflowEvent::PatchReviewRequested { .. } => {
            task.phase = ActiveDiscoveryPhase::AwaitingPatchReview;
        }
        DiscoveryWorkflowEvent::PatchReviewResolved { rebuild_units, .. } => {
            task.phase = if *rebuild_units {
                ActiveDiscoveryPhase::RebuildingUnits
            } else {
                ActiveDiscoveryPhase::PlanningFacts
            };
        }
        DiscoveryWorkflowEvent::UnitCheckpointCreated { units, .. } => {
            task.unit_count = *units;
            task.phase = ActiveDiscoveryPhase::PlanningFacts;
        }
        DiscoveryWorkflowEvent::FactPlanCreated { batches, tasks } => {
            task.batch_total = Some(*batches);
            task.total_tasks = *tasks;
            task.phase = ActiveDiscoveryPhase::DiscoveringFacts;
        }
        DiscoveryWorkflowEvent::FactCheckpointUpdated {
            completed_tasks,
            total_tasks,
        } => {
            task.completed_tasks = *completed_tasks;
            task.total_tasks = *total_tasks;
        }
        DiscoveryWorkflowEvent::AssemblyStarted => {
            task.phase = ActiveDiscoveryPhase::Assembling;
        }
        DiscoveryWorkflowEvent::WorkflowFinished { .. } => {
            task.phase = ActiveDiscoveryPhase::Finishing;
        }
        DiscoveryWorkflowEvent::AttemptFinished { .. } => {}
    }
}

struct DiscoveryPermit(Arc<Mutex<Option<ActiveDiscoveryControl>>>);

impl Drop for DiscoveryPermit {
    fn drop(&mut self) {
        *self.0.lock().unwrap_or_else(|error| error.into_inner()) = None;
    }
}

impl DesktopDiscoveryOutcome {
    fn from_workflow(
        view: DesktopDiscoveryView,
        review: Option<mathscape_domain::SourceReviewProjection>,
        attempts: &[WorkflowAttempt],
        run_id: StableId,
        profile: &LlmProfile,
        policy: EffectiveDiscoveryPolicy,
    ) -> Self {
        let outcome = view.status().clone();
        Self {
            outcome,
            view,
            review,
            run: DesktopDiscoveryRun::from_attempts(run_id, attempts, profile, policy),
        }
    }
}

impl DesktopDiscoveryRun {
    fn from_attempts(
        run_id: StableId,
        attempts: &[WorkflowAttempt],
        profile: &LlmProfile,
        policy: EffectiveDiscoveryPolicy,
    ) -> Self {
        fn total(
            attempts: &[WorkflowAttempt],
            select: impl Fn(&DiscoveryRunSummary) -> Option<u64>,
        ) -> Option<u64> {
            attempts.iter().try_fold(0u64, |sum, attempt| {
                select(&attempt.run).map(|value| sum.saturating_add(value))
            })
        }
        Self {
            run_id: run_id.to_string(),
            profile_id: profile.id.clone(),
            provider: profile.provider.clone(),
            requested_model: profile.model.clone(),
            admission: policy.admission,
            request_timeout_seconds: Some(policy.request_timeout.as_secs()),
            connect_timeout_seconds: Some(policy.connect_timeout.as_secs()),
            attempts: attempts.len(),
            input_tokens: total(attempts, |run| run.response.as_ref()?.usage.input_tokens),
            output_tokens: total(attempts, |run| run.response.as_ref()?.usage.output_tokens),
            total_tokens: total(attempts, |run| run.response.as_ref()?.usage.total_tokens),
            elapsed_ms: attempts
                .iter()
                .filter_map(|attempt| attempt.run.response.as_ref())
                .fold(0u64, |sum, response| {
                    sum.saturating_add(response.elapsed_ms)
                }),
        }
    }
}

impl DesktopDiscoveryAcceptance {
    fn from_registration(
        record: AcceptedRegistrationPlanRecord,
        receipt: SourceRegistrationReceipt,
        already_accepted: bool,
        already_registered: bool,
    ) -> Result<Self, DesktopDiscoveryError> {
        Self::from_parts(
            record,
            Some(receipt),
            None,
            already_accepted,
            already_registered,
        )
    }
    fn from_pending(
        record: AcceptedRegistrationPlanRecord,
        plan: mathscape_domain::SourceRegistrationCommitPlan,
        already: bool,
    ) -> Result<Self, DesktopDiscoveryError> {
        Self::from_parts(record, None, Some(plan), already, false)
    }
    fn from_parts(
        record: AcceptedRegistrationPlanRecord,
        receipt: Option<SourceRegistrationReceipt>,
        registration_plan: Option<mathscape_domain::SourceRegistrationCommitPlan>,
        already_accepted: bool,
        already_registered: bool,
    ) -> Result<Self, DesktopDiscoveryError> {
        let plan = record
            .plan
            .as_object()
            .ok_or_else(|| DesktopDiscoveryError::Acceptance("登记计划不是 JSON 对象".into()))?;
        let array =
            |field: &'static str| -> Result<&Vec<serde_json::Value>, DesktopDiscoveryError> {
                plan.get(field)
                    .and_then(serde_json::Value::as_array)
                    .ok_or_else(|| {
                        DesktopDiscoveryError::Acceptance(format!("登记计划缺少 {field}"))
                    })
            };
        let selected_units = array("units")?
            .iter()
            .map(|unit| {
                unit.get("source_ref")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        DesktopDiscoveryError::Acceptance("登记计划包含无效的来源单元".into())
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            state: if receipt.is_some() {
                "registered"
            } else {
                "registration_review_required"
            }
            .into(),
            decision_id: record.decision_id.to_string(),
            run_id: record.run_id.to_string(),
            result_fingerprint: record.expected_result_fingerprint,
            plan_fingerprint: record.plan_fingerprint,
            selected_units,
            partial: plan
                .get("partial")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(|| DesktopDiscoveryError::Acceptance("登记计划缺少 partial".into()))?,
            span_count: array("spans")?.len(),
            unit_count: array("units")?.len(),
            reference_count: array("references")?.len(),
            relation_count: array("relations")?.len(),
            pending_task_count: array("pending_tasks")?.len(),
            already_accepted,
            commit_id: receipt
                .as_ref()
                .map(|r| r.commit_id.to_string())
                .unwrap_or_default(),
            created_object_count: receipt
                .as_ref()
                .map(|r| r.created_objects.len())
                .unwrap_or(0),
            reused_object_count: receipt
                .as_ref()
                .map(|r| r.reused_objects.len())
                .unwrap_or(0),
            reactivated_object_count: receipt
                .as_ref()
                .map(|r| r.reactivated_objects.len())
                .unwrap_or(0),
            revised_object_count: receipt
                .as_ref()
                .map(|r| r.revised_objects.len())
                .unwrap_or(0),
            registration_plan,
            already_registered,
        })
    }
}

fn checkpoint_value<T: serde::de::DeserializeOwned>(
    snapshot: &serde_json::Value,
    field: &'static str,
) -> Result<T, DesktopDiscoveryError> {
    let value = snapshot
        .get(field)
        .cloned()
        .ok_or_else(|| DesktopDiscoveryError::Checkpoint(format!("检查点缺少 {field}")))?;
    if value.is_null() {
        return Err(DesktopDiscoveryError::Checkpoint(format!(
            "检查点中的 {field} 为空"
        )));
    }
    serde_json::from_value(value).map_err(Into::into)
}

pub(crate) fn timestamp_now() -> Result<Timestamp, mathscape_domain::DomainError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Timestamp::parse(format!(
        "unix:{}.{}",
        duration.as_secs(),
        duration.subsec_nanos()
    ))
}

fn new_discovery_run_id() -> Result<StableId, mathscape_domain::DomainError> {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    StableId::parse(format!(
        "mathscape:discovery-run/{timestamp:x}-{}-{:x}",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ))
}

fn persistence_checkpoint(
    workflow: &DiscoveryWorkflow<'_>,
    run_id: &StableId,
    profile: &LlmProfile,
    admission: DiscoveryAdmission,
) -> Result<DiscoveryRunCheckpoint, DesktopDiscoveryError> {
    let source = workflow.source_input().ok_or_else(|| {
        DesktopDiscoveryError::Worker("workflow did not retain its DiscoveryInput".into())
    })?;
    let result = workflow.result();
    let units = workflow
        .units()
        .into_iter()
        .flat_map(|unit_set| unit_set.units())
        .map(|unit| {
            let proposal = unit.proposal();
            Ok(DiscoveryUnitCheckpointRecord {
                unit_ref: proposal.unit_ref.clone(),
                parent_ref: proposal.parent.clone(),
                unit_kind: proposal.unit_kind,
                completeness: proposal.completeness,
                payload: serde_json::to_value(unit)?,
            })
        })
        .collect::<Result<Vec<_>, serde_json::Error>>()?;
    let facts = result
        .as_ref()
        .into_iter()
        .flat_map(|result| result.tasks())
        .map(|task| {
            Ok(DiscoveryFactCheckpointRecord {
                task_ref: task.task_ref().to_owned(),
                owner_unit_ref: task.owner().to_owned(),
                processed: task.processed(),
                requires_unit_revision: task.requires_unit_revision(),
                payload: serde_json::to_value(task)?,
            })
        })
        .collect::<Result<Vec<_>, serde_json::Error>>()?;
    let pending_tasks = result
        .as_ref()
        .into_iter()
        .flat_map(|result| result.pending_tasks())
        .map(|task_ref| PendingDiscoveryTask {
            task_ref: task_ref.clone(),
            owner_unit_ref: result
                .as_ref()
                .and_then(|result| result.pending_task_owner(task_ref))
                .map(str::to_owned),
        })
        .collect();
    let attempts = workflow
        .attempts()
        .iter()
        .map(|attempt| {
            Ok(DiscoveryAttemptCheckpoint {
                stage: attempt.stage.clone(),
                batch_fingerprint: attempt.batch.clone(),
                repair: attempt.repair,
                status: checkpoint_status(&attempt.status),
                request: serde_json::to_value(&attempt.request)?,
                raw_text: attempt.raw_text.clone(),
                run: serde_json::to_value(&attempt.run)?,
                error: attempt.error.clone(),
            })
        })
        .collect::<Result<Vec<_>, serde_json::Error>>()?;
    let reports = workflow
        .diagnostics()
        .iter()
        .map(|report| {
            let mut diagnostics = report
                .diagnostics
                .iter()
                .map(|diagnostic| DiscoveryDiagnosticCheckpoint {
                    code: diagnostic.code.clone(),
                    path: diagnostic.path.clone(),
                    message: diagnostic.message.clone(),
                    checked: true,
                })
                .collect::<Vec<_>>();
            diagnostics.extend(report.not_checked.iter().map(|path| {
                DiscoveryDiagnosticCheckpoint {
                    code: "not_checked".into(),
                    path: path.clone(),
                    message: "upstream validation failed".into(),
                    checked: false,
                }
            }));
            Ok(DiscoveryValidationCheckpoint {
                report: serde_json::to_value(report)?,
                diagnostics,
            })
        })
        .collect::<Result<Vec<_>, serde_json::Error>>()?;
    Ok(DiscoveryRunCheckpoint {
        run_id: run_id.clone(),
        artifact_id: source.snapshot.artifact_id.clone(),
        conversion_id: source.snapshot.conversion_id.clone(),
        pipeline_version: DISCOVERY_PIPELINE_VERSION.into(),
        input_fingerprint: source.fingerprint.clone(),
        selection_fingerprint: source.selection_fingerprint.clone(),
        unit_set_fingerprint: workflow.units().map(|units| units.fingerprint().clone()),
        result_fingerprint: result.as_ref().map(discovery_result_fingerprint),
        status: checkpoint_status(workflow.status()),
        admission,
        profile_id: profile.id.clone(),
        snapshot: serde_json::to_value(workflow.snapshot())?,
        attempts,
        reports,
        units,
        facts,
        pending_tasks,
    })
}

fn checkpoint_status(status: &WorkflowStatus) -> DiscoveryCheckpointStatus {
    match status {
        WorkflowStatus::Running => DiscoveryCheckpointStatus::Running,
        WorkflowStatus::AwaitingPatchReview => DiscoveryCheckpointStatus::AwaitingPatchReview,
        WorkflowStatus::Completed => DiscoveryCheckpointStatus::Completed,
        WorkflowStatus::Partial => DiscoveryCheckpointStatus::Partial,
        WorkflowStatus::NeedsAttention => DiscoveryCheckpointStatus::NeedsAttention,
        WorkflowStatus::Failed => DiscoveryCheckpointStatus::Failed,
        WorkflowStatus::Cancelled => DiscoveryCheckpointStatus::Cancelled,
    }
}

fn workflow_status(status: DiscoveryCheckpointStatus) -> WorkflowStatus {
    match status {
        DiscoveryCheckpointStatus::Running => WorkflowStatus::Running,
        DiscoveryCheckpointStatus::AwaitingPatchReview => WorkflowStatus::AwaitingPatchReview,
        DiscoveryCheckpointStatus::Completed => WorkflowStatus::Completed,
        DiscoveryCheckpointStatus::Partial => WorkflowStatus::Partial,
        DiscoveryCheckpointStatus::NeedsAttention => WorkflowStatus::NeedsAttention,
        DiscoveryCheckpointStatus::Failed => WorkflowStatus::Failed,
        DiscoveryCheckpointStatus::Cancelled => WorkflowStatus::Cancelled,
    }
}

impl From<StoredDiscoveryRunSummary> for DesktopDiscoveryRunSummary {
    fn from(summary: StoredDiscoveryRunSummary) -> Self {
        Self {
            run_id: summary.run_id.to_string(),
            artifact_id: summary.artifact_id.to_string(),
            artifact_name: summary.artifact_name,
            conversion_id: summary.conversion_id.map(|id| id.to_string()),
            input_fingerprint: summary.input_fingerprint,
            result_fingerprint: summary.result_fingerprint,
            pipeline_version: summary.pipeline_version,
            status: summary.status,
            review_status: summary.review_status,
            registrations: summary.registrations,
            disposition: summary.disposition,
            profile_id: summary.profile_id,
            unit_count: summary.unit_count,
            fact_count: summary.fact_count,
            pending_task_count: summary.pending_task_count,
            unresolved_reference_count: summary.unresolved_reference_count,
            relation_count: summary.relation_count,
            created_at: summary.created_at,
            updated_at: summary.updated_at,
        }
    }
}

#[tauri::command]
pub(crate) fn get_active_discovery(state: State<'_, DesktopState>) -> Option<ActiveDiscoveryTask> {
    state.service().active_discovery()
}

#[tauri::command]
pub(crate) async fn list_discovery_runs(
    state: State<'_, DesktopState>,
    request: ListDiscoveryRunsRequest,
) -> Result<Vec<DesktopDiscoveryRunSummary>, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || {
        service.list_discovery_runs(request.artifact_id.as_deref())
    })
    .await
    .map_err(|error| format!("读取内容发现记录线程失败：{error}"))?
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn get_discovery_review(
    state: State<'_, DesktopState>,
    run_id: String,
) -> Result<DesktopDiscoveryReviewEnvelope, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || service.get_discovery_review(&run_id))
        .await
        .map_err(|error| format!("恢复内容发现结果线程失败：{error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn get_discovery_diagnostics(
    state: State<'_, DesktopState>,
    run_id: String,
) -> Result<DesktopDiscoveryRunDiagnostics, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || service.get_discovery_diagnostics(&run_id))
        .await
        .map_err(|error| format!("读取内容发现诊断线程失败：{error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn discard_discovery_run(
    state: State<'_, DesktopState>,
    request: DiscardDiscoveryRunRequest,
) -> Result<DiscoveryRunDispositionReceipt, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || service.discard_discovery_run(request))
        .await
        .map_err(|error| format!("放弃内容发现结果线程失败：{error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn restore_discovery_review(
    state: State<'_, DesktopState>,
    request: RestoreDiscoveryReviewRequest,
) -> Result<DiscoveryRunDispositionReceipt, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || service.restore_discovery_review(request))
        .await
        .map_err(|e| format!("恢复审核线程失败：{e}"))?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn cancel_discovery(state: State<'_, DesktopState>) -> bool {
    state.service().cancel_discovery()
}

#[tauri::command]
pub(crate) async fn accept_discovery_result(
    state: State<'_, DesktopState>,
    request: AcceptDiscoveryRequest,
) -> Result<DesktopDiscoveryAcceptance, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || service.accept_discovery_result(request))
        .await
        .map_err(|error| format!("来源审核线程失败：{error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn run_discovery(
    state: State<'_, DesktopState>,
    request: RunDiscoveryRequest,
    on_progress: tauri::ipc::Channel<serde_json::Value>,
) -> Result<DesktopDiscoveryOutcome, String> {
    state
        .service()
        .run_discovery_observed(
            request,
            Some(Arc::new(move |progress| {
                if let Ok(value) = serde_json::to_value(progress) {
                    let _ = on_progress.send(value);
                }
            })),
        )
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn resume_discovery_after_patch(
    state: State<'_, DesktopState>,
    request: ResumeDiscoveryRequest,
    on_progress: tauri::ipc::Channel<serde_json::Value>,
) -> Result<DesktopDiscoveryOutcome, String> {
    state
        .service()
        .resume_discovery_after_patch_observed(
            request,
            Some(Arc::new(move |progress| {
                if let Ok(value) = serde_json::to_value(progress) {
                    let _ = on_progress.send(value);
                }
            })),
        )
        .await
        .map_err(|error| error.to_string())
}
