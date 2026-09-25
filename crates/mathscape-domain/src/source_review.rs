//! Read-only projection contract for reviewing source discovery results.
//!
//! This is a UI-facing view model, not a canonical entity and not an
//! acceptance command. The backend must still validate an acceptance request
//! and construct a SourceRegistrationPlan.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    ContentHash, DiscoveryRelationKind, DiscoveryUnitKind, StableId, TextByteRange,
    UnitCompleteness,
};

pub const SOURCE_REVIEW_PROJECTION_VERSION: &str = "mathscape-source-review-projection-1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceReviewRunStatus {
    Running,
    AwaitingPatchReview,
    Completed,
    Partial,
    NeedsAttention,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceReviewFactStatus {
    Pending,
    Processed,
    RequiresUnitRevision,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceReviewReferenceStatus {
    Unresolved,
    Candidate,
    ResolvedLocal,
    Ambiguous,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceReviewDiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceReviewAcceptanceState {
    NotSelected,
    Selectable,
    Blocked,
    Accepted,
    Registered,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceReviewAnchorPurpose {
    Unit,
    SourceTitle,
    SourceLabel,
    Reference,
    RelationEvidence,
    DiagnosticEvidence,
}

/// A lossless source location used by every interactive review surface.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceReviewAnchor {
    pub purpose: SourceReviewAnchorPurpose,
    pub range: TextByteRange,
    pub block_ids: Vec<String>,
    pub excerpt: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceReviewTextFact {
    pub text: String,
    pub anchor: SourceReviewAnchor,
    pub normalized: bool,
    pub precision_review: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceReviewUnit {
    pub unit_ref: String,
    pub unit_kind: DiscoveryUnitKind,
    pub display_title: Option<String>,
    pub source_title: Option<SourceReviewTextFact>,
    pub source_labels: Vec<SourceReviewTextFact>,
    pub parent: Option<String>,
    pub completeness: UnitCompleteness,
    pub fact_status: SourceReviewFactStatus,
    pub acceptance: SourceReviewAcceptanceState,
    pub anchors: Vec<SourceReviewAnchor>,
    pub source_excerpt: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceReviewReference {
    pub reference_ref: String,
    pub owner: String,
    pub text: String,
    pub anchor: SourceReviewAnchor,
    pub status: SourceReviewReferenceStatus,
    pub candidates: Vec<String>,
    pub reason: String,
    pub origins: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceReviewRelation {
    pub relation_ref: String,
    pub source: String,
    pub target: String,
    pub relation: DiscoveryRelationKind,
    pub evidence: Vec<SourceReviewAnchor>,
    pub bases: Vec<String>,
    pub origins: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceReviewStructureRelation {
    pub relation_ref: String,
    pub parent: String,
    pub child: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceReviewDiagnostic {
    pub diagnostic_ref: String,
    pub severity: SourceReviewDiagnosticSeverity,
    pub stage: String,
    pub code: String,
    pub path: String,
    pub message: String,
    pub checked: bool,
    pub related_units: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceReviewPendingTask {
    pub task_ref: String,
    pub owner: Option<String>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceReviewSource {
    pub artifact_id: StableId,
    pub conversion_id: Option<StableId>,
    pub content_hash: ContentHash,
    pub input_fingerprint: ContentHash,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceReviewAcceptanceScope {
    AllValidated,
    ExplicitUnits { unit_refs: Vec<String> },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceReviewAcceptance {
    pub state: SourceReviewAcceptanceState,
    pub selected_units: Vec<String>,
    pub required_units: Vec<String>,
    pub missing_targets: Vec<String>,
    pub blockers: Vec<String>,
    pub allowed_scopes: Vec<SourceReviewAcceptanceScope>,
}

/// Complete read-only payload consumed by a reusable source review module.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceReviewProjection {
    pub version: String,
    pub run_id: StableId,
    pub result_fingerprint: ContentHash,
    pub status: SourceReviewRunStatus,
    pub complete: bool,
    pub source: SourceReviewSource,
    pub units: Vec<SourceReviewUnit>,
    pub references: Vec<SourceReviewReference>,
    pub structure_relations: Vec<SourceReviewStructureRelation>,
    pub relations: Vec<SourceReviewRelation>,
    pub diagnostics: Vec<SourceReviewDiagnostic>,
    pub pending_tasks: Vec<SourceReviewPendingTask>,
    pub acceptance: SourceReviewAcceptance,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(digit: char) -> ContentHash {
        ContentHash::parse(format!("sha256:{}", digit.to_string().repeat(64))).unwrap()
    }

    #[test]
    fn projection_round_trips_without_canonical_entity_fields() {
        let projection = SourceReviewProjection {
            version: SOURCE_REVIEW_PROJECTION_VERSION.into(),
            run_id: StableId::parse("mathscape:discovery-run/review-fixture").unwrap(),
            result_fingerprint: hash('3'),
            status: SourceReviewRunStatus::Partial,
            complete: false,
            source: SourceReviewSource {
                artifact_id: StableId::parse("mathscape:artifact/markdown").unwrap(),
                conversion_id: None,
                content_hash: hash('1'),
                input_fingerprint: hash('2'),
            },
            units: vec![SourceReviewUnit {
                unit_ref: "u1".into(),
                unit_kind: DiscoveryUnitKind::Statement,
                display_title: Some("命题".into()),
                source_title: None,
                source_labels: vec![],
                parent: None,
                completeness: UnitCompleteness::Complete,
                fact_status: SourceReviewFactStatus::Pending,
                acceptance: SourceReviewAcceptanceState::Selectable,
                anchors: vec![SourceReviewAnchor {
                    purpose: SourceReviewAnchorPurpose::Unit,
                    range: TextByteRange { start: 0, end: 9 },
                    block_ids: vec!["b1".into()],
                    excerpt: "命题".into(),
                }],
                source_excerpt: "命题".into(),
            }],
            references: vec![],
            structure_relations: vec![],
            relations: vec![],
            diagnostics: vec![],
            pending_tasks: vec![SourceReviewPendingTask {
                task_ref: "t1".into(),
                owner: Some("u1".into()),
                reason: Some("等待事实发现".into()),
            }],
            acceptance: SourceReviewAcceptance {
                state: SourceReviewAcceptanceState::Selectable,
                selected_units: vec![],
                required_units: vec![],
                missing_targets: vec![],
                blockers: vec![],
                allowed_scopes: vec![SourceReviewAcceptanceScope::AllValidated],
            },
        };
        let value = serde_json::to_value(&projection).unwrap();
        assert_eq!(value["version"], SOURCE_REVIEW_PROJECTION_VERSION);
        assert!(value.get("trust").is_none());
        assert!(value.get("stable_id").is_none());
        assert_eq!(
            serde_json::from_value::<SourceReviewProjection>(value).unwrap(),
            projection
        );
    }
}
