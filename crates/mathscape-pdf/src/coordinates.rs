use crate::{PdfError, PdfPageIndex};

/// Axis-aligned rectangle in PDF user-space units, origin at bottom-left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PdfPointRect {
    left: f32,
    bottom: f32,
    right: f32,
    top: f32,
}

impl PdfPointRect {
    pub fn new(left: f32, bottom: f32, right: f32, top: f32) -> Result<Self, PdfError> {
        if ![left, bottom, right, top].into_iter().all(f32::is_finite) {
            return Err(PdfError::InvalidRectangle("coordinates must be finite"));
        }
        if left > right || bottom > top {
            return Err(PdfError::InvalidRectangle(
                "minimum coordinates must not exceed maximum coordinates",
            ));
        }
        Ok(Self {
            left,
            bottom,
            right,
            top,
        })
    }

    pub(crate) fn from_origin_size(
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Result<Self, PdfError> {
        if ![x, y, width, height].into_iter().all(f32::is_finite) {
            return Err(PdfError::InvalidRectangle("coordinates must be finite"));
        }
        let opposite_x = x + width;
        let opposite_y = y + height;
        Self::new(
            x.min(opposite_x),
            y.min(opposite_y),
            x.max(opposite_x),
            y.max(opposite_y),
        )
    }

    pub const fn left(self) -> f32 {
        self.left
    }

    pub const fn bottom(self) -> f32 {
        self.bottom
    }

    pub const fn right(self) -> f32 {
        self.right
    }

    pub const fn top(self) -> f32 {
        self.top
    }

    pub fn width(self) -> f32 {
        self.right - self.left
    }

    pub fn height(self) -> f32 {
        self.top - self.bottom
    }
}

/// Rectangle normalized to `[0, 1]` in displayed-page coordinates, origin at top-left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NormalizedRect {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

impl NormalizedRect {
    /// The whole displayed page.
    pub const FULL: Self = Self {
        left: 0.0,
        top: 0.0,
        right: 1.0,
        bottom: 1.0,
    };

    /// Validated constructor for caller-supplied rectangles: every edge in
    /// `[0, 1]`, `left < right` and `top < bottom`.
    pub fn try_new(left: f32, top: f32, right: f32, bottom: f32) -> Result<Self, PdfError> {
        let in_unit = |value: f32| value.is_finite() && (0.0..=1.0).contains(&value);
        if ![left, top, right, bottom].into_iter().all(in_unit) {
            return Err(PdfError::InvalidRectangle(
                "normalized coordinates must be finite and within [0, 1]",
            ));
        }
        if left >= right || top >= bottom {
            return Err(PdfError::InvalidRectangle(
                "normalized rectangle must satisfy left < right and top < bottom",
            ));
        }
        Ok(Self::new(left, top, right, bottom))
    }

    fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub const fn left(self) -> f32 {
        self.left
    }

    pub const fn top(self) -> f32 {
        self.top
    }

    pub const fn right(self) -> f32 {
        self.right
    }

    pub const fn bottom(self) -> f32 {
        self.bottom
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdfRotation {
    Zero,
    Clockwise90,
    Clockwise180,
    Clockwise270,
}

impl PdfRotation {
    pub(crate) fn from_degrees(degrees: i64) -> Result<Self, PdfError> {
        match degrees.rem_euclid(360) {
            0 => Ok(Self::Zero),
            90 => Ok(Self::Clockwise90),
            180 => Ok(Self::Clockwise180),
            270 => Ok(Self::Clockwise270),
            _ => Err(PdfError::UnsupportedRotation(degrees)),
        }
    }

    pub const fn degrees(self) -> u16 {
        match self {
            Self::Zero => 0,
            Self::Clockwise90 => 90,
            Self::Clockwise180 => 180,
            Self::Clockwise270 => 270,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PdfPageGeometry {
    page: PdfPageIndex,
    media_box: PdfPointRect,
    rotation: PdfRotation,
}

impl PdfPageGeometry {
    pub fn new(
        page: PdfPageIndex,
        media_box: PdfPointRect,
        rotation: PdfRotation,
    ) -> Result<Self, PdfError> {
        if media_box.width() <= 0.0 || media_box.height() <= 0.0 {
            return Err(PdfError::InvalidRectangle(
                "page media box must have positive width and height",
            ));
        }
        Ok(Self {
            page,
            media_box,
            rotation,
        })
    }

    pub const fn page(self) -> PdfPageIndex {
        self.page
    }

    pub const fn media_box(self) -> PdfPointRect {
        self.media_box
    }

    pub const fn rotation(self) -> PdfRotation {
        self.rotation
    }

    pub fn displayed_width(self) -> f32 {
        match self.rotation {
            PdfRotation::Zero | PdfRotation::Clockwise180 => self.media_box.width(),
            PdfRotation::Clockwise90 | PdfRotation::Clockwise270 => self.media_box.height(),
        }
    }

    pub fn displayed_height(self) -> f32 {
        match self.rotation {
            PdfRotation::Zero | PdfRotation::Clockwise180 => self.media_box.height(),
            PdfRotation::Clockwise90 | PdfRotation::Clockwise270 => self.media_box.width(),
        }
    }

    /// Converts an extracted PDF-space rectangle to a clipped, rotation-aware
    /// top-left normalized rectangle.
    pub fn normalize(self, bounds: PdfPointRect) -> Option<NormalizedRect> {
        let raw_width = self.media_box.width();
        let raw_height = self.media_box.height();
        let points = [
            (bounds.left(), bounds.bottom()),
            (bounds.left(), bounds.top()),
            (bounds.right(), bounds.bottom()),
            (bounds.right(), bounds.top()),
        ];
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for (absolute_x, absolute_y) in points {
            let x = absolute_x - self.media_box.left();
            let y = absolute_y - self.media_box.bottom();
            let (display_x, display_y) = match self.rotation {
                PdfRotation::Zero => (x, y),
                PdfRotation::Clockwise90 => (y, raw_width - x),
                PdfRotation::Clockwise180 => (raw_width - x, raw_height - y),
                PdfRotation::Clockwise270 => (raw_height - y, x),
            };
            min_x = min_x.min(display_x);
            min_y = min_y.min(display_y);
            max_x = max_x.max(display_x);
            max_y = max_y.max(display_y);
        }

        let width = self.displayed_width();
        let height = self.displayed_height();
        if max_x < 0.0 || max_y < 0.0 || min_x > width || min_y > height {
            return None;
        }
        min_x = min_x.clamp(0.0, width);
        max_x = max_x.clamp(0.0, width);
        min_y = min_y.clamp(0.0, height);
        max_y = max_y.clamp(0.0, height);
        Some(NormalizedRect::new(
            min_x / width,
            1.0 - max_y / height,
            max_x / width,
            1.0 - min_y / height,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 1.0e-6, "{actual} != {expected}");
    }

    #[test]
    fn normalizes_bottom_left_pdf_coordinates_to_top_left() {
        let page = PdfPageGeometry::new(
            PdfPageIndex::new(0),
            PdfPointRect::new(0.0, 0.0, 200.0, 100.0).unwrap(),
            PdfRotation::Zero,
        )
        .unwrap();
        let result = page
            .normalize(PdfPointRect::new(20.0, 60.0, 80.0, 90.0).unwrap())
            .unwrap();
        assert_close(result.left(), 0.1);
        assert_close(result.top(), 0.1);
        assert_close(result.right(), 0.4);
        assert_close(result.bottom(), 0.4);
    }

    #[test]
    fn normalization_resolves_page_rotation() {
        let page = PdfPageGeometry::new(
            PdfPageIndex::new(0),
            PdfPointRect::new(0.0, 0.0, 200.0, 100.0).unwrap(),
            PdfRotation::Clockwise90,
        )
        .unwrap();
        let result = page
            .normalize(PdfPointRect::new(20.0, 60.0, 80.0, 90.0).unwrap())
            .unwrap();
        assert_close(page.displayed_width(), 100.0);
        assert_close(page.displayed_height(), 200.0);
        assert_close(result.left(), 0.6);
        assert_close(result.right(), 0.9);
        assert_close(result.top(), 0.1);
        assert_close(result.bottom(), 0.4);
    }
}
