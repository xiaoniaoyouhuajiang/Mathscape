//! Registration adoption, withdrawal and explicitly approved re-registration.
use crate::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetirementPolicy {
    IncidentRelations,
    RegistrationScoped,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationState {
    Registered,
    Withdrawn,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReverseDiscoveryRequest {
    pub request_id: String,
    pub decision_id: String,
    pub registration_commit_id: String,
    pub base_commit: Option<String>,
    pub reason: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReversalCounts {
    pub total: u32,
    pub retire: u32,
    pub preserve: u32,
    pub already_inactive: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiscoveryReversalPlan {
    pub version: String,
    pub request: ReverseDiscoveryRequest,
    pub impact: ImpactPlan,
    pub summary: ReversalCounts,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitDiscoveryReversalRequest {
    pub request: ReverseDiscoveryRequest,
    pub expected_plan_fingerprint: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiscoveryReversalReceipt {
    pub version: String,
    pub request_id: String,
    pub decision_id: String,
    pub registration_commit_id: String,
    pub plan_fingerprint: String,
    pub state: RegistrationState,
    pub decided_at: String,
    pub actor: ActorRef,
    pub reason: String,
    pub map_commit: Option<CommitReceipt>,
    pub retired_objects: Vec<String>,
    pub preserved_objects: Vec<String>,
    pub already_inactive_objects: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegistrationSummary {
    pub decision_id: String,
    pub registration_commit_id: String,
    pub run_id: String,
    pub state: RegistrationState,
    pub registered_at: String,
    pub selected_unit_refs: Vec<String>,
    pub object_count: u32,
    pub reversal_receipt: Option<DiscoveryReversalReceipt>,
}
/// Authoritative adoption claims, including revision-bound history. Not an IPC mutation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegistrationMember {
    pub object_id: String,
    pub revision_id: String,
    pub outcome: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegistrationAdoption {
    pub summary: RegistrationSummary,
    pub members: Vec<RegistrationMember>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegistrationSnapshot {
    pub map: MapEditSnapshot,
    pub registrations: Vec<RegistrationAdoption>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistrationReplacementApproval {
    pub object_id: String,
    pub expected_revision: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRegistrationCommitRequest {
    pub request_id: String,
    pub decision_id: String,
    pub acceptance_plan_fingerprint: String,
    pub base_commit: Option<String>,
    pub replacement_approvals: Vec<RegistrationReplacementApproval>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationObjectAction {
    Create,
    Reuse,
    Reactivate,
    Revise,
    Block,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegistrationObjectChange {
    pub object_id: String,
    pub action: RegistrationObjectAction,
    pub display_title: Option<String>,
    pub current_revision: Option<String>,
    pub proposed_revision: Option<String>,
    pub changed_fields: Vec<String>,
    pub before: Option<Value>,
    pub proposed: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceRegistrationCommitPlan {
    pub version: String,
    pub request: SourceRegistrationCommitRequest,
    pub impact: ImpactPlan,
    pub requires_confirmation: bool,
    pub objects: Vec<RegistrationObjectChange>,
}
