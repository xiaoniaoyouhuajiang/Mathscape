//! Untrusted stage-specific transport contracts. None of these types grants
//! registration authority; frozen/validated values are constructed by orchestration.

use crate::{
    ContentHash, DiscoveryContextRequest, DiscoveryCoverageProposal, DiscoveryIssueKind,
    DiscoveryIssueProposal, DiscoveryQuoteSelector, DiscoveryRelationKind, DiscoverySelector,
    DiscoveryUnitKind, UnitCompleteness,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const DISCOVERY_PIPELINE_VERSION: &str = "mathscape-discovery-pipeline-2";
pub const UNIT_DISCOVERY_VERSION: &str = "mathscape-unit-discovery-1";
pub const UNIT_DISCOVERY_INPUT_VERSION: &str = "mathscape-unit-discovery-input-1";
pub const SOURCE_FACT_VERSION: &str = "mathscape-source-facts-1";
pub const SOURCE_FACT_INPUT_VERSION: &str = "mathscape-source-facts-input-1";
pub const DISCOVERY_REPAIR_VERSION: &str = "mathscape-discovery-repair-1";
pub const STAGED_DISCOVERY_RESULT_VERSION: &str = "mathscape-discovery-result-1";

macro_rules! version {
    ($name:ident, $wire:literal) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
        pub enum $name {
            #[serde(rename = $wire)]
            V1,
        }
    };
}
version!(UnitDiscoveryVersion, "mathscape-unit-discovery-1");
version!(
    UnitDiscoveryInputVersion,
    "mathscape-unit-discovery-input-1"
);
version!(SourceFactVersion, "mathscape-source-facts-1");
version!(SourceFactInputVersion, "mathscape-source-facts-input-1");
version!(DiscoveryRepairVersion, "mathscape-discovery-repair-1");
version!(DiscoveryResultVersion, "mathscape-discovery-result-1");

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StageBlock {
    pub block_ref: String,
    pub structural_kind: String,
    pub clipped_start: bool,
    pub clipped_end: bool,
    pub text: String,
    /// Lossless whitespace or other bytes between this fragment and the next.
    pub following_gap: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StageBoundary {
    pub at_document_start: bool,
    pub at_document_end: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnitDiscoveryInput {
    pub version: UnitDiscoveryInputVersion,
    pub request_ref: String,
    pub leading_gap: String,
    pub blocks: Vec<StageBlock>,
    pub boundary: StageBoundary,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnitProposal {
    #[serde(rename = "ref")]
    pub unit_ref: String,
    pub unit_kind: DiscoveryUnitKind,
    pub display_title: Option<String>,
    pub selectors: Vec<DiscoverySelector>,
    pub parent: Option<String>,
    pub completeness: UnitCompleteness,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnitDiscoveryProposal {
    pub version: UnitDiscoveryVersion,
    pub request_ref: String,
    pub units: Vec<UnitProposal>,
    pub issues: Vec<DiscoveryIssueProposal>,
    pub coverage: Vec<DiscoveryCoverageProposal>,
}

/// Text is specified exactly once. Occurrence is one-based within the supplied
/// fragment at the chosen exact-or-safe-normalized matching tier.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TextOccurrence {
    TextOccurrence {
        block: String,
        text: String,
        occurrence: Option<u32>,
    },
    RangeOccurrence {
        selector: DiscoveryQuoteSelector,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FrozenUnitEntry {
    #[serde(rename = "ref")]
    pub unit_ref: String,
    pub unit_kind: DiscoveryUnitKind,
    pub display_title: Option<String>,
    pub parent: Option<String>,
    pub selectors: Vec<DiscoverySelector>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FactContextBlock {
    pub block_ref: String,
    pub within_owner: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FactTask {
    pub task_ref: String,
    pub owner_unit_ref: String,
    pub evidence_block_refs: Vec<String>,
    pub context_block_refs: Vec<FactContextBlock>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceFactInput {
    pub version: SourceFactInputVersion,
    pub request_ref: String,
    pub blocks: Vec<StageBlock>,
    pub units: Vec<FrozenUnitEntry>,
    pub tasks: Vec<FactTask>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FactLabel {
    #[serde(rename = "ref")]
    pub fact_ref: String,
    pub anchor: TextOccurrence,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FactReference {
    #[serde(rename = "ref")]
    pub fact_ref: String,
    pub anchor: TextOccurrence,
    pub target_hint: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FactRelation {
    #[serde(rename = "ref")]
    pub fact_ref: String,
    pub relation: DiscoveryRelationKind,
    pub target: String,
    pub evidence: Vec<DiscoverySelector>,
    pub basis: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FactIssue {
    pub kind: DiscoveryIssueKind,
    pub evidence: Vec<DiscoverySelector>,
    pub observation: String,
    pub context_request: Option<DiscoveryContextRequest>,
    pub requires_unit_revision: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum FactDisposition {
    Processed {},
    Unprocessed { reason: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FactTaskProposal {
    pub task_ref: String,
    pub disposition: FactDisposition,
    pub source_labels: Vec<FactLabel>,
    pub source_title: Option<TextOccurrence>,
    pub references: Vec<FactReference>,
    pub relations: Vec<FactRelation>,
    pub issues: Vec<FactIssue>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceFactProposal {
    pub version: SourceFactVersion,
    pub request_ref: String,
    pub tasks: Vec<FactTaskProposal>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DiscoveryRepairAction {
    ChooseCandidate {
        issue_ref: String,
        candidate_ref: String,
    },
    ReplaceUnits {
        proposal: Box<UnitDiscoveryProposal>,
    },
    ReplaceTask {
        task_ref: String,
        proposal: Box<FactTaskProposal>,
    },
    Defer {
        issue_ref: String,
        reason: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryRepairResponse {
    pub version: DiscoveryRepairVersion,
    pub repair_ref: String,
    pub actions: Vec<DiscoveryRepairAction>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RepairCandidate {
    pub candidate_ref: String,
    pub text: String,
    pub prefix: String,
    pub suffix: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RepairIssue {
    pub issue_ref: String,
    pub path: String,
    pub code: String,
    pub message: String,
    pub candidates: Vec<RepairCandidate>,
    pub has_more: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "stage", rename_all = "snake_case", deny_unknown_fields)]
pub enum RepairScope {
    Units {},
    Facts { task_refs: Vec<String> },
}

/// Server retains the base proposal fingerprint and actual candidate bindings.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryRepairInput {
    pub version: DiscoveryRepairVersion,
    pub repair_ref: String,
    pub scope: RepairScope,
    pub blocks: Vec<StageBlock>,
    pub issues: Vec<RepairIssue>,
}

/// Replay envelope, never a validated UnitSet. Importers must revalidate it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitDiscoveryCheckpoint {
    pub input_fingerprint: ContentHash,
    pub unit_set_fingerprint: ContentHash,
    pub proposal: UnitDiscoveryProposal,
}

/// Future composition wire envelope. Validation/assembly belongs to M2.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StagedDiscoveryResult {
    pub version: DiscoveryResultVersion,
    pub units: UnitDiscoveryCheckpoint,
    pub facts: Vec<SourceFactProposal>,
    pub pending_tasks: Vec<String>,
}
