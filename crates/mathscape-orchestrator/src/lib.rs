//! Versioned, observable workflows for ingestion, extraction, and formalization.

mod discovery_input;
mod discovery_producer;
mod discovery_prompt;
mod discovery_validation;
mod markdown_anchors;
mod marker_promotion;
mod selection_plan;
mod selector_resolver;

pub use discovery_input::{
    DEFAULT_DISCOVERY_INPUT_BLOCKS, DEFAULT_DISCOVERY_INPUT_BYTES, DiscoveryInputError,
    DiscoveryInputLimits, build_discovery_input,
};
pub use discovery_producer::{
    DEFAULT_DISCOVERY_RESPONSE_BYTES, DiscoveryProducer, DiscoveryProducerLimits,
    DiscoveryProductionOutcome, DiscoveryProposalRejection, DiscoveryResponseSummary,
    DiscoveryRunSummary,
};
pub use discovery_prompt::{
    DISCOVERY_PROMPT_MANIFEST, DISCOVERY_SYSTEM_PROMPT, DiscoveryPromptManifest,
    PreparedDiscoveryRequest, build_discovery_request, discovery_output_schema,
    discovery_prompt_manifest,
};
pub use discovery_validation::{
    DEFAULT_PROPOSAL_ITEMS_PER_COLLECTION, DEFAULT_PROPOSAL_SELECTORS,
    DEFAULT_PROPOSAL_SERIALIZED_BYTES, DiscoveryDiagnostic, DiscoveryValidationCode,
    DiscoveryValidationLimits, DiscoveryValidationReport, ResolvedDiscoverySelector,
    ValidatedDiscoveryCoverage, ValidatedDiscoveryIssue, ValidatedDiscoveryProposal,
    ValidatedDiscoveryReference, ValidatedDiscoveryRelation, ValidatedDiscoverySourceLabel,
    ValidatedDiscoveryUnit, ValidatedProposalStatus, validate_discovery_proposal,
};
pub use markdown_anchors::{AnchorError, MAX_MARKDOWN_BYTES, anchor_markdown, resolve_selection};
pub use marker_promotion::{
    MarkerArtifactPromotion, MarkerPromotionError, MarkerPromotionResult, PRODUCT_PATHS_KEY,
    ProductPath, promote_marker_inventory,
};
pub use selection_plan::{ConversionSelectionPlan, ConversionSlice};
pub use selector_resolver::{
    DISCOVERY_MODEL_INPUT_VERSION, DiscoveryModelInput, DiscoveryModelInputRegion,
    DiscoveryModelInputSegment, ResolvedSelectorProposal, SelectorBinding, SelectorResolutionCode,
    SelectorResolutionDiagnostic, SelectorResolutionReport, SelectorResolver, SelectorRewrite,
    SelectorRewriteKind,
};
