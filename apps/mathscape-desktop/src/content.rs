use mathscape_artifact::ArtifactPool;
use mathscape_domain::{SourcePayload, StableId};
use mathscape_orchestrator::{PRODUCT_PATHS_KEY, ProductPath};
use mathscape_store_sqlite::SqliteArtifactStore;
use serde::Serialize;
use tauri::State;

use crate::{DesktopService, DesktopState};

#[derive(Debug, Serialize)]
pub struct ContentContext {
    conversion_id: String,
    paths: Vec<ProductPath>,
}

#[derive(Debug, Serialize)]
pub struct ArtifactContentInfo {
    artifact_id: String,
    media_type: String,
    byte_size: u64,
    contexts: Vec<ContentContext>,
}

impl DesktopService {
    pub fn content_info(&self, artifact_id: &str) -> Result<ArtifactContentInfo, String> {
        let id = StableId::parse(artifact_id).map_err(|e| e.to_string())?;
        let store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))
            .map_err(|e| e.to_string())?;
        let catalog = store.read_catalog_snapshot().map_err(|e| e.to_string())?;
        let artifact = catalog
            .artifacts()
            .iter()
            .find(|a| a.entity().header().stable_id() == &id)
            .ok_or("Artifact 不存在")?
            .entity()
            .payload();
        let mut contexts = Vec::new();
        for relation in catalog.conversions() {
            if !relation
                .endpoints()
                .iter()
                .any(|e| e.role().as_str() == "output" && e.entity() == &id)
            {
                continue;
            }
            let SourcePayload::ConversionRun(run) = relation.payload() else {
                continue;
            };
            let Some(json) = run.parameters().additional_options().get(PRODUCT_PATHS_KEY) else {
                continue;
            };
            let paths: Vec<ProductPath> =
                serde_json::from_str(json).map_err(|e| format!("产物路径映射损坏：{e}"))?;
            // Do not expose unrelated artifacts through a malformed manifest.
            if paths.iter().any(|p| {
                !relation
                    .endpoints()
                    .iter()
                    .any(|e| e.role().as_str() == "output" && e.entity() == &p.artifact_id)
            }) {
                return Err("产物路径映射引用了转换以外的文件".into());
            }
            contexts.push(ContentContext {
                conversion_id: relation.header().stable_id().to_string(),
                paths,
            });
        }
        Ok(ArtifactContentInfo {
            artifact_id: id.to_string(),
            media_type: artifact.media_type().to_string(),
            byte_size: artifact.byte_size(),
            contexts,
        })
    }

    pub fn read_artifact_bytes(
        &self,
        artifact_id: &str,
        offset: u64,
        length: u32,
    ) -> Result<Vec<u8>, String> {
        let id = StableId::parse(artifact_id).map_err(|e| e.to_string())?;
        ArtifactPool::open(&self.workspace_root)
            .map_err(|e| e.to_string())?
            .read_bytes(&id, offset, length)
            .map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub async fn get_artifact_content_info(
    state: State<'_, DesktopState>,
    artifact_id: String,
) -> Result<ArtifactContentInfo, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.content_info(&artifact_id))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn read_artifact_bytes(
    state: State<'_, DesktopState>,
    artifact_id: String,
    offset: u64,
    length: u32,
) -> Result<tauri::ipc::Response, String> {
    let service = state.service.clone();
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        service.read_artifact_bytes(&artifact_id, offset, length)
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(tauri::ipc::Response::new(bytes))
}
