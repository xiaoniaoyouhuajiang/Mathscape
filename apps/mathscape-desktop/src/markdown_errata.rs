//! Desktop commands and service helpers for the block-level Markdown errata overlay.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use mathscape_domain::StableId;
use mathscape_orchestrator::{
    ErrataOverlayEntry, normalize_errata_corrected_text, resolve_errata_target_range,
    validate_errata_corrected_text,
};
use mathscape_store_sqlite::{
    ErrataUnitReviewDecision, MarkdownBlockErrata, MarkdownErrataSourceKind, SqliteArtifactStore,
    UpsertMarkdownErrata,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::State;

use crate::{DesktopService, DesktopState};

const DESKTOP_ERRATA_ACTOR: &str = "mathscape:actor/desktop-user";
static ERRATA_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveMarkdownErrataRequest {
    pub artifact_id: String,
    #[serde(default)]
    pub base_content_hash: Option<String>,
    pub first_block_id: String,
    pub last_block_id: String,
    pub corrected_text: String,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub verified_pdf_page: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SaveMarkdownErrataResponse {
    pub errata: MarkdownBlockErrata,
    pub superseded_ids: Vec<String>,
    pub affected_unit_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RevertMarkdownErrataResponse {
    pub errata: MarkdownBlockErrata,
    pub affected_unit_ids: Vec<String>,
}

pub(crate) fn to_overlay_entries(items: &[MarkdownBlockErrata]) -> Vec<ErrataOverlayEntry> {
    items
        .iter()
        .map(|item| ErrataOverlayEntry {
            errata_id: item.errata_id.clone(),
            first_block_id: item.first_block_id.clone(),
            last_block_id: item.last_block_id.clone(),
            covered_block_ids: item.covered_block_ids.clone(),
            base_byte_start: item.base_byte_start,
            base_byte_end: item.base_byte_end,
            corrected_text: item.corrected_text.clone(),
        })
        .collect()
}

pub(crate) fn generate_errata_id(
    artifact_id: &str,
    first_block_id: &str,
    last_block_id: &str,
    corrected_text: &str,
) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = ERRATA_SEQ.fetch_add(1, Ordering::Relaxed);
    let mut hasher = Sha256::new();
    hasher.update(artifact_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(first_block_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(last_block_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(corrected_text.as_bytes());
    hasher.update(b"\0");
    hasher.update(nanos.to_le_bytes());
    hasher.update(seq.to_le_bytes());
    let hex = format!("{:x}", hasher.finalize());
    format!("mathscape:markdown-errata/{}", &hex[..24])
}

impl DesktopService {
    /// Returns active `SourceSemanticUnit` IDs whose Markdown span overlaps `[byte_start, byte_end)`
    /// or references any block in `block_ids` on `artifact_id`.
    pub(crate) fn affected_unit_ids_for_errata(
        &self,
        artifact_id: &str,
        byte_start: u64,
        byte_end: u64,
        block_ids: &[String],
    ) -> Result<Vec<String>, String> {
        let markers = self.markdown_unit_markers(&[artifact_id.to_owned()])?;
        let block_set: std::collections::HashSet<&str> =
            block_ids.iter().map(String::as_str).collect();
        let mut unit_ids = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for marker in markers {
            let block_overlap = marker
                .block_ids
                .iter()
                .any(|id| block_set.contains(id.as_str()));
            let byte_overlap =
                marker.byte_range.start < byte_end && marker.byte_range.end > byte_start;
            if (block_overlap || byte_overlap) && seen.insert(marker.unit_id.clone()) {
                unit_ids.push(marker.unit_id);
            }
        }
        Ok(unit_ids)
    }

    pub fn list_markdown_errata(
        &self,
        artifact_id: &str,
    ) -> Result<Vec<MarkdownBlockErrata>, String> {
        let store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))
            .map_err(|e| e.to_string())?;
        store
            .list_active_markdown_errata(artifact_id)
            .map_err(|e| e.to_string())
    }

    pub fn list_markdown_errata_history(
        &self,
        artifact_id: &str,
        block_id: Option<&str>,
    ) -> Result<Vec<MarkdownBlockErrata>, String> {
        let store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))
            .map_err(|e| e.to_string())?;
        store
            .list_markdown_errata_history(artifact_id, block_id)
            .map_err(|e| e.to_string())
    }

    pub fn save_markdown_errata(
        &self,
        request: SaveMarkdownErrataRequest,
    ) -> Result<SaveMarkdownErrataResponse, String> {
        validate_errata_corrected_text(&request.corrected_text).map_err(|e| e.to_string())?;
        let document = self.markdown_for_link(&request.artifact_id)?;
        let mut store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))
            .map_err(|e| e.to_string())?;
        let active = store
            .list_active_markdown_errata(&request.artifact_id)
            .map_err(|e| e.to_string())?;
        let overlay = to_overlay_entries(&active);
        let resolved = resolve_errata_target_range(
            &document,
            request.base_content_hash.as_deref(),
            &request.first_block_id,
            &request.last_block_id,
            &overlay,
        )
        .map_err(|e| e.to_string())?;

        let normalized_corrected =
            normalize_errata_corrected_text(&resolved.original_text, &request.corrected_text);
        let now = crate::discovery::timestamp_now()
            .map_err(|e| e.to_string())?
            .as_str()
            .to_owned();
        let errata_id = generate_errata_id(
            &request.artifact_id,
            &resolved.first_block_id,
            &resolved.last_block_id,
            &normalized_corrected,
        );

        let outcome = store
            .upsert_markdown_errata(UpsertMarkdownErrata {
                errata_id,
                artifact_id: request.artifact_id.clone(),
                base_content_hash: document.snapshot.content_hash.to_string(),
                first_block_id: resolved.first_block_id,
                last_block_id: resolved.last_block_id,
                covered_block_ids: resolved.covered_block_ids,
                base_byte_start: resolved.base_byte_start,
                base_byte_end: resolved.base_byte_end,
                base_text_hash: resolved.base_text_hash.to_string(),
                original_text: resolved.original_text,
                corrected_text: normalized_corrected,
                source_kind: MarkdownErrataSourceKind::Manual,
                actor: DESKTOP_ERRATA_ACTOR.to_owned(),
                note: request.note,
                verified_pdf_page: request.verified_pdf_page,
                discovery_run_id: None,
                patch_set_id: None,
                now,
            })
            .map_err(|e| e.to_string())?;
        drop(store);

        let affected_unit_ids = self.affected_unit_ids_for_errata(
            &outcome.errata.artifact_id,
            outcome.errata.base_byte_start,
            outcome.errata.base_byte_end,
            &outcome.errata.covered_block_ids,
        )?;
        self.notify_errata_changed_ink_cards(&outcome.errata.artifact_id, &affected_unit_ids);
        Ok(SaveMarkdownErrataResponse {
            errata: outcome.errata,
            superseded_ids: outcome.superseded_ids,
            affected_unit_ids,
        })
    }

    pub fn revert_markdown_errata(
        &self,
        errata_id: &str,
    ) -> Result<RevertMarkdownErrataResponse, String> {
        let mut store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))
            .map_err(|e| e.to_string())?;
        let now = crate::discovery::timestamp_now()
            .map_err(|e| e.to_string())?
            .as_str()
            .to_owned();
        let errata = store
            .revert_markdown_errata(errata_id, &now)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("未找到勘误记录：{errata_id}"))?;
        drop(store);

        let affected_unit_ids = self.affected_unit_ids_for_errata(
            &errata.artifact_id,
            errata.base_byte_start,
            errata.base_byte_end,
            &errata.covered_block_ids,
        )?;
        self.notify_errata_changed_ink_cards(&errata.artifact_id, &affected_unit_ids);
        Ok(RevertMarkdownErrataResponse {
            errata,
            affected_unit_ids,
        })
    }

    /// Broadcasts updated ProblemCards to any connected tablet whose active note
    /// links directly to `artifact_id` or to one of `affected_unit_ids`.
    pub(crate) fn notify_errata_changed_ink_cards(
        &self,
        artifact_id: &str,
        affected_unit_ids: &[String],
    ) {
        let Ok(store) = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))
        else {
            return;
        };
        let mut doc_ids = BTreeSet::new();
        if let Ok(links) = store.ink_links_to_target("markdown", artifact_id) {
            for linked in links {
                doc_ids.insert(linked.note.document_id);
            }
        }
        for unit_id in affected_unit_ids {
            if let Ok(links) = store.ink_links_to_target("source_object", unit_id) {
                for linked in links {
                    doc_ids.insert(linked.note.document_id);
                }
            }
        }
        drop(store);
        for doc_id in doc_ids {
            self.ink_problem_card_changed(&doc_id);
        }
    }

    pub fn mark_errata_unit_reviewed(
        &self,
        unit_id: &str,
        errata_ids: &[String],
        decision: &str,
    ) -> Result<usize, String> {
        let parsed = ErrataUnitReviewDecision::parse(decision).map_err(|e| e.to_string())?;
        let now = crate::discovery::timestamp_now()
            .map_err(|e| e.to_string())?
            .as_str()
            .to_owned();
        let mut store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))
            .map_err(|e| e.to_string())?;
        let mut count = 0;
        for errata_id in errata_ids {
            if errata_id.trim().is_empty() {
                continue;
            }
            store
                .mark_errata_unit_reviewed(unit_id, errata_id, parsed, &now)
                .map_err(|e| e.to_string())?;
            count += 1;
        }
        Ok(count)
    }

    /// Returns a map of `unit_id -> unreviewed active errata count` across the
    /// workspace for lightweight non-blocking badges on the map.
    pub fn list_unreviewed_errata_units(&self) -> Result<BTreeMap<String, usize>, String> {
        let store = SqliteArtifactStore::open(&self.workspace_root.join("mathscape.sqlite3"))
            .map_err(|e| e.to_string())?;
        let active_errata = store
            .list_all_active_markdown_errata()
            .map_err(|e| e.to_string())?;
        if active_errata.is_empty() {
            return Ok(BTreeMap::new());
        }
        let mut by_artifact: HashMap<&str, Vec<&MarkdownBlockErrata>> = HashMap::new();
        for errata in &active_errata {
            by_artifact
                .entry(errata.artifact_id.as_str())
                .or_default()
                .push(errata);
        }
        let acknowledged: HashSet<(String, String)> = store
            .list_errata_unit_reviews(None)
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|review| review.decision == ErrataUnitReviewDecision::Acknowledged)
            .map(|review| (review.unit_id, review.errata_id))
            .collect();
        let snapshot = store.source_map_snapshot().map_err(|e| e.to_string())?;
        let spans_by_id: HashMap<&StableId, &Value> = snapshot
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

        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for obj in &snapshot.objects {
            if obj.kind != mathscape_domain::EntityKind::SourceSemanticUnit
                || matches!(
                    obj.lifecycle,
                    mathscape_domain::LifecycleStatus::Superseded
                        | mathscape_domain::LifecycleStatus::Rejected
                        | mathscape_domain::LifecycleStatus::Deprecated
                )
            {
                continue;
            }
            let body = obj.payload.get("payload").unwrap_or(&obj.payload);
            let Some(span_ids) = body.get("source_spans").and_then(Value::as_array) else {
                continue;
            };
            let unit_id = obj.stable_id.to_string();
            let mut matched_errata = HashSet::new();
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
                let Some(errata_list) = by_artifact.get(art) else {
                    continue;
                };
                let byte_range = span_body
                    .pointer("/markdown/byte_range")
                    .and_then(|v| Some((v.get("start")?.as_u64()?, v.get("end")?.as_u64()?)));
                let span_blocks: HashSet<&str> = span_body
                    .get("marker_block_ids")
                    .and_then(Value::as_array)
                    .map(|arr| arr.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                for errata in errata_list {
                    let block_overlap = !span_blocks.is_empty()
                        && errata
                            .covered_block_ids
                            .iter()
                            .any(|b| span_blocks.contains(b.as_str()));
                    let byte_overlap = byte_range.is_some_and(|(s, e)| {
                        errata.base_byte_start < e && errata.base_byte_end > s
                    });
                    if !(block_overlap || byte_overlap) {
                        continue;
                    }
                    if acknowledged.contains(&(unit_id.clone(), errata.errata_id.clone())) {
                        continue;
                    }
                    matched_errata.insert(errata.errata_id.as_str());
                }
            }
            if !matched_errata.is_empty() {
                counts.insert(unit_id, matched_errata.len());
            }
        }
        Ok(counts)
    }
}

#[tauri::command]
pub async fn list_markdown_errata(
    state: State<'_, DesktopState>,
    artifact_id: String,
) -> Result<Vec<MarkdownBlockErrata>, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || service.list_markdown_errata(&artifact_id))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn list_markdown_errata_history(
    state: State<'_, DesktopState>,
    artifact_id: String,
    block_id: Option<String>,
) -> Result<Vec<MarkdownBlockErrata>, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || {
        service.list_markdown_errata_history(&artifact_id, block_id.as_deref())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn save_markdown_errata(
    state: State<'_, DesktopState>,
    request: SaveMarkdownErrataRequest,
) -> Result<SaveMarkdownErrataResponse, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || service.save_markdown_errata(request))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn revert_markdown_errata(
    state: State<'_, DesktopState>,
    errata_id: String,
) -> Result<RevertMarkdownErrataResponse, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || service.revert_markdown_errata(&errata_id))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn mark_errata_unit_reviewed(
    state: State<'_, DesktopState>,
    unit_id: String,
    errata_ids: Vec<String>,
    decision: String,
) -> Result<usize, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || {
        service.mark_errata_unit_reviewed(&unit_id, &errata_ids, &decision)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn list_unreviewed_errata_units(
    state: State<'_, DesktopState>,
) -> Result<BTreeMap<String, usize>, String> {
    let service = state.service();
    tauri::async_runtime::spawn_blocking(move || service.list_unreviewed_errata_units())
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use mathscape_artifact::{ArtifactMetadata, ArtifactPool};
    use mathscape_domain::{ActorRef, MediaType, RightsRecord, RightsStatus};
    use mathscape_store_sqlite::MarkdownErrataStatus;

    use super::*;

    fn temp_workspace(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("mathscape-errata-{label}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn save_expand_overlap_and_revert_markdown_errata_end_to_end() {
        let root = temp_workspace("roundtrip");
        let md_path = root.join("chapter1.md");
        let raw = "# Chapter 1\n\nTheorem 1: $x^a = 1$.\n\nProof: obvious.\n\n{1}------------------------------------------------\n\nPage 2 text.\n";
        std::fs::write(&md_path, raw).unwrap();

        let service = DesktopService::open(&root).unwrap();
        let mut pool = ArtifactPool::open(&root).unwrap();
        let imported = pool
            .import_file(
                &md_path,
                ArtifactMetadata::new(
                    None,
                    MediaType::parse("text/markdown").unwrap(),
                    Some("chapter1.md".into()),
                    None,
                    RightsRecord::new(RightsStatus::NotAssessed, vec![], None).unwrap(),
                ),
                ActorRef::parse("test-user").unwrap(),
            )
            .unwrap();
        let artifact_id = imported.stable_id().to_string();
        drop(pool);
        let doc = service.markdown_for_link(&artifact_id).unwrap();
        assert_eq!(doc.blocks.len(), 5);
        let hash = doc.snapshot.content_hash.to_string();

        // Reject range crossing page marker
        let marker_err = service.save_markdown_errata(SaveMarkdownErrataRequest {
            artifact_id: artifact_id.clone(),
            base_content_hash: Some(hash.clone()),
            first_block_id: doc.blocks[2].id.clone(),
            last_block_id: doc.blocks[4].id.clone(),
            corrected_text: "bad".into(),
            note: None,
            verified_pdf_page: None,
        });
        assert!(marker_err.is_err());

        // Save single-block errata on Theorem 1 (block 1)
        let saved1 = service
            .save_markdown_errata(SaveMarkdownErrataRequest {
                artifact_id: artifact_id.clone(),
                base_content_hash: Some(hash.clone()),
                first_block_id: doc.blocks[1].id.clone(),
                last_block_id: doc.blocks[1].id.clone(),
                corrected_text: "Theorem 1: $x^n = 1$.".into(),
                note: Some("核对原书".into()),
                verified_pdf_page: Some(0),
            })
            .unwrap();
        assert!(saved1.superseded_ids.is_empty());
        assert_eq!(saved1.errata.corrected_text, "Theorem 1: $x^n = 1$.\n");

        // Expand across blocks [1..=2] -> supersedes saved1 atomically
        let saved2 = service
            .save_markdown_errata(SaveMarkdownErrataRequest {
                artifact_id: artifact_id.clone(),
                base_content_hash: Some(hash.clone()),
                first_block_id: doc.blocks[1].id.clone(),
                last_block_id: doc.blocks[2].id.clone(),
                corrected_text: "Theorem 1: $x^n = 1$.\n\nProof: by induction.".into(),
                note: None,
                verified_pdf_page: Some(0),
            })
            .unwrap();
        assert_eq!(saved2.superseded_ids, vec![saved1.errata.errata_id.clone()]);
        assert_eq!(saved2.errata.covered_block_ids.len(), 2);

        let active = service.list_markdown_errata(&artifact_id).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].errata_id, saved2.errata.errata_id);

        // Revert cleanly
        let reverted = service
            .revert_markdown_errata(&saved2.errata.errata_id)
            .unwrap();
        assert_eq!(reverted.errata.status, MarkdownErrataStatus::Reverted);
        assert!(
            service
                .list_markdown_errata(&artifact_id)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            service
                .list_markdown_errata_history(&artifact_id, None)
                .unwrap()
                .len(),
            2
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
