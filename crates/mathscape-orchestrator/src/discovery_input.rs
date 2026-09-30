use std::collections::HashSet;

use mathscape_domain::{
    AnchoredMarkdown, ContentHash, DISCOVERY_INPUT_VERSION, DiscoveryInput, DiscoveryInputBoundary,
    DiscoveryInputRegion, DiscoveryInputSegment, ResolvedDiscoverySelection, TextByteRange,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{AnchorError, PAGE_MARKER_STRUCTURAL_KIND, is_page_marker_text, resolve_selection};

pub const DEFAULT_DISCOVERY_INPUT_BYTES: usize = 128 * 1024;
pub const DEFAULT_DISCOVERY_INPUT_BLOCKS: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiscoveryInputLimits {
    max_utf8_bytes: usize,
    max_blocks: usize,
}

impl DiscoveryInputLimits {
    pub fn new(max_utf8_bytes: usize, max_blocks: usize) -> Result<Self, DiscoveryInputError> {
        if max_utf8_bytes == 0 || max_blocks == 0 {
            return Err(DiscoveryInputError::InvalidLimits);
        }
        Ok(Self {
            max_utf8_bytes,
            max_blocks,
        })
    }

    pub const fn max_utf8_bytes(self) -> usize {
        self.max_utf8_bytes
    }

    pub const fn max_blocks(self) -> usize {
        self.max_blocks
    }
}

impl Default for DiscoveryInputLimits {
    fn default() -> Self {
        Self {
            max_utf8_bytes: DEFAULT_DISCOVERY_INPUT_BYTES,
            max_blocks: DEFAULT_DISCOVERY_INPUT_BLOCKS,
        }
    }
}

#[derive(Debug, Error)]
pub enum DiscoveryInputError {
    #[error("selection could not be resolved: {0}")]
    Selection(#[from] AnchorError),
    #[error("resolved selection does not match a fresh resolution")]
    ResolvedSelectionMismatch,
    #[error("anchored Markdown contains invalid, overlapping, or duplicate blocks")]
    InvalidDocument,
    #[error("selected source is {actual} UTF-8 bytes; input limit is {limit}")]
    ByteLimit { actual: usize, limit: usize },
    #[error("selected source contains {actual} blocks; input limit is {limit}")]
    BlockLimit { actual: usize, limit: usize },
    #[error("discovery input limits must be positive")]
    InvalidLimits,
    #[error("stored discovery input no longer matches the managed Markdown: {0}")]
    StaleCheckpoint(&'static str),
}

fn hash<T: Serialize>(value: &T) -> ContentHash {
    let bytes = serde_json::to_vec(value).expect("discovery input values are serializable");
    ContentHash::parse(format!("sha256:{:x}", Sha256::digest(bytes)))
        .expect("SHA-256 always produces a valid content hash")
}

pub(crate) fn discovery_input_fingerprint(input: &DiscoveryInput) -> ContentHash {
    hash(&(
        &input.version,
        &input.snapshot,
        &input.selection_fingerprint,
        &input.requested,
        &input.context,
        &input.boundary,
    ))
}

fn checked_range(range: &TextByteRange, source: &str) -> Option<std::ops::Range<usize>> {
    let start = usize::try_from(range.start).ok()?;
    let end = usize::try_from(range.end).ok()?;
    (start <= end
        && end <= source.len()
        && source.is_char_boundary(start)
        && source.is_char_boundary(end))
    .then_some(start..end)
}

fn validate_document(document: &AnchoredMarkdown) -> Result<(), DiscoveryInputError> {
    let mut prior_end = 0;
    let mut ids = HashSet::new();
    for block in &document.blocks {
        let range = checked_range(&block.range, &document.source)
            .ok_or(DiscoveryInputError::InvalidDocument)?;
        if range.is_empty() || range.start < prior_end || !ids.insert(&block.id) {
            return Err(DiscoveryInputError::InvalidDocument);
        }
        prior_end = range.end;
    }
    Ok(())
}

/// Rebinds an untrusted persisted DiscoveryInput to the current immutable
/// Markdown Artifact. This restores no validator authority; it only proves
/// that every persisted byte range and byte payload still names the same
/// managed source.
pub fn rebind_discovery_input(
    document: &AnchoredMarkdown,
    input: &DiscoveryInput,
    limits: DiscoveryInputLimits,
) -> Result<(), DiscoveryInputError> {
    validate_document(document)?;
    if input.version != mathscape_domain::DISCOVERY_INPUT_VERSION {
        return Err(DiscoveryInputError::StaleCheckpoint(
            "unsupported input version",
        ));
    }
    if input.snapshot != document.snapshot {
        return Err(DiscoveryInputError::StaleCheckpoint(
            "artifact snapshot changed",
        ));
    }
    if !input.context.is_empty() {
        return Err(DiscoveryInputError::StaleCheckpoint(
            "context is unsupported by this pipeline version",
        ));
    }
    if input.fingerprint != discovery_input_fingerprint(input) {
        return Err(DiscoveryInputError::StaleCheckpoint(
            "input fingerprint changed",
        ));
    }

    let requested = checked_range(&input.requested.source_range, &document.source).ok_or(
        DiscoveryInputError::StaleCheckpoint("requested range is invalid"),
    )?;
    if input.boundary.at_document_start != (requested.start == 0)
        || input.boundary.at_document_end != (requested.end == document.source.len())
    {
        return Err(DiscoveryInputError::StaleCheckpoint(
            "boundary flags changed",
        ));
    }
    if requested.len() > limits.max_utf8_bytes {
        return Err(DiscoveryInputError::ByteLimit {
            actual: requested.len(),
            limit: limits.max_utf8_bytes,
        });
    }

    let mut cursor = input.requested.source_range.start;
    let mut block_count = 0usize;
    for segment in &input.requested.segments {
        let (range, text) = match segment {
            DiscoveryInputSegment::Block {
                block_id,
                structural_kind,
                physical_range,
                source_range,
                text,
            } => {
                block_count += 1;
                let Some(block) = document.blocks.iter().find(|block| block.id == *block_id) else {
                    return Err(DiscoveryInputError::StaleCheckpoint(
                        "Markdown block disappeared",
                    ));
                };
                let legacy_page_marker_kind = structural_kind == "text"
                    && block.kind == PAGE_MARKER_STRUCTURAL_KIND
                    && document
                        .source
                        .get(block.range.start as usize..block.range.end as usize)
                        .is_some_and(is_page_marker_text);
                if block.range != *physical_range
                    || (block.kind != *structural_kind && !legacy_page_marker_kind)
                {
                    return Err(DiscoveryInputError::StaleCheckpoint(
                        "Markdown block changed",
                    ));
                }
                if source_range.start < physical_range.start
                    || source_range.end > physical_range.end
                {
                    return Err(DiscoveryInputError::StaleCheckpoint(
                        "block clipping changed",
                    ));
                }
                (source_range, text)
            }
            DiscoveryInputSegment::Gap { source_range, text } => (source_range, text),
        };
        if range.start != cursor || range.end <= range.start {
            return Err(DiscoveryInputError::StaleCheckpoint(
                "segments are not a lossless ordered partition",
            ));
        }
        let actual = checked_range(range, &document.source)
            .and_then(|range| document.source.get(range))
            .ok_or(DiscoveryInputError::StaleCheckpoint(
                "segment range is invalid",
            ))?;
        if actual != text {
            return Err(DiscoveryInputError::StaleCheckpoint("segment text changed"));
        }
        cursor = range.end;
    }
    if cursor != input.requested.source_range.end {
        return Err(DiscoveryInputError::StaleCheckpoint(
            "segments do not cover the requested range",
        ));
    }
    if block_count > limits.max_blocks {
        return Err(DiscoveryInputError::BlockLimit {
            actual: block_count,
            limit: limits.max_blocks,
        });
    }
    Ok(())
}

fn text_segment(
    source: &str,
    range: std::ops::Range<usize>,
) -> Result<String, DiscoveryInputError> {
    source
        .get(range)
        .map(str::to_owned)
        .ok_or(DiscoveryInputError::InvalidDocument)
}

/// Builds a finite, lossless payload from a selection already resolved against
/// this document. It re-resolves the request and never truncates to fit limits.
pub fn build_discovery_input(
    document: &AnchoredMarkdown,
    selection: &ResolvedDiscoverySelection,
    limits: DiscoveryInputLimits,
) -> Result<DiscoveryInput, DiscoveryInputError> {
    validate_document(document)?;
    let fresh = resolve_selection(document, selection.request.clone())?;
    if &fresh != selection {
        return Err(DiscoveryInputError::ResolvedSelectionMismatch);
    }
    build_discovery_input_for_range(
        document,
        selection.range.clone(),
        selection.input_fingerprint.clone(),
        limits,
    )
}

/// Rebuilds a DiscoveryInput after a deterministic Markdown patch. The caller
/// supplies a range map produced by the patch engine; no selector is guessed.
pub fn rebind_discovery_input_after_patch(
    document: &AnchoredMarkdown,
    original: &DiscoveryInput,
    offset_map: &crate::markdown_patch::OffsetMap,
    limits: DiscoveryInputLimits,
) -> Result<DiscoveryInput, DiscoveryInputError> {
    let range = offset_map
        .map_range(&original.requested.source_range)
        .ok_or(DiscoveryInputError::StaleCheckpoint(
            "patched selection boundary is ambiguous",
        ))?;
    let selection_fingerprint = hash(&(&document.snapshot.fingerprint, &range));
    build_discovery_input_for_range(document, range, selection_fingerprint, limits)
}

fn build_discovery_input_for_range(
    document: &AnchoredMarkdown,
    range: TextByteRange,
    selection_fingerprint: ContentHash,
    limits: DiscoveryInputLimits,
) -> Result<DiscoveryInput, DiscoveryInputError> {
    let selected =
        checked_range(&range, &document.source).ok_or(DiscoveryInputError::InvalidDocument)?;
    let selected_len = selected.len();
    if selected_len > limits.max_utf8_bytes {
        return Err(DiscoveryInputError::ByteLimit {
            actual: selected_len,
            limit: limits.max_utf8_bytes,
        });
    }

    let included: Vec<_> = document
        .blocks
        .iter()
        .filter(|block| block.range.end > range.start && block.range.start < range.end)
        .collect();
    if included.len() > limits.max_blocks {
        return Err(DiscoveryInputError::BlockLimit {
            actual: included.len(),
            limit: limits.max_blocks,
        });
    }

    let mut segments = Vec::with_capacity(included.len().saturating_mul(2) + 1);
    let mut cursor = selected.start;
    for block in &included {
        let physical = checked_range(&block.range, &document.source)
            .ok_or(DiscoveryInputError::InvalidDocument)?;
        let visible = physical.start.max(selected.start)..physical.end.min(selected.end);
        if cursor < visible.start {
            segments.push(DiscoveryInputSegment::Gap {
                source_range: TextByteRange {
                    start: cursor as u64,
                    end: visible.start as u64,
                },
                text: text_segment(&document.source, cursor..visible.start)?,
            });
        }
        segments.push(DiscoveryInputSegment::Block {
            block_id: block.id.clone(),
            structural_kind: block.kind.clone(),
            physical_range: block.range.clone(),
            source_range: TextByteRange {
                start: visible.start as u64,
                end: visible.end as u64,
            },
            text: text_segment(&document.source, visible.clone())?,
        });
        cursor = visible.end;
    }
    if cursor < selected.end {
        segments.push(DiscoveryInputSegment::Gap {
            source_range: TextByteRange {
                start: cursor as u64,
                end: selected.end as u64,
            },
            text: text_segment(&document.source, cursor..selected.end)?,
        });
    }

    let reconstructed: String = segments.iter().map(DiscoveryInputSegment::text).collect();
    let expected = document
        .source
        .get(selected.clone())
        .ok_or(DiscoveryInputError::InvalidDocument)?;
    if reconstructed != expected {
        return Err(DiscoveryInputError::ResolvedSelectionMismatch);
    }
    let mut input = DiscoveryInput {
        version: DISCOVERY_INPUT_VERSION.into(),
        snapshot: document.snapshot.clone(),
        selection_fingerprint,
        // Filled from every producer-visible field below.
        fingerprint: hash(&(&document.snapshot.fingerprint, &range)),
        requested: DiscoveryInputRegion {
            source_range: range.clone(),
            segments,
        },
        context: vec![],
        boundary: DiscoveryInputBoundary {
            at_document_start: selected.start == 0,
            at_document_end: selected.end == document.source.len(),
        },
    };
    input.fingerprint = discovery_input_fingerprint(&input);
    Ok(input)
}
