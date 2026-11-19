//! Deterministic continuation of a discovery run after Markdown patch review.
//!
//! The workflow pauses between A and B with a pending patch set. A reviewer's
//! accepted subset is applied here, the patched document is re-anchored and the
//! frozen UnitSet is rebound when the safety gate allows it. No artifact is
//! written; promotion into the Artifact Pool stays with the application shell.

use mathscape_domain::{
    AnchoredMarkdown, CanonicalDiscoveryIssue, ContentHash, DiscoveryInput, DiscoveryInputSegment,
    DiscoveryIssueKind, MarkdownPatchSet, PatchSafetyDiagnostic, PatchSafetyReport,
    PatchSafetyStatus, StableId, TextByteRange,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::markdown_patch::{
    AppliedMarkdownPatch, PatchApplyError, apply_patch_set, evaluate_patch_safety,
    markdown_patch_artifact_id, markdown_patch_content_hash, markdown_patch_conversion_id,
    patch_set_fingerprint,
};
use crate::{
    AnchorError, DiscoveryInputError, PreparedUnitDiscoveryRequest, ValidatedUnitSet,
    anchor_markdown, canonical_discovery_issues, prepare_unit_discovery_request_from_input,
    rebind_discovery_input, rebind_discovery_input_after_patch, rebind_unit_set_after_patch,
};

/// Whether a discovery run may stop between A and B for Markdown patch review.
/// `Skip` keeps the historical A→B path and passes no source warnings to B.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkdownPatchGate {
    #[default]
    Skip,
    ReviewBeforeApply,
}

/// Audit record of the patch applied inside this run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AppliedPatchRecord {
    pub patch_set_fingerprint: ContentHash,
    pub accepted_patch_refs: Vec<String>,
    pub base_artifact_id: StableId,
    pub patched_artifact_id: StableId,
    pub conversion_id: StableId,
    pub base_input_fingerprint: ContentHash,
    pub new_input_fingerprint: ContentHash,
    pub safety: PatchSafetyReport,
    /// Replacement ranges in the patched Markdown; the UnitSet that enters B
    /// must still own every one of them.
    pub new_patch_ranges: Vec<TextByteRange>,
}

/// Patch state carried by a workflow snapshot.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct MarkdownPatchWorkflowState {
    pub gate: MarkdownPatchGate,
    pub rounds: usize,
    pub round_exhausted: bool,
    pub pending: Option<MarkdownPatchSet>,
    pub applied: Option<AppliedPatchRecord>,
    /// A was rebuilt on the patched snapshot and its issues still need to be
    /// collected as B warnings.
    pub rebuild_pending: bool,
    pub source_warnings: Vec<CanonicalDiscoveryIssue>,
}

#[derive(Debug, Error)]
pub enum PatchReviewError {
    #[error("the discovery run is not awaiting Markdown patch review")]
    NotAwaiting,
    #[error("Markdown patch review is stale: {0}")]
    Stale(String),
    #[error("patch selection is invalid: {0}")]
    Selection(String),
    #[error(transparent)]
    Apply(#[from] PatchApplyError),
    #[error("patched Markdown cannot be anchored: {0}")]
    Anchor(#[from] AnchorError),
    #[error("patched selection cannot be rebound: {0}")]
    Input(#[from] DiscoveryInputError),
    #[error(transparent)]
    Domain(#[from] mathscape_domain::DomainError),
}

/// Reviewer outcome for the pending patch set of a paused run.
pub enum PatchReviewDecision {
    /// Nothing is applied; B runs on the original snapshot and every
    /// conversion issue becomes a read-only warning.
    RejectAll,
    Apply(Box<PatchContinuation>),
    /// The accepted patches were persisted by the application shell as
    /// block-level errata over the unchanged physical snapshot. B continues
    /// on the original input; every conversion issue stays a B warning so
    /// fact extraction knows these spans carry OCR damage in the raw bytes.
    AppliedAsErrata {
        accepted_patch_refs: Vec<String>,
    },
}

/// True when any evidence range of `issue` overlaps one of `ranges`.
/// Used to keep blocks that already carry human errata out of the P round,
/// so a model patch over raw OCR can never supersede a manual correction.
pub fn issue_touches_ranges(issue: &CanonicalDiscoveryIssue, ranges: &[TextByteRange]) -> bool {
    issue.evidence.iter().any(|evidence| {
        ranges
            .iter()
            .any(|range| range.start < evidence.end && evidence.start < range.end)
    })
}

/// Result of applying an accepted patch subset. `rebound` is present only
/// when the patch is `SafeForB` and the frozen UnitSet survived rebinding.
pub struct PatchContinuation {
    patch_set: MarkdownPatchSet,
    applied: AppliedMarkdownPatch,
    document: AnchoredMarkdown,
    input: DiscoveryInput,
    safety: PatchSafetyReport,
    rebound: Option<ValidatedUnitSet>,
}

impl PatchContinuation {
    pub fn patch_set(&self) -> &MarkdownPatchSet {
        &self.patch_set
    }
    pub fn applied(&self) -> &AppliedMarkdownPatch {
        &self.applied
    }
    /// The patched Markdown, anchored under its derived artifact identity.
    pub fn document(&self) -> &AnchoredMarkdown {
        &self.document
    }
    pub fn input(&self) -> &DiscoveryInput {
        &self.input
    }
    pub fn safety(&self) -> &PatchSafetyReport {
        &self.safety
    }
    pub fn patched_artifact_id(&self) -> &StableId {
        &self.document.snapshot.artifact_id
    }
    pub fn conversion_id(&self) -> Option<&StableId> {
        self.document.snapshot.conversion_id.as_ref()
    }
    pub fn rebinds_units(&self) -> bool {
        self.rebound.is_some()
    }
    /// The request the workflow must be resumed with.
    pub fn prepared(&self) -> PreparedUnitDiscoveryRequest {
        prepare_unit_discovery_request_from_input(self.input.clone())
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        MarkdownPatchSet,
        AppliedMarkdownPatch,
        DiscoveryInput,
        PatchSafetyReport,
        Option<ValidatedUnitSet>,
    ) {
        (
            self.patch_set,
            self.applied,
            self.input,
            self.safety,
            self.rebound,
        )
    }
}

fn input_blocks(input: &DiscoveryInput) -> Vec<TextByteRange> {
    input
        .requested
        .segments
        .iter()
        .filter_map(|segment| match segment {
            DiscoveryInputSegment::Block { source_range, .. } => Some(source_range.clone()),
            DiscoveryInputSegment::Gap { .. } => None,
        })
        .collect()
}

/// P v1 can only repair an issue whose every evidence range lies inside
/// exactly one physical block of this input.
pub(crate) fn is_patchable_issue(input: &DiscoveryInput, issue: &CanonicalDiscoveryIssue) -> bool {
    if issue.kind != DiscoveryIssueKind::SuspectedConversionError || issue.evidence.is_empty() {
        return false;
    }
    let blocks = input_blocks(input);
    issue.evidence.iter().all(|evidence| {
        let mut overlapping = blocks
            .iter()
            .filter(|block| block.end > evidence.start && block.start < evidence.end);
        matches!(
            (overlapping.next(), overlapping.next()),
            (Some(block), None) if block.start <= evidence.start && evidence.end <= block.end
        )
    })
}

pub(crate) fn conversion_warnings(
    units: &ValidatedUnitSet,
    excluded_refs: &[&str],
) -> Vec<CanonicalDiscoveryIssue> {
    canonical_discovery_issues(units)
        .into_iter()
        .filter(|issue| {
            issue.kind == DiscoveryIssueKind::SuspectedConversionError
                && !excluded_refs.contains(&issue.issue_ref.as_str())
        })
        .collect()
}

/// Every range must lie inside one validated unit selector.
pub fn ranges_covered(units: &ValidatedUnitSet, ranges: &[TextByteRange]) -> bool {
    ranges.iter().all(|range| {
        units.units().iter().any(|unit| {
            unit.selectors().iter().any(|selector| {
                selector.range().start <= range.start && range.end <= selector.range().end
            })
        })
    })
}

fn unit_ranges(units: &ValidatedUnitSet) -> Vec<(String, Vec<TextByteRange>)> {
    units
        .units()
        .iter()
        .map(|unit| {
            (
                unit.proposal().unit_ref.clone(),
                unit.selectors()
                    .iter()
                    .map(|selector| selector.range().clone())
                    .collect(),
            )
        })
        .collect()
}

fn check_selection(
    pending: &MarkdownPatchSet,
    selected: &MarkdownPatchSet,
) -> Result<(), PatchReviewError> {
    if selected.patches.is_empty() {
        return Err(PatchReviewError::Selection("no patch is accepted".into()));
    }
    if selected.run_id != pending.run_id
        || selected.base_snapshot != pending.base_snapshot
        || selected.base_input_fingerprint != pending.base_input_fingerprint
    {
        return Err(PatchReviewError::Stale(
            "accepted patches belong to another base snapshot".into(),
        ));
    }
    for patch in &selected.patches {
        let known = pending.patches.iter().any(|candidate| {
            candidate.patch_ref == patch.patch_ref
                && candidate.issue_ref == patch.issue_ref
                && candidate.range == patch.range
                && candidate.old_text_hash == patch.old_text_hash
        });
        if !known {
            return Err(PatchReviewError::Selection(format!(
                "patch {} is not part of the pending review",
                patch.patch_ref
            )));
        }
    }
    if patch_set_fingerprint(selected) != selected.fingerprint {
        return Err(PatchReviewError::Selection(
            "accepted patch set fingerprint does not match its patches".into(),
        ));
    }
    Ok(())
}

/// Applies an accepted subset of the pending patch set to the exact base
/// Markdown and decides whether the frozen UnitSet can be carried into B.
pub(crate) fn prepare_continuation(
    base_document: &AnchoredMarkdown,
    source: &DiscoveryInput,
    units: &ValidatedUnitSet,
    pending: &MarkdownPatchSet,
    selected: &MarkdownPatchSet,
) -> Result<PatchContinuation, PatchReviewError> {
    if base_document.snapshot != source.snapshot {
        return Err(PatchReviewError::Stale(
            "base Markdown snapshot no longer matches the discovery input".into(),
        ));
    }
    rebind_discovery_input(base_document, source, Default::default())?;
    check_selection(pending, selected)?;

    let applied = apply_patch_set(&base_document.source, selected)?;
    let artifact_id =
        markdown_patch_artifact_id(&markdown_patch_content_hash(applied.source.as_bytes()))?;
    let conversion_id = markdown_patch_conversion_id(selected)?;
    let document = anchor_markdown(artifact_id, Some(conversion_id), applied.source.clone())?;
    let mut safety = evaluate_patch_safety(base_document, &document, selected, &unit_ranges(units));
    // New headings, list items or fences are not a conversion fix; they are
    // refused outright instead of being downgraded to an A rebuild.
    if safety
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "block_structure_changed")
    {
        return Err(PatchReviewError::Selection(
            "accepted patches change the Markdown block structure".into(),
        ));
    }
    let input = rebind_discovery_input_after_patch(
        &document,
        source,
        &applied.offset_map,
        Default::default(),
    )?;
    let mut rebound = None;
    if safety.status == PatchSafetyStatus::SafeForB {
        let base_prepared = prepare_unit_discovery_request_from_input(source.clone());
        match rebind_unit_set_after_patch(&base_prepared, units, input.clone(), &applied.offset_map)
        {
            Ok((_, units)) => rebound = Some(units),
            Err(report) => {
                safety.status = PatchSafetyStatus::RequiresARebuild;
                safety.diagnostics.push(PatchSafetyDiagnostic {
                    code: "unit_set_rebind_failed".into(),
                    path: "unit_set".into(),
                    message: report
                        .diagnostics
                        .iter()
                        .map(|diagnostic| format!("{}: {}", diagnostic.path, diagnostic.code))
                        .collect::<Vec<_>>()
                        .join("; "),
                });
            }
        }
    }
    Ok(PatchContinuation {
        patch_set: selected.clone(),
        applied,
        document,
        input,
        safety,
        rebound,
    })
}
