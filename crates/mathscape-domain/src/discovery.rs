//! Transport contracts for source discovery preparation, not canonical semantic entities.
//! Deserialization does not certify a selection: the orchestrator must resolve it
//! against a freshly loaded immutable artifact before accepting its byte ranges.

use serde::{Deserialize, Serialize};

use crate::{ContentHash, StableId};

pub const MARKDOWN_ANCHOR_SCHEME: &str = "mathscape-markdown-blocks-1";
pub const DISCOVERY_SELECTION_VERSION: &str = "mathscape-discovery-selection-1";

/// Zero-based UTF-8 bytes, end exclusive. Validity depends on the source text.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextByteRange {
    pub start: u64,
    pub end: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkdownSnapshot {
    pub artifact_id: StableId,
    pub conversion_id: Option<StableId>,
    pub content_hash: ContentHash,
    pub anchor_scheme: String,
    pub fingerprint: ContentHash,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceSelector {
    WholeDocument,
    BlockRange {
        first: String,
        last: String,
    },
    /// Both quotes must occur exactly once in their specified blocks.
    QuoteRange {
        first: String,
        start_quote: String,
        last: String,
        end_quote: String,
    },
}

/// Admission policy is not a review/certification status.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryAdmission {
    #[default]
    ReviewBeforePublish,
    AutomaticCandidate,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoverySelectionRequest {
    pub version: String,
    pub snapshot: MarkdownSnapshot,
    pub selector: SourceSelector,
    pub admission: DiscoveryAdmission,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MarkdownBlock {
    pub id: String,
    pub range: TextByteRange,
    /// Structural display hint, not a mathematical unit kind.
    pub kind: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AnchoredMarkdown {
    pub snapshot: MarkdownSnapshot,
    pub source: String,
    pub blocks: Vec<MarkdownBlock>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ResolvedDiscoverySelection {
    pub request: DiscoverySelectionRequest,
    pub range: TextByteRange,
    pub block_ids: Vec<String>,
    pub raw_text: String,
    /// Preparation only: no automatic context expansion in v1.
    pub context_ranges: Vec<TextByteRange>,
    pub input_fingerprint: ContentHash,
}
