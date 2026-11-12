//! User intent and review contracts. JSON values occur only in read snapshots,
//! never as arbitrary user-supplied replacements.
use crate::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UserChangeRequest {
    pub request_id: String,
    pub base_commit: Option<String>,
    pub reason: String,
    pub operations: Vec<UserChangeOperation>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum UserChangeOperation {
    CreateSourceUnit {
        object_id: String,
        draft: SourceUnitDraft,
    },
    CreateSourceRelation {
        object_id: String,
        draft: SourceRelationDraft,
    },
    CreateSourceSpan {
        object_id: String,
        draft: SourceSpanDraft,
    },
    EditSourceUnit {
        object_id: String,
        expected_revision: String,
        changes: SourceUnitChanges,
    },
    EditSourceRelation {
        object_id: String,
        expected_revision: String,
        changes: SourceRelationChanges,
    },
    RetireObject {
        object_id: String,
        expected_revision: String,
    },
    // ---- semantic layer (docs/semantic-write-contract.md S4.2)
    CreateSemanticEntity {
        object_id: String,
        draft: SemanticEntityDraft,
    },
    EditSemanticEntity {
        object_id: String,
        expected_revision: String,
        changes: SemanticEntityChanges,
    },
    CreateAlignment {
        object_id: String,
        draft: AlignmentDraft,
    },
    EditAlignment {
        object_id: String,
        expected_revision: String,
        changes: AlignmentChanges,
    },
    CreateDebt {
        object_id: String,
        draft: DebtDraft,
    },
    EditDebt {
        object_id: String,
        expected_revision: String,
        changes: DebtChanges,
    },
    CreateDialectScope {
        object_id: String,
        draft: DialectScopeDraft,
    },
    EditDialectScope {
        object_id: String,
        expected_revision: String,
        changes: DialectScopeChanges,
    },
    /// User-only: explicit alignment review on the review page. Never the
    /// side effect of accepting a proposal; no proposal compiles to it.
    SetAlignmentReview {
        object_id: String,
        expected_revision: String,
        status: AlignmentReviewStatus,
        #[serde(default)]
        note: Option<String>,
    },
    /// User-only: confirm, waive or re-open a map debt.
    SetDebtSettlement {
        object_id: String,
        expected_revision: String,
        settlement: DebtSettlement,
        #[serde(default)]
        note: Option<String>,
    },
    /// Server-only: result of a mathlib baseline lookup in the local
    /// profile. Desktop mutation commands refuse it from the UI.
    RecordMathlibBaseline {
        object_id: String,
        expected_revision: String,
        outcome: BaselineCheckOutcome,
    },
}
impl UserChangeOperation {
    pub fn object_id(&self) -> &str {
        match self {
            Self::CreateSourceUnit { object_id, .. }
            | Self::CreateSourceRelation { object_id, .. }
            | Self::CreateSourceSpan { object_id, .. }
            | Self::EditSourceUnit { object_id, .. }
            | Self::EditSourceRelation { object_id, .. }
            | Self::RetireObject { object_id, .. }
            | Self::CreateSemanticEntity { object_id, .. }
            | Self::EditSemanticEntity { object_id, .. }
            | Self::CreateAlignment { object_id, .. }
            | Self::EditAlignment { object_id, .. }
            | Self::CreateDebt { object_id, .. }
            | Self::EditDebt { object_id, .. }
            | Self::CreateDialectScope { object_id, .. }
            | Self::EditDialectScope { object_id, .. }
            | Self::SetAlignmentReview { object_id, .. }
            | Self::SetDebtSettlement { object_id, .. }
            | Self::RecordMathlibBaseline { object_id, .. } => object_id,
        }
    }
    /// Operations only an explicit user action on the review page may
    /// produce. An Agent proposal never compiles to them.
    pub fn is_user_only(&self) -> bool {
        matches!(
            self,
            Self::SetAlignmentReview { .. } | Self::SetDebtSettlement { .. }
        )
    }
    /// Operations only the server itself authors.
    pub fn is_server_only(&self) -> bool {
        matches!(self, Self::RecordMathlibBaseline { .. })
    }
    /// Object id created by this operation, if it creates one.
    pub fn created_object_id(&self) -> Option<&str> {
        match self {
            Self::CreateSourceUnit { object_id, .. }
            | Self::CreateSourceRelation { object_id, .. }
            | Self::CreateSourceSpan { object_id, .. }
            | Self::CreateSemanticEntity { object_id, .. }
            | Self::CreateAlignment { object_id, .. }
            | Self::CreateDebt { object_id, .. }
            | Self::CreateDialectScope { object_id, .. } => Some(object_id),
            _ => None,
        }
    }
    pub fn expected_revision(&self) -> Option<&str> {
        match self {
            Self::EditSourceUnit {
                expected_revision, ..
            }
            | Self::EditSourceRelation {
                expected_revision, ..
            }
            | Self::RetireObject {
                expected_revision, ..
            }
            | Self::EditSemanticEntity {
                expected_revision, ..
            }
            | Self::EditAlignment {
                expected_revision, ..
            }
            | Self::EditDebt {
                expected_revision, ..
            }
            | Self::EditDialectScope {
                expected_revision, ..
            }
            | Self::SetAlignmentReview {
                expected_revision, ..
            }
            | Self::SetDebtSettlement {
                expected_revision, ..
            }
            | Self::RecordMathlibBaseline {
                expected_revision, ..
            } => Some(expected_revision),
            _ => None,
        }
    }
}

/// Fully materialized, server-authored source evidence used by the proposal
/// planner. Agents never send this structure directly: `propose_changes`
/// validates a locator and fills the fields before creating the operation.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceSpanDraft {
    pub granularity: SpanGranularity,
    pub layout: LayoutKind,
    pub page: Option<PageCoordinate>,
    pub bounding_boxes: Vec<BoundingBox>,
    pub marker_block_ids: Vec<String>,
    pub markdown: Option<MarkdownLocator>,
    pub exact_text_artifact: Option<StableId>,
    pub normalized_text_hash: Option<ContentHash>,
}

// Explicit tagged changes avoid serde's missing/null ambiguity for Option<Option<T>>.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(
    tag = "action",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum FieldChange<T> {
    #[default]
    Keep,
    Set(T),
    Clear,
}
impl<T> FieldChange<T> {
    pub fn is_keep(&self) -> bool {
        matches!(self, Self::Keep)
    }
    /// Applies the change to a nullable field.
    pub fn apply_to(&self, field: &mut Option<T>)
    where
        T: Clone,
    {
        match self {
            Self::Keep => {}
            Self::Set(value) => *field = Some(value.clone()),
            Self::Clear => *field = None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceUnitDraft {
    pub kind: DiscoveryUnitKind,
    pub display_title: Option<String>,
    pub parent: Option<String>,
    pub source_spans: Vec<String>,
    pub completeness: UnitCompleteness,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct SourceUnitChanges {
    #[serde(default)]
    pub display_title: FieldChange<String>,
    #[serde(default)]
    pub parent: FieldChange<String>,
    pub completeness: Option<UnitCompleteness>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceRelationDraft {
    pub relation: DiscoveryRelationKind,
    pub source: String,
    pub target: String,
    pub evidence_spans: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct SourceRelationChanges {
    pub relation: Option<DiscoveryRelationKind>,
    pub source: Option<String>,
    pub target: Option<String>,
    pub evidence_spans: Option<Vec<String>>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImpactPlanStatus {
    Ready,
    Blocked,
    Conflicted,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ImpactDiagnostic {
    pub code: String,
    pub object_id: Option<String>,
    pub message: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ImpactItem {
    pub object_id: String,
    pub kind: String,
    pub action: String,
    pub reason: String,
    pub current_revision: Option<String>,
    pub proposed_revision: Option<String>,
    #[serde(default)]
    pub display_title: Option<String>,
    #[serde(default)]
    pub reason_codes: Vec<String>,
    #[serde(default)]
    pub related_objects: Vec<ImpactObjectRef>,
    #[serde(default)]
    pub protected_by_decisions: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ImpactObjectRef {
    pub object_id: String,
    pub revision_id: String,
    pub kind: String,
    pub display_title: Option<String>,
    pub reference_kind: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ImpactPlan {
    pub version: String,
    pub request_id: String,
    pub base_commit: Option<String>,
    pub plan_fingerprint: String,
    pub status: ImpactPlanStatus,
    pub direct_changes: Vec<ImpactItem>,
    pub affected_objects: Vec<ImpactItem>,
    pub diagnostics: Vec<ImpactDiagnostic>,
    /// Problems that already exist in the head on objects the request does
    /// not touch. Only the staged-proposal planner fills this; they do not
    /// block and are shown as "workspace object problems".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub head_issues: Vec<ImpactDiagnostic>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ObjectRevisionReceipt {
    pub object_id: String,
    pub kind: String,
    pub previous_revision: Option<String>,
    pub current_revision: String,
    pub lifecycle: String,
    pub action: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct CommitReceipt {
    pub version: String,
    pub request_id: String,
    pub plan_fingerprint: String,
    pub base_commit: Option<String>,
    pub commit_id: String,
    pub committed_at: String,
    pub objects: Vec<ObjectRevisionReceipt>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EditOption {
    pub object_id: String,
    pub label: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EditableField {
    pub name: String,
    pub field_type: String,
    pub value: Value,
    pub constraints: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EditContext {
    pub version: String,
    pub base_commit: Option<String>,
    pub object_id: String,
    pub current_revision: String,
    pub lifecycle: String,
    pub kind: String,
    pub editable_fields: Vec<EditableField>,
    pub available_parents: Vec<EditOption>,
    pub related_relations: Vec<EditOption>,
    pub evidence_spans: Vec<EditOption>,
    pub can_retire: bool,
}

/// Backend-only snapshot shared by storage and the pure planner.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapEditObject {
    pub object_id: String,
    pub revision: String,
    pub kind: String,
    pub lifecycle: String,
    pub payload: Value,
    pub endpoints: Vec<RelationEndpoint>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapEditSnapshot {
    pub base_commit: Option<String>,
    pub objects: Vec<MapEditObject>,
    #[serde(default)]
    pub adopted_by: std::collections::BTreeMap<String, Vec<String>>,
}

/// Internal materialized payload, validated by the planner and transaction
/// invariants. This type is never accepted by a Desktop mutation command.
#[derive(Clone, Debug, Serialize)]
#[serde(transparent)]
pub struct MapEditPayload(pub Value);
impl EntityPayload for MapEditPayload {
    fn entity_kind(&self) -> EntityKind {
        match self.0["payload_kind"].as_str() {
            Some("unit_relation") => EntityKind::SourceUnitRelation,
            Some("span") => EntityKind::SourceSpan,
            Some(kind) => {
                SemanticPayload::entity_kind_of(kind).unwrap_or(EntityKind::SourceSemanticUnit)
            }
            None => EntityKind::SourceSemanticUnit,
        }
    }
    fn payload_shape(&self) -> PayloadShape {
        if self.entity_kind() == EntityKind::SourceUnitRelation {
            PayloadShape::Relation
        } else {
            PayloadShape::Entity
        }
    }
}
