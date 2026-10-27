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

use mathscape_agent::{ToolError, ToolErrorCode, ToolStatus};
use mathscape_domain::StableId;
use mathscape_pdf::{PdfEngine, PdfOpenOptions};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::material::{self, PDF_MEDIA_TYPE, find_material, managed_path_failure};
use super::{PDF_OUTLINE, diagnostic, parse_args, to_object};
use crate::McpWorkspace;

const DESCRIPTION: &str = "Read the embedded table of contents (bookmarks) of one PDF material. Input is artifact_id. persist_index must be omitted or false: the outline is returned only, never written to the workspace or catalog. Each entry has title, level (1 = top-level), and pdf_page_index (0-based). To read that page with material.read, use locator pdf_pages {from, to} where from = pdf_page_index + 1 (1-based, inclusive). A PDF with no bookmarks returns status ok and entries []. Read-only.";

const SOURCE_EMBEDDED: &str = "embedded_toc";

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
}

#[derive(Debug, Serialize)]
struct OutlineResult {
    status: ToolStatus,
    artifact_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<&'static str>,
    entries: Vec<OutlineEntry>,
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
    if outline.entries().is_empty() && outline.warnings().is_empty() {
        diagnostics.push(diagnostic(
            "outline_empty",
            "PDF has no embedded bookmarks; pdf.outline does not invent a heuristic table of contents",
        ));
    }
    for warning in outline.warnings() {
        diagnostics.push(diagnostic("outline_warning", warning.clone()));
    }

    result(OutlineResult {
        status: ToolStatus::Ok,
        artifact_id: material.id.to_string(),
        source: Some(SOURCE_EMBEDDED),
        entries: outline
            .entries()
            .iter()
            .map(|entry| OutlineEntry {
                title: entry.title().to_owned(),
                level: entry.level(),
                pdf_page_index: entry.page().get(),
            })
            .collect(),
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
        total_pages: None,
        revision: None,
        details: Some(json!({ "reason": reason })),
        diagnostics: vec![diagnostic(reason, message)],
    })
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
