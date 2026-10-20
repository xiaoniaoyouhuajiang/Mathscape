//! `search_workspace` (N3d): forwards to the store corpus read and the
//! `mathscape-query` search module. No scoring or filtering happens here.

use mathscape_query::search::{SearchQuery, SearchRegistry, SearchResponse, search};
use mathscape_store_sqlite::SqliteArtifactStore;
use tauri::State;

use crate::{DesktopService, DesktopState};

impl DesktopService {
    pub fn search_workspace(&self, query: &SearchQuery) -> Result<SearchResponse, String> {
        let store = SqliteArtifactStore::open(&self.workspace_root().join("mathscape.sqlite3"))
            .map_err(|error| format!("读取工作区失败：{error}"))?;
        let corpus = store
            .read_search_corpus()
            .map_err(|error| format!("读取搜索数据失败：{error}"))?;
        Ok(search(&SearchRegistry::with_defaults(), &corpus, query))
    }
}

#[tauri::command]
pub async fn search_workspace(
    state: State<'_, DesktopState>,
    query: SearchQuery,
) -> Result<SearchResponse, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.search_workspace(&query))
        .await
        .map_err(|error| format!("搜索任务失败：{error}"))?
}
