use mathscape_artifact::ArtifactPool;
use mathscape_domain::{
    AnchoredMarkdown, ContentHash, DiscoveryInput, DiscoverySelectionRequest,
    ResolvedDiscoverySelection, SourcePayload, StableId,
};
use mathscape_orchestrator::{
    DiscoveryInputLimits, MAX_MARKDOWN_BYTES, PRODUCT_PATHS_KEY, PreparedUnitDiscoveryRequest,
    ProductPath, anchor_markdown, build_discovery_input, build_unit_discovery_request,
    resolve_selection,
};
use mathscape_store_sqlite::SqliteArtifactStore;
use serde::Serialize;
use serde_json::Value;
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
    content_hash: ContentHash,
    contexts: Vec<ContentContext>,
}

#[derive(Debug, Serialize)]
pub struct SourceNodeDetail {
    pub node_id: String,
    pub kind: String,
    pub display_title: Option<String>,
    pub unit_kind: Option<String>,
    pub parent: Option<String>,
    pub artifact_id: Option<String>,
    pub byte_range: Option<ByteRangeView>,
    pub excerpt: Option<String>,
    pub markdown_block_ids: Vec<String>,
    pub payload: Value,
}

#[derive(Debug, Serialize)]
pub struct ByteRangeView {
    pub start: u64,
    pub end: u64,
}

impl DesktopService {
    pub fn source_node_detail(&self, node_id: &str) -> Result<SourceNodeDetail, String> {
        let id = StableId::parse(node_id).map_err(|e| e.to_string())?;
        let store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))
            .map_err(|e| e.to_string())?;
        let snapshot = store.source_map_snapshot().map_err(|e| e.to_string())?;
        let object = snapshot
            .objects
            .iter()
            .find(|object| object.stable_id == id)
            .ok_or("来源节点不存在")?;
        if !matches!(
            object.kind,
            mathscape_domain::EntityKind::SourceSpan
                | mathscape_domain::EntityKind::SourceSemanticUnit
        ) {
            return Err("该节点不是来源节点".into());
        }
        let payload = object.payload.clone();
        let body = payload.get("payload").unwrap_or(&payload);
        // Semantic units point to spans; use the first span as the readable
        // preview and keep the complete span list in the payload for detail UI.
        let span_body = if object.kind == mathscape_domain::EntityKind::SourceSemanticUnit {
            body.get("source_spans")
                .and_then(Value::as_array)
                .and_then(|spans| spans.first())
                .and_then(Value::as_str)
                .and_then(|span_id| StableId::parse(span_id).ok())
                .and_then(|span_id| {
                    snapshot
                        .objects
                        .iter()
                        .find(|candidate| candidate.stable_id == span_id)
                })
                .map(|span| span.payload.get("payload").unwrap_or(&span.payload))
        } else {
            None
        };
        let locator = span_body.unwrap_or(body);
        let artifact_id = locator
            .pointer("/markdown/artifact")
            .and_then(Value::as_str)
            .or_else(|| body.get("surface_content_artifact").and_then(Value::as_str));
        let range = locator.pointer("/markdown/byte_range").and_then(|value| {
            Some(ByteRangeView {
                start: value.get("start")?.as_u64()?,
                end: value.get("end")?.as_u64()?,
            })
        });
        let excerpt = match (artifact_id, &range) {
            (Some(artifact), Some(range)) if range.end >= range.start => {
                let artifact_id = StableId::parse(artifact).map_err(|e| e.to_string())?;
                let bytes = ArtifactPool::open(&self.workspace_root)
                    .map_err(|e| e.to_string())?
                    .read_bytes(
                        &artifact_id,
                        range.start,
                        (range.end - range.start).min(64 * 1024) as u32,
                    )
                    .map_err(|e| e.to_string())?;
                Some(String::from_utf8(bytes).map_err(|_| "来源文本不是有效 UTF-8")?)
            }
            _ => None,
        };
        let block_ids = locator
            .get("marker_block_ids")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        Ok(SourceNodeDetail {
            node_id: id.to_string(),
            kind: format!("{:?}", object.kind),
            display_title: body
                .get("display_title")
                .and_then(Value::as_str)
                .map(str::to_owned),
            unit_kind: body.get("kind").and_then(Value::as_str).map(str::to_owned),
            parent: body
                .get("parent_unit")
                .and_then(Value::as_str)
                .map(str::to_owned),
            artifact_id: artifact_id.map(str::to_owned),
            byte_range: range,
            excerpt,
            markdown_block_ids: block_ids,
            payload,
        })
    }
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
            let paths: Vec<ProductPath> =
                match run.parameters().additional_options().get(PRODUCT_PATHS_KEY) {
                    Some(json) => {
                        serde_json::from_str(json).map_err(|e| format!("产物路径映射损坏：{e}"))?
                    }
                    None => vec![],
                };
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
            content_hash: artifact.content_hash().clone(),
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

    pub fn markdown_document(
        &self,
        artifact_id: &str,
        conversion_id: Option<&str>,
    ) -> Result<AnchoredMarkdown, String> {
        let info = self.content_info(artifact_id)?;
        if info.media_type != "text/markdown" {
            return Err("块级选区只支持 Markdown 文件".into());
        }
        if info.byte_size > MAX_MARKDOWN_BYTES as u64 {
            return Err("Markdown 超过 16 MiB，当前块级选区暂不支持".into());
        }
        match conversion_id {
            Some(id) if !info.contexts.iter().any(|c| c.conversion_id == id) => {
                return Err("转换上下文不属于此 Markdown".into());
            }
            None if !info.contexts.is_empty() => return Err("请选择 Markdown 的转换来源".into()),
            _ => {}
        }
        let id = StableId::parse(artifact_id).map_err(|e| e.to_string())?;
        let conversion = conversion_id
            .map(StableId::parse)
            .transpose()
            .map_err(|e| e.to_string())?;
        let pool = ArtifactPool::open(&self.workspace_root).map_err(|e| e.to_string())?;
        let mut bytes = Vec::with_capacity(info.byte_size as usize);
        while (bytes.len() as u64) < info.byte_size {
            let length = (info.byte_size - bytes.len() as u64).min(4 * 1024 * 1024) as u32;
            let chunk = pool
                .read_bytes(&id, bytes.len() as u64, length)
                .map_err(|e| e.to_string())?;
            if chunk.len() != length as usize {
                return Err("文件长度已改变，请检查 Artifact 完整性".into());
            }
            bytes.extend(chunk);
        }
        // Detect growth as well as short reads, then verify catalog content identity.
        if !pool
            .read_bytes(&id, info.byte_size, 1)
            .map_err(|e| e.to_string())?
            .is_empty()
        {
            return Err("文件长度已改变，请检查 Artifact 完整性".into());
        }
        let source = String::from_utf8(bytes).map_err(|_| "Markdown 不是有效 UTF-8")?;
        let document = anchor_markdown(id, conversion, source).map_err(|e| e.to_string())?;
        if document.snapshot.content_hash != info.content_hash {
            return Err("Markdown 内容哈希与登记信息不符".into());
        }
        Ok(document)
    }

    pub fn resolve_markdown_selection(
        &self,
        request: DiscoverySelectionRequest,
    ) -> Result<ResolvedDiscoverySelection, String> {
        let document = self.markdown_document(
            request.snapshot.artifact_id.as_str(),
            request
                .snapshot
                .conversion_id
                .as_ref()
                .map(StableId::as_str),
        )?;
        resolve_selection(&document, request).map_err(|e| e.to_string())
    }

    pub fn prepare_discovery_input(
        &self,
        request: DiscoverySelectionRequest,
    ) -> Result<DiscoveryInput, String> {
        let document = self.markdown_document(
            request.snapshot.artifact_id.as_str(),
            request
                .snapshot
                .conversion_id
                .as_ref()
                .map(StableId::as_str),
        )?;
        let selection = resolve_selection(&document, request).map_err(|e| e.to_string())?;
        build_discovery_input(&document, &selection, DiscoveryInputLimits::default())
            .map_err(|e| e.to_string())
    }

    pub(crate) fn prepare_unit_discovery_request(
        &self,
        request: DiscoverySelectionRequest,
    ) -> Result<PreparedUnitDiscoveryRequest, String> {
        let document = self.markdown_document(
            request.snapshot.artifact_id.as_str(),
            request
                .snapshot
                .conversion_id
                .as_ref()
                .map(StableId::as_str),
        )?;
        let selection = resolve_selection(&document, request).map_err(|e| e.to_string())?;
        build_unit_discovery_request(&document, &selection, DiscoveryInputLimits::default())
            .map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub async fn get_markdown_document(
    state: State<'_, DesktopState>,
    artifact_id: String,
    conversion_id: Option<String>,
) -> Result<AnchoredMarkdown, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        service.markdown_document(&artifact_id, conversion_id.as_deref())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn resolve_markdown_selection(
    state: State<'_, DesktopState>,
    request: DiscoverySelectionRequest,
) -> Result<ResolvedDiscoverySelection, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.resolve_markdown_selection(request))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn prepare_discovery_input(
    state: State<'_, DesktopState>,
    request: DiscoverySelectionRequest,
) -> Result<DiscoveryInput, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.prepare_discovery_input(request))
        .await
        .map_err(|e| e.to_string())?
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
pub async fn get_source_node_detail(
    state: State<'_, DesktopState>,
    node_id: String,
) -> Result<SourceNodeDetail, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.source_node_detail(&node_id))
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
