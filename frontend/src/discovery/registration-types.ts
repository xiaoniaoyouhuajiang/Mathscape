import type { ImpactPlan, CommitReceipt } from "../map-edit/types";
export interface RegistrationSourceContext {
  artifact_id: string;
  conversion_id: string | null;
}
export interface ReverseDiscoveryRequest {
  request_id: string; decision_id: string; registration_commit_id: string;
  base_commit: string | null; reason: string;
}
export interface DiscoveryReversalReceipt {
  version: string; request_id: string; decision_id: string; registration_commit_id: string;
  plan_fingerprint: string; state: "withdrawn"; decided_at: string; actor: string; reason: string;
  map_commit: CommitReceipt | null; retired_objects: string[]; preserved_objects: string[]; already_inactive_objects: string[];
}
export interface RegistrationSummary {
  decision_id: string; registration_commit_id: string; run_id: string;
  state: "registered" | "withdrawn"; registered_at: string; selected_unit_refs: string[];
  object_count: number; reversal_receipt: DiscoveryReversalReceipt | null;
}
export interface DiscoveryReversalPlan {
  version: string; request: ReverseDiscoveryRequest; impact: ImpactPlan;
  summary: { total: number; retire: number; preserve: number; already_inactive: number };
}
export interface SourceRegistrationCommitRequest {
  request_id: string; decision_id: string; acceptance_plan_fingerprint: string; base_commit: string | null;
  replacement_approvals: { object_id: string; expected_revision: string }[];
}
export interface RegistrationObjectChange {
  object_id: string; action: "create" | "reuse" | "reactivate" | "revise" | "block";
  display_title: string | null; current_revision: string | null; proposed_revision: string | null;
  changed_fields: string[]; before: unknown; proposed: unknown;
}
export interface SourceRegistrationCommitPlan {
  version: string; request: SourceRegistrationCommitRequest; impact: ImpactPlan;
  requires_confirmation: boolean; objects: RegistrationObjectChange[];
}
export interface RegistrationReceipt {
  version: string; decision_id: string; commit_id: string; committed_at: string;
  created_objects: string[]; reused_objects: string[]; reactivated_objects?: string[]; revised_objects?: string[];
}
export interface HistoricalMapObject {
  object_id: string; revision: string; kind: string; lifecycle: string;
  payload: { payload_kind: string; payload: Record<string, unknown> };
  endpoints: { role: string; entity: string; ordinal: number | null }[];
}
