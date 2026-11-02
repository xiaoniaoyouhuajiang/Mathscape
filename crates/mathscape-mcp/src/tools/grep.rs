//! `material.grep`: linear scan of markdown bodies.
//!
//! This slice searches only `text/markdown`, and only the artifacts named in
//! a non-empty `artifact_ids` list (no workspace-wide grep). `pattern` is a
//! case-sensitive literal substring, not a regular expression; `terms` is
//! not accepted. Matches do not overlap: the next search starts at the end
//! of the previous match.
//!
//! Each hit's [`Locator`] is an `md_byte_range` covering that match, so a
//! follow-up `material.read` quotes it. `snippet` is a verbatim window of
//! the surrounding line (capped), not a second locator. `score` is always
//! [`LITERAL_SCORE`] — this scan does not rank.
//!
//! A missing id is business `not_found` with no partial hits. Non-markdown
//! media is `invalid_arg` (PDF text and OCR are out of scope). More matches
//! than `max_hits`, or a file longer than [`MAX_SCAN_BYTES`], is business
//! `truncated`.

use std::collections::HashSet;

use mathscape_agent::{Locator, ToolError, ToolErrorCode, ToolStatus};
use mathscape_artifact::{ArtifactPool, ArtifactPoolError};
use mathscape_domain::{MarkdownCoverage, StableId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::canonical::CanonicalIndex;
use super::material::{
    self, MARKDOWN_MEDIA_TYPE, ManagedPathFailure, Material, find_materials, managed_path_failure,
};
use super::{MATERIAL_GREP, diagnostic, parse_args, to_object};
use crate::McpWorkspace;

/// Default and hard maximum number of hits.
pub const MAX_GREP_HITS: usize = 50;
/// Bytes of each markdown file that this slice will scan.
pub const MAX_SCAN_BYTES: u64 = 1024 * 1024;
/// `pattern` longer than this (in Unicode scalar values) is `invalid_arg`.
pub const MAX_PATTERN_CHARS: usize = 200;
/// Maximum ids in one call.
pub const MAX_ARTIFACT_IDS: usize = 32;
/// Snippet window, in UTF-8 bytes. The match itself may be shorter.
pub const SNIPPET_MAX_BYTES: usize = 512;

const LITERAL_SCORE: u32 = 1;

const DESCRIPTION: &str = "Search the markdown body of specific materials for a case-sensitive literal substring (not a regular expression). artifact_ids is required and non-empty; this tool does not scan the whole workspace. Only text/markdown is searched (PDF text and OCR are out of scope). A canonical markdown id is resolved to its head snapshot. Hits on a canonical snapshot include canonical_id, coverage and partial. Each hit locator is an md_byte_range of that match and can be passed to material.read. A missing artifact_id yields status not_found and no partial hits. Stops at max_hits (default and maximum 50) or after the first 1 MiB of a file; either limit returns status truncated and truncated true. This tool never runs Marker. Read-only.";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GrepArgs {
    artifact_ids: Vec<String>,
    pattern: String,
    #[serde(default)]
    max_hits: Option<usize>,
}

#[derive(Debug, Serialize)]
struct GrepResult {
    status: ToolStatus,
    hits: Vec<GrepHit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    diagnostics: Vec<Value>,
}

#[derive(Debug, Serialize)]
struct GrepHit {
    artifact_id: String,
    locator: Locator,
    snippet: String,
    score: u32,
    revision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    canonical_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    coverage: Option<MarkdownCoverage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    partial: Option<bool>,
}

struct RawHit {
    start: u64,
    end: u64,
    snippet: String,
}

pub(super) fn definition() -> Value {
    json!({
        "name": MATERIAL_GREP,
        "description": DESCRIPTION,
        "inputSchema": {
            "type": "object",
            "properties": {
                "artifact_ids": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MAX_ARTIFACT_IDS,
                    "items": { "type": "string" },
                    "description": "Stable ids to scan, in order. Required and non-empty. Duplicates are rejected. Every id must be text/markdown.",
                },
                "pattern": {
                    "type": "string",
                    "minLength": 1,
                    "description": format!(
                        "Case-sensitive literal substring, not a regular expression. At most {MAX_PATTERN_CHARS} characters."
                    ),
                },
                "max_hits": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_GREP_HITS,
                    "default": MAX_GREP_HITS,
                    "description": format!(
                        "Maximum matches to return (1 to {MAX_GREP_HITS}). Defaults to {MAX_GREP_HITS}. Further matches yield status truncated."
                    ),
                },
            },
            "required": ["artifact_ids", "pattern"],
            "additionalProperties": false,
        },
    })
}

pub(super) fn call(workspace: &McpWorkspace, arguments: Value) -> Result<Value, ToolError> {
    let args: GrepArgs = parse_args(arguments)?;
    let max_hits = validated_max_hits(args.max_hits)?;
    validate_pattern(&args.pattern)?;
    let ids = parse_ids(&args.artifact_ids)?;
    let index = CanonicalIndex::load(workspace)?;
    let resolved: Vec<StableId> = ids
        .iter()
        .map(|id| index.markdown_target(id).clone())
        .collect();
    let materials = load_materials(workspace, &resolved)?;
    if let Some(missing) = materials.missing {
        return not_found("artifact_not_found", "no artifact with this id", missing);
    }
    let materials = materials.found;
    ensure_markdown(&materials)?;

    let pool = workspace.pool()?;
    let mut hits = Vec::new();
    let mut diagnostics = Vec::new();
    let mut hit_limited = false;
    let mut scan_capped = false;
    for material in &materials {
        let (text, capped) = match load_text(&pool, material)? {
            Loaded::Text { text, capped } => (text, capped),
            Loaded::Finished(value) => return Ok(value),
        };
        if capped {
            scan_capped = true;
            diagnostics.push(json!({
                "code": "scan_capped",
                "message": format!(
                    "scanned the first {} bytes of this markdown; the remainder was not searched",
                    text.len()
                ),
                "artifact_id": material.id.to_string(),
                "scanned_bytes": text.len(),
                "total_bytes": material.byte_size,
            }));
        }
        let room = max_hits - hits.len();
        let (found, limited) = scan_text(&text, &args.pattern, room);
        let attachment = index.for_markdown(material.id.as_str()).map(|view| {
            let coverage = view.coverage(None);
            (view.canonical_id.to_string(), coverage.partial, coverage)
        });
        for found in found {
            hits.push(GrepHit {
                artifact_id: material.id.to_string(),
                locator: Locator::MdByteRange {
                    start: found.start,
                    end: found.end,
                },
                snippet: found.snippet,
                score: LITERAL_SCORE,
                revision: material.revision.clone(),
                canonical_id: attachment.as_ref().map(|(id, _, _)| id.clone()),
                partial: attachment.as_ref().map(|(_, partial, _)| *partial),
                coverage: attachment.as_ref().map(|(_, _, coverage)| coverage.clone()),
            });
        }
        if limited {
            hit_limited = true;
            break;
        }
    }
    if hit_limited {
        diagnostics.push(diagnostic(
            "hit_limit",
            format!("stopped after {max_hits} matches; more matches may exist"),
        ));
    }
    let truncated = hit_limited || scan_capped;
    result(GrepResult {
        status: if truncated {
            ToolStatus::Truncated
        } else {
            ToolStatus::Ok
        },
        hits,
        truncated: Some(truncated),
        details: None,
        diagnostics,
    })
}

fn validated_max_hits(max_hits: Option<usize>) -> Result<usize, ToolError> {
    match max_hits {
        None => Ok(MAX_GREP_HITS),
        Some(value) if (1..=MAX_GREP_HITS).contains(&value) => Ok(value),
        Some(value) => Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            format!("max_hits must be from 1 to {MAX_GREP_HITS}, got {value}"),
        )),
    }
}

fn validate_pattern(pattern: &str) -> Result<(), ToolError> {
    if pattern.is_empty() {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            "pattern must be a non-empty literal substring",
        ));
    }
    let chars = pattern.chars().count();
    if chars > MAX_PATTERN_CHARS {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            format!("pattern is longer than {MAX_PATTERN_CHARS} characters"),
        )
        .with_details(json!({ "characters": chars, "maximum": MAX_PATTERN_CHARS })));
    }
    Ok(())
}

fn parse_ids(raw: &[String]) -> Result<Vec<StableId>, ToolError> {
    if raw.is_empty() {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            "artifact_ids must be a non-empty list; material.grep does not scan the whole workspace",
        ));
    }
    if raw.len() > MAX_ARTIFACT_IDS {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            format!("artifact_ids accepts at most {MAX_ARTIFACT_IDS} ids"),
        )
        .with_details(json!({ "count": raw.len(), "maximum": MAX_ARTIFACT_IDS })));
    }
    let mut ids = Vec::with_capacity(raw.len());
    let mut seen = HashSet::new();
    for (index, raw_id) in raw.iter().enumerate() {
        let id = StableId::parse(raw_id.as_str()).map_err(|error| {
            ToolError::new(
                ToolErrorCode::InvalidArg,
                format!("artifact_ids[{index}] is not a valid stable id: {error}"),
            )
            .with_details(json!({ "artifact_id": raw_id }))
        })?;
        if !seen.insert(id.clone()) {
            return Err(ToolError::new(
                ToolErrorCode::InvalidArg,
                "artifact_ids contains a duplicate",
            )
            .with_details(json!({ "artifact_id": id.to_string() })));
        }
        ids.push(id);
    }
    Ok(ids)
}

struct LoadedMaterials {
    found: Vec<Material>,
    missing: Option<Vec<String>>,
}

fn load_materials(
    workspace: &McpWorkspace,
    ids: &[StableId],
) -> Result<LoadedMaterials, ToolError> {
    let mut found = Vec::with_capacity(ids.len());
    let mut missing = Vec::new();
    for (id, material) in ids.iter().zip(find_materials(workspace, ids)?) {
        match material {
            Some(material) => found.push(material),
            None => missing.push(id.to_string()),
        }
    }
    Ok(LoadedMaterials {
        found,
        missing: if missing.is_empty() {
            None
        } else {
            Some(missing)
        },
    })
}

fn ensure_markdown(materials: &[Material]) -> Result<(), ToolError> {
    let unsupported: Vec<Value> = materials
        .iter()
        .filter(|material| material.media_type != MARKDOWN_MEDIA_TYPE)
        .map(|material| {
            json!({
                "artifact_id": material.id.to_string(),
                "media_type": material.media_type,
            })
        })
        .collect();
    if unsupported.is_empty() {
        return Ok(());
    }
    Err(ToolError::new(
        ToolErrorCode::InvalidArg,
        format!("material.grep only scans {MARKDOWN_MEDIA_TYPE} in this slice"),
    )
    .with_details(json!({
        "supported_media_types": [MARKDOWN_MEDIA_TYPE],
        "artifacts": unsupported,
    })))
}

/// Scanned text, or a finished business result (content missing).
enum Loaded {
    Text { text: String, capped: bool },
    Finished(Value),
}

fn load_text(pool: &ArtifactPool, material: &Material) -> Result<Loaded, ToolError> {
    // Same managed-path check and error classes as material.read / pdf.outline.
    if let Err(error) = material::open_managed_path(pool, &material.id) {
        return fail_load(material, error);
    }
    if material.byte_size == 0 {
        return Ok(Loaded::Text {
            text: String::new(),
            capped: false,
        });
    }
    let capped = material.byte_size > MAX_SCAN_BYTES;
    let length = u32::try_from(material.byte_size.min(MAX_SCAN_BYTES))
        .map_err(|_| ToolError::new(ToolErrorCode::Internal, "markdown scan window exceeds u32"))?;
    let bytes = match pool.read_bytes(&material.id, 0, length) {
        Ok(bytes) => bytes,
        Err(error) => return fail_load(material, error),
    };
    if bytes.len() as u64 != u64::from(length) {
        return Err(ToolError::new(
            ToolErrorCode::Internal,
            "managed artifact is shorter than its catalog byte size",
        ));
    }
    let text = decode_scanned(&bytes, capped)?;
    Ok(Loaded::Text { text, capped })
}

/// Business `not_found` (`Ok`) or a call error (`Err`), classified by the
/// shared [`managed_path_failure`].
fn fail_load(material: &Material, error: ArtifactPoolError) -> Result<Loaded, ToolError> {
    match managed_path_failure(&error) {
        ManagedPathFailure::NotFound => content_unavailable(material).map(Loaded::Finished),
        ManagedPathFailure::Forbidden => Err(ToolError::new(
            ToolErrorCode::Forbidden,
            "managed artifact path is unsafe or is a symbolic link",
        )
        .with_details(json!({ "artifact_id": material.id.to_string() }))),
        ManagedPathFailure::Internal => Err(ToolError::new(
            ToolErrorCode::Internal,
            format!("failed to read managed artifact: {error}"),
        )),
    }
}

fn content_unavailable(material: &Material) -> Result<Value, ToolError> {
    not_found(
        "content_unavailable",
        "the artifact is registered but has no readable managed copy",
        vec![material.id.to_string()],
    )
}

fn not_found(reason: &str, message: &str, artifact_ids: Vec<String>) -> Result<Value, ToolError> {
    let diagnostics = artifact_ids
        .iter()
        .map(|id| {
            json!({
                "code": reason,
                "message": message,
                "artifact_id": id,
            })
        })
        .collect();
    result(GrepResult {
        status: ToolStatus::NotFound,
        hits: Vec::new(),
        truncated: None,
        details: Some(json!({
            "reason": reason,
            "artifact_ids": artifact_ids,
        })),
        diagnostics,
    })
}

fn result(value: GrepResult) -> Result<Value, ToolError> {
    to_object(value).map(Value::Object)
}

/// Decodes a scanned prefix. When `capped` is set, an incomplete UTF-8
/// sequence at the cut is dropped; any other invalid sequence is an error.
fn decode_scanned(bytes: &[u8], capped: bool) -> Result<String, ToolError> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text.to_owned()),
        Err(error) if capped && error.error_len().is_none() => {
            std::str::from_utf8(&bytes[..error.valid_up_to()])
                .map(str::to_owned)
                .map_err(utf8_error)
        }
        Err(error) => Err(utf8_error(error)),
    }
}

fn utf8_error(error: std::str::Utf8Error) -> ToolError {
    ToolError::new(
        ToolErrorCode::Internal,
        format!("markdown material is not valid UTF-8: {error}"),
    )
}

/// Non-overlapping literal search. `limited` is true when another match
/// exists after the ones returned.
fn scan_text(text: &str, pattern: &str, limit: usize) -> (Vec<RawHit>, bool) {
    if pattern.is_empty() {
        return (Vec::new(), false);
    }
    let mut hits = Vec::new();
    let mut from = 0;
    while hits.len() < limit {
        let Some(relative) = text[from..].find(pattern) else {
            return (hits, false);
        };
        let start = from + relative;
        let end = start + pattern.len();
        let (snippet_start, snippet_end) = snippet_bounds(text, start, end);
        hits.push(RawHit {
            start: start as u64,
            end: end as u64,
            snippet: text[snippet_start..snippet_end].to_owned(),
        });
        from = end;
    }
    (hits, text[from..].contains(pattern))
}

/// Verbatim snippet window: the whole line when it fits, otherwise a
/// char-aligned window around the match that stays within [`SNIPPET_MAX_BYTES`].
fn snippet_bounds(text: &str, start: usize, end: usize) -> (usize, usize) {
    debug_assert!(start < end && end <= text.len());
    debug_assert!(text.is_char_boundary(start) && text.is_char_boundary(end));

    let line_start = match text[..start].rfind('\n') {
        Some(index) => index + 1,
        None => 0,
    };
    let line_end = match text[end..].find('\n') {
        Some(index) => end + index,
        None => text.len(),
    };
    if end - start <= SNIPPET_MAX_BYTES && line_end - line_start <= SNIPPET_MAX_BYTES {
        return (line_start, line_end);
    }
    if end - start > SNIPPET_MAX_BYTES {
        let mut snippet_end = start + SNIPPET_MAX_BYTES;
        while snippet_end > start && !text.is_char_boundary(snippet_end) {
            snippet_end -= 1;
        }
        if snippet_end == start {
            snippet_end = text[start..]
                .chars()
                .next()
                .map(|ch| start + ch.len_utf8())
                .unwrap_or(end);
        }
        return (start, snippet_end);
    }

    let slack = SNIPPET_MAX_BYTES - (end - start);
    let avail_before = start - line_start;
    let avail_after = line_end - end;
    let mut before = slack / 2;
    if before > avail_before {
        before = avail_before;
    }
    let mut after = slack - before;
    if after > avail_after {
        let extra = after - avail_after;
        after = avail_after;
        before += extra.min(avail_before - before);
    }
    let mut snippet_start = start - before;
    let mut snippet_end = end + after;
    while !text.is_char_boundary(snippet_start) {
        snippet_start += 1;
    }
    while !text.is_char_boundary(snippet_end) {
        snippet_end -= 1;
    }
    if snippet_start > start {
        snippet_start = start;
    }
    if snippet_end < end {
        snippet_end = end;
    }
    while snippet_end - snippet_start > SNIPPET_MAX_BYTES && snippet_end > end {
        let mut next = snippet_end - 1;
        while next > end && !text.is_char_boundary(next) {
            next -= 1;
        }
        if next == snippet_end {
            break;
        }
        snippet_end = next;
    }
    while snippet_end - snippet_start > SNIPPET_MAX_BYTES && snippet_start < start {
        let mut next = snippet_start + 1;
        while next < start && !text.is_char_boundary(next) {
            next += 1;
        }
        if next == snippet_start {
            break;
        }
        snippet_start = next;
    }
    debug_assert!(snippet_start <= start && snippet_end >= end);
    debug_assert!(text.is_char_boundary(snippet_start) && text.is_char_boundary(snippet_end));
    debug_assert!(snippet_end - snippet_start <= SNIPPET_MAX_BYTES);
    (snippet_start, snippet_end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_scan_is_case_sensitive_non_overlapping_and_not_a_regex() {
        let (hits, limited) = scan_text("aaaa", "aa", 10);
        assert!(!limited);
        assert_eq!(hits.len(), 2);
        assert_eq!((hits[0].start, hits[0].end), (0, 2));
        assert_eq!((hits[1].start, hits[1].end), (2, 4));

        let (hits, limited) = scan_text("aaaa", "aa", 1);
        assert!(limited);
        assert_eq!(hits.len(), 1);

        assert!(scan_text("Lemma", "lemma", 10).0.is_empty());
        assert!(scan_text("lemma", "l.mma", 10).0.is_empty());
        assert!(scan_text("aaa", "aa", 10).0.len() == 1);
    }

    #[test]
    fn multibyte_match_locator_is_the_exact_byte_range() {
        let text = "# 引理 Lemma\nnext\n";
        let (hits, limited) = scan_text(text, "引理", 10);
        assert!(!limited);
        assert_eq!(hits.len(), 1);
        let start = hits[0].start as usize;
        let end = hits[0].end as usize;
        assert_eq!(&text[start..end], "引理");
        assert_eq!(hits[0].snippet, "# 引理 Lemma");
        assert_eq!(hits[0].snippet.find("引理"), Some(2));
    }

    #[test]
    fn long_line_snippet_contains_the_match_and_stays_bounded() {
        let text = format!("{}NEEDLE{}", "α".repeat(400), "β".repeat(400));
        let start = text.find("NEEDLE").unwrap();
        let end = start + "NEEDLE".len();
        let (snippet_start, snippet_end) = snippet_bounds(&text, start, end);
        let snippet = &text[snippet_start..snippet_end];
        assert!(snippet.contains("NEEDLE"));
        assert!(snippet_start <= start && snippet_end >= end);
        assert!(snippet.len() <= SNIPPET_MAX_BYTES);
        assert!(text.is_char_boundary(snippet_start));
        assert!(text.is_char_boundary(snippet_end));

        let long = "a".repeat(600);
        let (snippet_start, snippet_end) = snippet_bounds(&long, 0, 600);
        assert_eq!(snippet_start, 0);
        assert!(snippet_end <= SNIPPET_MAX_BYTES);
        assert!(snippet_end > 0);
    }

    #[test]
    fn capped_prefix_drops_only_a_trailing_partial_character() {
        assert_eq!(decode_scanned(b"ab", false).unwrap(), "ab");
        assert_eq!(decode_scanned("α".as_bytes(), false).unwrap(), "α");
        assert!(decode_scanned(&[0xFF], false).is_err());
        assert!(decode_scanned(&[b'a', 0xFF], true).is_err());
        assert_eq!(decode_scanned(&[b'a', 0xE5, 0xBC], true).unwrap(), "a");
        assert_eq!(decode_scanned(&[0xCE], true).unwrap(), "");
    }
}
