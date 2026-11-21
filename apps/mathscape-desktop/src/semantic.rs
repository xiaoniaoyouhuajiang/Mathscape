//! 语义层的 Desktop 命令：对象详情（含沙箱暂存对象与前后对比）、材料研读概览（学习视图）与本地学习进度标记。

use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use mathscape_domain::MapEditSnapshot;
use mathscape_mcp::investigation::investigation_overlay;
use mathscape_orchestrator::semantic_view::{
    LearningView, LearningViewContext, SemanticObjectDetail, StagedSemanticMeta, StudyMarkEntry,
    learning_view_with_context, semantic_object_detail,
};
use mathscape_query::CanonicalAliasTable;
use mathscape_store_sqlite::{SqliteArtifactStore, StudyMark, StudyMarkUpdate};
use tauri::State;

use crate::{DesktopService, DesktopState};

fn open_store(service: &DesktopService) -> Result<SqliteArtifactStore, String> {
    SqliteArtifactStore::open(&service.workspace_root().join("mathscape.sqlite3"))
        .map_err(|error| error.to_string())
}

fn now_unix() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("unix:{secs}")
}

fn inner_payload(value: &serde_json::Value) -> &serde_json::Value {
    value.get("payload").unwrap_or(value)
}

pub fn merged_snapshot(
    store: &SqliteArtifactStore,
    investigation_id: Option<&str>,
    include_staged: bool,
) -> Result<
    (
        MapEditSnapshot,
        Option<mathscape_orchestrator::change_set_plan::InvestigationOverlay>,
    ),
    String,
> {
    let accepted = store
        .map_edit_snapshot()
        .map_err(|error| error.to_string())?;
    if !include_staged {
        return Ok((accepted, None));
    }
    let Some(investigation_id) = investigation_id else {
        return Ok((accepted, None));
    };
    let overlay =
        investigation_overlay(store, investigation_id).map_err(|error| error.to_string())?;
    let mut merged = accepted;
    for staged in &overlay.objects {
        if let Some(pos) = merged
            .objects
            .iter()
            .position(|existing| existing.object_id == staged.object.object_id)
        {
            merged.objects[pos] = staged.object.clone();
        } else {
            merged.objects.push(staged.object.clone());
        }
    }
    Ok((merged, Some(overlay)))
}

#[allow(dead_code)]
pub fn object_detail(
    service: &DesktopService,
    object_id: &str,
) -> Result<SemanticObjectDetail, String> {
    object_detail_with_scope(service, object_id, true)
}

pub fn object_detail_with_scope(
    service: &DesktopService,
    object_id: &str,
    include_staged: bool,
) -> Result<SemanticObjectDetail, String> {
    let store = open_store(service)?;
    let accepted = store
        .map_edit_snapshot()
        .map_err(|error| error.to_string())?;
    let (snapshot, overlay) = merged_snapshot(&store, service.investigation_id(), include_staged)?;
    let mut detail = semantic_object_detail(&snapshot, object_id)
        .ok_or_else(|| format!("not_found：{object_id} 不是语义对象"))?;
    if let Some(overlay) = overlay
        && let Some(staged) = overlay
            .objects
            .iter()
            .find(|item| item.object.object_id == object_id)
    {
        let accepted_payload = staged.replaces_revision.as_ref().and_then(|_| {
            accepted
                .objects
                .iter()
                .find(|obj| obj.object_id == object_id)
                .map(|obj| inner_payload(&obj.payload).clone())
        });
        detail.staged = Some(StagedSemanticMeta {
            change_set_id: staged.change_set_id.clone(),
            change_set_version: staged.version,
            change_sets: staged
                .change_sets
                .iter()
                .map(|(id, ver)| format!("{id}@{ver}"))
                .collect(),
            replaces_revision: staged.replaces_revision.clone(),
            accepted_payload,
        });
    }
    Ok(detail)
}

pub fn learning_with_scope(
    service: &DesktopService,
    include_staged: bool,
) -> Result<LearningView, String> {
    let store = open_store(service)?;
    let (snapshot, overlay) = merged_snapshot(&store, service.investigation_id(), include_staged)?;
    let canonical_views = store
        .list_canonical_markdown()
        .map_err(|error| error.to_string())?;
    let alias_table = CanonicalAliasTable::from_views(&canonical_views);
    let artifact_aliases: BTreeMap<String, String> = alias_table
        .aliases()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let study_marks: BTreeMap<String, StudyMarkEntry> = store
        .list_study_marks()
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|(id, mark)| {
            (
                id,
                StudyMarkEntry {
                    status: mark.status.as_str().to_owned(),
                    note: mark.note,
                },
            )
        })
        .collect();
    let staged_ids: BTreeSet<String> = overlay
        .as_ref()
        .map(|ov| {
            ov.objects
                .iter()
                .map(|item| item.object.object_id.clone())
                .collect()
        })
        .unwrap_or_default();
    let ctx = LearningViewContext {
        artifact_aliases,
        study_marks,
        staged_ids,
        includes_staged: include_staged && overlay.is_some(),
    };
    Ok(learning_view_with_context(&snapshot, &ctx))
}

#[allow(dead_code)]
pub fn learning(service: &DesktopService) -> Result<LearningView, String> {
    learning_with_scope(service, false)
}

pub fn update_study_marks(
    service: &DesktopService,
    updates: &[StudyMarkUpdate],
) -> Result<Vec<StudyMark>, String> {
    let mut store = open_store(service)?;
    store
        .set_study_marks_batch(updates, &now_unix())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_semantic_object(
    object_id: String,
    include_staged: Option<bool>,
    state: State<'_, DesktopState>,
) -> Result<SemanticObjectDetail, String> {
    let service = state.service();
    let include_staged = include_staged.unwrap_or(true);
    tauri::async_runtime::spawn_blocking(move || {
        object_detail_with_scope(&service, &object_id, include_staged)
    })
    .await
    .map_err(|error| format!("semantic object worker failed: {error}"))?
}

#[tauri::command]
pub async fn semantic_learning_view(
    include_staged: Option<bool>,
    state: State<'_, DesktopState>,
) -> Result<LearningView, String> {
    let service = state.service();
    let include_staged = include_staged.unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || learning_with_scope(&service, include_staged))
        .await
        .map_err(|error| format!("learning view worker failed: {error}"))?
}

#[tauri::command]
pub async fn set_study_marks(
    updates: Vec<StudyMarkUpdate>,
    state: State<'_, DesktopState>,
) -> Result<Vec<StudyMark>, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || update_study_marks(&service, &updates))
        .await
        .map_err(|error| format!("study marks worker failed: {error}"))?
}
