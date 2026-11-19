//! `material.read`: bounded reads of one registered material.
//!
//! - Markdown (`text/markdown`) by `md_byte_range` (bytes, half-open).
//!   `expand.before/after` are bytes. The returned range is widened to UTF-8
//!   character boundaries and echoed back as the actual locator.
//! - PDF (`application/pdf`) by `pdf_pages` (1-based, closed). A careful
//!   read of a PDF always serves canonical Marker markdown (规范解析正文),
//!   never the native text layer: an immutable canonical markdown snapshot
//!   whose pages fully contain the request is served, whether or not the
//!   pages have a text layer. If none does, a bounded internal ensure
//!   (`super::ensure`) may run Marker on exactly the requested range, promote
//!   a new snapshot and serve it in the same call (synchronous: the call
//!   waits for Marker, and a concurrent read of the same PDF queues behind
//!   the running parse). Otherwise the tool returns `capability_unavailable`
//!   with the Marker CapabilityProbe and `details.ensure`
//!   (`blocked_budget|capability_unavailable|failed`); it never falls back
//!   to the `mathscape-pdf` text layer. That text layer is only probed on
//!   this failure path, to tell a scanned page (`pdf_no_text_layer`) from a
//!   page whose text layer is deliberately not served
//!   (`pdf_text_layer_not_served`). Coarse location over the text layer is
//!   `material.grep`'s job. Results carry `canonical_id`, `coverage` and
//!   `partial` when a view exists.
//!
//! Every successful read is capped at [`MAX_READ_BYTES`] of UTF-8 text;
//! a capped read returns `status: "truncated"` and `truncated: true`.

use std::path::{Path, PathBuf};

use mathscape_agent::{
    Capability, CapabilityState, Locator, ToolError, ToolErrorCode, ToolStatus, probe_capability,
};
use mathscape_artifact::{ArtifactPool, ArtifactPoolError};
use mathscape_domain::{
    CanonicalMarkdownView, InclusivePageRange, PageCoverPiece, StableId, markdown_offset_of_page,
    marker_page_id, plan_page_cover,
};
use mathscape_orchestrator::{
    EffectiveByteSegmentKind, build_effective_markdown, render_effective_slice,
};
use mathscape_pdf::{PdfEngine, PdfOpenOptions, PdfPageIndex, PdfSelection};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::canonical::CanonicalIndex;
use super::ensure::{self, EnsureReport, EnsureStatus};
use super::errata::{self, load_active_errata};
use super::material::{
    self, MARKDOWN_MEDIA_TYPE, Material, PDF_MEDIA_TYPE, find_material, managed_path_failure,
};
use super::{MATERIAL_READ, diagnostic, parse_args, to_object};
use crate::McpWorkspace;

/// Hard cap for returned text, in UTF-8 bytes (contract §2.2: 32KiB).
pub const MAX_READ_BYTES: usize = 32 * 1024;

/// At most this many requested pages are probed for a text layer when a
/// careful PDF read cannot be served (diagnostics only, never the body).
const TEXT_LAYER_PROBE_PAGES: u32 = 8;

/// A snapshot larger than this is not read whole for page stitching (the
/// same cap as the Desktop unified reader).
const MAX_STITCH_SNAPSHOT_BYTES: u64 = 16 * 1024 * 1024;

/// Placed between stitched snapshot pieces in the returned text.
const STITCH_SEPARATOR: &str = "\n\n";

const DESCRIPTION: &str = "Careful read of part of one workspace material by locator. Markdown: locator md_byte_range {start,end} in bytes, half-open; optional expand {before,after} in bytes. PDF: locator pdf_pages {from,to}, 1-based and inclusive; the body is always canonical Marker markdown (content_type markdown), never the raw PDF text layer, even when the pages have one. Returns the actual locator read; text is capped at 32 KiB (status truncated, truncated true). When a canonical markdown view exists, the result includes canonical_id, coverage and partial (partial true means this request or the book is not fully covered). PDF pages are served from existing immutable markdown snapshots: one snapshot that contains every requested page (the body is that snapshot from byte 0, so text offsets are its byte offsets), or else several snapshots stitched in page order. details.served_segments lists each piece, also for a single snapshot, with its artifact_id, pages and the md_byte_range of the requested pages inside that snapshot; quote and cite those, and use material.locate to get the exact md_byte_range of a passage. A snapshot covering several pages without page markers yields diagnostic page_markers_absent: cite it with granularity inline. Pages already parsed are never parsed again. If some requested pages are not parsed yet, this call runs Marker locally on just those missing pages (one run per gap of missing pages, never re-parsing covered pages between gaps) and waits for it (synchronous, can take minutes; at most 12 newly parsed pages per call, rolling page budget, one Marker run per workspace slot), adds the new immutable snapshots to the workspace parse cache and returns the stitched markdown in the same result. A concurrent read of a PDF that is already being parsed waits for that parse instead of failing. The result is ok or truncated with the text, or capability_unavailable with details.ensure.status blocked_budget, capability_unavailable (Marker missing) or failed (Marker error or the sync time limit; nothing is left running); it never falls back to the PDF text layer. Do not poll or re-read to wait for parsing. To locate a phrase in a PDF quickly use material.grep (text-layer hits with a pdf_pages locator), then read that page here. It never modifies or deletes existing materials and never contacts the network. Keep PDF page ranges small. A canonical id may be passed as artifact_id with md_byte_range to read its head snapshot.";

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
    revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    diagnostics: Vec<Value>,
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
            read_markdown(workspace, &pool, &material, &args, start, end).and_then(|value| {
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
        // A careful PDF read only ever serves canonical markdown.
        Locator::PdfPages { .. } => "markdown",
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
    workspace: &McpWorkspace,
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

    let errata = load_active_errata(workspace, material.id.as_str())?;
    if !errata.is_empty()
        && let Some(raw_source) = read_markdown_whole(pool, material)?
    {
        let mut snapped_start = (start as usize).min(raw_source.len());
        while snapped_start > 0 && !raw_source.is_char_boundary(snapped_start) {
            snapped_start -= 1;
        }
        let mut snapped_end = (end as usize).min(raw_source.len()).max(snapped_start);
        while snapped_end < raw_source.len() && !raw_source.is_char_boundary(snapped_end) {
            snapped_end += 1;
        }
        let view = render_effective_slice(&raw_source, snapped_start..snapped_end, &errata.entries);
        let capped = cap_at_char_boundary(&view.effective_source, MAX_READ_BYTES).to_owned();
        let truncated = capped.len() < view.effective_source.len();
        let locator = if capped.is_empty() {
            Locator::MdByteRange {
                start: snapped_start as u64,
                end: snapped_end as u64,
            }
        } else if let Some(physical) = view.byte_map.effective_range_to_physical(0, capped.len()) {
            Locator::MdByteRange {
                start: physical.start,
                end: physical.end,
            }
        } else {
            Locator::MdByteRange {
                start: snapped_start as u64,
                end: snapped_end as u64,
            }
        };
        let applied = errata.applied_within_prefix(&view.byte_map, capped.len());
        let details = if applied.is_empty() {
            None
        } else {
            diagnostics.push(errata::applied_errata_diagnostic(applied.len()));
            Some(json!({ "applied_errata": applied }))
        };
        return result(ReadResult {
            status: if truncated {
                ToolStatus::Truncated
            } else {
                ToolStatus::Ok
            },
            artifact_id: material.id.to_string(),
            content_type: "markdown",
            text: Some(capped),
            locator,
            truncated: Some(truncated),
            total_bytes: Some(size),
            total_pages: None,
            revision: Some(material.revision.clone()),
            details,
            diagnostics,
        });
    }

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

    // Careful reads serve canonical markdown only: a covering snapshot first,
    // whether or not these pages have a native text layer.
    if let Some(served) = serve_canonical_snapshot(
        workspace,
        pool,
        material,
        args,
        index,
        from,
        to,
        page_count,
        diagnostics.clone(),
        None,
    )? {
        return Ok(served);
    }
    let requested = InclusivePageRange::new(from, to).ok();
    let Some(request) = requested else {
        return capability_unavailable(
            workspace,
            material,
            &path,
            from,
            to,
            page_count,
            diagnostics,
            None,
        );
    };
    // Synchronous: returns only after Marker finished (or failed, or the
    // sync deadline passed) and no lease of this call is left running.
    let mut report = ensure::ensure(workspace, &material.id, &path, request, page_count);
    let reloaded;
    let index = if matches!(report.status, EnsureStatus::Expanded | EnsureStatus::Reused) {
        reloaded = CanonicalIndex::load(workspace)?;
        if let Some(served) = serve_canonical_snapshot(
            workspace,
            pool,
            material,
            args,
            &reloaded,
            from,
            to,
            page_count,
            diagnostics.clone(),
            Some(&report),
        )? {
            return Ok(served);
        }
        report.status = EnsureStatus::Failed;
        report.message = format!(
            "{}; but snapshots covering pages {from}–{to} could not be served",
            report.message
        );
        &reloaded
    } else {
        index
    };
    // No silent fallback to the native text layer.
    let unavailable = capability_unavailable(
        workspace,
        material,
        &path,
        from,
        to,
        page_count,
        diagnostics,
        Some(&report),
    )?;
    annotate_view(
        unavailable,
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

/// Diagnostic-only probe of the native text layer on the first
/// [`TEXT_LAYER_PROBE_PAGES`] requested pages: `(probed_to, pages_with_text)`.
/// `None` when the probe itself fails; the read result does not depend on it.
fn probe_text_layer(path: &Path, page_count: u32, from: u32, to: u32) -> Option<(u32, Vec<u32>)> {
    let probe_to = to
        .min(from.saturating_add(TEXT_LAYER_PROBE_PAGES - 1))
        .min(page_count);
    if from == 0 || probe_to < from {
        return None;
    }
    let selection = PdfSelection::inclusive_range(
        page_count,
        PdfPageIndex::new(from - 1),
        PdfPageIndex::new(probe_to - 1),
    )
    .ok()?;
    let pages = PdfEngine
        .extract_positioned_text(path, &selection, &PdfOpenOptions::new())
        .ok()?
        .page_texts();
    let with_text = pages
        .into_iter()
        .filter(|(_, text)| !text.trim().is_empty())
        .map(|(index, _)| index.get() + 1)
        .collect();
    Some((probe_to, with_text))
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
    let mut value = read_markdown(workspace, pool, &head, args, start, end)?;
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

#[allow(clippy::too_many_arguments)]
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
    ensured: Option<&EnsureReport>,
) -> Result<Option<Value>, ToolError> {
    let Some(request) = InclusivePageRange::new(from, to).ok() else {
        return Ok(None);
    };
    let mut chosen = None;
    for view in index.all_for_pdf(material.id.as_str(), Some(request)) {
        let Some(segment) = choose_segment(view, request) else {
            continue;
        };
        let Some(served) = find_material(workspace, &segment.markdown_artifact_id)? else {
            continue;
        };
        match require_managed_path(pool, args, &served.id) {
            Ok(_) => {}
            Err(Ok(_)) => continue,
            Err(Err(error)) => return Err(error),
        }
        chosen = Some((view, segment, served));
        break;
    }
    let Some((view, segment, served)) = chosen else {
        return serve_stitched(
            workspace,
            pool,
            material,
            args,
            index,
            request,
            page_count,
            diagnostics,
            ensured,
        );
    };
    let errata = load_active_errata(workspace, served.id.as_str())?;
    let segment_pages = segment.pages;
    let whole_source = if segment_pages.is_some() || !errata.is_empty() {
        read_markdown_whole(pool, &served)?
    } else {
        None
    };
    let (text, truncated, applied_errata) =
        if !errata.is_empty() && let Some(source) = whole_source.as_deref() {
            let view = build_effective_markdown(source, &errata.entries);
            let capped = cap_at_char_boundary(&view.effective_source, MAX_READ_BYTES).to_owned();
            let truncated = capped.len() < view.effective_source.len();
            let applied = errata.applied_within_prefix(&view.byte_map, capped.len());
            (capped, truncated, applied)
        } else {
            let (text, truncated) = read_markdown_prefix(pool, &served)?;
            (text, truncated, Vec::new())
        };
    // The body is the snapshot from byte 0 (text offsets are snapshot
    // offsets); served_segments says where the requested pages are inside it.
    let mut segments = Vec::new();
    if let Some(snapshot_pages) = segment_pages
        && let Some(source) = whole_source.as_deref()
    {
        let (range, exact) = page_byte_range(source, snapshot_pages, request);
        if let Some(diagnostic) = page_markers_absent(source, &served.id, snapshot_pages) {
            diagnostics.push(diagnostic);
        }
        segments.push(ServedSegment {
            artifact_id: served.id.to_string(),
            canonical_id: view.canonical_id.to_string(),
            pages: request,
            snapshot_pages,
            md_byte_range: range,
            trimmed_to_pages: exact,
            is_head: view.head_artifact_id.as_ref() == Some(&served.id),
        });
    }
    if !applied_errata.is_empty() {
        diagnostics.push(errata::applied_errata_diagnostic(applied_errata.len()));
    }
    diagnostics.push(match ensured {
        None => diagnostic(
            "served_canonical_markdown",
            "returned the immutable canonical markdown snapshot that covers every requested page (careful PDF reads never serve the native text layer). Marker was not run",
        ),
        Some(report) => diagnostic(
            report.status.diagnostic_code(),
            format!(
                "no canonical markdown snapshot contained the requested pages; {}",
                report.message
            ),
        ),
    });
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
        revision: Some(material.revision.clone()),
        details: single_details(ensured, &material.id, &segments, &applied_errata)?,
        diagnostics,
    })?;
    annotate_view(value, Some(view), Some(request), Some(served.id.as_str())).map(Some)
}

/// One stitched piece as returned in `details.served_segments`.
#[derive(Debug, Serialize)]
struct ServedSegment {
    artifact_id: String,
    canonical_id: String,
    /// Requested pages this piece supplies.
    pages: InclusivePageRange,
    /// The snapshot's own ledger range.
    snapshot_pages: InclusivePageRange,
    /// Bytes of the snapshot selected for `pages` (before the 32 KiB cap).
    md_byte_range: ByteRange,
    /// False when the snapshot has no usable page markers and the whole
    /// snapshot was kept, so the piece may repeat neighbouring pages.
    trimmed_to_pages: bool,
    is_head: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct ByteRange {
    start: usize,
    end: usize,
}

/// Byte range of `piece.pages` inside a snapshot whose ledger range is
/// `piece.snapshot_pages`, cut at Marker page markers (`{N}------`, 0-based
/// absolute PDF pages). An edge that coincides with the snapshot edge needs
/// no marker; an edge whose marker is missing keeps the snapshot edge and
/// reports `false` (not trimmed).
fn piece_byte_range(source: &str, piece: &PageCoverPiece) -> (ByteRange, bool) {
    page_byte_range(source, piece.snapshot_pages, piece.pages)
}

/// Byte range of `pages` inside a snapshot covering `snapshot` (see
/// [`piece_byte_range`]).
fn page_byte_range(
    source: &str,
    snapshot: InclusivePageRange,
    pages: InclusivePageRange,
) -> (ByteRange, bool) {
    let mut exact = true;
    let start = if pages.from <= snapshot.from {
        0
    } else if let Some(at) = markdown_offset_of_page(source, snapshot, pages.from) {
        at
    } else {
        exact = false;
        0
    };
    let end = if pages.to >= snapshot.to {
        source.len()
    } else if let Some(at) = markdown_offset_of_page(source, snapshot, pages.to + 1) {
        at
    } else {
        exact = false;
        source.len()
    };
    let end = end.max(start);
    (ByteRange { start, end }, exact)
}

fn read_markdown_whole(
    pool: &ArtifactPool,
    material: &Material,
) -> Result<Option<String>, ToolError> {
    let size = material.byte_size;
    if size > MAX_STITCH_SNAPSHOT_BYTES {
        return Ok(None);
    }
    if size == 0 {
        return Ok(Some(String::new()));
    }
    let length = u32::try_from(size)
        .map_err(|_| ToolError::new(ToolErrorCode::Internal, "read window exceeds u32"))?;
    let bytes = pool
        .read_bytes(&material.id, 0, length)
        .map_err(pool_error)?;
    if bytes.len() as u64 != size {
        return Err(ToolError::new(
            ToolErrorCode::Internal,
            "managed artifact is shorter than its catalog byte size",
        ));
    }
    String::from_utf8(bytes).map(Some).map_err(|error| {
        ToolError::new(
            ToolErrorCode::Internal,
            format!("canonical markdown snapshot is not valid UTF-8: {error}"),
        )
    })
}

/// Largest prefix of `text` within `cap` bytes that ends on a char boundary.
fn cap_at_char_boundary(text: &str, cap: usize) -> &str {
    if text.len() <= cap {
        return text;
    }
    let mut end = cap;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Serves `request` by stitching several immutable snapshots page by page
/// (see [`plan_page_cover`]), across every lineage of the PDF. `None` when
/// the snapshots do not cover every requested page, or a chosen snapshot is
/// not readable markdown; the read then falls through to ensure.
#[allow(clippy::too_many_arguments)]
fn serve_stitched(
    workspace: &McpWorkspace,
    pool: &ArtifactPool,
    material: &Material,
    args: &ReadArgs,
    index: &CanonicalIndex,
    request: InclusivePageRange,
    page_count: u32,
    mut diagnostics: Vec<Value>,
    ensured: Option<&EnsureReport>,
) -> Result<Option<Value>, ToolError> {
    let views = index.all_for_pdf(material.id.as_str(), Some(request));
    let cover = plan_page_cover(&views, request);
    if !cover.is_complete() {
        return Ok(None);
    }
    let mut text = String::new();
    let mut segments = Vec::with_capacity(cover.pieces.len());
    let mut untrimmed = false;
    let mut stitched_errata: Vec<(usize, String, Value)> = Vec::new();
    for piece in &cover.pieces {
        let Some(snapshot) = find_material(workspace, &piece.markdown_artifact_id)? else {
            return Ok(None);
        };
        if snapshot.media_type != MARKDOWN_MEDIA_TYPE {
            return Ok(None);
        }
        match require_managed_path(pool, args, &snapshot.id) {
            Ok(_) => {}
            Err(Ok(_)) => return Ok(None),
            Err(Err(error)) => return Err(error),
        }
        let Some(source) = read_markdown_whole(pool, &snapshot)? else {
            return Ok(None);
        };
        let (range, exact) = piece_byte_range(&source, piece);
        untrimmed |= !exact;
        if let Some(diagnostic) = page_markers_absent(&source, &snapshot.id, piece.snapshot_pages) {
            diagnostics.push(diagnostic);
        }
        if !text.is_empty() {
            text.push_str(STITCH_SEPARATOR);
        }
        let errata = load_active_errata(workspace, snapshot.id.as_str())?;
        if errata.is_empty() {
            text.push_str(&source[range.start..range.end]);
        } else {
            let piece_offset = text.len();
            let view = render_effective_slice(&source, range.start..range.end, &errata.entries);
            text.push_str(&view.effective_source);
            for seg in view.byte_map.segments() {
                if let EffectiveByteSegmentKind::Errata { errata_id } = &seg.kind
                    && let Some(row) = errata.rows.iter().find(|r| r.errata_id == *errata_id)
                {
                    stitched_errata.push((
                        piece_offset + seg.eff_start,
                        row.errata_id.clone(),
                        errata::errata_provenance_json(row),
                    ));
                }
            }
        }
        segments.push(ServedSegment {
            artifact_id: snapshot.id.to_string(),
            canonical_id: views[piece.view_index].canonical_id.to_string(),
            pages: piece.pages,
            snapshot_pages: piece.snapshot_pages,
            md_byte_range: range,
            trimmed_to_pages: exact,
            is_head: piece.is_head,
        });
    }
    let total = text.len();
    let capped = cap_at_char_boundary(&text, MAX_READ_BYTES).to_owned();
    let truncated = capped.len() < total;
    let mut seen_errata = std::collections::HashSet::new();
    let applied_errata: Vec<Value> = stitched_errata
        .into_iter()
        .filter(|(offset, id, _)| *offset < capped.len() && seen_errata.insert(id.clone()))
        .map(|(_, _, entry)| entry)
        .collect();
    if !applied_errata.is_empty() {
        diagnostics.push(errata::applied_errata_diagnostic(applied_errata.len()));
    }
    diagnostics.push(diagnostic(
        "served_stitched_canonical_markdown",
        format!(
            "no single snapshot contains pages {}–{}; returned {} immutable canonical markdown snapshots stitched in page order (details.served_segments). {}",
            request.from,
            request.to,
            segments.len(),
            if ensured.is_some() {
                "Marker parsed only the pages no snapshot covered"
            } else {
                "Marker was not run"
            }
        ),
    ));
    if untrimmed {
        diagnostics.push(diagnostic(
            "stitched_snapshot_untrimmed",
            "a stitched snapshot has no usable page markers, so it was kept whole and may repeat or exceed the requested pages",
        ));
    }
    if let Some(report) = ensured {
        diagnostics.push(diagnostic(
            report.status.diagnostic_code(),
            report.message.clone(),
        ));
    }
    let mut details = match ensured {
        Some(report) => ensure_details(report, &material.id, None)?,
        None => json!({}),
    };
    if let Some(object) = details.as_object_mut() {
        object.insert(
            "served_segments".into(),
            to_json(&segments, "served segments")?,
        );
        if !applied_errata.is_empty() {
            object.insert("applied_errata".into(), Value::Array(applied_errata));
        }
    }
    let value = result(ReadResult {
        status: if truncated {
            ToolStatus::Truncated
        } else {
            ToolStatus::Ok
        },
        artifact_id: material.id.to_string(),
        content_type: "markdown",
        text: Some(capped),
        locator: Locator::PdfPages {
            from: request.from,
            to: request.to,
        },
        truncated: Some(truncated),
        total_bytes: None,
        total_pages: Some(page_count),
        revision: Some(material.revision.clone()),
        details: Some(details),
        diagnostics,
    })?;
    let primary = views[cover.pieces[0].view_index];
    annotate_view(value, Some(primary), Some(request), None).map(Some)
}

/// `page_markers_absent` when a snapshot covering several pages has no Marker
/// page markers (`{N}------`): its bytes cannot be attributed to one page, so
/// a SourceSpan over it must use inline granularity. User-imported Markdown
/// and Markdown from other sources often has no markers; this is normal.
fn page_markers_absent(
    source: &str,
    artifact: &StableId,
    snapshot_pages: InclusivePageRange,
) -> Option<Value> {
    if snapshot_pages.from == snapshot_pages.to
        || source
            .split_inclusive('\n')
            .any(|line| marker_page_id(line).is_some())
    {
        return None;
    }
    Some(json!({
        "code": "page_markers_absent",
        "message": format!(
            "snapshot {artifact} covers pages {}–{} but has no page markers; md_byte_range cannot be attributed to a single page, so SourceSpan over it must use granularity inline (page granularity is unavailable)",
            snapshot_pages.from, snapshot_pages.to
        ),
        "artifact_id": artifact.as_str(),
        "snapshot_pages": snapshot_pages,
    }))
}

/// `details` of a single-snapshot read: the ensure report (if any) plus the
/// served segment.
fn single_details(
    ensured: Option<&EnsureReport>,
    material: &StableId,
    segments: &[ServedSegment],
    applied_errata: &[Value],
) -> Result<Option<Value>, ToolError> {
    let mut details = ensured
        .map(|report| ensure_details(report, material, None))
        .transpose()?;
    if !segments.is_empty() {
        let object = details.get_or_insert_with(|| json!({}));
        if let Some(object) = object.as_object_mut() {
            object.insert(
                "served_segments".into(),
                to_json(&segments, "served segments")?,
            );
        }
    }
    if !applied_errata.is_empty() {
        let object = details.get_or_insert_with(|| json!({}));
        if let Some(object) = object.as_object_mut() {
            object.insert(
                "applied_errata".into(),
                Value::Array(applied_errata.to_vec()),
            );
        }
    }
    Ok(details)
}

/// Picks the snapshot to serve for `request`. Only a segment whose page range
/// fully contains the request is eligible: serving a merely overlapping
/// snapshot under the requested `pdf_pages` locator would mislabel pages the
/// body does not contain. The head wins when it covers; otherwise the newest
/// covering segment. `None` falls through to `capability_unavailable`.
fn choose_segment(
    view: &CanonicalMarkdownView,
    request: InclusivePageRange,
) -> Option<&mathscape_domain::CanonicalSegment> {
    let covers = |segment: &&mathscape_domain::CanonicalSegment| {
        segment.pages.is_some_and(|pages| pages.contains(request))
    };
    if let Some(head) = &view.head_artifact_id
        && let Some(segment) = view
            .segments
            .iter()
            .filter(covers)
            .find(|segment| &segment.markdown_artifact_id == head)
    {
        return Some(segment);
    }
    view.segments.iter().filter(covers).max_by(|left, right| {
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

#[allow(clippy::too_many_arguments)]
fn capability_unavailable(
    workspace: &McpWorkspace,
    material: &Material,
    path: &Path,
    from: u32,
    to: u32,
    page_count: u32,
    mut diagnostics: Vec<Value>,
    report: Option<&EnsureReport>,
) -> Result<Value, ToolError> {
    let probe = match report.and_then(|report| report.probe.clone()) {
        Some(probe) => probe,
        None => probe_capability(workspace.capability_runner(), Capability::Marker),
    };
    match probe_text_layer(path, page_count, from, to) {
        Some((probed_to, with_text)) if !with_text.is_empty() => diagnostics.push(json!({
            "code": "pdf_text_layer_not_served",
            "message": "these pages have a native PDF text layer, but material.read serves only canonical Marker markdown for careful reading and does not fall back to it. Use material.grep to locate text in the text layer; quote only from canonical markdown",
            "probed_pages": { "from": from, "to": probed_to },
            "pages_with_text": with_text,
        })),
        _ if probe.state != CapabilityState::Available => diagnostics.push(diagnostic(
            "pdf_no_text_layer",
            "requested pages have no text layer; install Marker to convert scanned PDFs",
        )),
        _ => {}
    }
    if let Some(report) = report {
        diagnostics.push(diagnostic(
            report.status.diagnostic_code(),
            report.message.clone(),
        ));
    }
    let details = match report {
        Some(report) => ensure_details(report, &material.id, Some(&probe))?,
        None => to_json(&probe, "capability probe")?,
    };
    result(ReadResult {
        status: ToolStatus::CapabilityUnavailable,
        artifact_id: material.id.to_string(),
        content_type: "markdown",
        text: None,
        locator: Locator::PdfPages { from, to },
        truncated: None,
        total_bytes: None,
        total_pages: Some(page_count),
        revision: Some(material.revision.clone()),
        details: Some(details),
        diagnostics,
    })
}

/// `details` for an ensured read: the probe fields (when known) plus
/// `ensure` and, when this call blocked on a Marker parse of this PDF,
/// `map_signal` (「解析中」 with how it ended, for the scene player).
fn ensure_details(
    report: &EnsureReport,
    pdf: &StableId,
    probe: Option<&mathscape_agent::CapabilityProbe>,
) -> Result<Value, ToolError> {
    let mut details = match probe {
        Some(probe) => to_json(probe, "capability probe")?,
        None => json!({}),
    };
    if let Some(object) = details.as_object_mut() {
        object.insert("ensure".into(), to_json(report, "ensure report")?);
        if let Some(signal) = report.map_signal(pdf) {
            object.insert("map_signal".into(), signal);
        }
    }
    Ok(details)
}

fn to_json(value: &impl Serialize, what: &str) -> Result<Value, ToolError> {
    serde_json::to_value(value).map_err(|error| {
        ToolError::new(
            ToolErrorCode::Internal,
            format!("failed to serialize {what}: {error}"),
        )
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

    fn segment(
        id: &str,
        from: u32,
        to: u32,
        finished_at: &str,
    ) -> mathscape_domain::CanonicalSegment {
        mathscape_domain::CanonicalSegment {
            conversion_id: StableId::parse(format!("mathscape:conversion/marker/{id}")).unwrap(),
            markdown_artifact_id: StableId::parse(format!("mathscape:artifact/test/{id}")).unwrap(),
            pages: Some(InclusivePageRange::new(from, to).unwrap()),
            finished_at: Some(finished_at.to_owned()),
        }
    }

    fn view(
        head: Option<&str>,
        segments: Vec<mathscape_domain::CanonicalSegment>,
    ) -> CanonicalMarkdownView {
        CanonicalMarkdownView {
            canonical_id: StableId::parse("mathscape:canonical-markdown/test").unwrap(),
            source_pdf_artifact_id: StableId::parse("mathscape:artifact/test/pdf").unwrap(),
            parser_profile: "profile".to_owned(),
            parser_policy_version: "marker-2.0.0".to_owned(),
            head_artifact_id: head
                .map(|id| StableId::parse(format!("mathscape:artifact/test/{id}")).unwrap()),
            head_revision: None,
            total_pages: Some(60),
            segments,
        }
    }

    #[test]
    fn choose_segment_requires_full_containment_not_overlap() {
        let request = InclusivePageRange::new(12, 40).unwrap();
        // Head snapshot only overlaps (30–50): must not be served as 12–40.
        let only_overlap = view(
            Some("late"),
            vec![segment("late", 30, 50, "2026-11-02T01:00:00Z")],
        );
        assert!(choose_segment(&only_overlap, request).is_none());

        // A non-covering head is skipped in favour of a covering segment.
        let fallback = view(
            Some("late"),
            vec![
                segment("late", 30, 50, "2026-11-02T02:00:00Z"),
                segment("wide", 1, 45, "2026-11-02T01:00:00Z"),
            ],
        );
        let chosen = choose_segment(&fallback, request).unwrap();
        assert_eq!(
            chosen.markdown_artifact_id.as_str(),
            "mathscape:artifact/test/wide"
        );

        // A covering head still wins over a newer covering segment.
        let head_covers = view(
            Some("head"),
            vec![
                segment("head", 10, 40, "2026-11-02T01:00:00Z"),
                segment("newer", 1, 60, "2026-11-02T03:00:00Z"),
            ],
        );
        let chosen = choose_segment(&head_covers, request).unwrap();
        assert_eq!(
            chosen.markdown_artifact_id.as_str(),
            "mathscape:artifact/test/head"
        );

        // Exact bounds count as coverage.
        let exact = view(None, vec![segment("exact", 12, 40, "2026-11-02T01:00:00Z")]);
        assert!(choose_segment(&exact, request).is_some());
    }

    #[test]
    fn piece_bytes_are_cut_at_absolute_page_markers_or_kept_whole() {
        let piece = |from, to, sf, st| PageCoverPiece {
            view_index: 0,
            conversion_id: StableId::parse("mathscape:conversion/c").unwrap(),
            markdown_artifact_id: StableId::parse("mathscape:artifact/m").unwrap(),
            snapshot_pages: InclusivePageRange::new(sf, st).unwrap(),
            pages: InclusivePageRange::new(from, to).unwrap(),
            is_head: false,
        };
        let dashes = "-".repeat(16);
        let source = format!("{{4}}{dashes}\nfive\n{{5}}{dashes}\nsix\n{{6}}{dashes}\nseven\n");
        let (range, exact) = piece_byte_range(&source, &piece(6, 6, 5, 7));
        assert!(exact);
        assert!(source[range.start..range.end].contains("six"));
        assert!(!source[range.start..range.end].contains("five"));
        assert!(!source[range.start..range.end].contains("seven"));
        // Whole snapshot: no markers needed.
        let (range, exact) = piece_byte_range("plain", &piece(5, 7, 5, 7));
        assert_eq!((range, exact), (ByteRange { start: 0, end: 5 }, true));
        // No markers but a sub-range: kept whole, reported untrimmed.
        let (range, exact) = piece_byte_range("plain", &piece(6, 7, 5, 7));
        assert_eq!((range, exact), (ByteRange { start: 0, end: 5 }, false));
        assert_eq!(cap_at_char_boundary("数学", 4), "数");
    }
}
