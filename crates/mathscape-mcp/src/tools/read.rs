//! `material.read`: bounded reads of one registered material.
//!
//! - Markdown (`text/markdown`) by `md_byte_range` (bytes, half-open).
//!   `expand.before/after` are bytes. The returned range is widened to UTF-8
//!   character boundaries and echoed back as the actual locator.
//! - PDF (`application/pdf`) by `pdf_pages` (1-based, closed), using the
//!   text layer extracted by `mathscape-pdf`. Pages without a text layer are
//!   never OCRed here: when no requested page has text, the tool returns
//!   `capability_unavailable` with the Marker CapabilityProbe in `details`.
//!   Marker itself is not invoked. If a canonical markdown snapshot already
//!   covers the request, that immutable snapshot is returned instead and the
//!   result carries `canonical_id`, `coverage` and `partial`.
//!
//! Every successful read is capped at [`MAX_READ_BYTES`] of UTF-8 text;
//! a capped read returns `status: "truncated"` and `truncated: true`.

use std::path::PathBuf;

use mathscape_agent::{
    Capability, CapabilityState, Locator, ToolError, ToolErrorCode, ToolStatus, probe_capability,
};
use mathscape_artifact::{ArtifactPool, ArtifactPoolError};
use mathscape_domain::{CanonicalMarkdownView, InclusivePageRange, StableId};
use mathscape_pdf::{PdfEngine, PdfOpenOptions, PdfPageIndex, PdfSelection};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::canonical::CanonicalIndex;
use super::material::{
    self, MARKDOWN_MEDIA_TYPE, Material, PDF_MEDIA_TYPE, find_material, managed_path_failure,
};
use super::{MATERIAL_READ, diagnostic, parse_args, to_object};
use crate::McpWorkspace;

/// Hard cap for returned text, in UTF-8 bytes (contract §2.2: 32KiB).
pub const MAX_READ_BYTES: usize = 32 * 1024;

/// PDF pages are extracted in small batches so a huge `pdf_pages` range
/// stops as soon as the cap is reached.
const PDF_PAGE_BATCH: u32 = 8;
const PAGE_SEPARATOR: &str = "\n\n";

const DESCRIPTION: &str = "Read part of one workspace material by locator. Markdown: locator md_byte_range {start,end} in bytes, half-open; optional expand {before,after} in bytes. PDF: locator pdf_pages {from,to}, 1-based and inclusive, text layer only. Returns the actual locator read; text is capped at 32 KiB (status truncated, truncated true). When a canonical markdown view exists, the result includes canonical_id, coverage and partial (partial true means this request or the book is not fully covered). A scanned PDF with no text layer is served from an existing immutable snapshot when one covers the request; this tool never runs Marker. A canonical id may be passed as artifact_id with md_byte_range to read its head snapshot. Read-only.";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadArgs {
    artifact_id: String,
    locator: Locator,
    #[serde(default)]
    expand: Option<Expand>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Expand {
    #[serde(default)]
    before: u64,
    #[serde(default)]
    after: u64,
}

#[derive(Debug, Serialize)]
struct ReadResult {
    status: ToolStatus,
    artifact_id: String,
    content_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    locator: Locator,
    #[serde(skip_serializing_if = "Option::is_none")]
    truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_pages: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    page_spans: Option<Vec<PageSpan>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    diagnostics: Vec<Value>,
}

/// Where each returned PDF page lives inside `text` (byte offsets, half-open).
#[derive(Clone, Copy, Debug, Serialize)]
struct PageSpan {
    page: u32,
    start: usize,
    end: usize,
}

pub(super) fn definition() -> Value {
    json!({
        "name": MATERIAL_READ,
        "description": DESCRIPTION,
        "inputSchema": {
            "type": "object",
            "properties": {
                "artifact_id": {
                    "type": "string",
                    "description": "Stable id of the material (e.g. from workspace.search material hits).",
                },
                "locator": {
                    "type": "object",
                    "description": "Exactly one of md_byte_range or pdf_pages.",
                    "oneOf": [
                        {
                            "type": "object",
                            "properties": {
                                "md_byte_range": {
                                    "type": "object",
                                    "properties": {
                                        "start": { "type": "integer", "minimum": 0 },
                                        "end": { "type": "integer", "minimum": 0 },
                                    },
                                    "required": ["start", "end"],
                                    "additionalProperties": false,
                                },
                            },
                            "required": ["md_byte_range"],
                            "additionalProperties": false,
                        },
                        {
                            "type": "object",
                            "properties": {
                                "pdf_pages": {
                                    "type": "object",
                                    "properties": {
                                        "from": { "type": "integer", "minimum": 1 },
                                        "to": { "type": "integer", "minimum": 1 },
                                    },
                                    "required": ["from", "to"],
                                    "additionalProperties": false,
                                },
                            },
                            "required": ["pdf_pages"],
                            "additionalProperties": false,
                        },
                    ],
                },
                "expand": {
                    "type": "object",
                    "description": "Markdown only: extra context bytes before/after the range.",
                    "properties": {
                        "before": { "type": "integer", "minimum": 0 },
                        "after": { "type": "integer", "minimum": 0 },
                    },
                    "additionalProperties": false,
                },
            },
            "required": ["artifact_id", "locator"],
            "additionalProperties": false,
        },
    })
}

pub(super) fn call(workspace: &McpWorkspace, arguments: Value) -> Result<Value, ToolError> {
    let args: ReadArgs = parse_args(arguments)?;
    args.locator.validate()?;
    let id = StableId::parse(args.artifact_id.as_str()).map_err(|error| {
        ToolError::new(
            ToolErrorCode::InvalidArg,
            format!("artifact_id is not a valid stable id: {error}"),
        )
        .with_details(json!({ "artifact_id": args.artifact_id }))
    })?;

    let index = CanonicalIndex::load(workspace)?;
    let pool = workspace.pool()?;
    let Some(material) = find_material(workspace, &id)? else {
        if let Some(view) = index.by_id(id.as_str()) {
            return read_canonical_view(workspace, &pool, view, &args);
        }
        return not_found(&args, "artifact_not_found", "no artifact with this id");
    };

    match (material.media_type.as_str(), args.locator) {
        (MARKDOWN_MEDIA_TYPE, Locator::MdByteRange { start, end }) => {
            read_markdown(&pool, &material, &args, start, end).and_then(|value| {
                annotate_view(value, index.for_markdown(material.id.as_str()), None, None)
            })
        }
        (PDF_MEDIA_TYPE, Locator::PdfPages { from, to }) => {
            read_pdf(workspace, &pool, &material, &args, from, to, &index)
        }
        (MARKDOWN_MEDIA_TYPE | PDF_MEDIA_TYPE, _) => Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            format!(
                "locator kind does not match material media type {}; use md_byte_range for {MARKDOWN_MEDIA_TYPE} and pdf_pages for {PDF_MEDIA_TYPE}",
                material.media_type
            ),
        )
        .with_details(json!({ "media_type": material.media_type }))),
        (other, _) => {
            let mut details = json!({
                "media_type": other,
                "supported_media_types": [MARKDOWN_MEDIA_TYPE, PDF_MEDIA_TYPE],
            });
            // Ink snapshots are rendered by ink.page_image (Phase1d), never
            // read as text. A failed lookup only drops the hint.
            let snapshot = workspace
                .store()
                .ok()
                .and_then(|store| store.ink_snapshot_for_artifact(id.as_str()).ok().flatten());
            if let Some(snapshot) = snapshot {
                details["hint"] = json!("use ink.page_image");
                details["snapshot_id"] = json!(snapshot.snapshot_id);
            }
            Err(ToolError::new(
                ToolErrorCode::InvalidArg,
                format!("material.read does not support media type {other}"),
            )
            .with_details(details))
        }
    }
}

fn not_found(args: &ReadArgs, reason: &str, message: &str) -> Result<Value, ToolError> {
    let content_type = match args.locator {
        Locator::MdByteRange { .. } => "markdown",
        Locator::PdfPages { .. } => "pdf_text",
    };
    result(ReadResult {
        status: ToolStatus::NotFound,
        artifact_id: args.artifact_id.clone(),
        content_type,
        text: None,
        locator: args.locator,
        truncated: None,
        total_bytes: None,
        total_pages: None,
        page_spans: None,
        revision: None,
        details: Some(json!({ "reason": reason })),
        diagnostics: vec![diagnostic(reason, message)],
    })
}

fn result(result: ReadResult) -> Result<Value, ToolError> {
    to_object(result).map(Value::Object)
}

fn pool_error(error: ArtifactPoolError) -> ToolError {
    ToolError::new(
        ToolErrorCode::Internal,
        format!("failed to read managed artifact: {error}"),
    )
}

/// Tool result (`Ok`) or call error (`Err`) produced while resolving content.
type ToolReply = Result<Value, ToolError>;

/// Resolves the managed object, or a finished tool reply when it cannot be read.
///
/// [`ArtifactPoolError::MissingManagedLocation`] and a missing object file are
/// business `not_found`. [`ArtifactPoolError::UnsafeManagedPath`] (path escape
/// or symbolic link) is call-error `forbidden`. Other I/O and catalog failures
/// are `internal`. The path checks themselves stay inside `managed_path`.
fn require_managed_path(
    pool: &ArtifactPool,
    args: &ReadArgs,
    id: &StableId,
) -> Result<PathBuf, ToolReply> {
    match material::open_managed_path(pool, id) {
        Ok(path) => Ok(path),
        Err(error) => Err(map_managed_path_error(args, error)),
    }
}

fn map_managed_path_error(args: &ReadArgs, error: ArtifactPoolError) -> ToolReply {
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

fn read_markdown(
    pool: &ArtifactPool,
    material: &Material,
    args: &ReadArgs,
    start: u64,
    end: u64,
) -> Result<Value, ToolError> {
    let size = material.byte_size;
    if start > size {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            format!("md_byte_range start {start} is beyond the material size {size}"),
        )
        .with_details(json!({ "total_bytes": size })));
    }
    if let Err(reply) = require_managed_path(pool, args, &material.id) {
        return reply;
    }
    let mut diagnostics = Vec::new();
    let expand = args.expand.unwrap_or_default();
    let requested_end = end.min(size);
    if requested_end < end {
        diagnostics.push(diagnostic(
            "range_clamped",
            format!("md_byte_range end clamped to material size {size}"),
        ));
    }
    let start = start.saturating_sub(expand.before);
    let end = requested_end.saturating_add(expand.after).min(size);

    // Read a window wide enough to snap both ends to UTF-8 boundaries.
    let cap = MAX_READ_BYTES as u64;
    let window_start = start.saturating_sub(3);
    let window_end = end
        .min(start.saturating_add(cap))
        .saturating_add(4)
        .min(size);
    let window = if window_end > window_start {
        let length = u32::try_from(window_end - window_start)
            .map_err(|_| ToolError::new(ToolErrorCode::Internal, "read window exceeds u32"))?;
        pool.read_bytes(&material.id, window_start, length)
            .map_err(pool_error)?
    } else {
        Vec::new()
    };
    if (window.len() as u64) < window_end - window_start {
        return Err(ToolError::new(
            ToolErrorCode::Internal,
            "managed artifact is shorter than its catalog byte size",
        ));
    }
    let at = |offset: u64| (offset - window_start) as usize;
    let is_boundary = |offset: u64| offset >= window_end || window[at(offset)] & 0xC0 != 0x80;

    let mut snapped_start = start;
    while snapped_start > window_start && !is_boundary(snapped_start) {
        snapped_start -= 1;
    }
    let mut snapped_end = end.max(snapped_start);
    while snapped_end < window_end && !is_boundary(snapped_end) {
        snapped_end += 1;
    }
    let mut truncated = false;
    if snapped_end - snapped_start > cap {
        truncated = true;
        snapped_end = snapped_start + cap;
        while snapped_end > snapped_start && !is_boundary(snapped_end) {
            snapped_end -= 1;
        }
    }
    let text = std::str::from_utf8(&window[at(snapped_start)..at(snapped_end)])
        .map_err(|error| {
            ToolError::new(
                ToolErrorCode::Internal,
                format!("markdown material is not valid UTF-8: {error}"),
            )
        })?
        .to_owned();

    result(ReadResult {
        status: if truncated {
            ToolStatus::Truncated
        } else {
            ToolStatus::Ok
        },
        artifact_id: material.id.to_string(),
        content_type: "markdown",
        text: Some(text),
        locator: Locator::MdByteRange {
            start: snapped_start,
            end: snapped_end,
        },
        truncated: Some(truncated),
        total_bytes: Some(size),
        total_pages: None,
        page_spans: None,
        revision: Some(material.revision.clone()),
        details: None,
        diagnostics,
    })
}

fn pdf_error(error: mathscape_pdf::PdfError) -> ToolError {
    ToolError::new(
        ToolErrorCode::Internal,
        format!("failed to read PDF text layer: {error}"),
    )
}

fn read_pdf(
    workspace: &McpWorkspace,
    pool: &ArtifactPool,
    material: &Material,
    args: &ReadArgs,
    from: u32,
    to: u32,
    index: &CanonicalIndex,
) -> Result<Value, ToolError> {
    let path = match require_managed_path(pool, args, &material.id) {
        Ok(path) => path,
        Err(reply) => return reply,
    };
    let engine = PdfEngine;
    let options = PdfOpenOptions::new();
    let page_count = engine
        .probe(&path, &options)
        .map_err(pdf_error)?
        .page_count();
    if from > page_count {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            format!("pdf_pages from {from} is beyond the page count {page_count}"),
        )
        .with_details(json!({ "total_pages": page_count })));
    }
    let mut diagnostics = Vec::new();
    if args.expand.is_some() {
        diagnostics.push(diagnostic(
            "expand_not_applicable",
            "expand applies to md_byte_range only and was ignored",
        ));
    }
    let to = to.min(page_count);
    if to < args_to(args) {
        diagnostics.push(diagnostic(
            "range_clamped",
            format!("pdf_pages to clamped to page count {page_count}"),
        ));
    }

    let mut text = String::new();
    let mut spans = Vec::new();
    let mut empty_pages = Vec::new();
    let mut truncated = false;
    let mut batch_from = from;
    'batches: while batch_from <= to {
        let batch_to = batch_from.saturating_add(PDF_PAGE_BATCH - 1).min(to);
        let selection = PdfSelection::inclusive_range(
            page_count,
            PdfPageIndex::new(batch_from - 1),
            PdfPageIndex::new(batch_to - 1),
        )
        .map_err(pdf_error)?;
        let pages = engine
            .extract_positioned_text(&path, &selection, &options)
            .map_err(pdf_error)?
            .page_texts();
        for (index, page_text) in pages {
            let page = index.get() + 1;
            // The separator is atomic: a one-byte remainder must not become a lone `\n`.
            if !spans.is_empty() && !push_page_separator(&mut text) {
                truncated = true;
                break 'batches;
            }
            if page_text.trim().is_empty() {
                empty_pages.push(page);
            }
            let start = text.len();
            let complete = push_capped(&mut text, &page_text);
            spans.push(PageSpan {
                page,
                start,
                end: text.len(),
            });
            if !complete {
                truncated = true;
                break 'batches;
            }
        }
        batch_from = batch_to + 1;
    }

    let returned_to = spans.last().map_or(from, |span| span.page);
    let requested = InclusivePageRange::new(from, returned_to).ok();
    if !truncated && empty_pages.len() == spans.len() {
        if let Some(served) = serve_canonical_snapshot(
            workspace,
            pool,
            material,
            args,
            index,
            from,
            returned_to,
            page_count,
            diagnostics.clone(),
        )? {
            return Ok(served);
        }
        let unavailable = capability_unavailable(
            workspace,
            material,
            from,
            returned_to,
            page_count,
            diagnostics,
        )?;
        return annotate_view(
            unavailable,
            index.for_pdf(material.id.as_str(), requested),
            requested,
            None,
        );
    }
    if !empty_pages.is_empty() {
        diagnostics.push(json!({
            "code": "pdf_pages_without_text",
            "message": "some pages have no text layer; OCR (Marker) is not run by material.read",
            "pages": empty_pages,
        }));
    }

    let value = result(ReadResult {
        status: if truncated {
            ToolStatus::Truncated
        } else {
            ToolStatus::Ok
        },
        artifact_id: material.id.to_string(),
        content_type: "pdf_text",
        text: Some(text),
        locator: Locator::PdfPages {
            from,
            to: returned_to,
        },
        truncated: Some(truncated),
        total_bytes: None,
        total_pages: Some(page_count),
        page_spans: Some(spans),
        revision: Some(material.revision.clone()),
        details: None,
        diagnostics,
    })?;
    annotate_view(
        value,
        index.for_pdf(material.id.as_str(), requested),
        requested,
        None,
    )
}

fn args_to(args: &ReadArgs) -> u32 {
    match args.locator {
        Locator::PdfPages { to, .. } => to,
        Locator::MdByteRange { .. } => 0,
    }
}

/// Appends [`PAGE_SEPARATOR`] only when both bytes fit under [`MAX_READ_BYTES`].
/// Returns whether it was written. Never stores a partial separator.
fn push_page_separator(text: &mut String) -> bool {
    if text.len() + PAGE_SEPARATOR.len() > MAX_READ_BYTES {
        return false;
    }
    text.push_str(PAGE_SEPARATOR);
    true
}

/// Appends as much of `piece` as fits under [`MAX_READ_BYTES`], cutting at a
/// character boundary. Returns whether all of it fit.
fn push_capped(text: &mut String, piece: &str) -> bool {
    let room = MAX_READ_BYTES.saturating_sub(text.len());
    if piece.len() <= room {
        text.push_str(piece);
        return true;
    }
    let mut cut = room;
    while !piece.is_char_boundary(cut) {
        cut -= 1;
    }
    text.push_str(&piece[..cut]);
    false
}

fn read_canonical_view(
    workspace: &McpWorkspace,
    pool: &ArtifactPool,
    view: &CanonicalMarkdownView,
    args: &ReadArgs,
) -> Result<Value, ToolError> {
    let Locator::MdByteRange { start, end } = args.locator else {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            "a canonical markdown id is read with md_byte_range; pdf_pages addresses the source PDF",
        )
        .with_details(json!({
            "canonical_id": view.canonical_id.as_str(),
            "source_pdf_artifact_id": view.source_pdf_artifact_id.as_str(),
        })));
    };
    let Some(head_id) = view.head_artifact_id.clone() else {
        return not_found(
            args,
            "canonical_head_missing",
            "canonical markdown has no head snapshot",
        );
    };
    let Some(head) = find_material(workspace, &head_id)? else {
        return not_found(
            args,
            "canonical_head_missing",
            "canonical markdown head artifact is not in the catalog",
        );
    };
    if head.media_type != MARKDOWN_MEDIA_TYPE {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            format!(
                "canonical markdown head is {} and cannot be read as markdown",
                head.media_type
            ),
        ));
    }
    let mut value = read_markdown(pool, &head, args, start, end)?;
    if let Some(object) = value.as_object_mut() {
        let diagnostics = object
            .entry("diagnostics")
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Some(diagnostics) = diagnostics.as_array_mut() {
            diagnostics.push(diagnostic(
                "canonical_id_resolved",
                "artifact_id was a canonical markdown id; the head snapshot was read",
            ));
        }
    }
    annotate_view(value, Some(view), None, Some(head.id.as_str()))
}

fn serve_canonical_snapshot(
    workspace: &McpWorkspace,
    pool: &ArtifactPool,
    material: &Material,
    args: &ReadArgs,
    index: &CanonicalIndex,
    from: u32,
    to: u32,
    page_count: u32,
    mut diagnostics: Vec<Value>,
) -> Result<Option<Value>, ToolError> {
    let Some(request) = InclusivePageRange::new(from, to).ok() else {
        return Ok(None);
    };
    let Some(view) = index.for_pdf(material.id.as_str(), Some(request)) else {
        return Ok(None);
    };
    let Some(segment) = choose_segment(view, request) else {
        return Ok(None);
    };
    let Some(served) = find_material(workspace, &segment.markdown_artifact_id)? else {
        return Ok(None);
    };
    match require_managed_path(pool, args, &served.id) {
        Ok(_) => {}
        Err(Ok(_)) => return Ok(None),
        Err(Err(error)) => return Err(error),
    }
    let (text, truncated) = read_markdown_prefix(pool, &served)?;
    diagnostics.push(diagnostic(
        "served_canonical_markdown",
        "requested pages have no text layer; returned the immutable canonical markdown snapshot. Marker was not run",
    ));
    let snapshot_covers = segment
        .pages
        .is_some_and(|pages| pages.from <= request.from && request.to <= pages.to);
    if !snapshot_covers {
        diagnostics.push(diagnostic(
            "canonical_snapshot_partial",
            "the served snapshot does not contain every requested page",
        ));
    }
    let value = result(ReadResult {
        status: if truncated {
            ToolStatus::Truncated
        } else {
            ToolStatus::Ok
        },
        artifact_id: material.id.to_string(),
        content_type: "markdown",
        text: Some(text),
        locator: Locator::PdfPages { from, to },
        truncated: Some(truncated),
        total_bytes: None,
        total_pages: Some(page_count),
        page_spans: None,
        revision: Some(material.revision.clone()),
        details: None,
        diagnostics,
    })?;
    annotate_view(value, Some(view), Some(request), Some(served.id.as_str())).map(Some)
}

fn choose_segment(
    view: &CanonicalMarkdownView,
    request: InclusivePageRange,
) -> Option<&mathscape_domain::CanonicalSegment> {
    let overlaps = |segment: &&mathscape_domain::CanonicalSegment| {
        segment.pages.is_some_and(|pages| pages.overlaps(request))
    };
    if let Some(head) = &view.head_artifact_id
        && let Some(segment) = view.segments.iter().find(|segment| {
            &segment.markdown_artifact_id == head
                && segment.pages.is_some_and(|pages| pages.overlaps(request))
        })
    {
        return Some(segment);
    }
    view.segments.iter().filter(overlaps).max_by(|left, right| {
        left.finished_at
            .cmp(&right.finished_at)
            .then_with(|| left.conversion_id.cmp(&right.conversion_id))
    })
}

fn read_markdown_prefix(
    pool: &ArtifactPool,
    material: &Material,
) -> Result<(String, bool), ToolError> {
    let size = material.byte_size;
    if size == 0 {
        return Ok((String::new(), false));
    }
    let cap = MAX_READ_BYTES as u64;
    let want = size.min(cap);
    let window_end = want.saturating_add(4).min(size);
    let length = u32::try_from(window_end)
        .map_err(|_| ToolError::new(ToolErrorCode::Internal, "read window exceeds u32"))?;
    let window = pool
        .read_bytes(&material.id, 0, length)
        .map_err(pool_error)?;
    if window.len() as u64 != window_end {
        return Err(ToolError::new(
            ToolErrorCode::Internal,
            "managed artifact is shorter than its catalog byte size",
        ));
    }
    let mut end = usize::try_from(want)
        .unwrap_or(window.len())
        .min(window.len());
    while end > 0 && end < window.len() && (window[end] as i8) < -0x40 {
        end -= 1;
    }
    let text = std::str::from_utf8(&window[..end]).map_err(|error| {
        ToolError::new(
            ToolErrorCode::Internal,
            format!("canonical markdown snapshot is not valid UTF-8: {error}"),
        )
    })?;
    Ok((text.to_owned(), want < size))
}

fn annotate_view(
    mut value: Value,
    view: Option<&CanonicalMarkdownView>,
    requested: Option<InclusivePageRange>,
    served_artifact_id: Option<&str>,
) -> Result<Value, ToolError> {
    let Some(view) = view else {
        return Ok(value);
    };
    let coverage = view.coverage(requested);
    let Some(object) = value.as_object_mut() else {
        return Ok(value);
    };
    object.insert("canonical_id".into(), json!(view.canonical_id.as_str()));
    object.insert("partial".into(), json!(coverage.partial));
    object.insert(
        "coverage".into(),
        serde_json::to_value(&coverage).map_err(|error| {
            ToolError::new(
                ToolErrorCode::Internal,
                format!("failed to serialize canonical coverage: {error}"),
            )
        })?,
    );
    if let Some(served) = served_artifact_id {
        object.insert("served_artifact_id".into(), json!(served));
    }
    Ok(value)
}

fn capability_unavailable(
    workspace: &McpWorkspace,
    material: &Material,
    from: u32,
    to: u32,
    page_count: u32,
    mut diagnostics: Vec<Value>,
) -> Result<Value, ToolError> {
    let probe = probe_capability(workspace.capability_runner(), Capability::Marker);
    if probe.state == CapabilityState::Available {
        diagnostics.push(diagnostic(
            "marker_not_wired",
            "requested pages have no text layer; Marker is installed but material.read does not run OCR in Phase1",
        ));
    } else {
        diagnostics.push(diagnostic(
            "pdf_no_text_layer",
            "requested pages have no text layer; install Marker to convert scanned PDFs",
        ));
    }
    let details = serde_json::to_value(&probe).map_err(|error| {
        ToolError::new(
            ToolErrorCode::Internal,
            format!("failed to serialize capability probe: {error}"),
        )
    })?;
    result(ReadResult {
        status: ToolStatus::CapabilityUnavailable,
        artifact_id: material.id.to_string(),
        content_type: "pdf_text",
        text: None,
        locator: Locator::PdfPages { from, to },
        truncated: None,
        total_bytes: None,
        total_pages: Some(page_count),
        page_spans: None,
        revision: Some(material.revision.clone()),
        details: Some(details),
        diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use mathscape_store_sqlite::SqliteStoreError;

    use super::*;

    fn sample_args() -> ReadArgs {
        serde_json::from_value(json!({
            "artifact_id": "mathscape:artifact/sha256/0000000000000000000000000000000000000000000000000000000000000000",
            "locator": { "md_byte_range": { "start": 0, "end": 1 } },
        }))
        .unwrap()
    }

    #[test]
    fn page_separator_is_not_split_at_the_byte_cap() {
        let mut one_byte_short = "x".repeat(MAX_READ_BYTES - 1);
        assert!(!push_page_separator(&mut one_byte_short));
        assert_eq!(one_byte_short.len(), MAX_READ_BYTES - 1);
        assert!(!one_byte_short.ends_with('\n'));

        let mut exact = "x".repeat(MAX_READ_BYTES - PAGE_SEPARATOR.len());
        assert!(push_page_separator(&mut exact));
        assert_eq!(exact.len(), MAX_READ_BYTES);
        assert!(exact.ends_with(PAGE_SEPARATOR));
    }

    #[test]
    fn managed_path_errors_map_to_not_found_forbidden_or_internal() {
        let args = sample_args();
        let unavailable = map_managed_path_error(
            &args,
            ArtifactPoolError::MissingManagedLocation(args.artifact_id.clone()),
        )
        .unwrap();
        assert_eq!(unavailable["status"], "not_found");
        assert_eq!(unavailable["details"]["reason"], "content_unavailable");

        let missing_object = map_managed_path_error(
            &args,
            ArtifactPoolError::Io {
                operation: "inspect managed object",
                path: PathBuf::from("objects/sha256/missing"),
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "missing object"),
            },
        )
        .unwrap();
        assert_eq!(missing_object["status"], "not_found");
        assert_eq!(missing_object["details"]["reason"], "content_unavailable");

        let secret = PathBuf::from("/tmp/secret-managed-target");
        let forbidden =
            map_managed_path_error(&args, ArtifactPoolError::UnsafeManagedPath(secret.clone()))
                .unwrap_err();
        assert_eq!(forbidden.code, ToolErrorCode::Forbidden);
        assert!(!forbidden.message.contains("secret-managed-target"));

        let internal = map_managed_path_error(
            &args,
            ArtifactPoolError::Io {
                operation: "inspect managed object",
                path: PathBuf::from("objects/sha256/blocked"),
                source: std::io::Error::other("permission denied"),
            },
        )
        .unwrap_err();
        assert_eq!(internal.code, ToolErrorCode::Internal);

        let catalog = map_managed_path_error(
            &args,
            ArtifactPoolError::Store(SqliteStoreError::InvalidSchema("catalog unreadable")),
        )
        .unwrap_err();
        assert_eq!(catalog.code, ToolErrorCode::Internal);

        let bad_uri = map_managed_path_error(
            &args,
            ArtifactPoolError::InvalidPoolUri("file:///etc/passwd".to_owned()),
        )
        .unwrap_err();
        assert_eq!(bad_uri.code, ToolErrorCode::Internal);
    }
}
