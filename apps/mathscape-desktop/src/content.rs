use std::collections::{HashMap, HashSet};

use mathscape_artifact::ArtifactPool;
use mathscape_domain::{
    AnchoredMarkdown, ContentHash, DiscoveryInput, DiscoverySelectionRequest,
    ResolvedDiscoverySelection, SourcePayload, StableId,
};
use mathscape_orchestrator::{
    DiscoveryInputLimits, MAX_MARKDOWN_BYTES, PRODUCT_PATHS_KEY, PreparedUnitDiscoveryRequest,
    ProductPath, anchor_markdown, build_discovery_input, build_unit_discovery_request,
    render_effective_slice, resolve_selection,
};
use mathscape_store_sqlite::{ErrataUnitReviewDecision, MarkdownBlockErrata, SqliteArtifactStore};
use serde::Serialize;
use serde_json::Value;
use tauri::State;

use crate::markdown_errata::to_overlay_entries;
use crate::{DesktopService, DesktopState};

#[derive(Debug, Serialize)]
pub struct ContentContext {
    pub(crate) conversion_id: String,
    paths: Vec<ProductPath>,
}

#[derive(Debug, Serialize)]
pub struct ArtifactContentInfo {
    artifact_id: String,
    pub(crate) media_type: String,
    pub(crate) byte_size: u64,
    content_hash: ContentHash,
    pub(crate) contexts: Vec<ContentContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ink_snapshot_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct InkSourceLocation {
    pub snapshot_id: String,
    pub document_id: String,
    pub page_id: String,
    pub stroke_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceErrataReviewItem {
    pub errata_id: String,
    pub artifact_id: String,
    pub first_block_id: String,
    pub last_block_id: String,
    pub original_text: String,
    pub corrected_text: String,
    pub source_kind: String,
    pub updated_at: String,
    pub sent_to_sandbox: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceUnitErrataReview {
    pub unit_id: String,
    pub items: Vec<SourceErrataReviewItem>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ink: Option<InkSourceLocation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pdf_backlink: Option<crate::SourceBacklink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errata_review: Option<SourceUnitErrataReview>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ByteRangeView {
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarkdownUnitMarker {
    pub unit_id: String,
    pub title: String,
    pub unit_kind: String,
    pub artifact_id: String,
    pub byte_range: ByteRangeView,
    pub block_ids: Vec<String>,
}

impl DesktopService {
    pub fn registration_object_detail(
        &self,
        decision: &str,
        node_id: &str,
    ) -> Result<SourceNodeDetail, String> {
        let store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))
            .map_err(|e| e.to_string())?;
        let history = store
            .registration_history(decision)
            .map_err(|e| e.to_string())?;
        let mut endpoints = std::collections::BTreeMap::new();
        let mut objects = vec![];
        for o in history.objects {
            let id = StableId::parse(&o.object_id).map_err(|e| e.to_string())?;
            endpoints.insert(
                id.clone(),
                o.endpoints
                    .iter()
                    .map(|e| mathscape_store_sqlite::StoredMapEndpoint {
                        role: e.role().as_str().into(),
                        ordinal: e.ordinal(),
                        entity_id: e.entity().clone(),
                    })
                    .collect(),
            );
            objects.push(mathscape_store_sqlite::StoredMapObject {
                stable_id: id,
                revision_id: mathscape_domain::RevisionId::parse(o.revision)
                    .map_err(|e| e.to_string())?,
                kind: serde_json::from_value(serde_json::json!(o.kind))
                    .map_err(|e| e.to_string())?,
                relation: o.kind == "SourceUnitRelation",
                payload: o.payload,
                lifecycle: serde_json::from_value(serde_json::json!(o.lifecycle))
                    .map_err(|e| e.to_string())?,
            });
        }
        self.read_source_detail(
            node_id,
            &mathscape_store_sqlite::SourceMapSnapshot {
                objects,
                relation_endpoints: endpoints,
            },
        )
    }
    pub fn source_node_detail(&self, node_id: &str) -> Result<SourceNodeDetail, String> {
        let store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))
            .map_err(|e| e.to_string())?;
        let snapshot = store.source_map_snapshot().map_err(|e| e.to_string())?;
        self.read_source_detail(node_id, &snapshot)
    }
    fn read_source_detail(
        &self,
        node_id: &str,
        snapshot: &mathscape_store_sqlite::SourceMapSnapshot,
    ) -> Result<SourceNodeDetail, String> {
        let id = StableId::parse(node_id).map_err(|e| e.to_string())?;
        let object = snapshot
            .objects
            .iter()
            .find(|object| object.stable_id == id)
            .ok_or("来源节点不存在")?;
        if !matches!(
            object.kind,
            mathscape_domain::EntityKind::SourceSpan
                | mathscape_domain::EntityKind::SourceSemanticUnit
                | mathscape_domain::EntityKind::SourceUnitRelation
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
        } else if object.kind == mathscape_domain::EntityKind::SourceUnitRelation {
            snapshot
                .relation_endpoints
                .get(&id)
                .and_then(|es| es.iter().find(|e| e.role == "evidence"))
                .and_then(|e| snapshot.objects.iter().find(|o| o.stable_id == e.entity_id))
                .map(|s| s.payload.get("payload").unwrap_or(&s.payload))
        } else {
            None
        };
        let locator = span_body.unwrap_or(body);
        let artifact_id = locator
            .pointer("/markdown/artifact")
            .and_then(Value::as_str)
            .or_else(|| {
                locator
                    .pointer("/ink/snapshot_artifact")
                    .and_then(Value::as_str)
            })
            .or_else(|| body.get("surface_content_artifact").and_then(Value::as_str));
        let ink = if let Some(value) = locator.get("ink").filter(|value| !value.is_null()) {
            let artifact = value
                .get("snapshot_artifact")
                .and_then(Value::as_str)
                .ok_or("Ink 来源缺少快照 Artifact")?;
            let page_id = value
                .get("page_id")
                .and_then(Value::as_str)
                .ok_or("Ink 来源缺少 page_id")?
                .to_owned();
            let stroke_ids: Vec<String> = serde_json::from_value(
                value
                    .get("stroke_ids")
                    .cloned()
                    .ok_or("Ink 来源缺少笔迹集合")?,
            )
            .map_err(|error| error.to_string())?;
            let store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))
                .map_err(|error| error.to_string())?;
            let snapshot = store
                .ink_snapshot_for_artifact(artifact)
                .map_err(|error| error.to_string())?
                .ok_or("Ink 来源快照不存在")?;
            let manifest: mathscape_domain::InkSnapshotManifest =
                serde_json::from_str(&snapshot.manifest_json).map_err(|error| error.to_string())?;
            manifest
                .validate_selection(&page_id, &stroke_ids)
                .map_err(|error| error.to_string())?;
            Some(InkSourceLocation {
                snapshot_id: snapshot.snapshot_id,
                document_id: snapshot.document_id,
                page_id,
                stroke_ids,
            })
        } else {
            None
        };
        let range = locator.pointer("/markdown/byte_range").and_then(|value| {
            Some(ByteRangeView {
                start: value.get("start")?.as_u64()?,
                end: value.get("end")?.as_u64()?,
            })
        });
        let errata_store =
            SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3")).ok();
        let mut active_errata_cache: HashMap<String, Vec<MarkdownBlockErrata>> = HashMap::new();
        let mut load_active_errata = |art: &str| -> Vec<MarkdownBlockErrata> {
            if let Some(existing) = active_errata_cache.get(art) {
                return existing.clone();
            }
            let loaded = errata_store
                .as_ref()
                .and_then(|store| store.list_active_markdown_errata(art).ok())
                .unwrap_or_default();
            active_errata_cache.insert(art.to_owned(), loaded.clone());
            loaded
        };
        let excerpt = match (artifact_id, &range) {
            (Some(artifact), Some(range)) if range.end >= range.start => {
                let parsed_id = StableId::parse(artifact).map_err(|e| e.to_string())?;
                let active_errata = load_active_errata(artifact);
                let has_overlap = active_errata
                    .iter()
                    .any(|e| e.base_byte_start < range.end && e.base_byte_end > range.start);
                if has_overlap {
                    let info = self.content_info(artifact)?;
                    let full_len = info.byte_size.min(MAX_MARKDOWN_BYTES as u64) as u32;
                    let full_bytes = ArtifactPool::open(&self.workspace_root)
                        .map_err(|e| e.to_string())?
                        .read_bytes(&parsed_id, 0, full_len)
                        .map_err(|e| e.to_string())?;
                    let full_source =
                        String::from_utf8(full_bytes).map_err(|_| "来源文本不是有效 UTF-8")?;
                    let overlay = to_overlay_entries(&active_errata);
                    let mut effective = render_effective_slice(
                        &full_source,
                        range.start as usize..range.end as usize,
                        &overlay,
                    )
                    .effective_source;
                    if effective.len() > 64 * 1024 {
                        let mut cut = 64 * 1024;
                        while cut > 0 && !effective.is_char_boundary(cut) {
                            cut -= 1;
                        }
                        effective.truncate(cut);
                    }
                    Some(effective)
                } else {
                    let bytes = ArtifactPool::open(&self.workspace_root)
                        .map_err(|e| e.to_string())?
                        .read_bytes(
                            &parsed_id,
                            range.start,
                            (range.end - range.start).min(64 * 1024) as u32,
                        )
                        .map_err(|e| e.to_string())?;
                    Some(String::from_utf8(bytes).map_err(|_| "来源文本不是有效 UTF-8")?)
                }
            }
            _ => None,
        };
        let block_ids: Vec<String> = locator
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
        let errata_review = if ink.is_none() {
            let mut span_locators: Vec<&Value> = Vec::new();
            if object.kind == mathscape_domain::EntityKind::SourceSemanticUnit {
                if let Some(spans) = body.get("source_spans").and_then(Value::as_array) {
                    for span_id_str in spans.iter().filter_map(Value::as_str) {
                        if let Ok(span_id) = StableId::parse(span_id_str)
                            && let Some(span_obj) =
                                snapshot.objects.iter().find(|o| o.stable_id == span_id)
                        {
                            span_locators
                                .push(span_obj.payload.get("payload").unwrap_or(&span_obj.payload));
                        }
                    }
                }
            } else {
                span_locators.push(locator);
            }
            let review_map: HashMap<String, ErrataUnitReviewDecision> = errata_store
                .as_ref()
                .and_then(|store| store.list_errata_unit_reviews(Some(id.as_str())).ok())
                .unwrap_or_default()
                .into_iter()
                .map(|r| (r.errata_id, r.decision))
                .collect();
            let mut seen_errata = HashSet::new();
            let mut review_items = Vec::new();
            for loc in span_locators {
                let Some(art) = loc.pointer("/markdown/artifact").and_then(Value::as_str) else {
                    continue;
                };
                let loc_range = loc
                    .pointer("/markdown/byte_range")
                    .and_then(|v| Some((v.get("start")?.as_u64()?, v.get("end")?.as_u64()?)));
                let loc_blocks: HashSet<&str> = loc
                    .get("marker_block_ids")
                    .and_then(Value::as_array)
                    .map(|arr| arr.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                for errata in load_active_errata(art) {
                    let block_overlap = !loc_blocks.is_empty()
                        && errata
                            .covered_block_ids
                            .iter()
                            .any(|b| loc_blocks.contains(b.as_str()));
                    let byte_overlap = loc_range.is_some_and(|(s, e)| {
                        errata.base_byte_start < e && errata.base_byte_end > s
                    });
                    if !(block_overlap || byte_overlap) {
                        continue;
                    }
                    if !seen_errata.insert(errata.errata_id.clone()) {
                        continue;
                    }
                    let decision = review_map.get(&errata.errata_id).copied();
                    if decision == Some(ErrataUnitReviewDecision::Acknowledged) {
                        continue;
                    }
                    review_items.push(SourceErrataReviewItem {
                        errata_id: errata.errata_id,
                        artifact_id: errata.artifact_id,
                        first_block_id: errata.first_block_id,
                        last_block_id: errata.last_block_id,
                        original_text: errata.original_text,
                        corrected_text: errata.corrected_text,
                        source_kind: errata.source_kind.as_str().to_owned(),
                        updated_at: errata.updated_at,
                        sent_to_sandbox: decision == Some(ErrataUnitReviewDecision::SentToSandbox),
                    });
                }
            }
            if review_items.is_empty() {
                None
            } else {
                Some(SourceUnitErrataReview {
                    unit_id: id.to_string(),
                    items: review_items,
                })
            }
        } else {
            None
        };
        let pdf_backlink = if ink.is_some() {
            None
        } else if let (Some(pdf_art), Some(page_idx)) = (
            locator.pointer("/page/artifact").and_then(Value::as_str),
            locator
                .pointer("/page/pdf_page_index")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok()),
        ) {
            Some(crate::SourceBacklink::ExactPdfPages {
                pdf_artifact_id: pdf_art.to_owned(),
                pdf_page_indices: vec![page_idx],
                marker_page_ids: vec![page_idx],
            })
        } else if let (Some(artifact), Some(range)) = (artifact_id, &range) {
            if range.end > range.start {
                self.get_source_backlink(crate::SourceBacklinkRequest {
                    artifact_id: artifact.to_owned(),
                    conversion_id: None,
                    start: range.start,
                    end: range.end,
                    block_ids: block_ids.clone(),
                })
                .ok()
            } else {
                None
            }
        } else {
            None
        };
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
            ink,
            pdf_backlink,
            errata_review,
            payload,
        })
    }
    pub fn content_info(&self, artifact_id: &str) -> Result<ArtifactContentInfo, String> {
        content_info_at(&self.workspace_root, artifact_id)
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
        markdown_document_at(&self.workspace_root, artifact_id, conversion_id)
    }

    /// Markdown for NoteLink resolution (§14.3): the conversion origin is not
    /// part of the target, so any valid context of the artifact is used.
    pub(crate) fn markdown_for_link(&self, artifact_id: &str) -> Result<AnchoredMarkdown, String> {
        let info = self.content_info(artifact_id)?;
        let conversion = info
            .contexts
            .first()
            .map(|context| context.conversion_id.clone());
        self.markdown_document(artifact_id, conversion.as_deref())
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
    let service = state.service();
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
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || service.resolve_markdown_selection(request))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn prepare_discovery_input(
    state: State<'_, DesktopState>,
    request: DiscoverySelectionRequest,
) -> Result<DiscoveryInput, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || service.prepare_discovery_input(request))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn get_artifact_content_info(
    state: State<'_, DesktopState>,
    artifact_id: String,
) -> Result<ArtifactContentInfo, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || service.content_info(&artifact_id))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn get_source_node_detail(
    state: State<'_, DesktopState>,
    node_id: String,
) -> Result<SourceNodeDetail, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || service.source_node_detail(&node_id))
        .await
        .map_err(|e| e.to_string())?
}

impl DesktopService {
    /// Lists active registered `SourceSemanticUnit`s anchored inside any of `artifact_ids`.
    pub fn markdown_unit_markers(
        &self,
        artifact_ids: &[String],
    ) -> Result<Vec<MarkdownUnitMarker>, String> {
        if artifact_ids.is_empty() {
            return Ok(Vec::new());
        }
        let wanted: std::collections::HashSet<&str> =
            artifact_ids.iter().map(String::as_str).collect();
        let store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))
            .map_err(|e| e.to_string())?;
        let snapshot = store.source_map_snapshot().map_err(|e| e.to_string())?;
        let spans_by_id: std::collections::HashMap<&StableId, &Value> = snapshot
            .objects
            .iter()
            .filter(|obj| {
                obj.kind == mathscape_domain::EntityKind::SourceSpan
                    && !matches!(
                        obj.lifecycle,
                        mathscape_domain::LifecycleStatus::Superseded
                            | mathscape_domain::LifecycleStatus::Rejected
                            | mathscape_domain::LifecycleStatus::Deprecated
                    )
            })
            .map(|obj| {
                (
                    &obj.stable_id,
                    obj.payload.get("payload").unwrap_or(&obj.payload),
                )
            })
            .collect();

        let mut out = Vec::new();
        for obj in &snapshot.objects {
            if obj.kind != mathscape_domain::EntityKind::SourceSemanticUnit {
                continue;
            }
            if matches!(
                obj.lifecycle,
                mathscape_domain::LifecycleStatus::Superseded
                    | mathscape_domain::LifecycleStatus::Rejected
                    | mathscape_domain::LifecycleStatus::Deprecated
            ) {
                continue;
            }
            let body = obj.payload.get("payload").unwrap_or(&obj.payload);
            let Some(span_ids) = body.get("source_spans").and_then(Value::as_array) else {
                continue;
            };
            let title = body
                .get("display_title")
                .or_else(|| body.get("title"))
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| obj.stable_id.as_str())
                .to_owned();
            let unit_kind = body
                .get("kind")
                .or_else(|| body.get("unit_kind"))
                .and_then(Value::as_str)
                .unwrap_or("statement")
                .to_owned();

            for raw_span_id in span_ids.iter().filter_map(Value::as_str) {
                let Ok(parsed_span) = StableId::parse(raw_span_id) else {
                    continue;
                };
                let Some(span_body) = spans_by_id.get(&parsed_span) else {
                    continue;
                };
                let Some(art) = span_body
                    .pointer("/markdown/artifact")
                    .and_then(Value::as_str)
                else {
                    continue;
                };
                if !wanted.contains(art) {
                    continue;
                }
                let Some(range_val) = span_body.pointer("/markdown/byte_range") else {
                    continue;
                };
                let (Some(start), Some(end)) = (
                    range_val.get("start").and_then(Value::as_u64),
                    range_val.get("end").and_then(Value::as_u64),
                ) else {
                    continue;
                };
                let block_ids: Vec<String> = span_body
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
                out.push(MarkdownUnitMarker {
                    unit_id: obj.stable_id.to_string(),
                    title,
                    unit_kind,
                    artifact_id: art.to_owned(),
                    byte_range: ByteRangeView { start, end },
                    block_ids,
                });
                break;
            }
        }
        out.sort_by(|a, b| {
            a.artifact_id
                .cmp(&b.artifact_id)
                .then(a.byte_range.start.cmp(&b.byte_range.start))
                .then(a.byte_range.end.cmp(&b.byte_range.end))
        });
        Ok(out)
    }
}

#[tauri::command]
pub async fn get_markdown_unit_markers(
    state: State<'_, DesktopState>,
    artifact_ids: Vec<String>,
) -> Result<Vec<MarkdownUnitMarker>, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || service.markdown_unit_markers(&artifact_ids))
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
    let service = state.service();
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        service.read_artifact_bytes(&artifact_id, offset, length)
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Content info of an artifact in an explicit workspace root (the bound
/// workspace, or the sandbox an investigation lives in).
pub(crate) fn content_info_at(
    workspace_root: &std::path::Path,
    artifact_id: &str,
) -> Result<ArtifactContentInfo, String> {
    let id = StableId::parse(artifact_id).map_err(|e| e.to_string())?;
    let store = SqliteArtifactStore::open(&workspace_root.join("mathscape.sqlite3"))
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
        ink_snapshot_id: store
            .ink_snapshot_for_artifact(id.as_str())
            .map_err(|error| error.to_string())?
            .map(|snapshot| snapshot.snapshot_id),
    })
}

/// The anchored Markdown of a snapshot in an explicit workspace root.
pub(crate) fn markdown_document_at(
    workspace_root: &std::path::Path,
    artifact_id: &str,
    conversion_id: Option<&str>,
) -> Result<AnchoredMarkdown, String> {
    let info = content_info_at(workspace_root, artifact_id)?;
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
    let pool = ArtifactPool::open(workspace_root).map_err(|e| e.to_string())?;
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
