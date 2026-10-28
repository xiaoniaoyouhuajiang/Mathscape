//! `ink.page_image` (Phase1d-b, `docs/phase1d-ink-snapshot-contract.md` §3):
//! renders one page of a note head, an ink snapshot or an ink evidence span
//! as a PNG with [`mathscape_ink_raster`].
//!
//! Read-only: every call opens a fresh catalog connection, reads the note or
//! the snapshot manifest, verifies the manifest hash and `validate()`, and
//! renders in memory. Nothing is cached, written or printed. Successful image
//! results bypass `structuredContent` (see [`super::ToolOutput::Image`]).

use mathscape_agent::{ToolError, ToolErrorCode, ToolStatus};
use mathscape_domain::{EntityKind, InkLabDocument, InkLabPage, InkLabStroke, InkSnapshotManifest};
use mathscape_ink_raster::{
    InkStrokeInput, Outline, RENDERER_ID, Rect, StrokeOptions, image_size, ink_bounds,
    outline_stroke, outline_touches, parse_hex_color, render_png_within,
};
use mathscape_store_sqlite::{InkLifecycle, SqliteArtifactStore, StoredInkSnapshot};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::graph::{
    INK_SNAPSHOT_PREFIX, as_of, bounded, canonicalized, inner_payload, invalid, status_value,
    str_at,
};
use super::{INK_PAGE_IMAGE, ToolOutput, parse_args};
use crate::McpWorkspace;

pub const DEFAULT_MAX_EDGE_PX: usize = 1024;
pub const MIN_MAX_EDGE_PX: usize = 256;
pub const MAX_MAX_EDGE_PX: usize = 2048;
pub const DEFAULT_MAX_BYTES: usize = 524_288;
pub const MIN_MAX_BYTES: usize = 32_768;
pub const MAX_MAX_BYTES: usize = 1_048_576;
pub const MAX_PAGE_INDEX: usize = 99;
pub const MAX_NODE_ID_CHARS: usize = 200;
/// Margin (page units) around ink for `crop:"strokes"` / `"selection"`.
pub const CROP_MARGIN: f64 = 24.0;
/// Opacity of span strokes outside the selection.
pub const FADED_OPACITY: f32 = 0.25;
const MAX_PAGE_IDS: usize = 100;
const MAX_MISSING_STROKE_IDS: usize = 20;
const ARTIFACT_PREFIX: &str = "mathscape:artifact/";
const INK_EVIDENCE_PREFIX: &str = "mathscape:ink-evidence/";

const DESCRIPTION: &str = "Render one page of a handwritten note, an ink snapshot, or an ink evidence span as a PNG the model can see. node_id accepts a note document_id, a mathscape:ink-snapshot/… id, its snapshot artifact id, or a mathscape:ink-evidence/… span id (pass graph content_ref.node_id unchanged). Notes render their current head (live, not frozen; cite a snapshot for stable evidence); snapshots and spans render immutable manifests. page_id or page_index (0-based) picks the page; default is the span's page, else page 0. crop defaults to page (span: selection, other strokes faded). max_edge_px defaults to 1024 (256..2048), max_bytes to 524288 (32768..1048576); over a hard cap is invalid_arg. include_image=false returns metadata only. Output: one text block with JSON metadata, then one image/png block; no body text, no OCR. Read-only.";

pub(super) fn definition() -> Value {
    json!({
        "name": INK_PAGE_IMAGE,
        "description": DESCRIPTION,
        "inputSchema": {
            "type": "object",
            "additionalProperties": false,
            "required": ["node_id"],
            "properties": {
                "node_id": { "type": "string", "minLength": 1, "maxLength": MAX_NODE_ID_CHARS },
                "page_id": { "type": "string", "minLength": 1, "maxLength": 200 },
                "page_index": { "type": "integer", "minimum": 0, "maximum": MAX_PAGE_INDEX },
                "crop": { "type": "string", "enum": ["page", "strokes", "selection"] },
                "bounds": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["x", "y", "width", "height"],
                    "properties": {
                        "x": { "type": "number" },
                        "y": { "type": "number" },
                        "width": { "type": "number", "exclusiveMinimum": 0 },
                        "height": { "type": "number", "exclusiveMinimum": 0 },
                    },
                },
                "format": { "type": "string", "enum": ["png"], "default": "png" },
                "max_edge_px": {
                    "type": "integer",
                    "minimum": MIN_MAX_EDGE_PX,
                    "maximum": MAX_MAX_EDGE_PX,
                    "default": DEFAULT_MAX_EDGE_PX,
                },
                "max_bytes": {
                    "type": "integer",
                    "minimum": MIN_MAX_BYTES,
                    "maximum": MAX_MAX_BYTES,
                    "default": DEFAULT_MAX_BYTES,
                },
                "include_image": { "type": "boolean", "default": true },
                "include_trashed": { "type": "boolean", "default": false },
            },
        },
    })
}

// ---------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    node_id: String,
    page_id: Option<String>,
    page_index: Option<u64>,
    crop: Option<String>,
    bounds: Option<BoundsArg>,
    format: Option<String>,
    max_edge_px: Option<u64>,
    max_bytes: Option<u64>,
    #[serde(default = "default_true")]
    include_image: bool,
    #[serde(default)]
    include_trashed: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundsArg {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CropMode {
    Page,
    Strokes,
    Selection,
}

enum PageChoice {
    Default,
    Id(String),
    Index(usize),
}

struct Applied {
    page: PageChoice,
    crop: Option<CropMode>,
    bounds: Option<Rect>,
    max_edge_px: u32,
    max_bytes: usize,
    include_image: bool,
    include_trashed: bool,
}

fn validate(args: &Args) -> Result<Applied, ToolError> {
    let chars = args.node_id.chars().count();
    if chars == 0 || chars > MAX_NODE_ID_CHARS {
        return Err(invalid(
            format!("node_id must be 1..{MAX_NODE_ID_CHARS} characters"),
            json!({ "field": "node_id", "min": 1, "max": MAX_NODE_ID_CHARS }),
        ));
    }
    let page = match (&args.page_id, args.page_index) {
        (Some(_), Some(_)) => {
            return Err(invalid(
                "page_id and page_index are mutually exclusive",
                json!({ "field": "page_id", "conflicts_with": "page_index" }),
            ));
        }
        (Some(id), None) => {
            let chars = id.chars().count();
            if chars == 0 || chars > 200 {
                return Err(invalid(
                    "page_id must be 1..200 characters",
                    json!({ "field": "page_id", "min": 1, "max": 200 }),
                ));
            }
            PageChoice::Id(id.clone())
        }
        (None, Some(index)) => {
            PageChoice::Index(bounded("page_index", Some(index), 0, 0, MAX_PAGE_INDEX)?)
        }
        (None, None) => PageChoice::Default,
    };
    let crop = match args.crop.as_deref() {
        None => None,
        Some("page") => Some(CropMode::Page),
        Some("strokes") => Some(CropMode::Strokes),
        Some("selection") => Some(CropMode::Selection),
        Some(other) => {
            return Err(invalid(
                format!("crop must be page, strokes or selection, got {other}"),
                json!({ "field": "crop", "value": other, "allowed": ["page", "strokes", "selection"] }),
            ));
        }
    };
    let bounds = match args.bounds {
        None => None,
        Some(BoundsArg {
            x,
            y,
            width,
            height,
        }) => {
            if ![x, y, width, height].iter().all(|value| value.is_finite())
                || width <= 0.0
                || height <= 0.0
            {
                return Err(invalid(
                    "bounds must be finite with width > 0 and height > 0",
                    json!({ "field": "bounds" }),
                ));
            }
            Some(Rect {
                x,
                y,
                width,
                height,
            })
        }
    };
    if let Some(format) = args.format.as_deref()
        && format != "png"
    {
        return Err(invalid(
            format!("format {format} is not supported"),
            json!({ "field": "format", "value": format, "supported_formats": ["png"] }),
        ));
    }
    let max_edge_px = bounded(
        "max_edge_px",
        args.max_edge_px,
        DEFAULT_MAX_EDGE_PX,
        MIN_MAX_EDGE_PX,
        MAX_MAX_EDGE_PX,
    )?;
    let max_bytes = bounded(
        "max_bytes",
        args.max_bytes,
        DEFAULT_MAX_BYTES,
        MIN_MAX_BYTES,
        MAX_MAX_BYTES,
    )?;
    Ok(Applied {
        page,
        crop,
        bounds,
        max_edge_px: u32::try_from(max_edge_px).unwrap_or(MAX_MAX_EDGE_PX as u32),
        max_bytes,
        include_image: args.include_image,
        include_trashed: args.include_trashed,
    })
}

// ---------------------------------------------------------------------------
// Output DTOs (field order is the serialized order, contract §3.4)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct Output {
    status: Value,
    truncated: bool,
    as_of: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<Source>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    page: Option<PageOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    page_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    page_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    crop: Option<CropOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    strokes: Option<StrokesOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<Option<ImageOut>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    renderer: Option<&'static str>,
    diagnostics: Vec<Value>,
}

impl Output {
    fn new(status: ToolStatus) -> Self {
        Self {
            status: status_value(status),
            truncated: false,
            as_of: as_of(),
            node_id: None,
            source: None,
            revision: None,
            page: None,
            page_count: None,
            page_ids: None,
            crop: None,
            strokes: None,
            image: None,
            renderer: None,
            diagnostics: Vec::new(),
        }
    }
}

#[derive(Clone, Serialize)]
struct Source {
    kind: &'static str,
    node_id: String,
    snapshot_id: Option<String>,
    artifact_id: Option<String>,
    document_id: String,
    note_title: Option<String>,
    label: Option<String>,
    through_seq: Option<u64>,
    manifest_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    span_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    head_seq: Option<u64>,
}

#[derive(Serialize)]
struct PageOut {
    id: String,
    index: usize,
    width: f64,
    height: f64,
}

#[derive(Serialize)]
struct CropOut {
    mode: &'static str,
    bounds: RectOut,
}

#[derive(Serialize)]
struct RectOut {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl From<Rect> for RectOut {
    fn from(rect: Rect) -> Self {
        Self {
            x: round3(rect.x),
            y: round3(rect.y),
            width: round3(rect.width),
            height: round3(rect.height),
        }
    }
}

#[derive(Serialize)]
struct StrokesOut {
    visible_on_page: usize,
    rendered: usize,
    selected: Option<usize>,
    ink_bounds: Option<RectOut>,
}

#[derive(Serialize)]
struct ImageOut {
    format: &'static str,
    width: u32,
    height: u32,
    scale: f64,
    bytes: Option<usize>,
    sha256: Option<String>,
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn finish(output: Output, png: Option<Vec<u8>>) -> Result<ToolOutput, ToolError> {
    let text = serde_json::to_string(&output).map_err(|error| {
        ToolError::new(
            ToolErrorCode::Internal,
            format!("failed to serialize ink.page_image metadata: {error}"),
        )
    })?;
    match png {
        Some(png) => Ok(ToolOutput::Image {
            text,
            png,
            mime_type: "image/png",
        }),
        None => {
            let structured = serde_json::to_value(&output).map_err(|error| {
                ToolError::new(
                    ToolErrorCode::Internal,
                    format!("failed to serialize ink.page_image metadata: {error}"),
                )
            })?;
            Ok(ToolOutput::Text { text, structured })
        }
    }
}

fn internal(reason: &'static str, message: impl Into<String>) -> ToolError {
    ToolError::new(ToolErrorCode::Internal, message).with_details(json!({ "reason": reason }))
}

fn store_error(error: impl std::fmt::Display) -> ToolError {
    ToolError::new(
        ToolErrorCode::Internal,
        format!("failed to read the ink store: {error}"),
    )
}

fn diag(code: &str, message: impl Into<String>, details: Value) -> Value {
    json!({ "code": code, "message": message.into(), "details": details })
}

// ---------------------------------------------------------------------------
// node_id resolution (§3.3)
// ---------------------------------------------------------------------------

struct Span {
    span_id: String,
    revision: String,
    page_id: Option<String>,
    stroke_ids: Vec<String>,
}

enum Node {
    Note(String),
    Snapshot(StoredInkSnapshot),
    Span(StoredInkSnapshot, Span),
}

// Short-lived per call; boxing would only add an allocation.
#[allow(clippy::large_enum_variant)]
enum Resolution {
    Found(Node),
    NotFound(&'static str),
}

fn is_document_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn not_ink(node_kind: &str) -> ToolError {
    invalid(
        format!("node_id is a {node_kind}, not a note, ink snapshot or ink span"),
        json!({
            "field": "node_id",
            "node_kind": node_kind,
            "hint": "use material.read with content_ref",
        }),
    )
}

fn resolve(
    store: &SqliteArtifactStore,
    node_id: &str,
    diagnostics: &mut Vec<Value>,
) -> Result<Resolution, ToolError> {
    if is_document_id(node_id) {
        return Ok(
            match store.open_ink_document(node_id).map_err(store_error)? {
                Some(_) => Resolution::Found(Node::Note(node_id.to_owned())),
                None => Resolution::NotFound("note_not_found"),
            },
        );
    }
    if let Err(error) = mathscape_domain::StableId::parse(node_id) {
        return Err(invalid(
            format!("node_id is not a note document_id or a valid stable id: {error}"),
            json!({
                "field": "node_id",
                "accepted_forms": [
                    "32-hex note document_id",
                    "mathscape:ink-snapshot/<hash>",
                    "mathscape:artifact/sha256/<hash> (ink snapshot artifact)",
                    "mathscape:ink-evidence/<hash>",
                ],
            }),
        ));
    }
    if node_id.starts_with(INK_SNAPSHOT_PREFIX) {
        return Ok(
            match store.open_ink_snapshot(node_id).map_err(store_error)? {
                Some(snapshot) => Resolution::Found(Node::Snapshot(snapshot)),
                None => Resolution::NotFound("snapshot_not_found"),
            },
        );
    }
    if node_id.starts_with(ARTIFACT_PREFIX) {
        if let Some(snapshot) = store
            .ink_snapshot_for_artifact(node_id)
            .map_err(store_error)?
        {
            diagnostics.push(canonicalized(node_id, &snapshot.snapshot_id));
            return Ok(Resolution::Found(Node::Snapshot(snapshot)));
        }
        let catalog = store.read_catalog_snapshot().map_err(store_error)?;
        if catalog
            .artifacts()
            .iter()
            .any(|record| record.entity().header().stable_id().as_str() == node_id)
        {
            return Err(not_ink("artifact"));
        }
        return Ok(Resolution::NotFound("artifact_not_found"));
    }
    let objects = store.map_objects().map_err(store_error)?;
    let Some(object) = objects
        .iter()
        .find(|object| object.stable_id.as_str() == node_id)
    else {
        return Ok(Resolution::NotFound(
            if node_id.starts_with(INK_EVIDENCE_PREFIX) {
                "span_not_found"
            } else {
                "node_not_found"
            },
        ));
    };
    let payload = inner_payload(&object.payload);
    let ink_artifact = str_at(payload, "/ink/snapshot_artifact");
    let node_kind = if object.relation {
        "relation"
    } else {
        match object.kind {
            EntityKind::SourceSpan => "source_span",
            EntityKind::SourceSemanticUnit => "source_unit",
            EntityKind::SourceArtifact => "artifact",
            _ => "unsupported",
        }
    };
    let Some(artifact) = ink_artifact.filter(|_| node_kind == "source_span") else {
        return Err(not_ink(node_kind));
    };
    let Some(snapshot) = store
        .ink_snapshot_for_artifact(artifact)
        .map_err(store_error)?
    else {
        diagnostics.push(diag(
            "snapshot_missing",
            "the span's snapshot artifact is not an ink snapshot in this workspace",
            json!({ "snapshot_artifact": artifact }),
        ));
        return Ok(Resolution::NotFound("snapshot_not_found"));
    };
    let span = Span {
        span_id: object.stable_id.as_str().to_owned(),
        revision: object.revision_id.as_str().to_owned(),
        page_id: str_at(payload, "/ink/page_id").map(str::to_owned),
        stroke_ids: payload
            .pointer("/ink/stroke_ids")
            .and_then(Value::as_array)
            .map(|ids| {
                ids.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
    };
    Ok(Resolution::Found(Node::Span(snapshot, span)))
}

/// Verified manifest of a snapshot: hash, parse, `validate()`.
fn verified_manifest(snapshot: &StoredInkSnapshot) -> Result<InkSnapshotManifest, ToolError> {
    let digest = Sha256::digest(snapshot.manifest_json.as_bytes());
    let computed = format!("sha256:{}", hex(&digest));
    if computed != snapshot.content_hash {
        return Err(internal(
            "integrity_failed",
            "ink snapshot manifest does not match its content hash",
        ));
    }
    let manifest: InkSnapshotManifest = serde_json::from_str(&snapshot.manifest_json)
        .map_err(|_| internal("invalid_manifest", "ink snapshot manifest does not parse"))?;
    manifest
        .validate()
        .map_err(|_| internal("invalid_manifest", "ink snapshot manifest is invalid"))?;
    if manifest.document_id != snapshot.document_id {
        return Err(internal(
            "invalid_manifest",
            "ink snapshot manifest belongs to another note",
        ));
    }
    Ok(manifest)
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

fn stroke_input(stroke: &InkLabStroke, opacity: f32) -> Result<InkStrokeInput, ToolError> {
    let color = parse_hex_color(&stroke.brush.color)
        .ok_or_else(|| internal("invalid_manifest", "ink stroke colour is invalid"))?;
    Ok(InkStrokeInput {
        points: stroke
            .points
            .iter()
            .map(|point| [point.x, point.y, point.pressure.unwrap_or(0.5)])
            .collect(),
        options: StrokeOptions {
            size: stroke.brush.width,
            thinning: stroke.brush.thinning,
            smoothing: stroke.brush.smoothing,
            streamline: stroke.brush.streamline,
        },
        color,
        opacity,
    })
}

// ---------------------------------------------------------------------------
// Call
// ---------------------------------------------------------------------------

pub(super) fn call(workspace: &McpWorkspace, arguments: Value) -> Result<ToolOutput, ToolError> {
    let args: Args = parse_args(arguments)?;
    let applied = validate(&args)?;
    let store = workspace.store()?;
    let mut diagnostics = Vec::new();

    let node = match resolve(&store, &args.node_id, &mut diagnostics)? {
        Resolution::Found(node) => node,
        Resolution::NotFound(reason) => {
            let mut output = Output::new(ToolStatus::NotFound);
            output.node_id = Some(args.node_id.clone());
            diagnostics.push(diag(
                reason,
                "no note, ink snapshot or ink span with this id",
                json!({ "node_id": args.node_id }),
            ));
            output.diagnostics = diagnostics;
            return finish(output, None);
        }
    };

    // Load the document to render and describe its source.
    let (document, source, revision, span): (InkLabDocument, Source, Option<String>, Option<Span>) =
        match node {
            Node::Note(document_id) => {
                let stored = store
                    .open_ink_document(&document_id)
                    .map_err(store_error)?
                    .ok_or_else(|| {
                        ToolError::new(ToolErrorCode::Internal, "ink note vanished while reading")
                    })?;
                if stored.note.lifecycle == InkLifecycle::Trashed && !applied.include_trashed {
                    return trashed(&args.node_id, diagnostics);
                }
                stored
                    .document
                    .validate()
                    .map_err(|_| internal("invalid_manifest", "ink note document is invalid"))?;
                diagnostics.push(diag(
                    "live_note_head",
                    "rendered the note's current head (live, not frozen); cite a snapshot for stable evidence",
                    json!({ "head_seq": stored.head_seq }),
                ));
                let source = Source {
                    kind: "note",
                    node_id: document_id.clone(),
                    snapshot_id: None,
                    artifact_id: None,
                    document_id,
                    note_title: Some(stored.note.title.clone()),
                    label: None,
                    through_seq: None,
                    manifest_version: None,
                    span_id: None,
                    head_seq: Some(stored.head_seq),
                };
                (stored.document, source, None, None)
            }
            Node::Snapshot(snapshot) => {
                let (source, document) = snapshot_source(&store, &snapshot, &applied)?;
                let Some(document) = document else {
                    return trashed(&args.node_id, diagnostics);
                };
                (document, source, Some(snapshot.content_hash.clone()), None)
            }
            Node::Span(snapshot, span) => {
                let (mut source, document) = snapshot_source(&store, &snapshot, &applied)?;
                let Some(document) = document else {
                    return trashed(&args.node_id, diagnostics);
                };
                source.kind = "span";
                source.node_id = span.span_id.clone();
                source.span_id = Some(span.span_id.clone());
                (document, source, Some(span.revision.clone()), Some(span))
            }
        };
    drop(store);

    // Page.
    let pages = &document.pages;
    let page_count = pages.len();
    let found = match &applied.page {
        PageChoice::Id(id) => pages.iter().position(|page| &page.id == id),
        PageChoice::Index(index) => (*index < page_count).then_some(*index),
        PageChoice::Default => match span.as_ref().and_then(|span| span.page_id.as_deref()) {
            Some(id) => pages.iter().position(|page| page.id == id),
            None => (page_count > 0).then_some(0),
        },
    };
    let Some(page_index) = found else {
        let mut output = Output::new(ToolStatus::NotFound);
        output.source = Some(source);
        output.revision = revision;
        output.page_count = Some(page_count);
        output.page_ids = Some(
            pages
                .iter()
                .take(MAX_PAGE_IDS)
                .map(|page| page.id.clone())
                .collect(),
        );
        diagnostics.push(diag(
            "page_not_found",
            "no such page in this document; use page_ids or a page_index below page_count",
            json!({ "page_count": page_count }),
        ));
        output.diagnostics = diagnostics;
        return finish(output, None);
    };
    let page: &InkLabPage = &pages[page_index];
    let page_rect = Rect {
        x: 0.0,
        y: 0.0,
        width: page.width,
        height: page.height,
    };
    let on_span_page = span
        .as_ref()
        .is_some_and(|span| span.page_id.as_deref() == Some(page.id.as_str()));

    // Crop mode validation (selection needs a span on its own page).
    let mode = match applied.crop {
        Some(CropMode::Selection) if span.is_none() => {
            return Err(invalid(
                "crop selection is only valid for an ink evidence span",
                json!({ "field": "crop", "value": "selection" }),
            ));
        }
        Some(CropMode::Selection) if !on_span_page => {
            return Err(invalid(
                "crop selection is only valid on the span's own page",
                json!({
                    "field": "crop",
                    "value": "selection",
                    "span_page_id": span.as_ref().and_then(|span| span.page_id.clone()),
                }),
            ));
        }
        Some(mode) => mode,
        None if on_span_page => CropMode::Selection,
        None => CropMode::Page,
    };

    // Project and outline the page's visible strokes.
    let visible = document
        .project_visible_strokes()
        .map_err(|_| internal("invalid_manifest", "ink document does not project"))?;
    let selection: Option<&[String]> = span
        .as_ref()
        .filter(|_| on_span_page)
        .map(|span| span.stroke_ids.as_slice());
    let mut outlines: Vec<Outline> = Vec::new();
    let mut selected_outlines: Vec<Outline> = Vec::new();
    let mut selected_ids: Vec<&str> = Vec::new();
    for stroke in visible.iter().filter(|stroke| stroke.page_id == page.id) {
        let chosen = selection.is_none_or(|ids| ids.contains(&stroke.id));
        let opacity = if chosen { 1.0 } else { FADED_OPACITY };
        let outline = outline_stroke(&stroke_input(stroke, opacity)?);
        if selection.is_some() && chosen {
            selected_ids.push(&stroke.id);
            selected_outlines.push(outline.clone());
        }
        outlines.push(outline);
    }
    let visible_on_page = outlines.len();
    let page_ink = ink_bounds(&outlines);
    if visible_on_page == 0 {
        diagnostics.push(diag(
            "page_blank",
            "this page has no visible strokes; the image is blank",
            json!({ "page_id": page.id }),
        ));
    }
    if let Some(ids) = selection {
        let missing: Vec<&String> = ids
            .iter()
            .filter(|id| !selected_ids.contains(&id.as_str()))
            .collect();
        if !ids.is_empty() && missing.len() == ids.len() {
            diagnostics.push(diag(
                "selection_hidden",
                "none of the span's strokes are visible in this snapshot",
                json!({ "stroke_count": ids.len() }),
            ));
        } else if !missing.is_empty() {
            diagnostics.push(diag(
                "selection_partially_hidden",
                "some of the span's strokes are not visible in this snapshot",
                json!({
                    "missing_count": missing.len(),
                    "missing_stroke_ids": missing.iter().take(MAX_MISSING_STROKE_IDS).collect::<Vec<_>>(),
                }),
            ));
        }
    }

    // Crop rectangle.
    let (mode_name, crop) = if let Some(bounds) = applied.bounds {
        let Some(crop) = bounds.intersect(&page_rect) else {
            return Err(invalid(
                "bounds do not intersect the page",
                json!({ "field": "bounds", "page": { "width": page.width, "height": page.height } }),
            ));
        };
        ("bounds", crop)
    } else {
        match mode {
            CropMode::Page => ("page", page_rect),
            CropMode::Strokes => (
                "strokes",
                page_ink
                    .and_then(|ink| ink.inflate(CROP_MARGIN).intersect(&page_rect))
                    .unwrap_or(page_rect),
            ),
            CropMode::Selection => (
                "selection",
                ink_bounds(&selected_outlines)
                    .and_then(|ink| ink.inflate(CROP_MARGIN).intersect(&page_rect))
                    .unwrap_or(page_rect),
            ),
        }
    };
    let rendered = outlines
        .iter()
        .filter(|outline| outline_touches(outline, &crop))
        .count();

    let mut output = Output::new(ToolStatus::Ok);
    output.source = Some(source);
    output.revision = revision;
    output.page = Some(PageOut {
        id: page.id.clone(),
        index: page_index,
        width: page.width,
        height: page.height,
    });
    output.page_count = Some(page_count);
    output.crop = Some(CropOut {
        mode: mode_name,
        bounds: crop.into(),
    });
    output.strokes = Some(StrokesOut {
        visible_on_page,
        rendered,
        selected: selection.map(|_| selected_ids.len()),
        ink_bounds: page_ink.map(RectOut::from),
    });
    output.renderer = Some(RENDERER_ID);

    if !applied.include_image {
        let (width, height, scale) = image_size(&crop, applied.max_edge_px);
        output.image = Some(Some(ImageOut {
            format: "png",
            width,
            height,
            scale,
            bytes: None,
            sha256: None,
        }));
        output.diagnostics = diagnostics;
        return finish(output, None);
    }

    let fitted = render_png_within(&outlines, &crop, applied.max_edge_px, applied.max_bytes)
        .map_err(|_| internal("render_failed", "the ink renderer failed"))?;
    let Some(image) = fitted.image else {
        output.status = status_value(ToolStatus::Truncated);
        output.truncated = true;
        output.image = Some(None);
        diagnostics.push(diag(
            "image_too_large",
            "the PNG is over max_bytes even at the smallest edge; pass bounds or crop:\"strokes\" to render less, or raise max_bytes",
            json!({
                "max_bytes": applied.max_bytes,
                "requested_edge_px": fitted.requested_edge_px,
                "smallest_edge_px": fitted.returned_edge_px,
                "smallest_bytes": fitted.last_bytes,
            }),
        ));
        output.diagnostics = diagnostics;
        return finish(output, None);
    };
    if fitted.returned_edge_px != fitted.requested_edge_px {
        diagnostics.push(diag(
            "image_downscaled",
            "the PNG was over max_bytes at the requested edge and was re-rendered smaller",
            json!({
                "requested_edge_px": fitted.requested_edge_px,
                "returned_edge_px": fitted.returned_edge_px,
                "reason": "max_bytes",
            }),
        ));
    }
    output.image = Some(Some(ImageOut {
        format: "png",
        width: image.width,
        height: image.height,
        scale: image.scale,
        bytes: Some(image.png.len()),
        sha256: Some(hex(&Sha256::digest(&image.png))),
    }));
    output.diagnostics = diagnostics;
    finish(output, Some(image.png))
}

/// Snapshot (or span) source; `None` document when its note is trashed and
/// `include_trashed` is false.
fn snapshot_source(
    store: &SqliteArtifactStore,
    snapshot: &StoredInkSnapshot,
    applied: &Applied,
) -> Result<(Source, Option<InkLabDocument>), ToolError> {
    let note = store
        .open_ink_document(&snapshot.document_id)
        .map_err(store_error)?
        .map(|stored| stored.note);
    let source = Source {
        kind: "snapshot",
        node_id: snapshot.snapshot_id.clone(),
        snapshot_id: Some(snapshot.snapshot_id.clone()),
        artifact_id: Some(snapshot.artifact_id.clone()),
        document_id: snapshot.document_id.clone(),
        note_title: note.as_ref().map(|note| note.title.clone()),
        label: snapshot.label.clone(),
        through_seq: Some(snapshot.through_seq),
        manifest_version: None,
        span_id: None,
        head_seq: None,
    };
    if note
        .as_ref()
        .is_some_and(|note| note.lifecycle == InkLifecycle::Trashed)
        && !applied.include_trashed
    {
        return Ok((source, None));
    }
    let manifest = verified_manifest(snapshot)?;
    let source = Source {
        manifest_version: Some(manifest.version.clone()),
        ..source
    };
    Ok((source, Some(manifest.document)))
}

fn trashed(node_id: &str, mut diagnostics: Vec<Value>) -> Result<ToolOutput, ToolError> {
    let mut output = Output::new(ToolStatus::NotFound);
    output.node_id = Some(node_id.to_owned());
    diagnostics.push(diag(
        "trashed",
        "the note is in the trash; pass include_trashed=true to render it",
        json!({ "node_id": node_id }),
    ));
    output.diagnostics = diagnostics;
    finish(output, None)
}
