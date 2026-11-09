//! Desktop-only review and commit commands for Phase2 staged proposals.
//!
//! Agent processes never receive these commands. They only create immutable
//! staged versions through MCP; this module binds a user decision to the
//! proposal fingerprint and a freshly computed map plan.

use std::collections::BTreeSet;

use mathscape_domain::{
    ActorRef, CommitReceipt, ImpactPlan, Timestamp, UserChangeOperation, UserChangeRequest,
};
use mathscape_orchestrator::map_edit::plan_map_change;
use mathscape_store_sqlite::{
    ChangeSetDecision, PendingChangeSet, ProposedChangeRecord, SqliteArtifactStore,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tauri::State;

use crate::{DesktopService, DesktopState};

const ACTOR: &str = "mathscape-desktop-user";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeSetSelection {
    pub change_set_id: String,
    pub version: i64,
    pub proposal_fingerprint: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingChangeSetView {
    pub change_set_id: String,
    pub version: i64,
    pub state: String,
    pub investigation_id: String,
    pub round_id: String,
    pub proposed_by: String,
    pub proposal_fingerprint: Option<String>,
    pub base_commit: Option<String>,
    pub proposal: Value,
    pub result: Value,
    pub dependencies: Vec<DependencyView>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyView {
    pub change_set_id: String,
    pub version: i64,
    pub referenced_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BatchPlan {
    pub investigation_id: String,
    pub selections: Vec<ChangeSetSelectionView>,
    pub impact: ImpactPlan,
    pub per_change_set: Vec<PerChangeSetPlan>,
    pub batch_plan_fingerprint: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ChangeSetSelectionView {
    pub change_set_id: String,
    pub version: i64,
    pub proposal_fingerprint: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct PerChangeSetPlan {
    pub change_set_id: String,
    pub version: i64,
    pub direct_changes: Vec<mathscape_domain::ImpactItem>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ChangeSetReview {
    pub proposal: PendingChangeSetView,
    pub impact: ImpactPlan,
}

fn store(service: &DesktopService) -> Result<SqliteArtifactStore, String> {
    SqliteArtifactStore::open(&service.workspace_root().join("mathscape.sqlite3"))
        .map_err(|error| error.to_string())
}

fn actor() -> Result<ActorRef, String> {
    ActorRef::parse(ACTOR).map_err(|error| error.to_string())
}

fn timestamp() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("unix:{seconds}")
}

fn stable_request_id(prefix: &str, value: &impl Serialize) -> Result<String, String> {
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    Ok(format!("mathscape:{prefix}/{:x}", Sha256::digest(bytes)))
}

fn selection_record(
    store: &SqliteArtifactStore,
    investigation_id: &str,
    selection: &ChangeSetSelection,
) -> Result<ProposedChangeRecord, String> {
    let Some((record, _)) = store
        .proposed_change(
            investigation_id,
            &selection.change_set_id,
            Some(selection.version),
        )
        .map_err(|error| error.to_string())?
    else {
        return Err("提案不存在或不属于当前调查".into());
    };
    if record.state != "staged" {
        return Err(format!("提案当前状态为 {}，不能接受", record.state));
    }
    let fingerprint = record
        .result
        .get("proposal_fingerprint")
        .and_then(Value::as_str)
        .unwrap_or("");
    if fingerprint != selection.proposal_fingerprint {
        return Err("proposal_fingerprint 已失效，请重新预览".into());
    }
    Ok(record)
}

fn operations(record: &ProposedChangeRecord) -> Result<Vec<UserChangeOperation>, String> {
    serde_json::from_value(
        record
            .result
            .get("compiled_operations")
            .cloned()
            .unwrap_or_else(|| json!([])),
    )
    .map_err(|error| format!("提案缺少可接受的编译操作：{error}"))
}

fn request_for(
    record: &ProposedChangeRecord,
    operations: Vec<UserChangeOperation>,
    base_commit: Option<String>,
) -> Result<UserChangeRequest, String> {
    Ok(UserChangeRequest {
        request_id: stable_request_id(
            "change-set-accept",
            &(record.change_set_id.as_str(), record.version),
        )?,
        base_commit,
        reason: record
            .proposal
            .get("rationale")
            .and_then(Value::as_str)
            .unwrap_or("接受来源提案")
            .to_owned(),
        operations,
    })
}

fn dependency_records(
    store: &SqliteArtifactStore,
    investigation_id: &str,
    record: &ProposedChangeRecord,
) -> Result<Vec<ProposedChangeRecord>, String> {
    let mut records = Vec::new();
    for dependency in store
        .dependencies_for(investigation_id, &record.change_set_id, record.version)
        .map_err(|error| error.to_string())?
    {
        let Some((dependency, _)) = store
            .proposed_change(
                investigation_id,
                &dependency.depends_on_change_set_id,
                Some(dependency.depends_on_version),
            )
            .map_err(|error| error.to_string())?
        else {
            return Err("依赖提案不存在".into());
        };
        if dependency.state != "accepted" {
            return Err(format!(
                "dependency_not_accepted: {}@{}",
                dependency.change_set_id, dependency.version
            ));
        }
        records.push(dependency);
    }
    Ok(records)
}

fn plan_one(
    service: &DesktopService,
    investigation_id: &str,
    selection: &ChangeSetSelection,
) -> Result<ImpactPlan, String> {
    let store = store(service)?;
    let record = selection_record(&store, investigation_id, selection)?;
    let _ = dependency_records(&store, investigation_id, &record)?;
    let request = request_for(&record, operations(&record)?, record.base_commit.clone())?;
    Ok(plan_map_change(&snapshot, &request, &actor()?).impact)
}

pub fn list_pending(
    service: &DesktopService,
    investigation_id: String,
) -> Result<Vec<PendingChangeSetView>, String> {
    let store = store(service)?;
    store
        .list_pending_change_sets(&investigation_id)
        .map_err(|error| error.to_string())
        .map(|items| items.into_iter().map(view).collect())
}

fn view(item: PendingChangeSet) -> PendingChangeSetView {
    PendingChangeSetView {
        change_set_id: item.record.change_set_id.clone(),
        version: item.record.version,
        state: item.record.state.clone(),
        investigation_id: item.record.investigation_id.clone(),
        round_id: item.record.round_id.clone(),
        proposed_by: item.record.proposed_by.clone(),
        proposal_fingerprint: item
            .record
            .result
            .get("proposal_fingerprint")
            .and_then(Value::as_str)
            .map(str::to_owned),
        base_commit: item.record.base_commit.clone(),
        proposal: item.record.proposal,
        result: item.record.result,
        dependencies: item
            .dependencies
            .into_iter()
            .map(|dependency| DependencyView {
                change_set_id: dependency.depends_on_change_set_id,
                version: dependency.depends_on_version,
                referenced_ids: dependency.referenced_ids,
            })
            .collect(),
    }
}

#[tauri::command]
pub async fn list_pending_change_sets(
    investigation_id: String,
    state: State<'_, DesktopState>,
) -> Result<Vec<PendingChangeSetView>, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || list_pending(&service, investigation_id))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn plan_change_set_accept(
    investigation_id: String,
    selection: ChangeSetSelection,
    state: State<'_, DesktopState>,
) -> Result<ImpactPlan, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || plan_one(&service, &investigation_id, &selection))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn get_change_set_review(
    investigation_id: String,
    selection: ChangeSetSelection,
    state: State<'_, DesktopState>,
) -> Result<ChangeSetReview, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || {
        let impact = plan_one(&service, &investigation_id, &selection)?;
        let item = list_pending(&service, investigation_id.clone())?
            .into_iter()
            .find(|item| {
                item.change_set_id == selection.change_set_id && item.version == selection.version
            })
            .ok_or("提案不存在")?;
        Ok(ChangeSetReview {
            proposal: PendingChangeSetView {
                change_set_id: item.change_set_id,
                version: item.version,
                state: item.state,
                investigation_id: item.investigation_id,
                round_id: item.round_id,
                proposed_by: item.proposed_by,
                proposal_fingerprint: item.proposal_fingerprint,
                base_commit: item.base_commit,
                proposal: item.proposal,
                result: item.result,
                dependencies: item.dependencies,
            },
            impact,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn accept_change_set(
    investigation_id: String,
    selection: ChangeSetSelection,
    plan_fingerprint: String,
    state: State<'_, DesktopState>,
) -> Result<CommitReceipt, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || {
        accept_one(&service, &investigation_id, selection, plan_fingerprint)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn accept_one(
    service: &DesktopService,
    investigation_id: &str,
    selection: ChangeSetSelection,
    plan_fingerprint: String,
) -> Result<CommitReceipt, String> {
    let mut store = store(service)?;
    let record = selection_record(&store, investigation_id, &selection)?;
    let _ = dependency_records(&store, investigation_id, &record)?;
    let actor = actor()?;
    let snapshot = store
        .map_edit_snapshot()
        .map_err(|error| error.to_string())?;
    let request = request_for(&record, operations(&record)?, record.base_commit.clone())?;
    let planner_request = request.clone();
    let planner_actor = actor.clone();
    let receipt = store
        .commit_map_change(
            &request,
            &plan_fingerprint,
            &actor,
            Timestamp::parse("unix:0").map_err(|error| error.to_string())?,
            move |snapshot| {
                let planned = plan_map_change(snapshot, &planner_request, &planner_actor);
                (planned.impact, planned.patch)
            },
        )
        .map_err(|error| error.to_string())?;
    let mut accepted_result = record.result.clone();
    accepted_result["status"] = json!("accepted");
    accepted_result["decision"] = json!({ "kind": "accepted", "plan_fingerprint": plan_fingerprint, "commit_id": receipt.commit_id, "receipt": receipt });
    store
        .update_proposed_state(
            investigation_id,
            &record.change_set_id,
            record.version,
            "accepted",
            Some(&accepted_result),
        )
        .map_err(|error| error.to_string())?;
    store
        .record_change_set_decision(&ChangeSetDecision {
            decision_id: stable_request_id(
                "change-set-decision",
                &(record.change_set_id.as_str(), record.version, "accepted"),
            )?,
            change_set_id: record.change_set_id,
            version: record.version,
            investigation_id: investigation_id.to_owned(),
            kind: "accepted".into(),
            decided_by: ACTOR.into(),
            plan_fingerprint: Some(plan_fingerprint),
            commit_id: Some(receipt.commit_id.clone()),
            receipt: Some(serde_json::to_value(&receipt).map_err(|error| error.to_string())?),
            batch_id: None,
            reason: None,
            decided_at: timestamp(),
        })
        .map_err(|error| error.to_string())?;
    Ok(receipt)
}

#[tauri::command]
pub async fn discard_change_set(
    investigation_id: String,
    selection: ChangeSetSelection,
    cascade: bool,
    reason: Option<String>,
    state: State<'_, DesktopState>,
) -> Result<(), String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || {
        discard_one(&service, &investigation_id, selection, cascade, reason)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn plan_change_set_batch(
    investigation_id: String,
    selections: Vec<ChangeSetSelection>,
    state: State<'_, DesktopState>,
) -> Result<BatchPlan, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || {
        plan_batch(&service, &investigation_id, selections)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn accept_change_set_batch(
    investigation_id: String,
    selections: Vec<ChangeSetSelection>,
    batch_plan_fingerprint: String,
    request_id: Option<String>,
    state: State<'_, DesktopState>,
) -> Result<CommitReceipt, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || {
        accept_batch(
            &service,
            &investigation_id,
            selections,
            batch_plan_fingerprint,
            request_id,
        )
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn discard_change_set_batch(
    investigation_id: String,
    selections: Vec<ChangeSetSelection>,
    cascade: bool,
    state: State<'_, DesktopState>,
) -> Result<(), String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || {
        for selection in selections {
            discard_one(
                &service,
                &investigation_id,
                selection,
                cascade,
                Some("批量审核丢弃".into()),
            )?;
        }
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())?
}

fn plan_batch(
    service: &DesktopService,
    investigation_id: &str,
    selections: Vec<ChangeSetSelection>,
) -> Result<BatchPlan, String> {
    if selections.is_empty() {
        return Err("至少选择一个提案".into());
    }
    let store = store(service)?;
    let mut records = Vec::new();
    let selected_ids: BTreeSet<(String, i64)> = selections
        .iter()
        .map(|selection| (selection.change_set_id.clone(), selection.version))
        .collect();
    for selection in &selections {
        let record = selection_record(&store, investigation_id, selection)?;
        for dependency in store
            .dependencies_for(investigation_id, &record.change_set_id, record.version)
            .map_err(|error| error.to_string())?
        {
            let dependency_key = (
                dependency.depends_on_change_set_id.clone(),
                dependency.depends_on_version,
            );
            let dependency_record = store
                .proposed_change(
                    investigation_id,
                    &dependency.depends_on_change_set_id,
                    Some(dependency.depends_on_version),
                )
                .map_err(|error| error.to_string())?
                .ok_or("依赖提案不存在")?
                .0;
            if dependency_record.state != "accepted" && !selected_ids.contains(&dependency_key) {
                return Err(format!(
                    "dependency_not_accepted: {}@{}",
                    dependency.depends_on_change_set_id, dependency.depends_on_version
                ));
            }
        }
        records.push(record);
    }
    records.sort_by_key(|record| record.created_at.clone());
    let all_operations = records
        .iter()
        .map(operations)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let first = records.first().expect("nonempty");
    let base_commit = records
        .first()
        .and_then(|record| record.base_commit.clone());
    if records
        .iter()
        .any(|record| record.base_commit != base_commit)
    {
        return Err("批量提案的 base_commit 不一致，请重新提议".into());
    }
    let request = UserChangeRequest {
        request_id: stable_request_id("change-set-batch", &selections)?,
        base_commit,
        reason: "批量接受来源提案".into(),
        operations: all_operations,
    };
    let impact = plan_map_change(&snapshot, &request, &actor()?).impact;
    // The existing map writer verifies the planner fingerprint under its
    // transaction. A batch is one combined UserChangeRequest, so its plan
    // fingerprint is already the required atomic batch fingerprint.
    let batch_plan_fingerprint = impact.plan_fingerprint.clone();
    let per_change_set = records
        .iter()
        .map(|record| PerChangeSetPlan {
            change_set_id: record.change_set_id.clone(),
            version: record.version,
            direct_changes: vec![],
        })
        .collect();
    let _ = first;
    Ok(BatchPlan {
        investigation_id: investigation_id.into(),
        selections: selections
            .into_iter()
            .map(|selection| ChangeSetSelectionView {
                change_set_id: selection.change_set_id,
                version: selection.version,
                proposal_fingerprint: selection.proposal_fingerprint,
            })
            .collect(),
        impact,
        per_change_set,
        batch_plan_fingerprint,
    })
}

fn accept_batch(
    service: &DesktopService,
    investigation_id: &str,
    selections: Vec<ChangeSetSelection>,
    batch_plan_fingerprint: String,
    _request_id: Option<String>,
) -> Result<CommitReceipt, String> {
    let mut store = store(service)?;
    let mut records = selections
        .iter()
        .map(|selection| selection_record(&store, investigation_id, selection))
        .collect::<Result<Vec<_>, _>>()?;
    records.sort_by_key(|record| record.created_at.clone());
    let snapshot = store
        .map_edit_snapshot()
        .map_err(|error| error.to_string())?;
    let all_operations = records
        .iter()
        .map(operations)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let base_commit = records
        .first()
        .and_then(|record| record.base_commit.clone());
    if records
        .iter()
        .any(|record| record.base_commit != base_commit)
    {
        return Err("批量提案的 base_commit 不一致，请重新提议".into());
    }
    let request = UserChangeRequest {
        request_id: stable_request_id("change-set-batch", &selections)?,
        base_commit,
        reason: "批量接受来源提案".into(),
        operations: all_operations,
    };
    let expected_batch =
        plan_batch(service, investigation_id, selections.clone())?.batch_plan_fingerprint;
    if expected_batch != batch_plan_fingerprint {
        return Err("batch_plan_conflict: 影响预览已失效".into());
    }
    let actor = actor()?;
    let planner_request = request.clone();
    let planner_actor = actor.clone();
    let receipt = store
        .commit_map_change(
            &request,
            &batch_plan_fingerprint,
            &actor,
            Timestamp::parse("unix:0").map_err(|error| error.to_string())?,
            move |snapshot| {
                let planned = plan_map_change(snapshot, &planner_request, &planner_actor);
                (planned.impact, planned.patch)
            },
        )
        .map_err(|error| error.to_string())?;
    for record in records {
        let mut accepted_result = record.result.clone();
        accepted_result["status"] = json!("accepted");
        accepted_result["decision"] = json!({ "kind": "accepted", "plan_fingerprint": batch_plan_fingerprint, "commit_id": receipt.commit_id, "receipt": receipt });
        store
            .update_proposed_state(
                investigation_id,
                &record.change_set_id,
                record.version,
                "accepted",
                Some(&accepted_result),
            )
            .map_err(|error| error.to_string())?;
        store
            .record_change_set_decision(&ChangeSetDecision {
                decision_id: stable_request_id(
                    "change-set-decision",
                    &(record.change_set_id.as_str(), record.version, "accepted"),
                )?,
                change_set_id: record.change_set_id,
                version: record.version,
                investigation_id: investigation_id.to_owned(),
                kind: "accepted".into(),
                decided_by: ACTOR.into(),
                plan_fingerprint: Some(batch_plan_fingerprint.clone()),
                commit_id: Some(receipt.commit_id.clone()),
                receipt: Some(serde_json::to_value(&receipt).map_err(|error| error.to_string())?),
                batch_id: Some(batch_plan_fingerprint.clone()),
                reason: None,
                decided_at: timestamp(),
            })
            .map_err(|error| error.to_string())?;
    }
    Ok(receipt)
}

fn discard_one(
    service: &DesktopService,
    investigation_id: &str,
    selection: ChangeSetSelection,
    cascade: bool,
    reason: Option<String>,
) -> Result<(), String> {
    let mut store = store(service)?;
    let record = selection_record(&store, investigation_id, &selection)?;
    store
        .update_proposed_state(
            investigation_id,
            &record.change_set_id,
            record.version,
            "discarded",
            None,
        )
        .map_err(|error| error.to_string())?;
    store
        .record_change_set_decision(&ChangeSetDecision {
            decision_id: stable_request_id(
                "change-set-decision",
                &(record.change_set_id.as_str(), record.version, "discarded"),
            )?,
            change_set_id: record.change_set_id.clone(),
            version: record.version,
            investigation_id: investigation_id.to_owned(),
            kind: "discarded".into(),
            decided_by: ACTOR.into(),
            plan_fingerprint: None,
            commit_id: None,
            receipt: None,
            batch_id: None,
            reason,
            decided_at: timestamp(),
        })
        .map_err(|error| error.to_string())?;
    let pending = store
        .list_pending_change_sets(investigation_id)
        .map_err(|error| error.to_string())?;
    for item in pending {
        if item.dependencies.iter().any(|dependency| {
            dependency.depends_on_change_set_id == record.change_set_id
                && dependency.depends_on_version == record.version
        }) {
            if cascade {
                store
                    .update_proposed_state(
                        investigation_id,
                        &item.record.change_set_id,
                        item.record.version,
                        "discarded",
                        None,
                    )
                    .map_err(|error| error.to_string())?;
            } else {
                let mut result = item.record.result.clone();
                let debts = result.get_mut("debts").and_then(Value::as_array_mut);
                if let Some(debts) = debts {
                    debts.push(json!({ "kind": "validation", "code": "dependency_discarded", "message": format!("依赖 {} 已丢弃", record.change_set_id) }));
                }
                store
                    .update_proposed_state(
                        investigation_id,
                        &item.record.change_set_id,
                        item.record.version,
                        "blocked",
                        Some(&result),
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
    }
    Ok(())
}
