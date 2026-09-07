//! Versioned, observable workflows for ingestion, extraction, and formalization.

mod markdown_anchors;
mod marker_promotion;
mod selection_plan;

pub use markdown_anchors::{AnchorError, MAX_MARKDOWN_BYTES, anchor_markdown, resolve_selection};
pub use marker_promotion::{
    MarkerArtifactPromotion, MarkerPromotionError, MarkerPromotionResult, PRODUCT_PATHS_KEY,
    ProductPath, promote_marker_inventory,
};
pub use selection_plan::{ConversionSelectionPlan, ConversionSlice};
