//! `pdf.page_image`: renders one page (or a normalized crop) of a PDF
//! material as a PNG with the pure-Rust rasterizer in [`mathscape_pdf`].
//!
//! Read-only: resolves the managed copy, rasterizes in memory and returns the
//! bytes. It never runs Marker or OCR, never ensures or writes canonical
//! markdown and never touches the PDF. Successful image results bypass
//! `structuredContent` exactly like `ink.page_image`
//! (see [`super::ToolOutput::Image`]).

use mathscape_agent::{ToolError, ToolErrorCode, ToolStatus};
use mathscape_domain::StableId;
use mathscape_pdf::{NormalizedRect, PDF_RENDERER, PdfEngine, PdfError, PdfPageBox, PdfPageIndex};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::graph::{as_of, bounded, invalid, status_value};
use super::ink::{
    DEFAULT_MAX_BYTES, DEFAULT_MAX_EDGE_PX, MAX_MAX_BYTES, MAX_MAX_EDGE_PX, MIN_MAX_BYTES,
    MIN_MAX_EDGE_PX,
};
use super::material::{self, ManagedPathFailure, PDF_MEDIA_TYPE, find_material};
use super::{PDF_PAGE_IMAGE, ToolOutput, parse_args};
use crate::McpWorkspace;

const DESCRIPTION: &str = "Render one page of a PDF material, or a crop of it, as a PNG the model can see, to check layout, figures, diagrams or formulas against canonical markdown. artifact_id is an application/pdf material id. page is 1-based, the same numbering as material.read pdf_pages {from, to} (pdf.outline pdf_page_index + 1). crop is optional {left, top, right, bottom}, normalized to [0, 1] on the displayed page (rotation applied, origin top-left, left < right, top < bottom); default is the whole page. max_edge_px defaults to 1024 (256..2048) and bounds the longest image edge; max_bytes defaults to 524288 (32768..1048576), and an over-budget PNG is re-rendered smaller (image_downscaled) or returned without image as status truncated (image_too_large). include_image=false returns metadata only. A page beyond page_count is not_found with page_count; a non-PDF id is invalid_arg. Output: one text block with JSON metadata (page, page_count, page_size in points, crop, image width/height/scale/bytes/sha256, renderer), then one image/png block; no text extraction, no OCR. Read-only: never runs Marker, never writes or ensures canonical markdown, never modifies the PDF.";

pub(super) fn definition() -> Value {
    json!({
        "name": PDF_PAGE_IMAGE,
        "description": DESCRIPTION,
        "inputSchema": {
            "type": "object",
            "additionalProperties": false,
            "required": ["artifact_id", "page"],
            "properties": {
                "artifact_id": {
                    "type": "string",
                    "description": "Stable id of a PDF material (application/pdf).",
                },
                "page": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "1-based PDF page, same numbering as material.read pdf_pages.",
                },
                "crop": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["left", "top", "right", "bottom"],
                    "properties": {
                        "left": { "type": "number", "minimum": 0, "maximum": 1 },
                        "top": { "type": "number", "minimum": 0, "maximum": 1 },
                        "right": { "type": "number", "minimum": 0, "maximum": 1 },
                        "bottom": { "type": "number", "minimum": 0, "maximum": 1 },
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
            },
        },
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    artifact_id: String,
    page: u64,
    crop: Option<CropArg>,
    format: Option<String>,
    max_edge_px: Option<u64>,
    max_bytes: Option<u64>,
    #[serde(default = "default_true")]
    include_image: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CropArg {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

#[derive(Serialize)]
struct Output {
    status: Value,
    truncated: bool,
    as_of: String,
    artifact_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
    /// 1-based, like `pdf_pages`.
    #[serde(skip_serializing_if = "Option::is_none")]
    page: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    page_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    page_size: Option<PageSize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    crop: Option<CropOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<Option<ImageOut>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    renderer: Option<&'static str>,
    diagnostics: Vec<Value>,
}

impl Output {
    fn new(status: ToolStatus, artifact_id: &str) -> Self {
        Self {
            status: status_value(status),
            truncated: false,
            as_of: as_of(),
            artifact_id: artifact_id.to_owned(),
            revision: None,
            page: None,
            page_count: None,
            page_size: None,
            crop: None,
            image: None,
            renderer: None,
            diagnostics: Vec::new(),
        }
    }
}

#[derive(Serialize)]
struct PageSize {
    width: f64,
    height: f64,
    unit: &'static str,
    rotation: u16,
}

#[derive(Serialize)]
struct CropOut {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

#[derive(Serialize)]
struct ImageOut {
    format: &'static str,
    width: u32,
    height: u32,
    /// Pixels per displayed PDF point.
    scale: f64,
    bytes: Option<usize>,
    sha256: Option<String>,
}

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

fn finish(output: Output, png: Option<Vec<u8>>) -> Result<ToolOutput, ToolError> {
    let serialize_error = |error: serde_json::Error| {
        ToolError::new(
            ToolErrorCode::Internal,
            format!("failed to serialize pdf.page_image metadata: {error}"),
        )
    };
    let text = serde_json::to_string(&output).map_err(serialize_error)?;
    match png {
        Some(png) => Ok(ToolOutput::Image {
            text,
            png,
            mime_type: "image/png",
        }),
        None => {
            let structured = serde_json::to_value(&output).map_err(serialize_error)?;
            Ok(ToolOutput::Text { text, structured })
        }
    }
}

fn diag(code: &str, message: impl Into<String>, details: Value) -> Value {
    json!({ "code": code, "message": message.into(), "details": details })
}

fn not_found(
    mut output: Output,
    reason: &str,
    message: &str,
    details: Value,
) -> Result<ToolOutput, ToolError> {
    output.status = status_value(ToolStatus::NotFound);
    output.diagnostics.push(diag(reason, message, details));
    finish(output, None)
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

fn render_error(error: PdfError) -> ToolError {
    let reason = match error {
        PdfError::Encrypted => "encrypted",
        PdfError::NotPdf(_) | PdfError::InvalidStructure | PdfError::Parse(_) => "unreadable_pdf",
        _ => "render_failed",
    };
    ToolError::new(
        ToolErrorCode::Internal,
        format!("failed to render PDF page: {error}"),
    )
    .with_details(json!({ "reason": reason }))
}

pub(super) fn call(workspace: &McpWorkspace, arguments: Value) -> Result<ToolOutput, ToolError> {
    let args: Args = parse_args(arguments)?;
    if args.page == 0 {
        return Err(invalid(
            "page is 1-based (same as material.read pdf_pages); 0 is not a page",
            json!({ "field": "page", "value": 0, "min": 1 }),
        ));
    }
    let crop = match args.crop {
        None => None,
        Some(CropArg {
            left,
            top,
            right,
            bottom,
        }) => Some(
            NormalizedRect::try_new(left as f32, top as f32, right as f32, bottom as f32)
                .map_err(|error| {
                    invalid(
                        format!("crop is invalid: {error}"),
                        json!({ "field": "crop", "left": left, "top": top, "right": right, "bottom": bottom }),
                    )
                })?,
        ),
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
    let max_edge_px = u32::try_from(max_edge_px).unwrap_or(MAX_MAX_EDGE_PX as u32);
    let max_bytes = bounded(
        "max_bytes",
        args.max_bytes,
        DEFAULT_MAX_BYTES,
        MIN_MAX_BYTES,
        MAX_MAX_BYTES,
    )?;
    let id = StableId::parse(args.artifact_id.as_str()).map_err(|error| {
        ToolError::new(
            ToolErrorCode::InvalidArg,
            format!("artifact_id is not a valid stable id: {error}"),
        )
        .with_details(json!({ "artifact_id": args.artifact_id }))
    })?;

    let output = Output::new(ToolStatus::Ok, &args.artifact_id);
    let Some(material) = find_material(workspace, &id)? else {
        return not_found(
            output,
            "artifact_not_found",
            "no artifact with this id",
            json!({ "artifact_id": args.artifact_id }),
        );
    };
    if material.media_type != PDF_MEDIA_TYPE {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArg,
            format!(
                "pdf.page_image requires media type {PDF_MEDIA_TYPE}, found {}",
                material.media_type
            ),
        )
        .with_details(json!({
            "media_type": material.media_type,
            "supported_media_types": [PDF_MEDIA_TYPE],
        })));
    }
    let mut output = output;
    output.revision = Some(material.revision.clone());

    let pool = workspace.pool()?;
    let path = match material::open_managed_path(&pool, &material.id) {
        Ok(path) => path,
        Err(error) => {
            return match material::managed_path_failure(&error) {
                ManagedPathFailure::NotFound => not_found(
                    output,
                    "content_unavailable",
                    "the artifact is registered but has no readable managed copy",
                    json!({ "artifact_id": args.artifact_id }),
                ),
                ManagedPathFailure::Forbidden => Err(ToolError::new(
                    ToolErrorCode::Forbidden,
                    "managed artifact path is unsafe or is a symbolic link",
                )),
                ManagedPathFailure::Internal => Err(ToolError::new(
                    ToolErrorCode::Internal,
                    format!("failed to resolve managed artifact: {error}"),
                )),
            };
        }
    };
    drop(pool);

    // MCP page numbers are 1-based; mathscape-pdf is 0-based.
    let index = u32::try_from(args.page - 1).unwrap_or(u32::MAX);
    let page = PdfPageIndex::new(index);
    let engine = PdfEngine;
    let page_box = match engine.page_box(&path, page) {
        Ok(page_box) => page_box,
        Err(PdfError::PageOutOfBounds { page_count, .. }) => {
            output.page_count = Some(page_count);
            return not_found(
                output,
                "page_not_found",
                "no such page; page is 1-based and must be at most page_count",
                json!({ "page": args.page, "page_count": page_count }),
            );
        }
        Err(error) => return Err(render_error(error)),
    };
    let crop = crop.unwrap_or(NormalizedRect::FULL);
    describe(&mut output, args.page, &page_box, crop);

    if !args.include_image {
        let (width, height, scale) = page_box.image_size(crop, max_edge_px);
        output.image = Some(Some(ImageOut {
            format: "png",
            width,
            height,
            scale: round4(scale),
            bytes: None,
            sha256: None,
        }));
        return finish(output, None);
    }

    let fitted = engine
        .render_page(&path, page, Some(crop), max_edge_px, max_bytes)
        .map_err(render_error)?;
    let Some(image) = fitted.image else {
        output.status = status_value(ToolStatus::Truncated);
        output.truncated = true;
        output.image = Some(None);
        output.diagnostics.push(diag(
            "image_too_large",
            "the PNG is over max_bytes even at the smallest edge; pass a smaller crop, or raise max_bytes",
            json!({
                "max_bytes": max_bytes,
                "requested_edge_px": fitted.requested_edge_px,
                "smallest_edge_px": fitted.returned_edge_px,
                "smallest_bytes": fitted.last_bytes,
            }),
        ));
        return finish(output, None);
    };
    if fitted.returned_edge_px != fitted.requested_edge_px {
        output.diagnostics.push(diag(
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
        scale: round4(image.scale),
        bytes: Some(image.png.len()),
        sha256: Some(hex(&Sha256::digest(&image.png))),
    }));
    finish(output, Some(image.png))
}

fn describe(output: &mut Output, page: u64, page_box: &PdfPageBox, crop: NormalizedRect) {
    output.page = Some(page);
    output.page_count = Some(page_box.page_count);
    output.page_size = Some(PageSize {
        width: round4(f64::from(page_box.width_pt)),
        height: round4(f64::from(page_box.height_pt)),
        unit: "pt",
        rotation: page_box.rotation_degrees,
    });
    output.crop = Some(CropOut {
        left: round4(f64::from(crop.left())),
        top: round4(f64::from(crop.top())),
        right: round4(f64::from(crop.right())),
        bottom: round4(f64::from(crop.bottom())),
    });
    output.renderer = Some(PDF_RENDERER);
}
