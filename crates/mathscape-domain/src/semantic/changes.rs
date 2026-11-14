//! Edit whitelists of semantic objects (contract S4.3). An absent field
//! keeps its value. Identity fields are not representable here: changing
//! them means proposing a new object.
use super::*;
use crate::FieldChange;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticEntityChanges {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_labels: Option<Vec<PreferredLabel>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aliases: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "FieldChange::is_keep")]
    pub plain_label: FieldChange<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_theory: Option<String>,
    /// Same keys, subjects and values as before, in the same order; only
    /// the wording (`text`, `basis`, …) may change or new entries be added.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conventions: Option<Vec<Convention>>,
    #[serde(default, skip_serializing_if = "FieldChange::is_keep")]
    pub mathlib_baseline: FieldChange<MathlibBaseline>,
    #[serde(default, skip_serializing_if = "FieldChange::is_keep")]
    pub description: FieldChange<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uses_entities: Option<Vec<String>>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AlignmentChanges {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation: Option<AlignmentRelation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_context: Option<SourceContext>,
    #[serde(default, skip_serializing_if = "FieldChange::is_keep")]
    pub target_context: FieldChange<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameter_map: Option<Vec<ParameterMapEntry>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub term_index_map: Option<Vec<TermIndexMapEntry>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_assumptions: Option<Vec<RequiredAssumption>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lost_information: Option<Vec<LostInformation>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_spans: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<AlignmentReviewStatus>,
    #[serde(default, skip_serializing_if = "FieldChange::is_keep")]
    pub map_label: FieldChange<String>,
    #[serde(default, skip_serializing_if = "FieldChange::is_keep")]
    pub plain_note: FieldChange<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DebtChanges {
    /// Only `open` ↔ `claimed_paid` through an edit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<DebtStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paid_by: Option<Vec<PaidByRef>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub obligation: Option<String>,
    #[serde(default, skip_serializing_if = "FieldChange::is_keep")]
    pub failure_note: FieldChange<String>,
    #[serde(default, skip_serializing_if = "FieldChange::is_keep")]
    pub payoff_hint: FieldChange<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DialectScopeChanges {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conventions: Option<Vec<Convention>>,
}

/// Result of one server-side mathlib lookup (decision S13-8). Only the
/// baseline re-check entry point builds this; no proposal compiles to it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BaselineCheckOutcome {
    /// `confirmed` or `not_found`; `unchecked` is rejected.
    pub status: BaselineStatus,
    pub check: BaselineCheck,
}

/// Debt settlement a user sets explicitly on the review page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DebtSettlement {
    /// Confirm a `claimed_paid` debt as paid.
    Paid,
    /// Waive an open or claimed debt (a reason is required).
    Waived,
    /// Re-open a claimed, paid or waived debt.
    Open,
}
