//! Untrusted, provider-neutral discovery output. Serde checks the wire shape only.
//! Range authorization, coverage, reference closure and evidence require a validator.
//! These DTOs deliberately do not implement EntityPayload or convert into a MapPatch.

use serde::{Deserialize, Serialize};

use crate::{
    ContentHash, SourceSelector, SourceUnitKind, SourceUnitRelationKind, UnitCompleteness,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DiscoveryProposalVersion {
    #[serde(rename = "mathscape-discovery-proposal-1")]
    V1,
}

/// References only blocks visible in DiscoveryInput. Whole-document authority is
/// reserved for user selection; a producer must name explicit endpoints.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DiscoverySelector {
    BlockRange {
        first: String,
        last: String,
    },
    QuoteRange {
        first: String,
        start_quote: String,
        last: String,
        end_quote: String,
    },
}

/// Syntax reuse only: this conversion does not authorize the selected range.
impl From<DiscoverySelector> for SourceSelector {
    fn from(selector: DiscoverySelector) -> Self {
        match selector {
            DiscoverySelector::BlockRange { first, last } => Self::BlockRange { first, last },
            DiscoverySelector::QuoteRange {
                first,
                start_quote,
                last,
                end_quote,
            } => Self::QuoteRange {
                first,
                start_quote,
                last,
                end_quote,
            },
        }
    }
}

/// Discovery excludes proof steps; proofs are first identified as whole units.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryUnitKind {
    Section,
    Definition,
    Statement,
    Proof,
    Problem,
    Hint,
    Solution,
    Example,
    Remark,
    Method,
    Notation,
    Narrative,
}

impl From<DiscoveryUnitKind> for SourceUnitKind {
    fn from(kind: DiscoveryUnitKind) -> Self {
        match kind {
            DiscoveryUnitKind::Section => Self::Section,
            DiscoveryUnitKind::Definition => Self::Definition,
            DiscoveryUnitKind::Statement => Self::Statement,
            DiscoveryUnitKind::Proof => Self::Proof,
            DiscoveryUnitKind::Problem => Self::Problem,
            DiscoveryUnitKind::Hint => Self::Hint,
            DiscoveryUnitKind::Solution => Self::Solution,
            DiscoveryUnitKind::Example => Self::Example,
            DiscoveryUnitKind::Remark => Self::Remark,
            DiscoveryUnitKind::Method => Self::Method,
            DiscoveryUnitKind::Notation => Self::Notation,
            DiscoveryUnitKind::Narrative => Self::Narrative,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryUnitProposal {
    /// Proposal-local handle, never a map StableId. Syntax/uniqueness checked later.
    pub temp_id: String,
    pub unit_kind: DiscoveryUnitKind,
    pub source_label: Option<String>,
    pub source_title: Option<String>,
    pub display_title: Option<String>,
    pub selectors: Vec<DiscoverySelector>,
    /// Sole containment record; references another unit's temp_id in this proposal.
    pub parent: Option<String>,
    pub completeness: UnitCompleteness,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryRelationKind {
    Proves,
    Solves,
    Hints,
    Explains,
    Cites,
    AppliesMethod,
}

impl From<DiscoveryRelationKind> for SourceUnitRelationKind {
    fn from(kind: DiscoveryRelationKind) -> Self {
        match kind {
            DiscoveryRelationKind::Proves => Self::Proves,
            DiscoveryRelationKind::Solves => Self::Solves,
            DiscoveryRelationKind::Hints => Self::Hints,
            DiscoveryRelationKind::Explains => Self::Explains,
            DiscoveryRelationKind::Cites => Self::Cites,
            DiscoveryRelationKind::AppliesMethod => Self::AppliesMethod,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryRelationProposal {
    pub source: String,
    pub target: String,
    pub relation: DiscoveryRelationKind,
    pub evidence: Vec<DiscoverySelector>,
    /// Short explanation of the source evidence; not a proof or certification.
    pub basis: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryReferenceProposal {
    pub source: String,
    pub selector: DiscoverySelector,
    pub surface_form: String,
    pub target_label: Option<String>,
    /// Only a local candidate target, even if the proposal names it confidently.
    pub target: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryIssueKind {
    LeftTruncated,
    RightTruncated,
    InternallyMissing,
    UncertainBoundary,
    UncertainParent,
    AmbiguousReference,
    SuspectedConversionError,
    MissingContext,
    UncertainClassification,
}

/// A requested source lookup, never permission to open arbitrary files or URLs.
/// Empty struct variants ensure Serde rejects extra fields even without payloads.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DiscoveryContextRequest {
    Before {},
    After {},
    EnclosingUnit {},
    LocalReference { target_label: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryIssueProposal {
    pub kind: DiscoveryIssueKind,
    pub unit: Option<String>,
    pub evidence: Vec<DiscoverySelector>,
    pub observation: String,
    /// Attached to its issue so range, target and reason are not duplicated.
    pub context_request: Option<DiscoveryContextRequest>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum DiscoveryCoverageDisposition {
    /// Producer claims it examined the region; not proof of exhaustive discovery.
    /// An empty struct variant rejects unknown fields in this tagged object.
    Processed {},
    /// Examined, but no unit is appropriate (e.g. a running header).
    Skipped { reason: String },
    /// Not examined, for example because the output budget was exhausted.
    Unprocessed { reason: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryCoverageProposal {
    pub selector: DiscoverySelector,
    pub disposition: DiscoveryCoverageDisposition,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryProposal {
    pub version: DiscoveryProposalVersion,
    /// Echo DiscoveryInput.fingerprint; the model does not compute it.
    pub input_fingerprint: ContentHash,
    pub units: Vec<DiscoveryUnitProposal>,
    pub relations: Vec<DiscoveryRelationProposal>,
    pub references: Vec<DiscoveryReferenceProposal>,
    pub issues: Vec<DiscoveryIssueProposal>,
    pub coverage: Vec<DiscoveryCoverageProposal>,
}
