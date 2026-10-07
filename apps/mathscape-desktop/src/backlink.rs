use std::collections::BTreeSet;

use mathscape_artifact::ArtifactPool;
use mathscape_domain::{
    MarkdownPdfBlockMap, MarkdownPdfMappingConfidence, SourcePayload, StableId, TextByteRange,
};
use mathscape_orchestrator::{
    MARKDOWN_PDF_BASE_CONVERSION_KEY, MARKDOWN_PDF_MAP_ARTIFACT_KEY,
    MARKDOWN_PDF_SOURCE_PDF_ARTIFACT_KEY,
};
use mathscape_store_sqlite::SqliteArtifactStore;
use serde::{Deserialize, Serialize};

use crate::DesktopService;

pub(crate) fn conversion_has_pdf_lineage(
    catalog: &mathscape_store_sqlite::ArtifactCatalogSnapshot,
    conversion_id: &StableId,
    expected_markdown_output: &StableId,
    pdf_artifact_id: &StableId,
    seen: &mut BTreeSet<StableId>,
) -> bool {
    if !seen.insert(conversion_id.clone()) {
        return false;
    }
    let Some(conversion) = catalog
        .conversions()
        .iter()
        .find(|conversion| conversion.header().stable_id() == conversion_id)
    else {
        return false;
    };
    if !conversion.endpoints().iter().any(|endpoint| {
        endpoint.role().as_str() == "output" && endpoint.entity() == expected_markdown_output
    }) {
        return false;
    }
    if conversion
        .endpoints()
        .iter()
        .any(|endpoint| endpoint.role().as_str() == "input" && endpoint.entity() == pdf_artifact_id)
    {
        return true;
    }
    let SourcePayload::ConversionRun(run) = conversion.payload() else {
        return false;
    };
    let Some(base_conversion_id) = run
        .parameters()
        .additional_options()
        .get(MARKDOWN_PDF_BASE_CONVERSION_KEY)
        .and_then(|value| StableId::parse(value).ok())
    else {
        return false;
    };
    let Some(source_pdf) = run
        .parameters()
        .additional_options()
        .get(MARKDOWN_PDF_SOURCE_PDF_ARTIFACT_KEY)
        .and_then(|value| StableId::parse(value).ok())
    else {
        return false;
    };
    if source_pdf != *pdf_artifact_id {
        return false;
    }
    let Some(base) = catalog
        .conversions()
        .iter()
        .find(|candidate| candidate.header().stable_id() == &base_conversion_id)
    else {
        return false;
    };
    let Some(base_markdown) = conversion.endpoints().iter().find_map(|endpoint| {
        (endpoint.role().as_str() == "input"
            && base.endpoints().iter().any(|base_endpoint| {
                base_endpoint.role().as_str() == "output"
                    && base_endpoint.entity() == endpoint.entity()
            }))
        .then(|| endpoint.entity().clone())
    }) else {
        return false;
    };
    conversion_has_pdf_lineage(
        catalog,
        &base_conversion_id,
        &base_markdown,
        pdf_artifact_id,
        seen,
    )
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceBacklinkRequest {
    pub artifact_id: String,
    pub conversion_id: Option<String>,
    pub start: u64,
    pub end: u64,
    #[serde(default)]
    pub block_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceBacklink {
    ExactPdfPages {
        pdf_artifact_id: String,
        pdf_page_indices: Vec<u32>,
        marker_page_ids: Vec<u32>,
    },
    CandidatePdfPages {
        pdf_artifact_id: String,
        pdf_page_indices: Vec<u32>,
        marker_page_ids: Vec<u32>,
        reason: String,
    },
    MarkdownOnly {
        reason: String,
    },
}

impl DesktopService {
    pub fn get_source_backlink(
        &self,
        request: SourceBacklinkRequest,
    ) -> Result<SourceBacklink, String> {
        if request.start >= request.end {
            return Ok(SourceBacklink::MarkdownOnly {
                reason: "锚点范围为空或无效".into(),
            });
        }
        let markdown_id = StableId::parse(&request.artifact_id).map_err(|e| e.to_string())?;
        let conversion_id = request
            .conversion_id
            .as_deref()
            .map(StableId::parse)
            .transpose()
            .map_err(|e| e.to_string())?;
        let store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))
            .map_err(|e| e.to_string())?;
        let catalog = store.read_catalog_snapshot().map_err(|e| e.to_string())?;
        let conversion = catalog
            .conversions()
            .iter()
            .filter(|conversion| {
                conversion_id
                    .as_ref()
                    .is_none_or(|id| conversion.header().stable_id() == id)
            })
            .find(|conversion| {
                conversion.endpoints().iter().any(|endpoint| {
                    endpoint.role().as_str() == "output" && endpoint.entity() == &markdown_id
                })
            });
        let Some(conversion) = conversion else {
            return Ok(SourceBacklink::MarkdownOnly {
                reason: "没有找到属于当前 Markdown 的转换上下文".into(),
            });
        };
        let Some(run) = (match conversion.payload() {
            SourcePayload::ConversionRun(run) => Some(run),
            _ => None,
        }) else {
            return Ok(SourceBacklink::MarkdownOnly {
                reason: "转换关系负载不是 ConversionRun".into(),
            });
        };
        let Some(map_id) = run
            .parameters()
            .additional_options()
            .get(MARKDOWN_PDF_MAP_ARTIFACT_KEY)
            .and_then(|value| StableId::parse(value).ok())
        else {
            return Ok(SourceBacklink::MarkdownOnly {
                reason: "该转换没有生成 Markdown-PDF 页映射（旧转换或未启用分页输出）".into(),
            });
        };
        let map_record = catalog
            .artifacts()
            .iter()
            .find(|record| record.entity().header().stable_id() == &map_id)
            .ok_or_else(|| "页映射 Artifact 不存在".to_owned())?;
        let byte_size = map_record.entity().payload().byte_size();
        let pool = ArtifactPool::open(&self.workspace_root).map_err(|e| e.to_string())?;
        let mut bytes = Vec::with_capacity(usize::try_from(byte_size).map_err(|_| "页映射过大")?);
        let mut offset = 0_u64;
        while offset < byte_size {
            let length = (byte_size - offset).min(4 * 1024 * 1024) as u32;
            let chunk = pool
                .read_bytes(&map_id, offset, length)
                .map_err(|e| e.to_string())?;
            if chunk.len() != length as usize {
                return Err("页映射 Artifact 长度已改变".into());
            }
            bytes.extend(chunk);
            offset += u64::from(length);
        }
        let map: MarkdownPdfBlockMap =
            serde_json::from_slice(&bytes).map_err(|e| format!("页映射损坏：{e}"))?;
        if map.conversion_id != *conversion.header().stable_id()
            || map.markdown_artifact_id != markdown_id
        {
            return Ok(SourceBacklink::MarkdownOnly {
                reason: "页映射与当前转换身份不一致".into(),
            });
        }
        if !conversion_has_pdf_lineage(
            &catalog,
            conversion.header().stable_id(),
            &markdown_id,
            &map.pdf_artifact_id,
            &mut BTreeSet::new(),
        ) {
            return Ok(SourceBacklink::MarkdownOnly {
                reason: "页映射中的 PDF 不属于当前转换或其已验证的基础转换".into(),
            });
        }
        if !catalog
            .artifacts()
            .iter()
            .any(|record| record.entity().header().stable_id() == &map.pdf_artifact_id)
        {
            return Ok(SourceBacklink::MarkdownOnly {
                reason: "页映射指向的 PDF Artifact 不存在".into(),
            });
        }
        let document = self.markdown_document(
            &markdown_id.to_string(),
            Some(conversion.header().stable_id().as_str()),
        )?;
        if document.snapshot.content_hash != map.markdown_content_hash {
            return Ok(SourceBacklink::MarkdownOnly {
                reason: "当前 Markdown 内容已变化，页映射失效".into(),
            });
        }
        let range = TextByteRange {
            start: request.start,
            end: request.end,
        };
        let requested_blocks = request.block_ids.iter().collect::<BTreeSet<_>>();
        let entries = map
            .entries
            .iter()
            .filter(|entry| {
                let by_id =
                    !requested_blocks.is_empty() && requested_blocks.contains(&entry.block_id);
                let by_range = entry.markdown_range.end > range.start
                    && entry.markdown_range.start < range.end;
                by_id || by_range
            })
            .collect::<Vec<_>>();
        let pages = entries
            .iter()
            .flat_map(|entry| entry.pdf_page_indices.iter().copied())
            .collect::<BTreeSet<_>>();
        if pages.is_empty() {
            return Ok(SourceBacklink::MarkdownOnly {
                reason: "锚点没有对应的 PDF 页面".into(),
            });
        }
        let pages = pages.into_iter().collect::<Vec<_>>();
        let exact = entries
            .iter()
            .all(|entry| entry.confidence == MarkdownPdfMappingConfidence::Exact);
        let marker_pages = pages.clone();
        let pdf_artifact_id = map.pdf_artifact_id.to_string();
        if exact {
            Ok(SourceBacklink::ExactPdfPages {
                pdf_artifact_id,
                pdf_page_indices: pages,
                marker_page_ids: marker_pages,
            })
        } else {
            Ok(SourceBacklink::CandidatePdfPages {
                pdf_artifact_id,
                pdf_page_indices: pages,
                marker_page_ids: marker_pages,
                reason: "该 Markdown 锚点跨越多个页段或页边界不唯一".into(),
            })
        }
    }
}

#[tauri::command]
pub(crate) async fn get_source_backlink(
    state: tauri::State<'_, crate::DesktopState>,
    request: SourceBacklinkRequest,
) -> Result<SourceBacklink, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.get_source_backlink(request))
        .await
        .map_err(|error| format!("读取 PDF 回链线程失败：{error}"))?
        .map_err(|error| error.to_string())
}
