use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use mathscape_domain::{
    ActorRef, ContentHash, DISCOVERY_PIPELINE_VERSION, DiscoveryAdmission, DiscoveryInput,
    DiscoverySelectionRequest, FactTaskProposal, StableId, Timestamp, UnitDiscoveryCheckpoint,
};
use mathscape_llm::{GenaiBackend, GenaiBackendOptions, LlmError, LlmProfile};
use mathscape_orchestrator::{
    AcceptanceAuthority, AcceptanceSelection, DiscoveryAcceptanceDecision, DiscoveryRunSummary,
    DiscoveryWorkflow, DiscoveryWorkflowEvent, DiscoveryWorkflowLimits,
    PreparedUnitDiscoveryRequest, SourceRegistrationPlan, WorkflowAttempt, WorkflowStatus,
    acceptance_decision_id, discovery_result_fingerprint, materialize_source_registration,
    plan_source_registration, prepare_unit_discovery_request_from_input, project_source_review,
    rebind_discovery_input, revalidate_discovery_checkpoint,
};
use mathscape_store_sqlite::{
    AcceptedRegistrationPlanRecord, DiscoveryAttemptCheckpoint, DiscoveryCheckpointStatus,
    DiscoveryDiagnosticCheckpoint, DiscoveryFactCheckpointRecord, DiscoveryRunCheckpoint,
    DiscoveryUnitCheckpointRecord, DiscoveryValidationCheckpoint, PendingDiscoveryTask,
    SourceRegistrationReceipt, SqliteArtifactStore, SqliteStoreError, StoredAcceptanceAuthority,
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
}

impl RunDiscoveryRequest {
    pub fn new(selection: DiscoverySelectionRequest, profile_id: Option<String>) -> Self {
        Self {
            selection,
            profile_id,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActiveDiscoveryPhase {
    Starting,
    DiscoveringUnits,
    ValidatingUnits,
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DesktopDiscoveryRun {
    run_id: String,
    profile_id: String,
    provider: String,
    requested_model: String,
    admission: DiscoveryAdmission,
    request_timeout_seconds: u64,
    connect_timeout_seconds: u64,
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
}

#[derive(Clone, Debug, Default)]
struct StreamContext {
    attempt: usize,
    stage: String,
    batch_index: Option<usize>,
    batch_total: Option<usize>,
}

impl DesktopService {
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
                    actor,
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
        let receipt = store.commit_source_registration(
            &record.decision_id,
            &record.plan_fingerprint,
            &patch,
            timestamp_now()?,
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
        let (prepared, profile, policy) = tauri::async_runtime::spawn_blocking(move || {
            service.prepare_discovery_execution(request)
        })
        .await
        .map_err(|error| DesktopDiscoveryError::Worker(error.to_string()))??;

        let (cancel, mut cancelled) = watch::channel(false);
        let task = ActiveDiscoveryTask {
            artifact_id: prepared.input().snapshot.artifact_id.to_string(),
            input_fingerprint: prepared.input().fingerprint.clone(),
            profile_id: profile.id.clone(),
            request_timeout_seconds: policy.request_timeout.as_secs(),
            phase: ActiveDiscoveryPhase::Starting,
            attempt: 0,
            batch_index: None,
            batch_total: None,
            unit_count: 0,
            completed_tasks: 0,
            total_tasks: 0,
            cancellation_requested: false,
        };
        let _permit = self.begin_discovery(task, cancel)?;

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
        let mut workflow =
            DiscoveryWorkflow::new(&backend, limits).with_observer(workflow_observer);
        let run_id = new_discovery_run_id()?;
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
        store.save_discovery_checkpoint(&checkpoint)?;
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
    ) -> Result<
        (
            PreparedUnitDiscoveryRequest,
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
            .prepare_unit_discovery_request(request.selection)
            .map_err(DesktopDiscoveryError::Input)?;
        Ok((prepared, profile, policy))
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
            task.phase = ActiveDiscoveryPhase::DiscoveringUnits;
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
            } else if stage == "unit-discovery" {
                ActiveDiscoveryPhase::DiscoveringUnits
            } else {
                ActiveDiscoveryPhase::DiscoveringFacts
            };
        }
        DiscoveryWorkflowEvent::ValidationStarted { stage, .. } => {
            task.phase = if stage == "unit-discovery" {
                ActiveDiscoveryPhase::ValidatingUnits
            } else {
                ActiveDiscoveryPhase::ValidatingFacts
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
            request_timeout_seconds: policy.request_timeout.as_secs(),
            connect_timeout_seconds: policy.connect_timeout.as_secs(),
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
            state: "registered".into(),
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
            commit_id: receipt.commit_id.to_string(),
            created_object_count: receipt.created_objects.len(),
            reused_object_count: receipt.reused_objects.len(),
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
        .ok_or_else(|| DesktopDiscoveryError::Acceptance(format!("检查点缺少 {field}")))?;
    if value.is_null() {
        return Err(DesktopDiscoveryError::Acceptance(format!(
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
        WorkflowStatus::Completed => DiscoveryCheckpointStatus::Completed,
        WorkflowStatus::Partial => DiscoveryCheckpointStatus::Partial,
        WorkflowStatus::NeedsAttention => DiscoveryCheckpointStatus::NeedsAttention,
        WorkflowStatus::Failed => DiscoveryCheckpointStatus::Failed,
        WorkflowStatus::Cancelled => DiscoveryCheckpointStatus::Cancelled,
    }
}

#[tauri::command]
pub(crate) fn get_active_discovery(state: State<'_, DesktopState>) -> Option<ActiveDiscoveryTask> {
    state.service.active_discovery()
}

#[tauri::command]
pub(crate) fn cancel_discovery(state: State<'_, DesktopState>) -> bool {
    state.service.cancel_discovery()
}

#[tauri::command]
pub(crate) async fn accept_discovery_result(
    state: State<'_, DesktopState>,
    request: AcceptDiscoveryRequest,
) -> Result<DesktopDiscoveryAcceptance, String> {
    let service = state.service.clone();
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
        .service
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
