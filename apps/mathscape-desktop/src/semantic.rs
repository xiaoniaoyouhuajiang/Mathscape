//! 语义层的只读 Desktop 命令：对象详情与学习视图。

use mathscape_orchestrator::semantic_view::{
    LearningView, SemanticObjectDetail, learning_view, semantic_object_detail,
};
use mathscape_store_sqlite::SqliteArtifactStore;
use tauri::State;

use crate::{DesktopService, DesktopState};

fn snapshot(service: &DesktopService) -> Result<mathscape_domain::MapEditSnapshot, String> {
    SqliteArtifactStore::open(&service.workspace_root().join("mathscape.sqlite3"))
        .and_then(|store| store.map_edit_snapshot())
        .map_err(|error| error.to_string())
}

pub fn object_detail(
    service: &DesktopService,
    object_id: &str,
) -> Result<SemanticObjectDetail, String> {
    semantic_object_detail(&snapshot(service)?, object_id)
        .ok_or_else(|| format!("not_found：{object_id} 不是已接受的语义对象"))
}

pub fn learning(service: &DesktopService) -> Result<LearningView, String> {
    Ok(learning_view(&snapshot(service)?))
}

#[tauri::command]
pub async fn get_semantic_object(
    object_id: String,
    state: State<'_, DesktopState>,
) -> Result<SemanticObjectDetail, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || object_detail(&service, &object_id))
        .await
        .map_err(|error| format!("semantic object worker failed: {error}"))?
}

#[tauri::command]
pub async fn semantic_learning_view(
    state: State<'_, DesktopState>,
) -> Result<LearningView, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || learning(&service))
        .await
        .map_err(|error| format!("learning view worker failed: {error}"))?
}
