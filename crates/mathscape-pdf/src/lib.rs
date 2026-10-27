//! PDF inspection and source-addressing infrastructure for Mathscape.
//!
//! The public types in this crate deliberately hide backend page-number and
//! coordinate conventions. Physical PDF pages are always zero-based here;
//! conversions to one-based engine APIs happen only inside adapters.

mod coordinates;
mod error;
mod inspection;
mod outline;
mod selection;
mod text;

pub use coordinates::{NormalizedRect, PdfPageGeometry, PdfPointRect, PdfRotation};
pub use error::PdfError;
pub use inspection::{
    ClassificationMode, OcrReason, PageOcrRecommendation, PdfClassification, PdfContentKind,
    PdfEngine, PdfOpenOptions, PdfProbe, ScanCoverage,
};
pub use outline::{PdfOutline, PdfOutlineEntry};
pub use selection::{PdfPageIndex, PdfPageRange, PdfSelection};
pub use text::{PdfTextItem, PdfTextItemKind, TextExtraction};
