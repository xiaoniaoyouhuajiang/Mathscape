//! `pdf.outline`: the embedded table of contents of one PDF material.
//!
//! The outline is returned in the tool result only. Nothing is written to
//! the workspace or the catalog, and there is no document index to cite
//! (`document_index_id` is omitted, not `null`).
//!
//! `pdf_page_index` is 0-based. `material.read`'s `pdf_pages` locator stays
//! 1-based and inclusive, so the page of an entry is
//! `pdf_pages.from = pdf_page_index + 1`.
//!
//! A PDF with no bookmarks is `status: ok` and `entries: []` with an
//! `outline_empty` diagnostic. That is not `not_found` (the artifact is
//! there) and not `capability_unavailable` (no external tool is required to
//! read an embedded outline; this tool does not invent a heuristic TOC).
//!
//! A bookmark graph that is cyclic, over the reader limits, or otherwise
//! corrupt is also `status: ok` and `entries: []`, with `outline_unreadable`.
//! Failing to open the file stays a call error (`internal`), the same split
//! `material.read` uses for a missing text layer versus an unreadable file.
//!
//! The response is capped so a hostile PDF cannot flood the agent context or
//! the stdio pipe:
//! - each title is cut to [`MAX_OUTLINE_TITLE_CHARS`] Unicode scalar values
//!   (on a char boundary; the entry gets `title_truncated: true`);
//! - at most [`MAX_OUTLINE_ENTRIES`] entries are returned;
//! - the serialized `entries` array stays under [`MAX_OUTLINE_ENTRIES_BYTES`];
//! - at most [`MAX_OUTLINE_DIAGNOSTICS`] reader warnings are passed through,
//!   each cut to [`MAX_DIAGNOSTIC_MESSAGE_CHARS`].
//!
//! Any cut makes the result `status: "truncated"` with `truncated: true`
//! (contract §2.2 requires both); the entries that fit are kept in order, and
//! `total_entries` reports how many the reader produced. Successful results
//! always carry `truncated` (`false` when nothing was cut).

use mathscape_agent::{ToolError, ToolErrorCode, ToolStatus};
use mathscape_domain::StableId;
use mathscape_pdf::{PdfEngine, PdfOpenOptions};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::material::{self, PDF_MEDIA_TYPE, find_material, managed_path_failure};
use super::{PDF_OUTLINE, diagnostic, parse_args, to_object};
use crate::McpWorkspace;

const DESCRIPTION: &str = "Read the embedded table of contents (bookmarks) of one PDF material. Input is artifact_id. persist_index must be omitted or false: the outline is returned only, never written to the workspace or catalog. Each entry has title, level (1 = top-level), and pdf_page_index (0-based). To read that page with material.read, use locator pdf_pages {from, to} where from = pdf_page_index + 1 (1-based, inclusive). A PDF with no bookmarks returns status ok and entries []. A cyclic, oversized, or corrupt bookmark tree also returns status ok and entries [] with diagnostic outline_unreadable; a file that cannot be opened is an internal error. Duplicate titles collapse to one entry (first position, last page and level). Graphs deeper than 64 or with more than 10000 nodes are unreadable, not truncated. Output is capped: each title at 512 characters (entry gets title_truncated true), at most 1000 entries, and at most 64 KiB of serialized entries; on any cut the result is status truncated with truncated true, keeps the entries that fit in order, and reports total_entries. Read-only.";

const SOURCE_EMBEDDED: &str = "embedded_toc";

/// Longest title returned, in Unicode scalar values (chars, not bytes).
pub const MAX_OUTLINE_TITLE_CHARS: usize = 512;
/// Most entries returned in one result.
pub const MAX_OUTLINE_ENTRIES: usize = 1000;
/// Budget for the serialized JSON `entries` array, in bytes.
pub const MAX_OUTLINE_ENTRIES_BYTES: usize = 64 * 1024;
/// Most reader warnings passed through as diagnostics.
pub const MAX_OUTLINE_DIAGNOSTICS: usize = 16;
/// Longest diagnostic message, in chars.
pub const MAX_DIAGNOSTIC_MESSAGE_CHARS: usize = 256;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutlineArgs {
    artifact_id: String,
    /// Draft field. Only `false` (the default) is accepted; this slice never
    /// persists an outline.
    #[serde(default)]
    persist_index: bool,
}

#[derive(Debug, Serialize)]
struct OutlineEntry {
    title: String,
    level: u32,
    pdf_page_index: u32,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    title_truncated: bool,
}

#[derive(Debug, Serialize)]
struct OutlineResult {
    status: ToolStatus,
    artifact_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<&'static str>,
    entries: Vec<OutlineEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_entries: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_pages: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    diagnostics: Vec<Value>,
}

pub(super) fn definition() -> Value {
    json!({
        "name": PDF_OUTLINE,
        "description": DESCRIPTION,
        "inputSchema": {
            "type": "object",
            "properties": {
                "artifact_id": {
                    "type": "string",
                    "description": "Stable id of a PDF material (application/pdf).",
                },
                "persist_index": {
                    "type": "boolean",
                    "default": false,
                    "description": "Must be false or omitted. pdf.outline never writes a document index.",
                },
            },
            "required": ["artifact_id"],
            "additionalProperties": false,
        },
    })
}

pub(super) fn call(workspace: &McpWorkspace, arguments: Value) -> Result<Value, ToolError> {
    let args: OutlineArgs = parse_args(arguments)?;
    if args.persist_index {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            "persist_index must be false or omitted; pdf.outline does not write the outline into the workspace",
        ));
    }
    let id = StableId::parse(args.artifact_id.as_str()).map_err(|error| {
        ToolError::new(
            ToolErrorCode::InvalidArg,
            format!("artifact_id is not a valid stable id: {error}"),
        )
        .with_details(json!({ "artifact_id": args.artifact_id }))
    })?;

    let Some(material) = find_material(workspace, &id)? else {
        return not_found(&args, "artifact_not_found", "no artifact with this id");
    };
    if material.media_type != PDF_MEDIA_TYPE {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            format!(
                "pdf.outline requires media type {PDF_MEDIA_TYPE}, found {}",
                material.media_type
            ),
        )
        .with_details(json!({
            "media_type": material.media_type,
            "supported_media_types": [PDF_MEDIA_TYPE],
        })));
    }

    let pool = workspace.pool()?;
    let path = match material::open_managed_path(&pool, &material.id) {
        Ok(path) => path,
        Err(error) => return map_managed_path_error(&args, error),
    };
    let outline = PdfEngine
        .outline(&path, &PdfOpenOptions::new())
        .map_err(|error| {
            ToolError::new(
                ToolErrorCode::Internal,
                format!("failed to read PDF outline: {error}"),
            )
        })?;

    let mut diagnostics = Vec::new();
    let mut warnings_truncated = false;
    if outline.is_unreadable() {
        // Content and parse failures from the outline reader, including a
        // panic inside lopdf, stay a successful tool result. Opening the file
        // failed earlier and is still `internal`.
        if outline.warnings().is_empty() {
            diagnostics.push(diagnostic(
                "outline_unreadable",
                "embedded outline could not be read",
            ));
        } else {
            warnings_truncated =
                push_warnings(&mut diagnostics, "outline_unreadable", outline.warnings());
        }
    } else if outline.entries().is_empty() && outline.warnings().is_empty() {
        diagnostics.push(diagnostic(
            "outline_empty",
            "PDF has no embedded bookmarks; pdf.outline does not invent a heuristic table of contents",
        ));
    } else {
        warnings_truncated = push_warnings(&mut diagnostics, "outline_warning", outline.warnings());
    }

    let total_entries = outline.entries().len();
    let capped = cap_entries(
        outline
            .entries()
            .iter()
            .map(|entry| (entry.title(), entry.level(), entry.page().get())),
    )?;
    let truncated = warnings_truncated || capped.truncated;
    if capped.truncated {
        diagnostics.push(diagnostic(
            "outline_truncated",
            format!(
                "returned {} of {total_entries} entries ({} titles shortened); caps: {MAX_OUTLINE_TITLE_CHARS} chars per title, {MAX_OUTLINE_ENTRIES} entries, {MAX_OUTLINE_ENTRIES_BYTES} bytes of entries",
                capped.entries.len(),
                capped.titles_shortened,
            ),
        ));
    }

    result(OutlineResult {
        status: if truncated {
            ToolStatus::Truncated
        } else {
            ToolStatus::Ok
        },
        artifact_id: material.id.to_string(),
        source: Some(SOURCE_EMBEDDED),
        entries: capped.entries,
        truncated: Some(truncated),
        total_entries: Some(total_entries),
        total_pages: Some(outline.page_count()),
        revision: Some(material.revision.clone()),
        details: None,
        diagnostics,
    })
}

fn not_found(args: &OutlineArgs, reason: &str, message: &str) -> Result<Value, ToolError> {
    result(OutlineResult {
        status: ToolStatus::NotFound,
        artifact_id: args.artifact_id.clone(),
        source: None,
        entries: Vec::new(),
        truncated: None,
        total_entries: None,
        total_pages: None,
        revision: None,
        details: Some(json!({ "reason": reason })),
        diagnostics: vec![diagnostic(reason, message)],
    })
}

struct CappedEntries {
    entries: Vec<OutlineEntry>,
    titles_shortened: usize,
    truncated: bool,
}

/// Cuts `title` to at most `max_chars` chars on a char boundary.
fn cut_chars(title: &str, max_chars: usize) -> (&str, bool) {
    match title.char_indices().nth(max_chars) {
        Some((cut, _)) => (&title[..cut], true),
        None => (title, false),
    }
}

/// Applies the title, count, and byte caps, keeping entries in order.
fn cap_entries<'a>(
    source: impl Iterator<Item = (&'a str, u32, u32)>,
) -> Result<CappedEntries, ToolError> {
    let mut capped = CappedEntries {
        entries: Vec::new(),
        titles_shortened: 0,
        truncated: false,
    };
    // `[` and `]`; each entry after the first adds a `,`.
    let mut bytes = 2usize;
    for (title, level, pdf_page_index) in source {
        if capped.entries.len() >= MAX_OUTLINE_ENTRIES {
            capped.truncated = true;
            break;
        }
        let (title, title_truncated) = cut_chars(title, MAX_OUTLINE_TITLE_CHARS);
        let entry = OutlineEntry {
            title: title.to_owned(),
            level,
            pdf_page_index,
            title_truncated,
        };
        let size = serde_json::to_vec(&entry)
            .map_err(|error| {
                ToolError::new(
                    ToolErrorCode::Internal,
                    format!("failed to serialize outline entry: {error}"),
                )
            })?
            .len()
            + usize::from(!capped.entries.is_empty());
        if bytes + size > MAX_OUTLINE_ENTRIES_BYTES {
            capped.truncated = true;
            break;
        }
        bytes += size;
        if title_truncated {
            capped.titles_shortened += 1;
            capped.truncated = true;
        }
        capped.entries.push(entry);
    }
    Ok(capped)
}

/// Pushes at most [`MAX_OUTLINE_DIAGNOSTICS`] warnings, each cut to
/// [`MAX_DIAGNOSTIC_MESSAGE_CHARS`]. Returns whether anything was cut.
fn push_warnings(diagnostics: &mut Vec<Value>, code: &str, warnings: &[String]) -> bool {
    let mut truncated = warnings.len() > MAX_OUTLINE_DIAGNOSTICS;
    for warning in warnings.iter().take(MAX_OUTLINE_DIAGNOSTICS) {
        let (message, cut) = cut_chars(warning, MAX_DIAGNOSTIC_MESSAGE_CHARS);
        truncated |= cut;
        let message = if cut {
            format!("{message}…")
        } else {
            message.to_owned()
        };
        diagnostics.push(diagnostic(code, message));
    }
    if warnings.len() > MAX_OUTLINE_DIAGNOSTICS {
        diagnostics.push(diagnostic(
            "outline_truncated",
            format!(
                "{} more outline warnings omitted",
                warnings.len() - MAX_OUTLINE_DIAGNOSTICS
            ),
        ));
    }
    truncated
}

fn result(result: OutlineResult) -> Result<Value, ToolError> {
    to_object(result).map(Value::Object)
}

fn map_managed_path_error(
    args: &OutlineArgs,
    error: mathscape_artifact::ArtifactPoolError,
) -> Result<Value, ToolError> {
    match managed_path_failure(&error) {
        material::ManagedPathFailure::NotFound => not_found(
            args,
            "content_unavailable",
            "the artifact is registered but has no readable managed copy",
        ),
        material::ManagedPathFailure::Forbidden => Err(ToolError::new(
            ToolErrorCode::Forbidden,
            "managed artifact path is unsafe or is a symbolic link",
        )),
        material::ManagedPathFailure::Internal => Err(ToolError::new(
            ToolErrorCode::Internal,
            format!("failed to resolve managed artifact: {error}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cut_chars_stops_on_a_char_boundary() {
        assert_eq!(cut_chars("αβγ", 2), ("αβ", true));
        assert_eq!(cut_chars("αβγ", 3), ("αβγ", false));
        assert_eq!(cut_chars("", 0), ("", false));
    }

    #[test]
    fn warnings_are_capped_in_count_and_length() {
        let warnings: Vec<String> = (0..MAX_OUTLINE_DIAGNOSTICS + 5)
            .map(|index| format!("w{index} {}", "x".repeat(10_000)))
            .collect();
        let mut diagnostics = Vec::new();
        assert!(push_warnings(
            &mut diagnostics,
            "outline_warning",
            &warnings
        ));
        assert_eq!(diagnostics.len(), MAX_OUTLINE_DIAGNOSTICS + 1);
        for item in &diagnostics[..MAX_OUTLINE_DIAGNOSTICS] {
            assert_eq!(item["code"], "outline_warning");
            let chars = item["message"].as_str().unwrap().chars().count();
            assert_eq!(chars, MAX_DIAGNOSTIC_MESSAGE_CHARS + 1);
        }
        assert_eq!(
            diagnostics[MAX_OUTLINE_DIAGNOSTICS]["code"],
            "outline_truncated"
        );

        let mut few = Vec::new();
        assert!(!push_warnings(
            &mut few,
            "outline_warning",
            &["short".to_owned()]
        ));
        assert_eq!(few, vec![diagnostic("outline_warning", "short")]);
    }

    #[test]
    fn byte_budget_keeps_a_prefix_and_marks_truncated() {
        let titles: Vec<String> = (0..MAX_OUTLINE_ENTRIES)
            .map(|index| format!("{index:04}{}", "\u{1}".repeat(600)))
            .collect();
        let capped = cap_entries(titles.iter().map(|title| (title.as_str(), 1, 0))).unwrap();
        assert!(capped.truncated);
        assert!(capped.entries.len() < MAX_OUTLINE_ENTRIES);
        let bytes = serde_json::to_vec(&capped.entries).unwrap().len();
        assert!(bytes <= MAX_OUTLINE_ENTRIES_BYTES, "{bytes}");
        assert!(capped.entries[0].title.starts_with("0000"));
        assert_eq!(
            capped.entries[0].title.chars().count(),
            MAX_OUTLINE_TITLE_CHARS
        );
    }
}
