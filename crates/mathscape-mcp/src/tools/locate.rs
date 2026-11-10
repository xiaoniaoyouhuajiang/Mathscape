//! `material.locate`: turn a passage of an immutable Markdown snapshot into
//! the exact `md_byte_range` a `propose_changes` SourceSpan needs.
//!
//! Matching is the `propose_changes` R2 rule run as a search: exact
//! occurrences first; only when there is none, occurrences equal after
//! TeX-aware normalization of both sides, mapped back to raw bytes. Every
//! returned range satisfies R2 for the very `quote` passed in, so citing
//! `{artifact_id, md_byte_range, quote}` unchanged always passes the quote
//! check. Block ids follow the same rule as the proposal's
//! `marker_block_ids`; a page is given only when it maps to exactly one PDF
//! page. Read-only: it never writes the map, the catalog or the pool.

use mathscape_agent::{ToolError, ToolErrorCode, ToolStatus};
use mathscape_domain::StableId;
use mathscape_orchestrator::locate_quote;
use serde::Deserialize;
use serde_json::{Value, json};

use super::MATERIAL_LOCATE;
use super::propose::{ByteRangeInput, MAX_SPAN_BYTES, load_snapshot, map_page};
use crate::McpWorkspace;

/// At most this many candidates are returned for an ambiguous quote.
pub const MAX_LOCATE_CANDIDATES: usize = 20;

const DESCRIPTION: &str = "Find the exact md_byte_range of a passage inside one immutable Markdown snapshot, for citing it as a propose_changes source_span. artifact_id is the snapshot (served_artifact_id or details.served_segments[].artifact_id of material.read); quote is the passage's own text, copied from material.read (at most 4096 bytes: one problem or paragraph, never a whole chapter). Optional within {start,end} (bytes, half-open) restricts the search, e.g. to one served segment. Matching is the propose_changes quote rule: exact text first, otherwise equal after whitespace/TeX-spacing normalization of both sides, mapped back to raw bytes. One match: status ok with md_byte_range, block_ids and, when it maps to exactly one PDF page, page; several: status ambiguous with every candidate (at most 20, narrow with within or a longer quote); none: not_found. Pass the returned md_byte_range together with the same quote to propose_changes and the quote check always passes. Read-only.";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocateArgs {
    artifact_id: String,
    quote: String,
    #[serde(default)]
    within: Option<ByteRangeInput>,
}

pub(super) fn definition() -> Value {
    json!({
        "name": MATERIAL_LOCATE,
        "description": DESCRIPTION,
        "inputSchema": {
            "type": "object",
            "properties": {
                "artifact_id": { "type": "string", "minLength": 1 },
                "quote": { "type": "string", "minLength": 1, "maxLength": MAX_SPAN_BYTES },
                "within": {
                    "type": "object",
                    "properties": {
                        "start": { "type": "integer", "minimum": 0 },
                        "end": { "type": "integer", "minimum": 0 }
                    },
                    "required": ["start", "end"],
                    "additionalProperties": false
                }
            },
            "required": ["artifact_id", "quote"],
            "additionalProperties": false
        }
    })
}

pub(super) fn call(workspace: &McpWorkspace, arguments: Value) -> Result<Value, ToolError> {
    let args: LocateArgs = super::parse_args(arguments)?;
    if args.quote.trim().is_empty() {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            "quote must contain non-whitespace text",
        ));
    }
    if args.quote.len() as u64 > MAX_SPAN_BYTES {
        return Ok(json!({
            "status": ToolStatus::NotFound,
            "artifact_id": args.artifact_id,
            "candidates": [],
            "diagnostics": [{
                "code": "span_too_long",
                "message": format!(
                    "quote has {} bytes, more than the {MAX_SPAN_BYTES}-byte SourceSpan limit; split the passage into one span per problem or paragraph and locate each piece",
                    args.quote.len()
                )
            }]
        }));
    }
    let not_found = |code: &str, message: String| {
        json!({
            "status": ToolStatus::NotFound,
            "artifact_id": args.artifact_id,
            "candidates": [],
            "diagnostics": [{ "code": code, "message": message }]
        })
    };
    let artifact = match StableId::parse(args.artifact_id.clone()) {
        Ok(id) => id,
        Err(error) => return Ok(not_found("snapshot_not_found", error.to_string())),
    };
    let store = workspace.store()?;
    let context = match load_snapshot(workspace, &store, &artifact) {
        Ok(context) => context,
        Err(diagnostic) => {
            return Ok(not_found(
                &diagnostic.code,
                format!(
                    "{}; material.locate needs a text/markdown snapshot id (served_artifact_id or served_segments[].artifact_id)",
                    diagnostic.message
                ),
            ));
        }
    };
    let source = context.source.as_str();
    let (offset, haystack) = match &args.within {
        None => (0, source),
        Some(within) => {
            let (start, end) = (within.start as usize, within.end as usize);
            if start > end
                || end > source.len()
                || !source.is_char_boundary(start)
                || !source.is_char_boundary(end)
            {
                return Err(ToolError::new(
                    ToolErrorCode::InvalidArg,
                    format!(
                        "within must be a range inside the snapshot ({} bytes) on UTF-8 boundaries",
                        source.len()
                    ),
                ));
            }
            (start, &source[start..end])
        }
    };
    let (ranges, normalized) = locate_quote(haystack, &args.quote);
    let total = ranges.len();
    let candidates = ranges
        .into_iter()
        .take(MAX_LOCATE_CANDIDATES)
        .map(|(start, end)| {
            let (start, end) = ((start + offset) as u64, (end + offset) as u64);
            let page = map_page(&context, &ByteRangeInput { start, end })
                .ok()
                .map(|page| {
                    json!({
                        "pdf_page_index": page.pdf_page_index(),
                        "page_number": page.pdf_page_index() + 1
                    })
                });
            json!({
                "md_byte_range": { "start": start, "end": end },
                "block_ids": context.block_ids(start, end),
                "page": page,
                "page_granularity_available": page.is_some(),
            })
        })
        .collect::<Vec<_>>();
    let mut diagnostics = Vec::new();
    if normalized && total > 0 {
        diagnostics.push(json!({
            "code": "matched_normalized",
            "message": "no exact occurrence; matched after whitespace/TeX-spacing normalization. Cite the range with this same quote"
        }));
    }
    if total > MAX_LOCATE_CANDIDATES {
        diagnostics.push(json!({
            "code": "candidates_truncated",
            "message": format!("{total} occurrences, only the first {MAX_LOCATE_CANDIDATES} are listed; use a longer quote or within")
        }));
    }
    let status = match total {
        0 => ToolStatus::NotFound,
        1 => ToolStatus::Ok,
        _ => ToolStatus::Ambiguous,
    };
    let mut result = json!({
        "status": status,
        "artifact_id": artifact.as_str(),
        "match": if total == 0 { Value::Null } else if normalized { json!("normalized") } else { json!("exact") },
        "candidate_count": total,
        "truncated": total > MAX_LOCATE_CANDIDATES,
        "candidates": candidates.clone(),
        "diagnostics": diagnostics,
    });
    if total == 1
        && let (Some(object), Some(only)) = (result.as_object_mut(), candidates.first())
    {
        for key in [
            "md_byte_range",
            "block_ids",
            "page",
            "page_granularity_available",
        ] {
            object.insert(key.into(), only[key].clone());
        }
    }
    Ok(result)
}
