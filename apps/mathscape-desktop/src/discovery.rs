use std::sync::{Arc, Mutex};

use mathscape_domain::{ContentHash, DiscoveryAdmission, DiscoverySelectionRequest};
use mathscape_llm::{GenaiBackend, GenaiBackendOptions, LlmError, LlmProfile};
use mathscape_orchestrator::{
    DiscoveryRunSummary, DiscoveryWorkflow, DiscoveryWorkflowEvent, DiscoveryWorkflowLimits,
    PreparedUnitDiscoveryRequest, WorkflowAttempt, WorkflowStatus,
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
    run: DesktopDiscoveryRun,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DesktopDiscoveryRun {
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
        let view = DesktopDiscoveryView::project(&workflow);
        Ok(DesktopDiscoveryOutcome::from_workflow(
            view,
            workflow.attempts(),
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
        attempts: &[WorkflowAttempt],
        profile: &LlmProfile,
        policy: EffectiveDiscoveryPolicy,
    ) -> Self {
        let outcome = view.status().clone();
        Self {
            outcome,
            view,
            run: DesktopDiscoveryRun::from_attempts(attempts, profile, policy),
        }
    }
}

impl DesktopDiscoveryRun {
    fn from_attempts(
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

#[tauri::command]
pub(crate) fn get_active_discovery(state: State<'_, DesktopState>) -> Option<ActiveDiscoveryTask> {
    state.service.active_discovery()
}

#[tauri::command]
pub(crate) fn cancel_discovery(state: State<'_, DesktopState>) -> bool {
    state.service.cancel_discovery()
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
