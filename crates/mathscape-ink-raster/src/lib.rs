//! Mathscape ink raster (Phase1d-b): turns projected ink strokes into a
//! deterministic PNG the way the Desktop canvas draws them.
//!
//! - [`freehand`]: a port of `perfect-freehand@1.2.3` `getStroke`.
//! - [`outline_stroke`] + [`render_png`]: non-zero, anti-aliased `tiny-skia`
//!   fills on an opaque white background, encoded as an 8-bit RGB PNG with no
//!   text or time chunks, so identical input gives identical bytes.
//!
//! The crate is pure computation: no files, no logging, no stdout.

pub mod freehand;

pub use freehand::{StrokeOptions, Vec2, get_stroke};

/// Renderer id reported next to every image.
pub const RENDERER_ID: &str = "mathscape-ink-raster/1 (perfect-freehand@1.2.3 port, tiny-skia)";
/// Smallest and largest bitmap edge the renderer accepts.
pub const MIN_EDGE_PX: u32 = 256;
pub const MAX_EDGE_PX: u32 = 2048;
/// Downscale factor and attempts used when a PNG is over its byte budget.
pub const DOWNSCALE_FACTOR: f64 = 0.75;
pub const MAX_DOWNSCALE_STEPS: usize = 3;

/// An axis-aligned rectangle in page units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn right(&self) -> f64 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }

    /// Overlap of two rectangles, or `None` when it has no area.
    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        (right > x && bottom > y).then_some(Rect {
            x,
            y,
            width: right - x,
            height: bottom - y,
        })
    }

    /// Grows the rectangle by `margin` on every side.
    pub fn inflate(&self, margin: f64) -> Rect {
        Rect {
            x: self.x - margin,
            y: self.y - margin,
            width: self.width + 2.0 * margin,
            height: self.height + 2.0 * margin,
        }
    }

    fn union(&self, other: &Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Rect {
            x,
            y,
            width: self.right().max(other.right()) - x,
            height: self.bottom().max(other.bottom()) - y,
        }
    }
}

/// One stroke ready to outline: points are `[x, y, pressure ?? 0.5]`.
#[derive(Clone, Debug, PartialEq)]
pub struct InkStrokeInput {
    pub points: Vec<[f64; 3]>,
    pub options: StrokeOptions,
    /// `#rrggbb` as bytes.
    pub color: [u8; 3],
    /// 1.0 for normal ink; span renders fade unselected strokes.
    pub opacity: f32,
}

/// A filled outline in page coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Outline {
    pub points: Vec<Vec2>,
    pub color: [u8; 3],
    pub opacity: f32,
}

impl Outline {
    /// Bounding box of the finite outline points (what the fill can touch).
    pub fn bounds(&self) -> Option<Rect> {
        let mut finite = self
            .points
            .iter()
            .filter(|point| point[0].is_finite() && point[1].is_finite());
        let first = finite.next()?;
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (first[0], first[1], first[0], first[1]);
        for point in finite {
            min_x = min_x.min(point[0]);
            min_y = min_y.min(point[1]);
            max_x = max_x.max(point[0]);
            max_y = max_y.max(point[1]);
        }
        Some(Rect {
            x: min_x,
            y: min_y,
            width: max_x - min_x,
            height: max_y - min_y,
        })
    }
}

/// Outlines a stroke exactly like `canvas.ts::drawInkStroke`.
pub fn outline_stroke(stroke: &InkStrokeInput) -> Outline {
    Outline {
        points: get_stroke(&stroke.points, stroke.options),
        color: stroke.color,
        opacity: stroke.opacity,
    }
}

/// Union of outline bounds.
pub fn ink_bounds(outlines: &[Outline]) -> Option<Rect> {
    outlines
        .iter()
        .filter_map(Outline::bounds)
        .reduce(|acc, rect| acc.union(&rect))
}

/// Whether an outline can paint inside `crop` (touching the edge counts:
/// anti-aliasing bleeds in). Empty outlines never do.
pub fn outline_touches(outline: &Outline, crop: &Rect) -> bool {
    outline.bounds().is_some_and(|bounds| {
        bounds.right() >= crop.x
            && bounds.bottom() >= crop.y
            && bounds.x <= crop.right()
            && bounds.y <= crop.bottom()
    })
}

/// Parses `#rrggbb` (validated ink colours).
pub fn parse_hex_color(value: &str) -> Option<[u8; 3]> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |index: usize| u8::from_str_radix(&hex[index..index + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

/// Pixel size for `crop` with its longer edge at `max_edge_px`.
///
/// Returns `(width, height, scale)`; `scale` is pixels per page unit along the
/// longer edge (1200×1600 at 1024 → 768×1024, scale 0.64).
pub fn image_size(crop: &Rect, max_edge_px: u32) -> (u32, u32, f64) {
    let longest = crop.width.max(crop.height);
    let scale = f64::from(max_edge_px) / longest;
    let edge = |value: f64| ((value * scale).round() as u32).clamp(1, max_edge_px);
    (edge(crop.width), edge(crop.height), scale)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RasterError {
    /// Crop or edge out of the renderer's bounds (caller bug).
    InvalidSize(String),
    /// PNG encoding failed.
    Encode(String),
}

impl std::fmt::Display for RasterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSize(message) => write!(f, "invalid raster size: {message}"),
            Self::Encode(message) => write!(f, "png encoding failed: {message}"),
        }
    }
}

impl std::error::Error for RasterError {}

/// A rendered PNG.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderedPng {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    /// Outlines that intersected the crop and were filled.
    pub rendered: usize,
}

/// Fills `outlines` (in order) over `crop` on white and encodes a PNG.
pub fn render_png(
    outlines: &[Outline],
    crop: &Rect,
    max_edge_px: u32,
) -> Result<RenderedPng, RasterError> {
    let finite = [crop.x, crop.y, crop.width, crop.height]
        .iter()
        .all(|value| value.is_finite());
    if !finite || crop.width <= 0.0 || crop.height <= 0.0 {
        return Err(RasterError::InvalidSize(format!("crop {crop:?}")));
    }
    if !(MIN_EDGE_PX..=MAX_EDGE_PX).contains(&max_edge_px) {
        return Err(RasterError::InvalidSize(format!(
            "max_edge_px {max_edge_px}"
        )));
    }
    let (width, height, scale) = image_size(crop, max_edge_px);
    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| RasterError::InvalidSize(format!("{width}x{height}")))?;
    pixmap.fill(tiny_skia::Color::WHITE);
    let sx = f64::from(width) / crop.width;
    let sy = f64::from(height) / crop.height;
    let mut rendered = 0;
    for outline in outlines {
        if !outline_touches(outline, crop) {
            continue;
        }
        let Some(path) = outline_path(outline, crop, sx, sy) else {
            continue;
        };
        let mut paint = tiny_skia::Paint::default();
        let [r, g, b] = outline.color;
        let alpha = (outline.opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
        paint.set_color_rgba8(r, g, b, alpha);
        paint.anti_alias = true;
        pixmap.fill_path(
            &path,
            &paint,
            tiny_skia::FillRule::Winding,
            tiny_skia::Transform::identity(),
            None,
        );
        rendered += 1;
    }
    let png = encode_rgb_png(&pixmap)?;
    Ok(RenderedPng {
        png,
        width,
        height,
        scale,
        rendered,
    })
}

/// The canvas ignores non-finite `moveTo`/`lineTo`; so does this path.
fn outline_path(outline: &Outline, crop: &Rect, sx: f64, sy: f64) -> Option<tiny_skia::Path> {
    let mut builder = tiny_skia::PathBuilder::new();
    let mut started = false;
    for point in &outline.points {
        if !(point[0].is_finite() && point[1].is_finite()) {
            continue;
        }
        let x = ((point[0] - crop.x) * sx) as f32;
        let y = ((point[1] - crop.y) * sy) as f32;
        if started {
            builder.line_to(x, y);
        } else {
            builder.move_to(x, y);
            started = true;
        }
    }
    if !started {
        return None;
    }
    builder.close();
    builder.finish()
}

fn encode_rgb_png(pixmap: &tiny_skia::Pixmap) -> Result<Vec<u8>, RasterError> {
    // The background is opaque white, so every pixel is opaque and the
    // premultiplied RGBA bytes are plain RGB.
    let rgb: Vec<u8> = pixmap
        .data()
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect();
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, pixmap.width(), pixmap.height());
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Default);
        encoder.set_filter(png::FilterType::Paeth);
        encoder.set_adaptive_filter(png::AdaptiveFilterType::Adaptive);
        let mut writer = encoder
            .write_header()
            .map_err(|error| RasterError::Encode(error.to_string()))?;
        writer
            .write_image_data(&rgb)
            .map_err(|error| RasterError::Encode(error.to_string()))?;
        writer
            .finish()
            .map_err(|error| RasterError::Encode(error.to_string()))?;
    }
    Ok(bytes)
}

/// Result of fitting a render into a byte budget.
#[derive(Clone, Debug, PartialEq)]
pub struct FittedPng {
    /// `None` when even the last attempt was over `max_bytes`.
    pub image: Option<RenderedPng>,
    pub requested_edge_px: u32,
    /// Edge of the returned image, or of the last attempt when none fits.
    pub returned_edge_px: u32,
    /// Bytes of the last attempt (useful when nothing fits).
    pub last_bytes: usize,
}

/// Renders at `max_edge_px`; if the PNG is over `max_bytes`, re-renders with
/// the edge ×0.75 up to three times, never below 256 px.
pub fn render_png_within(
    outlines: &[Outline],
    crop: &Rect,
    max_edge_px: u32,
    max_bytes: usize,
) -> Result<FittedPng, RasterError> {
    let mut edge = max_edge_px;
    let mut attempt = render_png(outlines, crop, edge)?;
    let mut steps = 0;
    while attempt.png.len() > max_bytes && steps < MAX_DOWNSCALE_STEPS && edge > MIN_EDGE_PX {
        edge = ((f64::from(edge) * DOWNSCALE_FACTOR).floor() as u32).max(MIN_EDGE_PX);
        attempt = render_png(outlines, crop, edge)?;
        steps += 1;
    }
    let last_bytes = attempt.png.len();
    Ok(FittedPng {
        image: (last_bytes <= max_bytes).then_some(attempt),
        requested_edge_px: max_edge_px,
        returned_edge_px: edge,
        last_bytes,
    })
}
