//! Read-only page rasterization (`pdf.page_image`).
//!
//! The engine is the pure-Rust [`hayro`] rasterizer: no Marker, no OCR, no
//! subprocess and no native library. The PDF bytes are read once and never
//! written. Pages are zero-based [`PdfPageIndex`] values like everywhere else
//! in this crate; the one-based MCP page number is converted by the caller.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_syntax::page::{Page, Rotation};
use hayro::hayro_syntax::{LoadPdfError, Pdf};
use hayro::kurbo::Affine;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::vello_cpu::{Pixmap, RasterizerSettings, RenderContext, Resources, TargetInit};
use hayro::{RenderCache, RenderSettings, render_into};

use crate::coordinates::NormalizedRect;
use crate::error::PdfError;
use crate::inspection::PdfEngine;
use crate::selection::PdfPageIndex;

/// Stable identifier of the rasterizer reported to callers.
pub const PDF_RENDERER: &str = "hayro-0.8";
/// Smallest longest edge a fitted render may shrink to.
pub const MIN_RENDER_EDGE_PX: u32 = 256;
/// Largest longest edge this crate will rasterize.
pub const MAX_RENDER_EDGE_PX: u32 = 2048;
const DOWNSCALE_FACTOR: f64 = 0.75;
const MAX_DOWNSCALE_STEPS: u32 = 3;

/// One encoded PNG of a page (or crop).
#[derive(Clone, Debug, PartialEq)]
pub struct PdfPagePng {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Output pixels per displayed PDF point.
    pub scale: f64,
}

/// Displayed geometry of one page as the rasterizer sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PdfPageBox {
    pub page: PdfPageIndex,
    pub page_count: u32,
    /// Displayed (rotation-applied, crop-box) page size in PDF points.
    pub width_pt: f32,
    pub height_pt: f32,
    pub rotation_degrees: u16,
}

impl PdfPageBox {
    /// Output size and scale (pixels per point) for `crop` at `max_edge_px`.
    pub fn image_size(&self, crop: NormalizedRect, max_edge_px: u32) -> (u32, u32, f64) {
        let crop_width = f64::from(crop.right() - crop.left()) * f64::from(self.width_pt);
        let crop_height = f64::from(crop.bottom() - crop.top()) * f64::from(self.height_pt);
        let scale = f64::from(max_edge_px) / crop_width.max(crop_height);
        let edge = |value: f64| ((value * scale).round() as u32).clamp(1, max_edge_px);
        (edge(crop_width), edge(crop_height), scale)
    }
}

/// A rendered page plus the metadata needed to place it.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfPageRender {
    pub page_box: PdfPageBox,
    /// The crop actually rendered (full page when none was requested).
    pub crop: NormalizedRect,
    /// `None` when even the smallest attempt was over `max_bytes`.
    pub image: Option<PdfPagePng>,
    pub requested_edge_px: u32,
    /// Edge of the returned image, or of the last attempt when none fits.
    pub returned_edge_px: u32,
    /// Bytes of the last attempt.
    pub last_bytes: usize,
}

impl PdfEngine {
    /// Page count and displayed geometry of one page, without rasterizing.
    pub fn page_box(
        &self,
        path: impl AsRef<Path>,
        page: PdfPageIndex,
    ) -> Result<PdfPageBox, PdfError> {
        let pdf = load(path.as_ref())?;
        let (_, page_box) = locate(&pdf, page)?;
        Ok(page_box)
    }

    /// Rasterizes one page (optionally a normalized crop of it) to PNG on a
    /// white background. The longest output edge is `max_edge_px`; when the
    /// PNG is over `max_bytes` the edge is shrunk by ×0.75 up to three times,
    /// never below [`MIN_RENDER_EDGE_PX`].
    pub fn render_page(
        &self,
        path: impl AsRef<Path>,
        page: PdfPageIndex,
        crop: Option<NormalizedRect>,
        max_edge_px: u32,
        max_bytes: usize,
    ) -> Result<PdfPageRender, PdfError> {
        if !(MIN_RENDER_EDGE_PX..=MAX_RENDER_EDGE_PX).contains(&max_edge_px) {
            return Err(PdfError::InvalidRectangle(
                "max_edge_px is outside the renderer bounds",
            ));
        }
        let pdf = load(path.as_ref())?;
        let (target, page_box) = locate(&pdf, page)?;
        let crop = crop.unwrap_or(NormalizedRect::FULL);

        let mut edge = max_edge_px;
        let mut attempt = rasterize(target, &page_box, crop, edge)?;
        let mut steps = 0;
        while attempt.png.len() > max_bytes
            && steps < MAX_DOWNSCALE_STEPS
            && edge > MIN_RENDER_EDGE_PX
        {
            edge = ((f64::from(edge) * DOWNSCALE_FACTOR).floor() as u32).max(MIN_RENDER_EDGE_PX);
            attempt = rasterize(target, &page_box, crop, edge)?;
            steps += 1;
        }
        let last_bytes = attempt.png.len();
        Ok(PdfPageRender {
            page_box,
            crop,
            image: (last_bytes <= max_bytes).then_some(attempt),
            requested_edge_px: max_edge_px,
            returned_edge_px: edge,
            last_bytes,
        })
    }
}

fn load(path: &Path) -> Result<Pdf, PdfError> {
    let bytes = std::fs::read(path)?;
    if !bytes.starts_with(b"%PDF-") && !bytes.windows(5).take(1024).any(|w| w == b"%PDF-") {
        return Err(PdfError::NotPdf("missing %PDF- header".to_owned()));
    }
    Pdf::new(bytes).map_err(|error| match error {
        LoadPdfError::Decryption(_) => PdfError::Encrypted,
        LoadPdfError::Invalid => PdfError::InvalidStructure,
    })
}

fn locate(pdf: &Pdf, page: PdfPageIndex) -> Result<(&Page<'_>, PdfPageBox), PdfError> {
    let pages = pdf.pages();
    let page_count = u32::try_from(pages.len())
        .map_err(|_| PdfError::Parse("page count exceeds u32".to_owned()))?;
    if page_count == 0 {
        return Err(PdfError::EmptyDocument);
    }
    let Some(target) = pages.get(page.get() as usize) else {
        return Err(PdfError::PageOutOfBounds {
            index: page.get(),
            page_count,
        });
    };
    let (width_pt, height_pt) = target.render_dimensions();
    if !(width_pt.is_finite() && height_pt.is_finite() && width_pt > 0.0 && height_pt > 0.0) {
        return Err(PdfError::InvalidRectangle(
            "page render box must have positive width and height",
        ));
    }
    let rotation_degrees = match target.rotation() {
        Rotation::None => 0,
        Rotation::Horizontal => 90,
        Rotation::Flipped => 180,
        Rotation::FlippedHorizontal => 270,
    };
    Ok((
        target,
        PdfPageBox {
            page,
            page_count,
            width_pt,
            height_pt,
            rotation_degrees,
        },
    ))
}

fn rasterize(
    page: &Page<'_>,
    page_box: &PdfPageBox,
    crop: NormalizedRect,
    max_edge_px: u32,
) -> Result<PdfPagePng, PdfError> {
    let crop_x = f64::from(crop.left()) * f64::from(page_box.width_pt);
    let crop_y = f64::from(crop.top()) * f64::from(page_box.height_pt);
    let (width, height, scale) = page_box.image_size(crop, max_edge_px);
    // Bounded by MAX_RENDER_EDGE_PX, so the u16 casts cannot truncate.
    let (width_u16, height_u16) = (width as u16, height as u16);

    // hayro is young; a malformed page must fail this call, not the server.
    let rendered = catch_unwind(AssertUnwindSafe(|| {
        let cache = RenderCache::new();
        let interpreter = InterpreterSettings::default();
        let mut ctx = RenderContext::new(width_u16, height_u16);
        let transform = Affine::scale(scale)
            * Affine::translate((-crop_x, -crop_y))
            * page.initial_transform(true).to_kurbo();
        render_into(
            page,
            &cache,
            &interpreter,
            &RenderSettings::default(),
            &mut ctx,
            transform,
        );
        ctx.flush();
        let mut pixmap = Pixmap::new(width_u16, height_u16);
        ctx.render_with(
            &mut pixmap,
            &mut Resources::default(),
            RasterizerSettings {
                target_init: TargetInit::Clear(WHITE),
                ..Default::default()
            },
        );
        pixmap.into_png()
    }))
    .map_err(|_| PdfError::Render("the rasterizer panicked on this page".to_owned()))?
    .map_err(|error| PdfError::Render(format!("png encoding failed: {error}")))?;
    Ok(PdfPagePng {
        png: rendered,
        width,
        height,
        scale,
    })
}
