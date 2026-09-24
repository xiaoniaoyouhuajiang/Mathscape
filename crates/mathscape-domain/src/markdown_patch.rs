//! Contracts for deterministic Markdown corrections between source discovery
//! and source-fact discovery. A Markdown patch creates a derived artifact; it
//! never mutates the bytes of the base artifact in place.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{ContentHash, DiscoveryIssueKind, StableId, TextByteRange, TextOccurrence};

pub const MARKDOWN_PATCH_INPUT_VERSION: &str = "mathscape-markdown-patch-input-1";
pub const MARKDOWN_PATCH_VERSION: &str = "mathscape-markdown-patch-1";
pub const MARKDOWN_PATCH_SET_VERSION: &str = "mathscape-markdown-patch-set-1";
pub const MARKDOWN_PATCH_RECEIPT_VERSION: &str = "mathscape-markdown-patch-receipt-1";

pub const MAX_PATCHES_PER_RESPONSE: usize = 64;
pub const MAX_REPLACEMENT_BYTES: usize = 16 * 1024;
pub const MAX_PATCH_CONTEXT_BLOCKS_BEFORE: usize = 2;
pub const MAX_PATCH_CONTEXT_BLOCKS_AFTER: usize = 2;
pub const MAX_PATCH_OUTPUT_BYTES: usize = 512 * 1024;
pub const MAX_PATCH_ROUNDS_PER_RUN: usize = 1;

/// A system-assigned reference to an A-stage issue. Model-provided temporary
/// references are deliberately not part of the persisted identity.
#[derive(
    Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct DiscoveryIssueRef(String);

impl DiscoveryIssueRef {
    pub fn parse(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        let valid = value.starts_with("a-i")
            && value.len() > 3
            && value[3..].bytes().all(|byte| byte.is_ascii_digit());
        valid.then_some(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The A-stage issue after deterministic validation and system identity
/// assignment. It is the only issue shape that P may consume.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonicalDiscoveryIssue {
    pub issue_ref: DiscoveryIssueRef,
    pub kind: DiscoveryIssueKind,
    pub unit_ref: Option<String>,
    pub evidence: Vec<TextByteRange>,
    pub observation: String,
}

/// Minimal block context sent to the P producer. The block handle is request
/// local and must never become a persisted identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MarkdownPatchBlock {
    pub block_ref: String,
    pub structural_kind: String,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MarkdownPatchUnitContext {
    pub unit_ref: String,
    pub unit_kind: String,
    pub parent: Option<String>,
    pub ranges: Vec<TextByteRange>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MarkdownPatchInput {
    pub version: String,
    pub request_ref: String,
    pub base_snapshot: crate::MarkdownSnapshot,
    pub input_fingerprint: ContentHash,
    pub blocks: Vec<MarkdownPatchBlock>,
    pub units: Vec<MarkdownPatchUnitContext>,
    pub issues: Vec<CanonicalDiscoveryIssue>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MarkdownPatchProposal {
    /// A request-local model handle. The orchestrator assigns the persisted
    /// patch reference after validation.
    pub patch_ref: String,
    pub issue_ref: DiscoveryIssueRef,
    pub target: TextOccurrence,
    pub replacement: String,
    pub rationale: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MarkdownPatchResponse {
    pub version: String,
    pub request_ref: String,
    pub patches: Vec<MarkdownPatchProposal>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MarkdownPatchStatus {
    Proposed,
    Accepted,
    Rejected,
    Stale,
    Applied,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolvedMarkdownPatch {
    pub patch_ref: String,
    pub issue_ref: DiscoveryIssueRef,
    pub range: TextByteRange,
    pub old_text: String,
    pub old_text_hash: ContentHash,
    pub replacement: String,
    pub replacement_hash: ContentHash,
    pub rationale: String,
    pub status: MarkdownPatchStatus,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkdownPatchSet {
    pub version: String,
    pub run_id: StableId,
    pub base_snapshot: crate::MarkdownSnapshot,
    pub base_input_fingerprint: ContentHash,
    pub patches: Vec<ResolvedMarkdownPatch>,
    pub fingerprint: ContentHash,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PatchSafetyStatus {
    SafeForB,
    RequiresARebuild,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PatchSafetyDiagnostic {
    pub code: String,
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PatchSafetyReport {
    pub status: PatchSafetyStatus,
    pub diagnostics: Vec<PatchSafetyDiagnostic>,
    pub old_block_fingerprint: ContentHash,
    pub new_block_fingerprint: ContentHash,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MarkdownPatchDecisionKind {
    Accept,
    Reject,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MarkdownPatchDecision {
    pub decision_id: StableId,
    pub run_id: StableId,
    pub patch_set_fingerprint: ContentHash,
    pub kind: MarkdownPatchDecisionKind,
    pub accepted_patch_refs: Vec<String>,
    pub actor: StableId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MarkdownPatchApplyReceipt {
    pub version: String,
    pub run_id: StableId,
    pub patch_set_fingerprint: ContentHash,
    pub base_artifact_id: StableId,
    pub patched_artifact_id: Option<StableId>,
    pub conversion_id: Option<StableId>,
    pub old_input_fingerprint: ContentHash,
    pub new_input_fingerprint: Option<ContentHash>,
    pub safety: PatchSafetyReport,
    pub status: MarkdownPatchStatus,
}
