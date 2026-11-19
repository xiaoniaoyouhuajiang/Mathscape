//! Desktop commands and service helpers for the block-level Markdown errata overlay.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use mathscape_orchestrator::{
    ErrataOverlayEntry, normalize_errata_corrected_text, resolve_errata_target_range,
    validate_errata_corrected_text,
};
use mathscape_store_sqlite::{
    MarkdownBlockErrata, MarkdownErrataSourceKind, SqliteArtifactStore, UpsertMarkdownErrata,
};
use serde::{Deserialize, Serialize};
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

fn generate_errata_id(
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
        Ok(RevertMarkdownErrataResponse {
            errata,
            affected_unit_ids,
        })
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
        assert!(service.list_markdown_errata(&artifact_id).unwrap().is_empty());
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
