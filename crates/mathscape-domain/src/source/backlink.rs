//! Deterministic Markdown-to-PDF page provenance.

use serde::{Deserialize, Serialize};

use crate::{ContentHash, StableId, TextByteRange};

pub const MARKDOWN_PDF_BLOCK_MAP_VERSION: &str = "mathscape-markdown-pdf-block-map-1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkdownPdfMappingConfidence {
    Exact,
    Candidate,
    Unmapped,
}

/// A stable, content-addressed sidecar produced by a Marker conversion.
/// Page indexes are zero-based and refer to the original PDF, not the
/// user-facing one-based page number.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MarkdownPdfBlockMap {
    pub version: String,
    pub conversion_id: StableId,
    pub markdown_artifact_id: StableId,
    pub pdf_artifact_id: StableId,
    pub markdown_content_hash: ContentHash,
    pub marker_page_ids: Vec<u32>,
    pub entries: Vec<MarkdownPdfBlockEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MarkdownPdfBlockEntry {
    pub block_id: String,
    pub markdown_range: TextByteRange,
    pub pdf_page_indices: Vec<u32>,
    pub confidence: MarkdownPdfMappingConfidence,
    pub text_fingerprint: ContentHash,
}
