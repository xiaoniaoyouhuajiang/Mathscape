//! Transport contracts for source discovery preparation, not canonical semantic entities.
//! Deserialization does not certify a selection: the orchestrator must resolve it
//! against a freshly loaded immutable artifact before accepting its byte ranges.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{ContentHash, StableId};

pub const MARKDOWN_ANCHOR_SCHEME: &str = "mathscape-markdown-blocks-1";
pub const DISCOVERY_SELECTION_VERSION: &str = "mathscape-discovery-selection-1";
pub const DISCOVERY_INPUT_VERSION: &str = "mathscape-discovery-input-2";

/// Zero-based UTF-8 bytes, end exclusive. Validity depends on the source text.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextByteRange {
    pub start: u64,
    pub end: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
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
    pub input_fingerprint: ContentHash,
}

/// A lossless partition of one region. Block segments can be referenced by a
/// producer; gaps retain the original bytes between blocks for replay.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "segment_kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DiscoveryInputSegment {
    Block {
        block_id: String,
        structural_kind: String,
        /// Complete physical block, even if only part is visible in this input.
        physical_range: TextByteRange,
        /// Portion of the physical block included in this input.
        source_range: TextByteRange,
        text: String,
    },
    Gap {
        source_range: TextByteRange,
        text: String,
    },
}

impl DiscoveryInputSegment {
    pub fn text(&self) -> &str {
        match self {
            Self::Block { text, .. } | Self::Gap { text, .. } => text,
        }
    }

    /// Physical clipping only; says nothing about mathematical completeness.
    pub fn clipped_start(&self) -> bool {
        matches!(self, Self::Block { physical_range, source_range, .. }
            if source_range.start > physical_range.start)
    }

    pub fn clipped_end(&self) -> bool {
        matches!(self, Self::Block { physical_range, source_range, .. }
            if source_range.end < physical_range.end)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryInputRegion {
    pub source_range: TextByteRange,
    pub segments: Vec<DiscoveryInputSegment>,
}

impl DiscoveryInputRegion {
    /// Counts actual payload bytes; does not certify that ranges match the text.
    pub fn utf8_bytes(&self) -> usize {
        self.segments
            .iter()
            .map(|segment| segment.text().len())
            .sum()
    }

    pub fn block_count(&self) -> usize {
        self.segments
            .iter()
            .filter(|segment| matches!(segment, DiscoveryInputSegment::Block { .. }))
            .count()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryInputBoundary {
    pub at_document_start: bool,
    pub at_document_end: bool,
}

/// Provider-neutral, replayable source payload prepared for discovery. It does
/// not include admission policy so automatic publication cannot bias extraction.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryInput {
    pub version: String,
    pub snapshot: MarkdownSnapshot,
    pub selection_fingerprint: ContentHash,
    pub fingerprint: ContentHash,
    pub requested: DiscoveryInputRegion,
    /// Currently empty; future context must remain distinguishable from requested text.
    pub context: Vec<DiscoveryInputRegion>,
    pub boundary: DiscoveryInputBoundary,
}
