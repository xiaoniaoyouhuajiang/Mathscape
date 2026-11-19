//! Shared active Markdown errata loading and provenance formatting for MCP tools.

use std::collections::{HashMap, HashSet};

use mathscape_agent::{ToolError, ToolErrorCode};
use mathscape_orchestrator::{
    EffectiveByteMap, EffectiveByteSegmentKind, ErrataOverlayEntry,
};
use mathscape_store_sqlite::{MarkdownBlockErrata, SqliteArtifactStore, SqliteStoreError};
use serde_json::{Value, json};

use crate::McpWorkspace;

/// Active errata rows for one physical Markdown snapshot, pre-projected into
/// [`ErrataOverlayEntry`] values for `mathscape_orchestrator::markdown_errata`.
#[derive(Clone, Debug, Default)]
pub(super) struct LoadedErrata {
    pub(super) rows: Vec<MarkdownBlockErrata>,
    pub(super) entries: Vec<ErrataOverlayEntry>,
}

impl LoadedErrata {
    pub(super) fn from_rows(rows: Vec<MarkdownBlockErrata>) -> Self {
        let entries = rows
            .iter()
            .map(|row| ErrataOverlayEntry {
                errata_id: row.errata_id.clone(),
                first_block_id: row.first_block_id.clone(),
                last_block_id: row.last_block_id.clone(),
                covered_block_ids: row.covered_block_ids.clone(),
                base_byte_start: row.base_byte_start,
                base_byte_end: row.base_byte_end,
                corrected_text: row.corrected_text.clone(),
            })
            .collect();
        Self { rows, entries }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns compact provenance records for every active errata whose
    /// effective segment begins inside `[0, prefix_len)`.
    pub(super) fn applied_within_prefix(
        &self,
        byte_map: &EffectiveByteMap,
        prefix_len: usize,
    ) -> Vec<Value> {
        if self.rows.is_empty() || prefix_len == 0 {
            return Vec::new();
        }
        let by_id: HashMap<&str, &MarkdownBlockErrata> = self
            .rows
            .iter()
            .map(|row| (row.errata_id.as_str(), row))
            .collect();
        let mut seen = HashSet::new();
        let mut applied = Vec::new();
        for seg in byte_map.segments() {
            if seg.eff_start >= prefix_len {
                break;
            }
            if let EffectiveByteSegmentKind::Errata { errata_id } = &seg.kind
                && seen.insert(errata_id.as_str())
                && let Some(row) = by_id.get(errata_id.as_str())
            {
                applied.push(errata_provenance_json(row));
            }
        }
        applied
    }
}

pub(super) fn load_active_errata_from_store(
    store: &SqliteArtifactStore,
    artifact_id: &str,
) -> Result<LoadedErrata, SqliteStoreError> {
    let rows = store.list_active_markdown_errata(artifact_id)?;
    Ok(LoadedErrata::from_rows(rows))
}

pub(super) fn load_active_errata(
    workspace: &McpWorkspace,
    artifact_id: &str,
) -> Result<LoadedErrata, ToolError> {
    let store = workspace.store()?;
    load_active_errata_from_store(&store, artifact_id).map_err(|error| {
        ToolError::new(
            ToolErrorCode::Internal,
            format!("failed to load active markdown errata for {artifact_id}: {error}"),
        )
    })
}

pub(super) fn errata_provenance_json(row: &MarkdownBlockErrata) -> Value {
    json!({
        "errata_id": row.errata_id,
        "artifact_id": row.artifact_id,
        "physical_byte_range": {
            "start": row.base_byte_start,
            "end": row.base_byte_end,
        },
        "source_kind": row.source_kind.as_str(),
    })
}

pub(super) fn applied_errata_diagnostic(count: usize) -> Value {
    super::diagnostic(
        "markdown_errata_applied",
        format!(
            "{count} active markdown errata overlay(s) applied to the returned text (human-verified against the source book; prefer this effective text over raw OCR; use material.locate for exact physical md_byte_range)"
        ),
    )
}
