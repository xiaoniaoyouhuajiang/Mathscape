//! Versioned, observable workflows for ingestion, extraction, and formalization.

mod discovery_input;
mod discovery_producer;
mod discovery_prompt;
mod discovery_validation;
mod discovery_workflow;
mod fact_planner;
mod fact_preflight;
mod ink;
pub use ink::plan_ink_evidence;
pub mod change_set_plan;
pub mod map_edit;
pub mod registration;
mod source_facts;
pub mod source_integrity;
mod source_locator;
mod source_registration;
pub use source_locator::{
    locator_matches, markdown_span_id, normalized_locator_string, quote_matches_range,
    source_text_hash, stable_hash_id, valid_temp_id,
};
mod source_review;
pub use discovery_workflow::*;
pub use fact_planner::*;
pub use source_facts::*;
pub use source_registration::*;
pub use source_review::project_source_review;
mod markdown_anchors;
pub mod markdown_patch;
mod markdown_patch_producer;
mod markdown_patch_prompt;
mod markdown_pdf_map;
mod marker_promotion;
mod occurrence_resolver;
mod patch_review;
mod selection_plan;
mod selector_resolver;
mod unit_discovery;

pub use fact_preflight::preflight_source_facts;
pub use occurrence_resolver::*;
pub use unit_discovery::*;

pub use discovery_input::{
    DEFAULT_DISCOVERY_INPUT_BLOCKS, DEFAULT_DISCOVERY_INPUT_BYTES, DiscoveryInputError,
    DiscoveryInputLimits, build_discovery_input, rebind_discovery_input,
    rebind_discovery_input_after_patch,
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
pub use markdown_anchors::{
    AnchorError, MAX_MARKDOWN_BYTES, PAGE_MARKER_STRUCTURAL_KIND, anchor_markdown,
    is_page_marker_text, page_marker_id, resolve_selection,
};
pub use markdown_patch::MarkdownPdfMapInheritance;
pub use markdown_patch_producer::{
    MarkdownPatchProducer, MarkdownPatchProductionOutcome, MarkdownPatchProductionSummary,
    MarkdownPatchRejection, MarkdownPatchResponseSummary,
};
pub use markdown_patch_prompt::{
    MarkdownPatchPromptManifest, PreparedMarkdownPatchRequest, build_markdown_patch_request,
    build_markdown_patch_request_with_issues, markdown_patch_output_schema,
    markdown_patch_prompt_manifest,
};
pub use markdown_pdf_map::build_markdown_pdf_block_map;
pub use marker_promotion::{
    MARKDOWN_PDF_BASE_CONVERSION_KEY, MARKDOWN_PDF_MAP_ARTIFACT_KEY,
    MARKDOWN_PDF_SOURCE_PDF_ARTIFACT_KEY, MarkerArtifactPromotion, MarkerPromotionError,
    MarkerPromotionResult, PRODUCT_PATHS_KEY, ProductPath, promote_marker_inventory,
};
pub use patch_review::{
    AppliedPatchRecord, MarkdownPatchGate, MarkdownPatchWorkflowState, PatchContinuation,
    PatchReviewDecision, PatchReviewError, ranges_covered,
};
pub use selection_plan::{ConversionSelectionPlan, ConversionSlice};
pub use selector_resolver::{
    DISCOVERY_MODEL_INPUT_VERSION, DiscoveryModelInput, DiscoveryModelInputRegion,
    DiscoveryModelInputSegment, ResolvedSelectorProposal, SelectorBinding, SelectorResolutionCode,
    SelectorResolutionDiagnostic, SelectorResolutionReport, SelectorResolver, SelectorRewrite,
    SelectorRewriteKind,
};
