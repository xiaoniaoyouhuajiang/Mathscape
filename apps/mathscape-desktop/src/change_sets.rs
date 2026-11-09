//! Desktop-only review and commit commands for Phase2 staged proposals.
//!
//! Agent processes never receive these commands. They only create immutable
//! staged versions through MCP; this module binds a user decision to the
//! proposal fingerprints and a plan computed against the *current* head.
//!
//! * Every command names an investigation; it must be the window's own
//!   (`sandbox:{session}` of the bound sandbox) or a live sandbox session's,
//!   whose isolated copy is then the workspace. Anything else is refused.
//! * Accepting one or several versions is always one `UserChangeRequest`
//!   (topological order, SourceSpan creations deduplicated), one plan
//!   fingerprint and one map commit: [`SqliteArtifactStore::accept_change_sets`]
//!   does the write, the decisions and the state changes in one
//!   `BEGIN IMMEDIATE` transaction. Unselected versions stay `staged`.
//!   Accepting never raises Trust and is not a statement of correctness.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use mathscape_domain::{
    ActorRef, CommitReceipt, ImpactItem, ImpactPlan, Timestamp, UserChangeOperation,
    UserChangeRequest,
};
use mathscape_mcp::investigation::{
    record_operations, sandbox_investigation_id, staged_change_sets,
};
use mathscape_orchestrator::change_set_plan::{
    ChangeSetKey, StagedChangeSet, combined_operations, ordered_selection,
    staged_dependency_closure,
};
use mathscape_orchestrator::map_edit::plan_map_change;
use mathscape_store_sqlite::{
    ChangeSetAcceptance, ChangeSetSelectionRef, PendingChangeSet, SqliteArtifactStore,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::State;

use crate::DesktopState;
use crate::sandbox::SandboxState;

const ACTOR: &str = "mathscape-desktop-user";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct ChangeSetSelection {
    pub change_set_id: String,
    pub version: i64,
    pub proposal_fingerprint: String,
}

impl ChangeSetSelection {
    fn key(&self) -> ChangeSetKey {
        (self.change_set_id.clone(), self.version)
    }

    fn store_ref(&self) -> ChangeSetSelectionRef {
        ChangeSetSelectionRef {
            change_set_id: self.change_set_id.clone(),
            version: self.version,
            proposal_fingerprint: self.proposal_fingerprint.clone(),
        }
    }
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
    pub created_at: String,
    pub proposal_fingerprint: String,
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
#[serde(rename_all = "camelCase")]
pub struct ChangeSetQueue {
    pub investigation_id: String,
    pub base_commit: Option<String>,
    pub items: Vec<PendingChangeSetView>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchPlan {
    pub investigation_id: String,
    /// Head the plan was computed against; pass it back to accept.
    pub base_commit: Option<String>,
    /// Selections in the topological order they are written.
    pub selections: Vec<ChangeSetSelection>,
    pub impact: ImpactPlan,
    pub per_change_set: Vec<PerChangeSetPlan>,
    /// The planner fingerprint of the combined request (OCC token).
    pub batch_plan_fingerprint: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerChangeSetPlan {
    pub change_set_id: String,
    pub version: i64,
    pub direct_changes: Vec<ImpactItem>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImpactPreview {
    pub change_set_id: String,
    pub version: i64,
    /// Not-yet-accepted versions planned together with this one.
    pub planned_with: Vec<DependencyView>,
    pub impact: ImpactPlan,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchReceipt {
    pub commit_id: String,
    pub accepted: Vec<ChangeSetSelection>,
    pub receipt: CommitReceipt,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscardReceipt {
    pub discarded: Vec<DependencyView>,
    pub blocked: Vec<DependencyView>,
}

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

fn decided_at() -> String {
    format!(
        "unix:{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    )
}

fn actor() -> Result<ActorRef, String> {
    ActorRef::parse(ACTOR).map_err(|error| error.to_string())
}

fn open_store(root: &Path) -> Result<SqliteArtifactStore, String> {
    SqliteArtifactStore::open(&root.join("mathscape.sqlite3")).map_err(|error| error.to_string())
}

fn stable_id(prefix: &str, value: &impl Serialize) -> Result<String, String> {
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    Ok(format!("mathscape:{prefix}/{:x}", Sha256::digest(bytes)))
}

fn key_view((change_set_id, version): ChangeSetKey) -> DependencyView {
    DependencyView {
        change_set_id,
        version,
        referenced_ids: Vec::new(),
    }
}

/// Resolves the workspace of `investigation_id` and verifies it belongs to
/// this window: the bound workspace's own investigation, or a live sandbox
/// session (the dialogue dock may review a session the window is not in).
async fn workspace_for(
    desktop: &DesktopState,
    sandbox: &SandboxState,
    investigation_id: &str,
) -> Result<PathBuf, String> {
    let service = desktop.service();
    if service.investigation_id() == Some(investigation_id) {
        return Ok(service.workspace_root().to_path_buf());
    }
    if let Some(session_id) = investigation_id.strip_prefix("sandbox:") {
        let session_id = session_id.to_owned();
        let view = sandbox
            .run("change set workspace", move |home| {
                home.live_session(&session_id)
            })
            .await
            .map_err(|_| investigation_foreign())?;
        if sandbox_investigation_id(&view.sandbox_session_id) == investigation_id
            && let Some(root) = view.workspace_root
        {
            return Ok(root);
        }
    }
    Err(investigation_foreign())
}

fn investigation_foreign() -> String {
    "investigation_not_in_workspace: 调查不属于当前工作区".into()
}

async fn blocking<T, F>(work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| error.to_string())?
}

fn view(item: &PendingChangeSet) -> PendingChangeSetView {
    PendingChangeSetView {
        change_set_id: item.record.change_set_id.clone(),
        version: item.record.version,
        state: item.record.state.clone(),
        investigation_id: item.record.investigation_id.clone(),
        round_id: item.record.round_id.clone(),
        proposed_by: item.record.proposed_by.clone(),
        created_at: item.record.created_at.clone(),
        proposal_fingerprint: item.record.proposal_fingerprint().to_owned(),
        base_commit: item.record.base_commit.clone(),
        proposal: item.record.proposal.clone(),
        result: item.record.result.clone(),
        dependencies: item
            .dependencies
            .iter()
            .map(|dependency| DependencyView {
                change_set_id: dependency.depends_on_change_set_id.clone(),
                version: dependency.depends_on_version,
                referenced_ids: dependency.referenced_ids.clone(),
            })
            .collect(),
    }
}

pub fn list_queue(root: &Path, investigation_id: &str) -> Result<ChangeSetQueue, String> {
    let store = open_store(root)?;
    let items = store
        .list_pending_change_sets(investigation_id)
        .map_err(|error| error.to_string())?;
    let base_commit = store
        .map_edit_snapshot()
        .map_err(|error| error.to_string())?
        .base_commit;
    Ok(ChangeSetQueue {
        investigation_id: investigation_id.to_owned(),
        base_commit,
        items: items.iter().map(view).collect(),
    })
}

/// Checks each selection names the active `staged` version with the given
/// proposal fingerprint.
fn check_selections(
    versions: &[PendingChangeSet],
    selections: &[ChangeSetSelection],
) -> Result<(), String> {
    if selections.is_empty() {
        return Err("empty_selection: 至少选择一个提案".into());
    }
    let mut seen = BTreeSet::new();
    for selection in selections {
        if !seen.insert(selection.key()) {
            return Err(format!(
                "duplicate_selection: {}@{} 重复",
                selection.change_set_id, selection.version
            ));
        }
        let Some(item) = versions.iter().find(|item| {
            item.record.change_set_id == selection.change_set_id
                && item.record.version == selection.version
        }) else {
            return Err("proposal_not_found: 提案不存在或不属于当前调查".into());
        };
        if item.record.state != "staged" {
            return Err(format!(
                "proposal_not_staged: {}@{} 当前为 {}，不能接受",
                item.record.change_set_id, item.record.version, item.record.state
            ));
        }
        if item.record.proposal_fingerprint() != selection.proposal_fingerprint {
            return Err("proposal_fingerprint_conflict: 提案已被替换，请重新预览".into());
        }
    }
    Ok(())
}

/// The one combined request of a selection against `base_commit`.
fn batch_request(
    sets: &[StagedChangeSet],
    selections: &[ChangeSetSelection],
    base_commit: Option<String>,
) -> Result<(UserChangeRequest, Vec<ChangeSetKey>), String> {
    let keys: Vec<ChangeSetKey> = selections.iter().map(ChangeSetSelection::key).collect();
    let ordered = ordered_selection(sets, &keys).map_err(|error| closure_error(&error))?;
    let order: Vec<ChangeSetKey> = ordered.iter().map(|set| set.key()).collect();
    let mut sorted = selections.to_vec();
    sorted.sort();
    let request = UserChangeRequest {
        request_id: stable_id("change-set-accept", &(&sorted, &base_commit))?,
        base_commit,
        reason: if order.len() == 1 {
            "接受来源提案".into()
        } else {
            format!("批量接受 {} 个来源提案", order.len())
        },
        operations: combined_operations(ordered, &[]),
    };
    Ok((request, order))
}

fn closure_error(error: &mathscape_orchestrator::change_set_plan::ClosureError) -> String {
    format!("{}: {}", error.code(), error.message())
}

fn operation_target(operation: &UserChangeOperation) -> &str {
    match operation {
        UserChangeOperation::CreateSourceUnit { object_id, .. }
        | UserChangeOperation::CreateSourceRelation { object_id, .. }
        | UserChangeOperation::CreateSourceSpan { object_id, .. }
        | UserChangeOperation::EditSourceUnit { object_id, .. }
        | UserChangeOperation::EditSourceRelation { object_id, .. }
        | UserChangeOperation::RetireObject { object_id, .. } => object_id,
    }
}

pub fn plan_batch(
    root: &Path,
    investigation_id: &str,
    selections: Vec<ChangeSetSelection>,
) -> Result<BatchPlan, String> {
    let store = open_store(root)?;
    let versions = store
        .list_change_set_versions(investigation_id)
        .map_err(|error| error.to_string())?;
    check_selections(&versions, &selections)?;
    let sets = staged_change_sets(&versions);
    let snapshot = store
        .map_edit_snapshot()
        .map_err(|error| error.to_string())?;
    let (request, order) = batch_request(&sets, &selections, snapshot.base_commit.clone())?;
    let impact = plan_map_change(&snapshot, &request, &actor()?).impact;
    // Attribute each direct change to the first selected version touching it.
    let mut owner = BTreeMap::<String, ChangeSetKey>::new();
    for key in &order {
        let record = versions
            .iter()
            .find(|item| (item.record.change_set_id.clone(), item.record.version) == *key)
            .expect("selection was checked");
        for operation in record_operations(&record.record) {
            owner
                .entry(operation_target(&operation).to_owned())
                .or_insert_with(|| key.clone());
        }
    }
    let per_change_set = order
        .iter()
        .map(|key| PerChangeSetPlan {
            change_set_id: key.0.clone(),
            version: key.1,
            direct_changes: impact
                .direct_changes
                .iter()
                .filter(|item| owner.get(&item.object_id) == Some(key))
                .cloned()
                .collect(),
        })
        .collect();
    let by_key: BTreeMap<ChangeSetKey, ChangeSetSelection> = selections
        .into_iter()
        .map(|selection| (selection.key(), selection))
        .collect();
    Ok(BatchPlan {
        investigation_id: investigation_id.to_owned(),
        base_commit: snapshot.base_commit,
        selections: order.iter().map(|key| by_key[key].clone()).collect(),
        batch_plan_fingerprint: impact.plan_fingerprint.clone(),
        impact,
        per_change_set,
    })
}

/// Read-only impact preview of one version: planned against the current
/// head together with its not-yet-accepted dependency closure.
pub fn preview_impact(
    root: &Path,
    investigation_id: &str,
    change_set_id: &str,
    version: i64,
) -> Result<ImpactPreview, String> {
    let store = open_store(root)?;
    let versions = store
        .list_change_set_versions(investigation_id)
        .map_err(|error| error.to_string())?;
    let sets = staged_change_sets(&versions);
    let key = (change_set_id.to_owned(), version);
    let own = sets
        .iter()
        .find(|set| set.key() == key)
        .ok_or("proposal_not_found: 提案不存在或不属于当前调查")?;
    let closure = staged_dependency_closure(&sets, std::slice::from_ref(&key))
        .map_err(|e| closure_error(&e))?;
    let planned_with = closure.iter().map(|set| key_view(set.key())).collect();
    let snapshot = store
        .map_edit_snapshot()
        .map_err(|error| error.to_string())?;
    let request = UserChangeRequest {
        request_id: stable_id("change-set-preview", &(&key, &snapshot.base_commit))?,
        base_commit: snapshot.base_commit.clone(),
        reason: "影响预览".into(),
        operations: combined_operations(closure, &own.operations),
    };
    Ok(ImpactPreview {
        change_set_id: change_set_id.to_owned(),
        version,
        planned_with,
        impact: plan_map_change(&snapshot, &request, &actor()?).impact,
    })
}

/// Accepts `selections` as one commit. `base_commit` and
/// `batch_plan_fingerprint` come from [`plan_batch`]; any change of the head
/// or of the plan in between is a conflict and nothing is written.
pub fn accept_batch(
    root: &Path,
    investigation_id: &str,
    selections: Vec<ChangeSetSelection>,
    base_commit: Option<String>,
    batch_plan_fingerprint: String,
) -> Result<BatchReceipt, String> {
    let mut store = open_store(root)?;
    let versions = store
        .list_change_set_versions(investigation_id)
        .map_err(|error| error.to_string())?;
    check_selections(&versions, &selections)?;
    let sets = staged_change_sets(&versions);
    let (request, order) = batch_request(&sets, &selections, base_commit)?;
    let actor = actor()?;
    let acceptance = ChangeSetAcceptance {
        investigation_id: investigation_id.to_owned(),
        selections: selections
            .iter()
            .map(ChangeSetSelection::store_ref)
            .collect(),
        request: request.clone(),
        expected_plan: batch_plan_fingerprint.clone(),
        actor: actor.clone(),
        time: now()?,
        batch_id: (order.len() > 1).then(|| batch_plan_fingerprint.clone()),
        decided_at: decided_at(),
    };
    let receipt = store
        .accept_change_sets(&acceptance, move |snapshot| {
            let planned = plan_map_change(snapshot, &request, &actor);
            (planned.impact, planned.patch)
        })
        .map_err(|error| error.to_string())?;
    let by_key: BTreeMap<ChangeSetKey, ChangeSetSelection> = selections
        .into_iter()
        .map(|selection| (selection.key(), selection))
        .collect();
    Ok(BatchReceipt {
        commit_id: receipt.commit_id.clone(),
        accepted: order.iter().map(|key| by_key[key].clone()).collect(),
        receipt,
    })
}

pub fn discard(
    root: &Path,
    investigation_id: &str,
    selections: Vec<ChangeSetSelection>,
    cascade: bool,
    reason: Option<String>,
) -> Result<DiscardReceipt, String> {
    let mut store = open_store(root)?;
    let outcome = store
        .discard_change_sets(
            investigation_id,
            &selections
                .iter()
                .map(ChangeSetSelection::store_ref)
                .collect::<Vec<_>>(),
            cascade,
            ACTOR,
            reason.as_deref(),
            &decided_at(),
        )
        .map_err(|error| error.to_string())?;
    Ok(DiscardReceipt {
        discarded: outcome.discarded.into_iter().map(key_view).collect(),
        blocked: outcome.blocked.into_iter().map(key_view).collect(),
    })
}

/// The investigation of a sandbox session, or of this window when
/// `sandbox_session_id` is omitted (`None` on the mother workspace).
#[tauri::command]
pub async fn get_change_set_investigation(
    sandbox_session_id: Option<String>,
    desktop: State<'_, DesktopState>,
    sandbox: State<'_, SandboxState>,
) -> Result<Option<String>, String> {
    let Some(session_id) = sandbox_session_id else {
        return Ok(desktop.service().investigation_id().map(str::to_owned));
    };
    let investigation = sandbox_investigation_id(&session_id);
    workspace_for(&desktop, &sandbox, &investigation).await?;
    Ok(Some(investigation))
}

#[tauri::command]
pub async fn list_pending_change_sets(
    investigation_id: String,
    desktop: State<'_, DesktopState>,
    sandbox: State<'_, SandboxState>,
) -> Result<ChangeSetQueue, String> {
    let root = workspace_for(&desktop, &sandbox, &investigation_id).await?;
    blocking(move || list_queue(&root, &investigation_id)).await
}

#[tauri::command]
pub async fn preview_change_set_impact(
    investigation_id: String,
    change_set_id: String,
    version: i64,
    desktop: State<'_, DesktopState>,
    sandbox: State<'_, SandboxState>,
) -> Result<ImpactPreview, String> {
    let root = workspace_for(&desktop, &sandbox, &investigation_id).await?;
    blocking(move || preview_impact(&root, &investigation_id, &change_set_id, version)).await
}

#[tauri::command]
pub async fn plan_change_set_batch(
    investigation_id: String,
    selections: Vec<ChangeSetSelection>,
    desktop: State<'_, DesktopState>,
    sandbox: State<'_, SandboxState>,
) -> Result<BatchPlan, String> {
    let root = workspace_for(&desktop, &sandbox, &investigation_id).await?;
    blocking(move || plan_batch(&root, &investigation_id, selections)).await
}

/// A single accept is a batch of one.
#[tauri::command]
pub async fn accept_change_set_batch(
    investigation_id: String,
    selections: Vec<ChangeSetSelection>,
    base_commit: Option<String>,
    batch_plan_fingerprint: String,
    desktop: State<'_, DesktopState>,
    sandbox: State<'_, SandboxState>,
) -> Result<BatchReceipt, String> {
    let root = workspace_for(&desktop, &sandbox, &investigation_id).await?;
    blocking(move || {
        accept_batch(
            &root,
            &investigation_id,
            selections,
            base_commit,
            batch_plan_fingerprint,
        )
    })
    .await
}

#[tauri::command]
pub async fn discard_change_set_batch(
    investigation_id: String,
    selections: Vec<ChangeSetSelection>,
    cascade: bool,
    reason: Option<String>,
    desktop: State<'_, DesktopState>,
    sandbox: State<'_, SandboxState>,
) -> Result<DiscardReceipt, String> {
    let root = workspace_for(&desktop, &sandbox, &investigation_id).await?;
    blocking(move || discard(&root, &investigation_id, selections, cascade, reason)).await
}

#[cfg(test)]
mod tests {
    //! End-to-end: proposals are created through the real MCP server, then
    //! reviewed with the Desktop commands above on the same workspace.

    use super::*;
    use mathscape_artifact::{ArtifactMetadata, ArtifactPool};
    use mathscape_domain::{MediaType, RightsRecord, RightsStatus};
    use mathscape_mcp::{McpServer, McpWorkspace};
    use serde_json::json;

    const MD: &str = "# 定理 A\n\n第一段内容。\n\n# 定理 B\n\n第二段内容。\n";
    const INV: &str = "sandbox:session-test";

    struct Fx {
        _dir: tempfile::TempDir,
        root: PathBuf,
        md: String,
    }

    fn fixture_at(root: PathBuf, dir: tempfile::TempDir) -> Fx {
        let mut pool = ArtifactPool::create(&root).unwrap();
        let metadata = ArtifactMetadata::new(
            None,
            MediaType::parse("text/markdown").unwrap(),
            Some("notes.md".into()),
            None,
            RightsRecord::new(RightsStatus::NotAssessed, vec![], None).unwrap(),
        );
        let md = pool
            .import_bytes(MD.as_bytes(), metadata, ActorRef::parse("test").unwrap())
            .unwrap()
            .stable_id()
            .to_string();
        drop(pool);
        Fx {
            _dir: dir,
            root,
            md,
        }
    }

    fn fixture() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ws");
        fixture_at(root, dir)
    }

    fn call_as(
        fx: &Fx,
        round: &str,
        investigation: Option<&str>,
        tool: &str,
        args: Value,
    ) -> (Value, bool) {
        let server = McpServer::new(
            McpWorkspace::open(&fx.root)
                .unwrap()
                .with_round(Some(round.into()))
                .with_investigation(investigation.map(str::to_owned)),
        );
        let response = server
            .handle_message(json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": tool, "arguments": args }
            }))
            .unwrap();
        (
            response["result"]["structuredContent"].clone(),
            response["result"]["isError"].as_bool().unwrap_or(true),
        )
    }

    fn propose(fx: &Fx, body: Value) -> Value {
        let (result, is_error) = call_as(fx, "round-1", Some(INV), "propose_changes", body);
        assert!(!is_error, "{result}");
        result
    }

    fn range(text: &str) -> (usize, usize) {
        let start = MD.find(text).unwrap();
        (start, start + text.len())
    }

    fn span(fx: &Fx, local: &str, text: &str, granularity: &str) -> Value {
        let (start, end) = range(text);
        json!({ "local_ref": local, "kind": "source_span", "op": "create", "payload": {
            "artifact_id": fx.md, "md_byte_range": { "start": start, "end": end },
            "granularity": granularity, "quote": text
        }})
    }

    fn unit(local: &str, title: &str, parent: Value, spans: Value) -> Value {
        json!({ "local_ref": local, "kind": "source_unit", "op": "create", "payload": {
            "kind": "statement", "display_title": title, "parent": parent,
            "source_spans": spans, "completeness": "complete"
        }})
    }

    /// One unit with its own span over `text`.
    fn unit_proposal(fx: &Fx, request: &str, title: &str, text: &str, parent: Value) -> Value {
        propose(
            fx,
            json!({
                "request_id": request, "rationale": format!("登记 {title}"),
                "proposed_objects": [
                    span(fx, "s", text, "inline"),
                    unit("u", title, parent, json!(["$new:s"]))
                ]
            }),
        )
    }

    fn sel(value: &Value) -> ChangeSetSelection {
        ChangeSetSelection {
            change_set_id: value["change_set_id"].as_str().unwrap().into(),
            version: value["version"].as_i64().unwrap(),
            proposal_fingerprint: value["proposal_fingerprint"].as_str().unwrap().into(),
        }
    }

    fn accept(fx: &Fx, selections: Vec<ChangeSetSelection>) -> Result<BatchReceipt, String> {
        let plan = plan_batch(&fx.root, INV, selections.clone())?;
        accept_batch(
            &fx.root,
            INV,
            selections,
            plan.base_commit,
            plan.batch_plan_fingerprint,
        )
    }

    fn state(fx: &Fx, value: &Value) -> String {
        open_store(&fx.root)
            .unwrap()
            .proposed_change(
                INV,
                value["change_set_id"].as_str().unwrap(),
                value["version"].as_i64(),
            )
            .unwrap()
            .unwrap()
            .0
            .state
    }

    fn record(fx: &Fx, value: &Value) -> mathscape_store_sqlite::ProposedChangeRecord {
        open_store(&fx.root)
            .unwrap()
            .proposed_change(
                INV,
                value["change_set_id"].as_str().unwrap(),
                value["version"].as_i64(),
            )
            .unwrap()
            .unwrap()
            .0
    }

    fn has_object(fx: &Fx, id: &str) -> bool {
        open_store(&fx.root)
            .unwrap()
            .map_edit_snapshot()
            .unwrap()
            .objects
            .iter()
            .any(|object| object.object_id == id)
    }

    fn commit_count(fx: &Fx) -> i64 {
        rusqlite::Connection::open(fx.root.join("mathscape.sqlite3"))
            .unwrap()
            .query_row("SELECT COUNT(*) FROM map_commits", [], |row| row.get(0))
            .unwrap()
    }

    fn staged_id(value: &Value, local: &str) -> String {
        value["staged_ids"][local].as_str().unwrap().to_owned()
    }

    fn codes(value: &Value) -> Vec<String> {
        value["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["code"].as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn single_accept_commits_once_and_leaves_unselected_staged() {
        let fx = fixture();
        let a = unit_proposal(&fx, "a", "定理 A", "# 定理 A", Value::Null);
        let b = unit_proposal(&fx, "b", "定理 B", "# 定理 B", Value::Null);
        assert_eq!(a["status"], "staged", "{a}");
        let receipt = accept(&fx, vec![sel(&a)]).unwrap();
        assert_eq!(commit_count(&fx), 1);
        assert_eq!(state(&fx, &a), "accepted");
        assert_eq!(state(&fx, &b), "staged");
        assert!(has_object(&fx, &staged_id(&a, "u")));
        assert!(!has_object(&fx, &staged_id(&b, "u")));
        let decisions = open_store(&fx.root)
            .unwrap()
            .change_set_decisions(INV)
            .unwrap();
        assert_eq!(decisions.len(), 1);
        assert_eq!(
            decisions[0].commit_id.as_deref(),
            Some(receipt.commit_id.as_str())
        );
        // Accepting is not a mathematical endorsement: Trust is untouched.
        let unit = open_store(&fx.root)
            .unwrap()
            .map_edit_snapshot()
            .unwrap()
            .objects
            .into_iter()
            .find(|object| object.object_id == staged_id(&a, "u"))
            .unwrap();
        assert!(unit.payload.get("trust").is_none());
    }

    #[test]
    fn batch_accept_is_one_commit_and_deduplicates_a_shared_span() {
        let fx = fixture();
        // Both proposals create the same span (same range → same id).
        let a = unit_proposal(&fx, "a", "定理 A", "第一段内容。", Value::Null);
        let b = unit_proposal(&fx, "b", "定理 A 的别名", "第一段内容。", Value::Null);
        assert_eq!(staged_id(&a, "s"), staged_id(&b, "s"));
        assert_eq!(b["status"], "staged", "{b}");
        let plan = plan_batch(&fx.root, INV, vec![sel(&a), sel(&b)]).unwrap();
        assert_eq!(
            plan.impact.status,
            mathscape_domain::ImpactPlanStatus::Ready,
            "{:?}",
            plan.impact.diagnostics
        );
        let receipt = accept(&fx, vec![sel(&a), sel(&b)]).unwrap();
        assert_eq!(commit_count(&fx), 1);
        let decisions = open_store(&fx.root)
            .unwrap()
            .change_set_decisions(INV)
            .unwrap();
        assert_eq!(decisions.len(), 2);
        assert!(
            decisions
                .iter()
                .all(|d| d.commit_id.as_deref() == Some(receipt.commit_id.as_str()))
        );
        assert!(decisions.iter().all(|d| d.batch_id.is_some()));
        let snapshot = open_store(&fx.root).unwrap().map_edit_snapshot().unwrap();
        assert_eq!(
            snapshot
                .objects
                .iter()
                .filter(|o| o.object_id == staged_id(&a, "s"))
                .count(),
            1
        );
    }

    #[test]
    fn stale_head_is_an_occ_conflict_and_writes_nothing() {
        let fx = fixture();
        let a = unit_proposal(&fx, "a", "定理 A", "# 定理 A", Value::Null);
        let b = unit_proposal(&fx, "b", "定理 B", "# 定理 B", Value::Null);
        let stale = plan_batch(&fx.root, INV, vec![sel(&a)]).unwrap();
        accept(&fx, vec![sel(&b)]).unwrap();
        let error = accept_batch(
            &fx.root,
            INV,
            vec![sel(&a)],
            stale.base_commit,
            stale.batch_plan_fingerprint,
        )
        .unwrap_err();
        assert!(error.contains("base_commit_conflict"), "{error}");
        assert_eq!(state(&fx, &a), "staged");
        assert_eq!(commit_count(&fx), 1);
        // Re-planned against the new head it goes through.
        accept(&fx, vec![sel(&a)]).unwrap();
        assert_eq!(state(&fx, &a), "accepted");
        // A forged fingerprint for the current head is a plan conflict.
        let c = unit_proposal(&fx, "c", "定理 C", "第二段内容。", Value::Null);
        let plan = plan_batch(&fx.root, INV, vec![sel(&c)]).unwrap();
        let error = accept_batch(
            &fx.root,
            INV,
            vec![sel(&c)],
            plan.base_commit,
            "sha256:00".into(),
        )
        .unwrap_err();
        assert!(error.contains("plan_conflict"), "{error}");
    }

    #[test]
    fn dependent_needs_its_dependency_accepted_first_or_in_the_same_batch() {
        let fx = fixture();
        let a = unit_proposal(&fx, "a", "父", "# 定理 A", Value::Null);
        let b = unit_proposal(&fx, "b", "子", "第一段内容。", json!(staged_id(&a, "u")));
        assert_eq!(b["status"], "staged", "{b}");
        assert_eq!(b["dependencies"][0]["change_set_id"], a["change_set_id"]);
        let error = plan_batch(&fx.root, INV, vec![sel(&b)]).unwrap_err();
        assert!(error.contains("dependency_not_accepted"), "{error}");
        accept(&fx, vec![sel(&a)]).unwrap();
        accept(&fx, vec![sel(&b)]).unwrap();
        assert_eq!(state(&fx, &b), "accepted");
        assert_eq!(commit_count(&fx), 2);
    }

    #[test]
    fn multi_level_chain_plans_the_full_closure_and_a_deferred_item_is_accepted_later() {
        let fx = fixture();
        let a = unit_proposal(&fx, "a", "A", "# 定理 A", Value::Null);
        let b = unit_proposal(&fx, "b", "B", "第一段内容。", json!(staged_id(&a, "u")));
        // C only references B; staging it requires planning A as well.
        let c = unit_proposal(&fx, "c", "C", "# 定理 B", json!(staged_id(&b, "u")));
        assert_eq!(c["status"], "staged", "{c}");
        assert_eq!(c["dependencies"].as_array().unwrap().len(), 1);
        let preview =
            preview_impact(&fx.root, INV, c["change_set_id"].as_str().unwrap(), 1).unwrap();
        assert_eq!(preview.planned_with.len(), 2);
        assert_eq!(
            preview.impact.status,
            mathscape_domain::ImpactPlanStatus::Ready
        );
        // Batch order is topological regardless of the selection order.
        let plan = plan_batch(&fx.root, INV, vec![sel(&b), sel(&a)]).unwrap();
        assert_eq!(
            plan.selections[0].change_set_id,
            a["change_set_id"].as_str().unwrap()
        );
        accept(&fx, vec![sel(&b), sel(&a)]).unwrap();
        assert_eq!(state(&fx, &c), "staged");
        accept(&fx, vec![sel(&c)]).unwrap();
        assert_eq!(state(&fx, &c), "accepted");
    }

    #[test]
    fn discard_blocks_dependents_by_default_and_cascades_on_request() {
        let fx = fixture();
        let a = unit_proposal(&fx, "a", "A", "# 定理 A", Value::Null);
        let b = unit_proposal(&fx, "b", "B", "第一段内容。", json!(staged_id(&a, "u")));
        let c = unit_proposal(&fx, "c", "C", "# 定理 B", json!(staged_id(&b, "u")));
        let outcome = discard(&fx.root, INV, vec![sel(&a)], false, None).unwrap();
        assert_eq!(outcome.discarded.len(), 1);
        assert_eq!(outcome.blocked.len(), 2);
        assert_eq!(state(&fx, &b), "blocked");
        assert_eq!(state(&fx, &c), "blocked");
        let b_debts = record(&fx, &b).result["debts"].to_string();
        assert!(b_debts.contains("dependency_discarded"), "{b_debts}");
        assert!(
            record(&fx, &c).result["debts"]
                .to_string()
                .contains("dependency_blocked")
        );
        // Blocked versions can still be discarded.
        discard(&fx.root, INV, vec![sel(&b)], false, None).unwrap();
        assert_eq!(state(&fx, &b), "discarded");

        let fx = fixture();
        let a = unit_proposal(&fx, "a", "A", "# 定理 A", Value::Null);
        let b = unit_proposal(&fx, "b", "B", "第一段内容。", json!(staged_id(&a, "u")));
        let c = unit_proposal(&fx, "c", "C", "# 定理 B", json!(staged_id(&b, "u")));
        let outcome = discard(&fx.root, INV, vec![sel(&a)], true, Some("不需要".into())).unwrap();
        assert_eq!(outcome.discarded.len(), 3);
        assert_eq!(state(&fx, &b), "discarded");
        assert_eq!(state(&fx, &c), "discarded");
        let decisions = open_store(&fx.root)
            .unwrap()
            .change_set_decisions(INV)
            .unwrap();
        assert_eq!(
            decisions.iter().filter(|d| d.kind == "discarded").count(),
            3
        );
        assert_eq!(commit_count(&fx), 0);
    }

    #[test]
    fn superseding_blocks_dependents_without_retargeting() {
        let fx = fixture();
        let a = unit_proposal(&fx, "a", "A", "# 定理 A", Value::Null);
        let b = unit_proposal(&fx, "b", "B", "第一段内容。", json!(staged_id(&a, "u")));
        let c = unit_proposal(&fx, "c", "C", "# 定理 B", json!(staged_id(&b, "u")));
        let a2 = propose(
            &fx,
            json!({
                "request_id": "a-v2", "rationale": "改标题",
                "supersedes": {
                    "change_set_id": a["change_set_id"], "version": 1,
                    "proposal_fingerprint": a["proposal_fingerprint"]
                },
                "proposed_objects": [
                    span(&fx, "s", "# 定理 A", "inline"),
                    unit("u", "A（新）", Value::Null, json!(["$new:s"]))
                ]
            }),
        );
        assert_eq!(a2["version"], 2);
        assert_eq!(state(&fx, &a), "superseded");
        assert_eq!(state(&fx, &b), "blocked");
        assert_eq!(state(&fx, &c), "blocked");
        assert!(
            record(&fx, &b).result["debts"]
                .to_string()
                .contains("dependency_superseded")
        );
        let dependencies = open_store(&fx.root)
            .unwrap()
            .dependencies_for(INV, b["change_set_id"].as_str().unwrap(), 1)
            .unwrap();
        assert_eq!(dependencies[0].depends_on_version, 1, "never retargeted");
        assert!(
            plan_batch(&fx.root, INV, vec![sel(&b)])
                .unwrap_err()
                .contains("proposal_not_staged")
        );
    }

    #[test]
    fn investigation_spans_rounds_and_rounds_are_bound_to_one_investigation() {
        let fx = fixture();
        let a = unit_proposal(&fx, "a", "A", "# 定理 A", Value::Null);
        // A second round of the same sandbox session sees and builds on A.
        let (read, is_error) = call_as(
            &fx,
            "round-2",
            Some(INV),
            "get_change_set",
            json!({ "change_set_id": a["change_set_id"] }),
        );
        assert!(!is_error);
        assert_eq!(read["status"], "ok");
        let (b, _) = call_as(
            &fx,
            "round-2",
            Some(INV),
            "propose_changes",
            json!({
                "request_id": "b", "rationale": "子",
                "proposed_objects": [span(&fx, "s", "第一段内容。", "inline"),
                    unit("u", "B", json!(staged_id(&a, "u")), json!(["$new:s"]))]
            }),
        );
        assert_eq!(b["status"], "staged", "{b}");
        assert_eq!(b["investigation_id"], INV);
        assert_eq!(b["dependencies"][0]["change_set_id"], a["change_set_id"]);
        // round-1 is bound to INV; claiming another investigation is refused.
        let (refused, is_error) = call_as(
            &fx,
            "round-1",
            Some("sandbox:other"),
            "propose_changes",
            json!({
                "request_id": "x", "rationale": "越权",
                "proposed_objects": [span(&fx, "s", "# 定理 B", "inline"),
                    unit("u", "X", Value::Null, json!(["$new:s"]))]
            }),
        );
        assert!(is_error, "{refused}");
        assert!(
            refused.to_string().contains("investigation_mismatch"),
            "{refused}"
        );
        // Without an adapter value the registered binding is used.
        let (read, _) = call_as(
            &fx,
            "round-1",
            None,
            "get_change_set",
            json!({ "change_set_id": a["change_set_id"] }),
        );
        assert_eq!(read["status"], "ok");
        // Another investigation cannot read it.
        let (read, _) = call_as(
            &fx,
            "round-9",
            Some("sandbox:other"),
            "get_change_set",
            json!({ "change_set_id": a["change_set_id"] }),
        );
        assert_eq!(read["status"], "not_found");
        let error = plan_batch(&fx.root, "sandbox:other", vec![sel(&a)]).unwrap_err();
        assert!(error.contains("proposal_not_found"), "{error}");
    }

    #[test]
    fn locator_failures_are_explicit_diagnostics() {
        let fx = fixture();
        let missing = propose(
            &fx,
            json!({
                "request_id": "dangling", "rationale": "悬空",
                "proposed_objects": [span(&fx, "s", "# 定理 A", "inline"),
                    unit("u", "X", json!("mathscape:source-unit/nope"), json!(["$new:s"]))]
            }),
        );
        assert_eq!(missing["status"], "blocked");
        assert!(
            codes(&missing).contains(&"dangling_staged_ref".to_owned()),
            "{missing}"
        );

        // No canonical PDF mapping for this snapshot.
        let page = propose(
            &fx,
            json!({
                "request_id": "page", "rationale": "页",
                "proposed_objects": [span(&fx, "s", "# 定理 A", "page"),
                    unit("u", "X", Value::Null, json!(["$new:s"]))]
            }),
        );
        assert!(
            codes(&page).contains(&"page_mapping_unavailable".to_owned()),
            "{page}"
        );

        // The blank line between blocks belongs to no canonical block.
        let (start, _) = range("\n\n第一段");
        let block = propose(
            &fx,
            json!({
                "request_id": "block", "rationale": "块",
                "proposed_objects": [
                    { "local_ref": "s", "kind": "source_span", "op": "create", "payload": {
                        "artifact_id": fx.md, "md_byte_range": { "start": start + 1, "end": start + 2 },
                        "granularity": "block", "quote": "\n" }},
                    unit("u", "X", Value::Null, json!(["$new:s"]))]
            }),
        );
        assert!(
            codes(&block).contains(&"block_locator_unavailable".to_owned()),
            "{block}"
        );

        // Quote is cut-then-compared; text elsewhere in the snapshot does not count.
        let (start, end) = range("# 定理 A");
        let mismatch = propose(
            &fx,
            json!({
                "request_id": "quote", "rationale": "引文",
                "proposed_objects": [
                    { "local_ref": "s", "kind": "source_span", "op": "create", "payload": {
                        "artifact_id": fx.md, "md_byte_range": { "start": start, "end": end },
                        "granularity": "inline", "quote": "# 定理 B" }},
                    unit("u", "X", Value::Null, json!(["$new:s"]))]
            }),
        );
        assert_eq!(codes(&mismatch)[0], "quote_mismatch");
        let normalized = propose(
            &fx,
            json!({
                "request_id": "quote-ws", "rationale": "空白",
                "proposed_objects": [
                    { "local_ref": "s", "kind": "source_span", "op": "create", "payload": {
                        "artifact_id": fx.md, "md_byte_range": { "start": start, "end": end },
                        "granularity": "inline", "quote": "#  定理   A" }},
                    unit("u", "X", Value::Null, json!(["$new:s"]))]
            }),
        );
        assert_eq!(normalized["status"], "staged", "{normalized}");
    }

    #[test]
    fn proposal_span_hash_matches_discovery_semantics() {
        let fx = fixture();
        let a = unit_proposal(&fx, "a", "A", "第一段内容。", Value::Null);
        let draft = &a["compiled_operations"][0]["CreateSourceSpan"]["draft"];
        let draft = if draft.is_null() {
            &a["compiled_operations"][0]["draft"]
        } else {
            draft
        };
        assert_eq!(
            draft["normalized_text_hash"],
            json!(mathscape_orchestrator::source_text_hash("第一段内容。").as_str())
        );
        let (start, end) = range("第一段内容。");
        let expected_id = mathscape_orchestrator::markdown_span_id(
            &mathscape_domain::StableId::parse(fx.md.clone()).unwrap(),
            &mathscape_domain::TextByteRange {
                start: start as u64,
                end: end as u64,
            },
        );
        assert_eq!(staged_id(&a, "s"), expected_id.to_string());
    }

    #[test]
    fn retired_span_with_the_same_id_blocks_as_span_inactive() {
        let fx = fixture();
        let a = unit_proposal(&fx, "a", "A", "# 定理 A", Value::Null);
        accept(&fx, vec![sel(&a)]).unwrap();
        rusqlite::Connection::open(fx.root.join("mathscape.sqlite3"))
            .unwrap()
            .execute(
                "UPDATE map_object_revisions SET lifecycle = 'deprecated' WHERE object_id = ?1",
                [staged_id(&a, "s")],
            )
            .unwrap();
        let again = unit_proposal(&fx, "again", "A2", "# 定理 A", Value::Null);
        assert_eq!(again["status"], "blocked");
        assert!(
            codes(&again).contains(&"span_inactive".to_owned()),
            "{again}"
        );
    }

    #[test]
    fn sandbox_session_proposals_never_touch_the_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let mother = dir.path().join("mother");
        let fx = fixture_at(mother.clone(), tempfile::tempdir().unwrap());
        let before = mathscape_sandbox::fingerprint_workspace(&mother).unwrap();
        let home = mathscape_sandbox::SandboxHome::open(dir.path().join("sandbox")).unwrap();
        let checkpoint = home.register_checkpoint(&mother, None).unwrap();
        let session = home.start_session(&checkpoint.checkpoint_id).unwrap();
        let copy = Fx {
            _dir: tempfile::tempdir().unwrap(),
            root: session.workspace_root.clone().unwrap(),
            md: fx.md.clone(),
        };
        let a = unit_proposal(&copy, "a", "A", "# 定理 A", Value::Null);
        accept(&copy, vec![sel(&a)]).unwrap();
        assert!(has_object(&copy, &staged_id(&a, "u")));
        assert!(!has_object(&fx, &staged_id(&a, "u")));
        assert!(
            mathscape_sandbox::fingerprint_workspace(&mother)
                .unwrap()
                .same_content(&before)
        );
    }
}
