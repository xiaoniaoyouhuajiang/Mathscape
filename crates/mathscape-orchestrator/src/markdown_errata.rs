//! Pure, deterministic Markdown block-errata projection & validation engine.
//!
//! Physical Markdown snapshots remain immutable. Active errata entries override
//! disjoint contiguous physical block ranges `[base_byte_start, base_byte_end)`.
//! Both Desktop (reader, problem cards, inspector) and MCP (`material.read`,
//! `material.grep`, `material.locate`, `propose_changes`) share this exact
//! projection so every consumer sees identical effective text while physical
//! byte locators never drift.

use std::ops::Range;

use mathscape_domain::{AnchoredMarkdown, ContentHash, TextByteRange};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::markdown_anchors::{PAGE_MARKER_STRUCTURAL_KIND, is_page_marker_text};

/// Minimal, store-agnostic representation of an active Markdown errata entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrataOverlayEntry {
    pub errata_id: String,
    pub first_block_id: String,
    pub last_block_id: String,
    pub covered_block_ids: Vec<String>,
    pub base_byte_start: u64,
    pub base_byte_end: u64,
    pub corrected_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectiveByteSegmentKind {
    Raw,
    Errata { errata_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveByteSegment {
    pub eff_start: usize,
    pub eff_end: usize,
    pub raw_start: usize,
    pub raw_end: usize,
    pub kind: EffectiveByteSegmentKind,
}

/// Bidirectional coordinate map between an effective (errata-overlaid) string
/// and its underlying immutable physical Markdown snapshot bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EffectiveByteMap {
    segments: Vec<EffectiveByteSegment>,
}

impl EffectiveByteMap {
    pub fn segments(&self) -> &[EffectiveByteSegment] {
        &self.segments
    }

    /// Maps a non-empty UTF-8 byte range in `effective_source` back to a valid
    /// physical byte range in the immutable snapshot.
    ///
    /// - Offsets inside unmodified raw regions map 1:1 with exact translation.
    /// - Offsets landing inside an errata-overridden region snap cleanly to
    ///   that errata's physical block slot boundaries (`raw_start` / `raw_end`),
    ///   guaranteeing valid UTF-8 boundaries and stable `SourceSpan` anchoring.
    pub fn effective_range_to_physical(&self, eff_start: usize, eff_end: usize) -> Option<TextByteRange> {
        if eff_start >= eff_end || self.segments.is_empty() {
            return None;
        }
        let start_seg = self
            .segments
            .iter()
            .find(|seg| eff_start >= seg.eff_start && eff_start < seg.eff_end)?;
        let last_byte = eff_end - 1;
        let end_seg = self
            .segments
            .iter()
            .find(|seg| last_byte >= seg.eff_start && last_byte < seg.eff_end)?;

        let phys_start = match start_seg.kind {
            EffectiveByteSegmentKind::Raw => start_seg.raw_start + (eff_start - start_seg.eff_start),
            EffectiveByteSegmentKind::Errata { .. } => start_seg.raw_start,
        };
        let phys_end = match end_seg.kind {
            EffectiveByteSegmentKind::Raw => end_seg.raw_start + (eff_end - end_seg.eff_start),
            EffectiveByteSegmentKind::Errata { .. } => end_seg.raw_end,
        };
        if phys_start >= phys_end {
            return None;
        }
        Some(TextByteRange {
            start: phys_start as u64,
            end: phys_end as u64,
        })
    }

    /// Maps a physical byte offset within the projected slice to the
    /// corresponding effective offset (useful for `within` range clamping).
    pub fn physical_offset_to_effective(&self, raw_offset: usize, prefer_end: bool) -> usize {
        for seg in &self.segments {
            if raw_offset < seg.raw_start {
                return seg.eff_start;
            }
            if raw_offset < seg.raw_end {
                return match seg.kind {
                    EffectiveByteSegmentKind::Raw => seg.eff_start + (raw_offset - seg.raw_start),
                    EffectiveByteSegmentKind::Errata { .. } => {
                        if prefer_end {
                            seg.eff_end
                        } else {
                            seg.eff_start
                        }
                    }
                };
            }
            if raw_offset == seg.raw_end && prefer_end {
                return seg.eff_end;
            }
        }
        self.segments.last().map_or(0, |seg| seg.eff_end)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveMarkdownView {
    pub effective_source: String,
    pub byte_map: EffectiveByteMap,
    pub applied_errata_ids: Vec<String>,
}

/// Overlays active errata entries onto `raw_source[slice_range]`, returning both
/// the effective Markdown string and the bidirectional `EffectiveByteMap`.
pub fn render_effective_slice(
    raw_source: &str,
    slice_range: Range<usize>,
    active_errata: &[ErrataOverlayEntry],
) -> EffectiveMarkdownView {
    let slice_start = slice_range.start.min(raw_source.len());
    let slice_end = slice_range.end.min(raw_source.len()).max(slice_start);

    let mut overlapping: Vec<&ErrataOverlayEntry> = active_errata
        .iter()
        .filter(|e| {
            let s = e.base_byte_start as usize;
            let end = e.base_byte_end as usize;
            end > slice_start && s < slice_end && end > s && end <= raw_source.len()
        })
        .collect();
    overlapping.sort_by_key(|e| (e.base_byte_start, e.base_byte_end));

    let mut effective_source = String::with_capacity(slice_end.saturating_sub(slice_start));
    let mut segments = Vec::new();
    let mut applied_errata_ids = Vec::new();
    let mut cursor = slice_start;

    for errata in overlapping {
        let e_start = errata.base_byte_start as usize;
        let e_end = errata.base_byte_end as usize;
        if e_end <= cursor {
            continue;
        }
        if e_start > cursor {
            let eff_start = effective_source.len();
            effective_source.push_str(&raw_source[cursor..e_start]);
            let eff_end = effective_source.len();
            segments.push(EffectiveByteSegment {
                eff_start,
                eff_end,
                raw_start: cursor,
                raw_end: e_start,
                kind: EffectiveByteSegmentKind::Raw,
            });
        }
        let eff_start = effective_source.len();
        effective_source.push_str(&errata.corrected_text);
        // Preserve the physical block's trailing newline when the stored correction
        // omitted it, so adjacent Markdown blocks never collapse into one paragraph.
        if raw_source[e_start..e_end].ends_with('\n')
            && !errata.corrected_text.ends_with('\n')
            && e_end < raw_source.len()
        {
            if raw_source[e_start..e_end].ends_with("\r\n") {
                effective_source.push_str("\r\n");
            } else {
                effective_source.push('\n');
            }
        }
        let eff_end = effective_source.len();
        segments.push(EffectiveByteSegment {
            eff_start,
            eff_end,
            raw_start: e_start,
            raw_end: e_end,
            kind: EffectiveByteSegmentKind::Errata {
                errata_id: errata.errata_id.clone(),
            },
        });
        applied_errata_ids.push(errata.errata_id.clone());
        cursor = cursor.max(e_end);
    }

    if cursor < slice_end {
        let eff_start = effective_source.len();
        effective_source.push_str(&raw_source[cursor..slice_end]);
        let eff_end = effective_source.len();
        segments.push(EffectiveByteSegment {
            eff_start,
            eff_end,
            raw_start: cursor,
            raw_end: slice_end,
            kind: EffectiveByteSegmentKind::Raw,
        });
    }

    EffectiveMarkdownView {
        effective_source,
        byte_map: EffectiveByteMap { segments },
        applied_errata_ids,
    }
}

/// Projects the full immutable Markdown document through its active errata overlay.
pub fn build_effective_markdown(
    raw_source: &str,
    active_errata: &[ErrataOverlayEntry],
) -> EffectiveMarkdownView {
    render_effective_slice(raw_source, 0..raw_source.len(), active_errata)
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ErrataValidationError {
    #[error("快照内容哈希不匹配，请重新打开文件后再校对")]
    StaleSnapshot,
    #[error("所选块不存在或不属于当前快照")]
    UnknownBlock,
    #[error("终点块不能早于起点块")]
    ReversedRange,
    #[error("所选区间包含 PDF 分页标记，不能覆盖或删除分页标记")]
    ContainsPageMarker,
    #[error("校对后的正文不能包含或伪造 Marker 分页标记行")]
    CorrectedContainsPageMarker,
    #[error("校对后的正文不能为空")]
    EmptyCorrectedText,
    #[error("校对后的正文超过单次勘误上限（256 KiB）")]
    CorrectedTextTooLarge,
}

pub const MAX_ERRATA_CORRECTED_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedErrataTarget {
    pub first_block_id: String,
    pub last_block_id: String,
    pub covered_block_ids: Vec<String>,
    pub base_byte_start: u64,
    pub base_byte_end: u64,
    pub base_text_hash: ContentHash,
    pub original_text: String,
    pub effective_initial_text: String,
    pub overlapping_errata_ids: Vec<String>,
}

fn sha256_hash(bytes: &[u8]) -> ContentHash {
    ContentHash::parse(format!("sha256:{:x}", Sha256::digest(bytes)))
        .expect("SHA-256 produces valid content hash")
}

/// Validates a user's block-range selection for errata editing and expands it
/// to the minimal covering union of any overlapping active errata entries.
pub fn resolve_errata_target_range(
    doc: &AnchoredMarkdown,
    expected_content_hash: Option<&str>,
    first_block_id: &str,
    last_block_id: &str,
    active_errata: &[ErrataOverlayEntry],
) -> Result<ResolvedErrataTarget, ErrataValidationError> {
    if let Some(expected) = expected_content_hash
        && doc.snapshot.content_hash.as_str() != expected
    {
        return Err(ErrataValidationError::StaleSnapshot);
    }
    let first_idx = doc
        .blocks
        .iter()
        .position(|b| b.id == first_block_id)
        .ok_or(ErrataValidationError::UnknownBlock)?;
    let last_idx = doc
        .blocks
        .iter()
        .position(|b| b.id == last_block_id)
        .ok_or(ErrataValidationError::UnknownBlock)?;
    if first_idx > last_idx {
        return Err(ErrataValidationError::ReversedRange);
    }

    // Fixed-point expansion so the resolved block range strictly contains every
    // overlapping active errata (preserving disjoint active errata invariant).
    let mut start_idx = first_idx;
    let mut end_idx = last_idx;
    loop {
        let range_start = doc.blocks[start_idx].range.start;
        let range_end = doc.blocks[end_idx].range.end;
        let mut expanded = false;
        for errata in active_errata {
            if errata.base_byte_start < range_end && errata.base_byte_end > range_start {
                if let Some(e_first) = doc.blocks.iter().position(|b| b.id == errata.first_block_id)
                    && e_first < start_idx
                {
                    start_idx = e_first;
                    expanded = true;
                }
                if let Some(e_last) = doc.blocks.iter().position(|b| b.id == errata.last_block_id)
                    && e_last > end_idx
                {
                    end_idx = e_last;
                    expanded = true;
                }
            }
        }
        if !expanded {
            break;
        }
    }

    let blocks = &doc.blocks[start_idx..=end_idx];
    if blocks
        .iter()
        .any(|b| b.kind == PAGE_MARKER_STRUCTURAL_KIND)
    {
        return Err(ErrataValidationError::ContainsPageMarker);
    }

    let base_byte_start = blocks[0].range.start;
    let base_byte_end = blocks[blocks.len() - 1].range.end;
    let original_text = doc
        .source
        .get(base_byte_start as usize..base_byte_end as usize)
        .ok_or(ErrataValidationError::UnknownBlock)?
        .to_owned();
    let base_text_hash = sha256_hash(original_text.as_bytes());
    let effective_view = render_effective_slice(
        &doc.source,
        base_byte_start as usize..base_byte_end as usize,
        active_errata,
    );

    Ok(ResolvedErrataTarget {
        first_block_id: blocks[0].id.clone(),
        last_block_id: blocks[blocks.len() - 1].id.clone(),
        covered_block_ids: blocks.iter().map(|b| b.id.clone()).collect(),
        base_byte_start,
        base_byte_end,
        base_text_hash,
        original_text,
        effective_initial_text: effective_view.effective_source,
        overlapping_errata_ids: effective_view.applied_errata_ids,
    })
}

/// Normalizes user-submitted corrected Markdown text to preserve the physical
/// block's trailing newline structure if present.
pub fn normalize_errata_corrected_text(original_text: &str, corrected_text: &str) -> String {
    if original_text.ends_with("\r\n") && !corrected_text.ends_with('\n') {
        format!("{corrected_text}\r\n")
    } else if original_text.ends_with('\n') && !corrected_text.ends_with('\n') {
        format!("{corrected_text}\n")
    } else {
        corrected_text.to_owned()
    }
}

/// Validates user-submitted corrected Markdown text before persisting errata.
pub fn validate_errata_corrected_text(corrected_text: &str) -> Result<(), ErrataValidationError> {
    if corrected_text.trim().is_empty() {
        return Err(ErrataValidationError::EmptyCorrectedText);
    }
    if corrected_text.len() > MAX_ERRATA_CORRECTED_BYTES {
        return Err(ErrataValidationError::CorrectedTextTooLarge);
    }
    if corrected_text.lines().any(is_page_marker_text) {
        return Err(ErrataValidationError::CorrectedContainsPageMarker);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use mathscape_domain::StableId;

    use super::*;
    use crate::anchor_markdown;

    fn sample_doc(source: &str) -> AnchoredMarkdown {
        anchor_markdown(
            StableId::parse("mathscape:artifact/md-test").unwrap(),
            None,
            source.to_owned(),
        )
        .unwrap()
    }

    #[test]
    fn effective_projection_splices_blocks_and_maps_offsets_without_drift() {
        let raw = "Intro paragraph.\n\nTheorem 1: $x^a = 1$.\n\nCorollary 2: $y^2 = 4$.\n";
        let doc = sample_doc(raw);
        assert_eq!(doc.blocks.len(), 3);
        let b1 = &doc.blocks[1];
        let b2 = &doc.blocks[2];

        let errata = vec![ErrataOverlayEntry {
            errata_id: "e1".into(),
            first_block_id: b1.id.clone(),
            last_block_id: b1.id.clone(),
            covered_block_ids: vec![b1.id.clone()],
            base_byte_start: b1.range.start,
            base_byte_end: b1.range.end,
            corrected_text: "Theorem 1 (Corrected): $x^n + 1 = 0$.".into(),
        }];

        let view = build_effective_markdown(raw, &errata);
        assert_eq!(
            view.effective_source,
            "Intro paragraph.\n\nTheorem 1 (Corrected): $x^n + 1 = 0$.\n\nCorollary 2: $y^2 = 4$.\n"
        );
        assert_eq!(view.applied_errata_ids, vec!["e1"]);

        // Match inside the errata snaps cleanly to physical b1 range.
        let hit_in_errata = view.effective_source.find("$x^n + 1 = 0$").unwrap();
        let mapped_errata = view
            .byte_map
            .effective_range_to_physical(hit_in_errata, hit_in_errata + "$x^n + 1 = 0$".len())
            .unwrap();
        assert_eq!(mapped_errata, b1.range);

        // Match AFTER the errata translates back to exact physical bytes of b2 (zero drift!).
        let hit_after = view.effective_source.find("$y^2 = 4$").unwrap();
        let mapped_after = view
            .byte_map
            .effective_range_to_physical(hit_after, hit_after + "$y^2 = 4$".len())
            .unwrap();
        let raw_y = raw.find("$y^2 = 4$").unwrap() as u64;
        assert_eq!(
            mapped_after,
            TextByteRange {
                start: raw_y,
                end: raw_y + "$y^2 = 4$".len() as u64,
            }
        );
        assert!(mapped_after.start >= b2.range.start && mapped_after.end <= b2.range.end);
    }

    #[test]
    fn resolve_target_rejects_page_marker_and_expands_overlapping_errata_union() {
        let raw = "Block A\n\nBlock B\n\nBlock C\n\n{1}------------------------------------------------\n\nBlock D\n";
        let doc = sample_doc(raw);
        assert_eq!(doc.blocks.len(), 5);
        assert_eq!(doc.blocks[3].kind, PAGE_MARKER_STRUCTURAL_KIND);

        // Selecting across a page marker is rejected.
        let marker_err = resolve_errata_target_range(
            &doc,
            Some(doc.snapshot.content_hash.as_str()),
            &doc.blocks[2].id,
            &doc.blocks[4].id,
            &[],
        );
        assert_eq!(marker_err, Err(ErrataValidationError::ContainsPageMarker));

        // Existing errata covers blocks [0..=1]; selecting [1..=2] expands to union [0..=2].
        let existing = vec![ErrataOverlayEntry {
            errata_id: "e-ab".into(),
            first_block_id: doc.blocks[0].id.clone(),
            last_block_id: doc.blocks[1].id.clone(),
            covered_block_ids: vec![doc.blocks[0].id.clone(), doc.blocks[1].id.clone()],
            base_byte_start: doc.blocks[0].range.start,
            base_byte_end: doc.blocks[1].range.end,
            corrected_text: "Merged AB".into(),
        }];
        let resolved = resolve_errata_target_range(
            &doc,
            Some(doc.snapshot.content_hash.as_str()),
            &doc.blocks[1].id,
            &doc.blocks[2].id,
            &existing,
        )
        .unwrap();
        assert_eq!(resolved.first_block_id, doc.blocks[0].id);
        assert_eq!(resolved.last_block_id, doc.blocks[2].id);
        assert_eq!(resolved.covered_block_ids.len(), 3);
        assert_eq!(resolved.overlapping_errata_ids, vec!["e-ab"]);
        assert_eq!(resolved.effective_initial_text, "Merged AB\n\nBlock C\n");
    }

    #[test]
    fn validate_corrected_text_rejects_empty_and_forged_page_markers() {
        assert_eq!(
            validate_errata_corrected_text("   \n  "),
            Err(ErrataValidationError::EmptyCorrectedText)
        );
        assert_eq!(
            validate_errata_corrected_text("ok\n{2}--------------------------------\nmore"),
            Err(ErrataValidationError::CorrectedContainsPageMarker)
        );
        assert!(validate_errata_corrected_text("$$\\sum_{n=0}^\\infty a_n x^n$$").is_ok());
    }
}
