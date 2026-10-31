use std::time::{SystemTime, UNIX_EPOCH};

use mathscape_domain::{ActorRef, CommitReceipt, ImpactPlan, Timestamp, UserChangeRequest};
use mathscape_orchestrator::map_edit::{get_edit_context, plan_map_change as plan_discovery};
use mathscape_store_sqlite::SqliteArtifactStore;
use tauri::State;

use crate::{DesktopService, DesktopState};

const ACTOR: &str = "mathscape-desktop";

fn now() -> Result<Timestamp, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Timestamp::parse(format!(
        "unix:{}.{}",
        duration.as_secs(),
        duration.subsec_nanos()
    ))
    .map_err(|error| error.to_string())
}

fn actor() -> Result<ActorRef, String> {
    ActorRef::parse(ACTOR).map_err(|error| error.to_string())
}

fn store(service: &DesktopService) -> Result<SqliteArtifactStore, String> {
    SqliteArtifactStore::open(&service.workspace_root().join("mathscape.sqlite3"))
        .map_err(|error| error.to_string())
}

pub fn plan(service: &DesktopService, request: UserChangeRequest) -> Result<ImpactPlan, String> {
    let store = store(service)?;
    let snapshot = store
        .map_edit_snapshot()
        .map_err(|error| error.to_string())?;
    let actor = actor()?;
    Ok(plan_discovery(&snapshot, &request, &actor).impact)
}

pub fn commit(
    service: &DesktopService,
    request: UserChangeRequest,
    plan_fingerprint: String,
) -> Result<CommitReceipt, String> {
    let mut store = store(service)?;
    let actor = actor()?;
    let planner_actor = actor.clone();
    let expected = plan_fingerprint;
    let request_for_plan = request.clone();
    store
        .commit_map_change(&request, &expected, &actor, now()?, move |snapshot| {
            let planned = plan_discovery(snapshot, &request_for_plan, &planner_actor);
            (planned.impact, planned.patch)
        })
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_map_object_edit_context(
    object_id: String,
    state: State<'_, DesktopState>,
) -> Result<mathscape_domain::EditContext, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || {
        let store = store(&service)?;
        let snapshot = store
            .map_edit_snapshot()
            .map_err(|error| error.to_string())?;
        get_edit_context(&snapshot, &object_id)
    })
    .await
    .map_err(|error| format!("edit context worker failed: {error}"))?
}

#[tauri::command]
pub async fn plan_map_change(
    request: UserChangeRequest,
    state: State<'_, DesktopState>,
) -> Result<ImpactPlan, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || plan(&service, request))
        .await
        .map_err(|error| format!("impact planner failed: {error}"))?
}

#[tauri::command]
pub async fn commit_map_change(
    request: UserChangeRequest,
    plan_fingerprint: String,
    state: State<'_, DesktopState>,
) -> Result<CommitReceipt, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || commit(&service, request, plan_fingerprint))
        .await
        .map_err(|error| format!("map commit worker failed: {error}"))?
}
