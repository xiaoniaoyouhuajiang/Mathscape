//! Versioned, observable workflows for ingestion, extraction, and formalization.

mod marker_promotion;
mod selection_plan;

pub use marker_promotion::{
    MarkerArtifactPromotion, MarkerPromotionError, MarkerPromotionResult, PRODUCT_PATHS_KEY,
    ProductPath, promote_marker_inventory,
};
pub use selection_plan::{ConversionSelectionPlan, ConversionSlice};
