//! Readonly canonical markdown lookup for the existing material tools.
//!
//! Coverage is projected from conversion history. This module never writes the
//! registry and never runs Marker. A persisted head, when present, replaces
//! the projected head so an explicit registry row wins.

use std::collections::BTreeMap;

use mathscape_agent::{ToolError, ToolErrorCode};
use mathscape_domain::{CanonicalMarkdownView, InclusivePageRange, StableId};
use mathscape_store_sqlite::project_canonical_markdown;

use crate::McpWorkspace;

pub(super) struct CanonicalIndex {
    views: Vec<CanonicalMarkdownView>,
}

impl CanonicalIndex {
    pub(super) fn load(workspace: &McpWorkspace) -> Result<Self, ToolError> {
        let store = workspace.store()?;
        let catalog = store.read_catalog_snapshot().map_err(|error| {
            ToolError::new(
                ToolErrorCode::Internal,
                format!("failed to read artifact catalog: {error}"),
            )
        })?;
        let mut views = project_canonical_markdown(&catalog).map_err(|error| {
            ToolError::new(
                ToolErrorCode::Internal,
                format!("failed to project canonical markdown coverage: {error}"),
            )
        })?;
        let stored = store.list_canonical_markdown().map_err(|error| {
            ToolError::new(
                ToolErrorCode::Internal,
                format!("failed to read canonical markdown registry: {error}"),
            )
        })?;
        let mut heads: BTreeMap<String, CanonicalMarkdownView> = BTreeMap::new();
        for view in stored {
            heads.insert(view.canonical_id.to_string(), view);
        }
        for view in &mut views {
            if let Some(stored) = heads.remove(view.canonical_id.as_str()) {
                view.head_artifact_id = stored.head_artifact_id;
                view.head_revision = stored.head_revision;
            }
        }
        views.extend(heads.into_values());
        views.sort_by(|left, right| left.canonical_id.cmp(&right.canonical_id));
        Ok(Self { views })
    }

    pub(super) fn by_id(&self, id: &str) -> Option<&CanonicalMarkdownView> {
        self.views
            .iter()
            .find(|view| view.canonical_id.as_str() == id)
    }

    pub(super) fn for_pdf(
        &self,
        pdf_id: &str,
        requested: Option<InclusivePageRange>,
    ) -> Option<&CanonicalMarkdownView> {
        let mut matches: Vec<_> = self
            .views
            .iter()
            .filter(|view| view.source_pdf_artifact_id.as_str() == pdf_id)
            .collect();
        matches.sort_by(|left, right| {
            rank(right, requested)
                .cmp(&rank(left, requested))
                .then_with(|| left.canonical_id.cmp(&right.canonical_id))
        });
        matches.into_iter().next()
    }

    pub(super) fn for_markdown(&self, artifact_id: &str) -> Option<&CanonicalMarkdownView> {
        self.views.iter().find(|view| {
            view.head_artifact_id
                .as_ref()
                .is_some_and(|head| head.as_str() == artifact_id)
                || view
                    .segments
                    .iter()
                    .any(|segment| segment.markdown_artifact_id.as_str() == artifact_id)
        })
    }

    /// Physical markdown id for a canonical id, when the caller addressed the view.
    pub(super) fn markdown_target<'a>(&'a self, id: &'a StableId) -> &'a StableId {
        self.by_id(id.as_str())
            .and_then(|view| view.head_artifact_id.as_ref())
            .unwrap_or(id)
    }
}

fn rank(view: &CanonicalMarkdownView, requested: Option<InclusivePageRange>) -> (u8, u32) {
    let intersects = requested.is_some_and(|request| {
        view.segments
            .iter()
            .any(|segment| segment.pages.is_some_and(|pages| pages.overlaps(request)))
    });
    let covered = view
        .coverage(None)
        .covered
        .iter()
        .map(|range| range.page_count())
        .fold(0_u32, u32::saturating_add);
    (u8::from(intersects), covered)
}
