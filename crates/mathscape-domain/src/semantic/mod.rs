//! Semantic layer of the map (EMIR v0.2, first slice): shared objects
//! (`SemanticEntity`), alignments, map debts and a minimal dialect scope.
//!
//! Contract: `docs/semantic-write-contract.md`. These payloads are stored in
//! the same `{payload_kind, payload}` envelope as [`SourcePayload`]; the
//! `payload_kind` values never overlap, so [`DomainPayload`] can wrap both
//! without changing the shape of rows already in a workspace.
//!
//! Drafts (`*Draft`) are what an Agent or a user may propose. Fields that
//! only the server may author (trust, derived alignment axis, confirmed
//! mathlib baselines, user-only statuses) are absent from drafts and are
//! filled by the planner.
use crate::{
    ActorRef, AlignmentStatus, Confidence, CurationStatus, EntityKind, EntityPayload, ParseStatus,
    PayloadShape, ProofStatus, ProvenanceStatus, SegmentationStatus, SourcePayload, TrustRecord,
    TypingStatus,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

mod changes;
mod display;
pub use changes::*;
pub use display::*;

/// Schema version written in the header of every semantic map object.
pub const SEMANTIC_SCHEMA_VERSION: &str = "mathscape-semantic-1";

/// 唯一能写 `RecordMathlibBaseline` 的作者：服务端在本地 profile 的 mathlib
/// 源码里核对名称后写入。
pub const MATHLIB_BASELINE_ACTOR: &str = "system:mathlib-baseline";

/// Stable id prefixes of server-generated semantic objects (contract S3 & S14).
pub const SEMANTIC_ENTITY_ID_KIND: &str = "semantic-entity";
pub const ALIGNMENT_ID_KIND: &str = "alignment";
pub const DEBT_ID_KIND: &str = "debt";
pub const DIALECT_SCOPE_ID_KIND: &str = "dialect-scope";
pub const REPRESENTATION_CORE_ID_KIND: &str = "representation-core";
pub const REPRESENTATION_BINDING_ID_KIND: &str = "representation-binding";

/// Either layer's payload. Untagged: both inner enums carry their own
/// `payload_kind` and their value spaces are disjoint.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum DomainPayload {
    Source(SourcePayload),
    Semantic(SemanticPayload),
}

impl EntityPayload for DomainPayload {
    fn entity_kind(&self) -> EntityKind {
        match self {
            Self::Source(payload) => payload.entity_kind(),
            Self::Semantic(payload) => payload.entity_kind(),
        }
    }

    fn payload_shape(&self) -> PayloadShape {
        match self {
            Self::Source(payload) => payload.payload_shape(),
            Self::Semantic(payload) => payload.payload_shape(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "payload_kind", content = "payload", rename_all = "snake_case")]
pub enum SemanticPayload {
    SemanticEntity(SemanticEntity),
    Alignment(Alignment),
    Debt(DebtRecord),
    DialectScope(DialectScope),
    RepresentationCore(RepresentationCore),
    RepresentationBinding(RepresentationBinding),
}

impl SemanticPayload {
    /// The `payload_kind` → entity kind table shared with untyped readers.
    pub fn entity_kind_of(payload_kind: &str) -> Option<EntityKind> {
        Some(match payload_kind {
            "semantic_entity" => EntityKind::SemanticEntity,
            "alignment" => EntityKind::Alignment,
            "debt" => EntityKind::Debt,
            "dialect_scope" => EntityKind::DialectScope,
            "representation_core" => EntityKind::RepresentationCore,
            "representation_binding" => EntityKind::RepresentationBinding,
            _ => return None,
        })
    }
}

impl EntityPayload for SemanticPayload {
    fn entity_kind(&self) -> EntityKind {
        match self {
            Self::SemanticEntity(_) => EntityKind::SemanticEntity,
            Self::Alignment(_) => EntityKind::Alignment,
            Self::Debt(_) => EntityKind::Debt,
            Self::DialectScope(_) => EntityKind::DialectScope,
            Self::RepresentationCore(_) => EntityKind::RepresentationCore,
            Self::RepresentationBinding(_) => EntityKind::RepresentationBinding,
        }
    }
    // Alignments and representation bindings are revisable entities that the
    // map draws as edges (contract S2.4, S14.4), so every semantic payload has
    // the entity shape.
}

// ---------------------------------------------------------------- shared

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreferredLabel {
    pub lang: String,
    pub label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConventionKey {
    SequenceIndexOrigin,
    Notation,
    Terminology,
    Baseline,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Explicitness {
    Explicit,
    Inherited,
    Conventional,
    Inferred,
}

/// Scalar value of a convention (`0`, `1`, `"mathlib"`, `true`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ConventionValue {
    Bool(bool),
    Integer(i64),
    Text(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Convention {
    pub key: ConventionKey,
    pub subject: String,
    pub value: ConventionValue,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basis: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explicitness: Option<Explicitness>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_spans: Vec<String>,
}

/// Structured text (no expression AST, contract S2.3). `evidence_spans`
/// are wording evidence, never ownership.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StructuredText {
    pub text: String,
    #[serde(default = "markdown_tex")]
    pub format: String,
    #[serde(default)]
    pub evidence_spans: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn markdown_tex() -> String {
    "markdown+tex".into()
}

// ---------------------------------------------------------- SemanticEntity

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SemanticKind {
    Concept,
    Declaration,
    Theory,
    ProblemSchema,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeclarationKind {
    Definition,
    Axiom,
    Theorem,
    Lemma,
    Proposition,
    Conjecture,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BinderRole {
    Universal,
    Index,
    Parameter,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Binder {
    pub name: String,
    #[serde(rename = "type")]
    pub binder_type: String,
    pub role: BinderRole,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GuardPartitionStatus {
    Partition,
    DisjointPartial,
    Overlapping,
    Unspecified,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GuardPartition {
    pub status: GuardPartitionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GuardSpec {
    pub guard_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub predicate: StructuredText,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_spans: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeclarationBranch {
    pub branch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guards: Vec<String>,
    #[serde(default)]
    pub is_degenerate: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub binders: Vec<Binder>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assumptions: Vec<StructuredText>,
    pub conclusion: StructuredText,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_spans: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProblemBranch {
    pub branch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guards: Vec<String>,
    #[serde(default)]
    pub is_degenerate: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<StructuredText>,
    pub task: StructuredText,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_spans: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExclusionKind {
    Undefined,
    Diverges,
    OutOfScope,
    Counterexample,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExclusionSpec {
    pub case: StructuredText,
    pub kind: ExclusionKind,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_spans: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeclarationBody {
    pub declaration_kind: DeclarationKind,
    #[serde(default)]
    pub binders: Vec<Binder>,
    #[serde(default)]
    pub assumptions: Vec<StructuredText>,
    #[serde(default)]
    pub statement: Option<StructuredText>,
    #[serde(default)]
    pub definiens: Option<StructuredText>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guards: Vec<GuardSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard_partition: Option<GuardPartition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub branches: Vec<DeclarationBranch>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclusions: Vec<ExclusionSpec>,
    #[serde(default)]
    pub uses_entities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concept: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProblemSchemaBody {
    pub parameters: Vec<Binder>,
    #[serde(default)]
    pub assumptions: Vec<StructuredText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<StructuredText>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guards: Vec<GuardSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard_partition: Option<GuardPartition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub branches: Vec<ProblemBranch>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclusions: Vec<ExclusionSpec>,
    #[serde(default)]
    pub known_instances: Vec<Value>,
    #[serde(default)]
    pub uses_entities: Vec<String>,
}

/// mathlib reference baseline (contract S2.2, decision S13-8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BaselineStatus {
    /// Written by an Agent or user; nobody has looked the name up yet.
    Unchecked,
    /// The server found the name in the local profile's mathlib.
    Confirmed,
    /// The server looked in the local profile's mathlib and did not find it.
    NotFound,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BaselineCheckMethod {
    MathlibSource,
    Loogle,
}

/// Server-authored record of one baseline lookup.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BaselineCheck {
    pub mathlib_version: String,
    pub profile_id: String,
    pub method: BaselineCheckMethod,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MathlibBaseline {
    pub decl: Option<String>,
    #[serde(default)]
    pub module: Option<String>,
    #[serde(default)]
    pub relation_hint: Option<AlignmentRelation>,
    #[serde(default)]
    pub note: Option<String>,
    pub status: BaselineStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<BaselineCheck>,
}

/// What an Agent or user may write for a shared object.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticEntityDraft {
    pub semantic_kind: SemanticKind,
    #[serde(default)]
    pub home_theory: Option<String>,
    pub preferred_labels: Vec<PreferredLabel>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub plain_label: Option<String>,
    #[serde(default)]
    pub conventions: Vec<Convention>,
    #[serde(default)]
    pub mathlib_baseline: Option<MathlibBaseline>,
    #[serde(default)]
    pub declaration: Option<Box<DeclarationBody>>,
    #[serde(default)]
    pub problem_schema: Option<Box<ProblemSchemaBody>>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Stored shared object: the draft plus server-authored trust.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SemanticEntity {
    #[serde(flatten)]
    pub draft: SemanticEntityDraft,
    pub trust: TrustRecord,
}

// --------------------------------------------------------------- Alignment

/// EMIR §8.2 relations. There is deliberately no `same_as`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AlignmentRelation {
    Expresses,
    Specializes,
    Generalizes,
    InstanceOf,
    RepresentationOf,
    LogicallyEquivalent,
    DefinitionallyEquivalent,
    ExactRealization,
    NotationFor,
    OverlapsWith,
    IncompatibleWith,
}

/// Review status of one alignment (EMIR §8.3). Different from the coarse
/// trust axis [`AlignmentStatus`], which is derived from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AlignmentReviewStatus {
    Unmatched,
    RetrievedCandidate,
    ContextChecked,
    PartiallyAligned,
    Ambiguous,
    Disputed,
    Incompatible,
    FormallyRelated,
    SemanticallyReviewed,
    Accepted,
    Rejected,
    Superseded,
}

impl AlignmentReviewStatus {
    /// Values an Agent may write (contract S2.4).
    pub fn agent_writable(self) -> bool {
        matches!(
            self,
            Self::Unmatched
                | Self::RetrievedCandidate
                | Self::ContextChecked
                | Self::PartiallyAligned
                | Self::Ambiguous
                | Self::Disputed
                | Self::Incompatible
        )
    }

    /// Values only an explicit user review may set. `formally_related` is
    /// not open in the first slice; `rejected` / `superseded` are system.
    pub fn user_reviewable(self) -> bool {
        matches!(self, Self::SemanticallyReviewed | Self::Accepted) || self.agent_writable()
    }

    /// The fixed mapping to the trust axis (decision S13-3). The trust axis
    /// is never written directly.
    pub fn trust_axis(self) -> AlignmentStatus {
        match self {
            Self::Unmatched | Self::Rejected | Self::Superseded => AlignmentStatus::Unmatched,
            Self::RetrievedCandidate
            | Self::ContextChecked
            | Self::PartiallyAligned
            | Self::Ambiguous
            | Self::FormallyRelated => AlignmentStatus::Candidate,
            Self::SemanticallyReviewed => AlignmentStatus::Reviewed,
            Self::Accepted => AlignmentStatus::Accepted,
            Self::Disputed | Self::Incompatible => AlignmentStatus::Disputed,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum SourceContext {
    Scoped {
        dialect_scope: String,
    },
    Uncontextualized {
        binding: UncontextualizedBinding,
        reason: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UncontextualizedBinding {
    Uncontextualized,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParameterRole {
    StatementParameter,
    ProblemParameter,
}

/// "Statement n ↔ statement n+1": which statement or problem parameter
/// corresponds to which. Not a sequence index map.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParameterMapEntry {
    pub source: String,
    pub target: String,
    pub role: ParameterRole,
    pub map: String,
    #[serde(default)]
    pub source_range: Option<String>,
    #[serde(default)]
    pub target_range: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

/// Term-by-term correspondence of sequences (`f_k ↔ F_{k+shift}`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TermIndexMapEntry {
    pub source: String,
    pub target: String,
    pub index_shift: i64,
    #[serde(default)]
    pub valid_for: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequiredAssumption {
    pub text: String,
    #[serde(default)]
    pub debt: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LostDirection {
    TargetHasSourceLacks,
    SourceHasTargetLacks,
    Both,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LostInformation {
    pub short: String,
    pub claim: String,
    pub direction: LostDirection,
    #[serde(default)]
    pub see_also: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AlignmentDraft {
    pub source: String,
    pub target: String,
    pub relation: AlignmentRelation,
    pub source_context: SourceContext,
    #[serde(default)]
    pub target_context: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_branches: Vec<String>,
    #[serde(default)]
    pub parameter_map: Vec<ParameterMapEntry>,
    #[serde(default)]
    pub term_index_map: Vec<TermIndexMapEntry>,
    #[serde(default)]
    pub required_assumptions: Vec<RequiredAssumption>,
    #[serde(default)]
    pub lost_information: Vec<LostInformation>,
    pub evidence_spans: Vec<String>,
    pub status: AlignmentReviewStatus,
    #[serde(default)]
    pub map_label: Option<String>,
    #[serde(default)]
    pub plain_note: Option<String>,
}

/// Who last changed an alignment's review status or a debt's settlement,
/// and why. Server-authored.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewMark {
    pub by: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Alignment {
    #[serde(flatten)]
    pub draft: AlignmentDraft,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed: Option<ReviewMark>,
    pub trust: TrustRecord,
}

// -------------------------------------------------------------------- Debt

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DebtKind {
    ParseDebt,
    ScopeDebt,
    TypingDebt,
    AssumptionDebt,
    AlignmentDebt,
    FormalizationDebt,
    ProofDebt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DebtStatus {
    Open,
    ClaimedPaid,
    Paid,
    Waived,
    Obsolete,
}

impl DebtStatus {
    /// An Agent writes at most `claimed_paid` (decision S13-1).
    pub fn agent_writable(self) -> bool {
        matches!(self, Self::Open | Self::ClaimedPaid)
    }
    pub fn is_outstanding(self) -> bool {
        matches!(self, Self::Open | Self::ClaimedPaid)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FailureMode {
    Theorem,
    Statement,
    Route,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PaidByRef {
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DebtDraft {
    pub debt_kind: DebtKind,
    pub attached_to: String,
    pub status: DebtStatus,
    pub obligation: String,
    pub quantifier: String,
    pub failure_mode: FailureMode,
    #[serde(default)]
    pub failure_note: Option<String>,
    #[serde(default)]
    pub evidence_spans: Vec<String>,
    #[serde(default)]
    pub paid_by: Vec<PaidByRef>,
    #[serde(default)]
    pub payoff_hint: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DebtRecord {
    #[serde(flatten)]
    pub draft: DebtDraft,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settled: Option<ReviewMark>,
}

// ------------------------------------------------------------ DialectScope

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScopeApplication {
    /// The anchor unit and every descendant along its parent chain.
    AnchorSubtree,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LocalSymbolRole {
    Sequence,
    Series,
    Polynomial,
    Function,
    Operator,
    Variable,
    Index,
    Constant,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LocalSymbolEntry {
    pub symbol: String,
    pub role: LocalSymbolRole,
    pub meaning: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<StructuredText>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_spans: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DialectScopeDraft {
    pub label: String,
    pub anchor_unit: String,
    pub applies_to: ScopeApplication,
    /// Always null in the first slice (no inheritance tree).
    #[serde(default)]
    pub parent_scope: Option<String>,
    #[serde(default)]
    pub introduced_by_unit: Option<String>,
    pub conventions: Vec<Convention>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub local_symbols: Vec<LocalSymbolEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DialectScope {
    #[serde(flatten)]
    pub draft: DialectScopeDraft,
}

// ------------------------------------------------------ RepresentationCore

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RepresentationOperation {
    pub name: String,
    pub signature: StructuredText,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RepresentationCoreDraft {
    pub home_theory: String,
    pub preferred_labels: Vec<PreferredLabel>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub plain_label: Option<String>,
    pub carrier: StructuredText,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub represented_object: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub binders: Vec<Binder>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<RepresentationOperation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invariants_and_domain: Vec<StructuredText>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conventions: Vec<Convention>,
    #[serde(default)]
    pub mathlib_baseline: Option<MathlibBaseline>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uses_entities: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RepresentationCore {
    #[serde(flatten)]
    pub draft: RepresentationCoreDraft,
    pub trust: TrustRecord,
}

// --------------------------------------------------- RepresentationBinding

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SymbolBindingEntry {
    pub local_symbol: String,
    pub target_expr: StructuredText,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<LocalSymbolRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RepresentationBindingDraft {
    pub dialect_scope: String,
    pub representation: String,
    pub symbol_map: Vec<SymbolBindingEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub index_and_domain_adaptations: Vec<StructuredText>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lost_information: Vec<LostInformation>,
    pub evidence_spans: Vec<String>,
    pub status: AlignmentReviewStatus,
    #[serde(default)]
    pub map_label: Option<String>,
    #[serde(default)]
    pub plain_note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RepresentationBinding {
    #[serde(flatten)]
    pub draft: RepresentationBindingDraft,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed: Option<ReviewMark>,
    pub trust: TrustRecord,
}

/// Default trust of a newly proposed semantic object (contract S7): nothing
/// is promoted by proposing or accepting. `alignment` comes from
/// [`AlignmentReviewStatus::trust_axis`] for alignments and is `Unmatched`
/// otherwise.
pub fn semantic_trust(alignment: AlignmentStatus, actor: &ActorRef) -> TrustRecord {
    TrustRecord::new(
        ProvenanceStatus::Located,
        ParseStatus::Raw,
        SegmentationStatus::Proposed,
        TypingStatus::Unchecked,
        ProofStatus::None,
        alignment,
        CurationStatus::Raw,
        Confidence::new(0.0).expect("zero confidence is valid"),
        vec![],
        vec![actor.clone()],
    )
}

#[cfg(test)]
mod tests;
