//! Thin registration commands; planners and atomic writes stay below Desktop.
use crate::{DesktopService, DesktopState};
use mathscape_domain::*;
use mathscape_orchestrator::{
    SourceRegistrationPlan, materialize_source_registration,
    registration::{plan_registration_commit, plan_registration_reversal},
};
use mathscape_store_sqlite::{SourceRegistrationReceipt, SqliteArtifactStore};
use tauri::State;

/// Provenance hint from the immutable accepted plan, independent of A/B replay.
#[derive(Clone, Debug, serde::Serialize)]
pub struct RegistrationSourceContext {
    pub artifact_id: StableId,
    pub conversion_id: Option<StableId>,
}

impl DesktopService {
    pub fn registration_source_context(
        &self,
        decision_id: &str,
    ) -> Result<RegistrationSourceContext, String> {
        let decision = StableId::parse(decision_id).map_err(|e| e.to_string())?;
        let record = store(self)?
            .registration_plan(&decision)
            .map_err(|e| e.to_string())?
            .ok_or("接受计划不存在")?;
        // Read only identity fields: older checkpoints need not replay under
        // today's validators just to render their registered source bytes.
        #[derive(serde::Deserialize)]
        struct SourceIdentity {
            markdown_artifact_id: StableId,
            conversion_id: Option<StableId>,
        }
        let source: SourceIdentity =
            serde_json::from_value(record.plan).map_err(|e| format!("登记来源身份无效：{e}"))?;
        Ok(RegistrationSourceContext {
            artifact_id: source.markdown_artifact_id,
            conversion_id: source.conversion_id,
        })
    }
}

#[tauri::command]
pub(crate) async fn get_source_registration_context(
    decision_id: String,
    state: State<'_, DesktopState>,
) -> Result<RegistrationSourceContext, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.registration_source_context(&decision_id))
        .await
        .map_err(|e| e.to_string())?
}

pub(crate) fn store(service: &DesktopService) -> Result<SqliteArtifactStore, String> {
    SqliteArtifactStore::open(&service.workspace_root().join("mathscape.sqlite3"))
        .map_err(|e| e.to_string())
}
pub(crate) fn actor() -> ActorRef {
    ActorRef::parse("mathscape-desktop-user").expect("fixed actor")
}
fn now() -> Timestamp {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    Timestamp::parse(format!("unix:{}.{}", d.as_secs(), d.subsec_nanos())).expect("timestamp")
}
pub(crate) fn candidate(
    store: &SqliteArtifactStore,
    request: &SourceRegistrationCommitRequest,
) -> Result<MapPatch<SourcePayload>, String> {
    let record = store
        .registration_plan(&StableId::parse(&request.decision_id).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?
        .ok_or("接受计划不存在")?;
    if record.plan_fingerprint.as_str() != request.acceptance_plan_fingerprint {
        return Err("acceptance_plan_conflict".into());
    }
    let plan: SourceRegistrationPlan =
        serde_json::from_value(record.plan).map_err(|e| e.to_string())?;
    materialize_source_registration(&plan).map_err(|e| e.to_string())
}
pub(crate) fn plan_commit(
    service: &DesktopService,
    request: &SourceRegistrationCommitRequest,
) -> Result<SourceRegistrationCommitPlan, String> {
    let s = store(service)?;
    let p = candidate(&s, request)?;
    Ok(plan_registration_commit(
        &s.registration_snapshot().map_err(|e| e.to_string())?,
        request,
        &p,
        &actor(),
    )
    .plan)
}
pub(crate) fn commit_registration(
    service: &DesktopService,
    request: &SourceRegistrationCommitRequest,
    expected: &str,
) -> Result<SourceRegistrationReceipt, String> {
    let mut s = store(service)?;
    let p = candidate(&s, request)?;
    s.commit_bound_source_registration(request, expected, &actor(), now(), |snapshot| {
        let execution = plan_registration_commit(snapshot, request, &p, &actor());
        (execution.plan, execution.patch)
    })
    .map_err(|e| e.to_string())
}
#[tauri::command]
pub(crate) async fn get_map_head(state: State<'_, DesktopState>) -> Result<Option<String>, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        Ok(store(&service)?
            .map_edit_snapshot()
            .map_err(|e| e.to_string())?
            .base_commit)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub(crate) async fn get_pending_source_registration_plans(
    run_id: String,
    state: State<'_, DesktopState>,
) -> Result<Vec<SourceRegistrationCommitPlan>, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let s = store(&service)?;
        let mut result = vec![];
        for decision in s
            .pending_registration_decisions(&run_id)
            .map_err(|e| e.to_string())?
        {
            let record = s
                .registration_plan(&StableId::parse(&decision).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
                .ok_or("接受计划缺失")?;
            let request = SourceRegistrationCommitRequest {
                request_id: format!(
                    "mathscape:source-registration/{}-{}",
                    now().as_str(),
                    decision
                ),
                decision_id: decision,
                acceptance_plan_fingerprint: record.plan_fingerprint.to_string(),
                base_commit: s
                    .map_edit_snapshot()
                    .map_err(|e| e.to_string())?
                    .base_commit,
                replacement_approvals: vec![],
            };
            result.push(plan_commit(&service, &request)?);
        }
        Ok(result)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn list_discovery_registrations(
    run_id: String,
    state: State<'_, DesktopState>,
) -> Result<Vec<RegistrationSummary>, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        store(&service)?
            .list_discovery_registrations(&run_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn plan_discovery_registration_reversal(
    request: ReverseDiscoveryRequest,
    state: State<'_, DesktopState>,
) -> Result<DiscoveryReversalPlan, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let s = store(&service)?
            .registration_snapshot()
            .map_err(|e| e.to_string())?;
        Ok(plan_registration_reversal(&s, &request, &actor()).plan)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn commit_discovery_registration_reversal(
    request: CommitDiscoveryReversalRequest,
    state: State<'_, DesktopState>,
) -> Result<DiscoveryReversalReceipt, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        store(&service)?
            .commit_registration_reversal(
                &request.request,
                &request.expected_plan_fingerprint,
                &actor(),
                now(),
                |snapshot| {
                    let e = plan_registration_reversal(snapshot, &request.request, &actor());
                    (e.plan, e.patch)
                },
            )
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn get_discovery_registration_reversal(
    decision_id: String,
    state: State<'_, DesktopState>,
) -> Result<Option<DiscoveryReversalReceipt>, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        store(&service)?
            .discovery_registration_reversal(&decision_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn get_registration_history(
    decision_id: String,
    state: State<'_, DesktopState>,
) -> Result<MapEditSnapshot, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        store(&service)?
            .registration_history(&decision_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn get_registration_object_detail(
    decision_id: String,
    object_id: String,
    state: State<'_, DesktopState>,
) -> Result<crate::SourceNodeDetail, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        service.registration_object_detail(&decision_id, &object_id)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn plan_source_registration_commit(
    request: SourceRegistrationCommitRequest,
    state: State<'_, DesktopState>,
) -> Result<SourceRegistrationCommitPlan, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || plan_commit(&service, &request))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn commit_source_registration_commit(
    request: SourceRegistrationCommitRequest,
    expected_plan_fingerprint: String,
    state: State<'_, DesktopState>,
) -> Result<SourceRegistrationReceipt, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        commit_registration(&service, &request, &expected_plan_fingerprint)
    })
    .await
    .map_err(|e| e.to_string())?
}
