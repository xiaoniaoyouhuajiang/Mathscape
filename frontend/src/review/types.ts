import type { SelectionRequest, TextRange } from "../reader/selection";

export interface DiscoveryAnchorView {
  start: number;
  end: number;
  block_ids: string[];
  excerpt: string;
  purpose?: string;
}

export interface LocatedTextView {
  fact_ref: string;
  text: string;
  anchor: DiscoveryAnchorView;
  normalized: boolean;
  precision_review: boolean;
}

export interface DiscoveryUnitView {
  unit_ref: string;
  unit_kind: string;
  title: string;
  display_title: string | null;
  parent: string | null;
  completeness: string;
  fact_status: "processed" | "pending" | "requires_unit_revision";
  source_excerpt: string;
  anchors: DiscoveryAnchorView[];
  source_labels: LocatedTextView[];
  source_title: LocatedTextView | null;
}

export interface DiscoveryReferenceView {
  owner: string;
  text: string;
  anchor: DiscoveryAnchorView;
  status: "unresolved" | "candidate" | "resolved_local" | "ambiguous";
  candidates: string[];
  reason: string;
  origins: string[];
}

export interface DiscoveryRelationView {
  source: string;
  target: string;
  relation: string;
  evidence: DiscoveryAnchorView[];
  bases: string[];
  origins: string[];
}

export interface DiscoveryIssueView {
  issue_ref: string;
  issue_refs?: string[];
  stage: "unit_discovery" | "source_facts";
  stages?: ("unit_discovery" | "source_facts")[];
  owner: string | null;
  kind: string;
  observation: string;
  observations?: string[];
  occurrences?: number;
  requires_unit_revision: boolean;
  evidence: DiscoveryAnchorView[];
}

export interface DiscoveryDiagnosticView {
  report: number;
  reports: number[];
  occurrences: number;
  code: string;
  path: string;
  message: string;
  checked: boolean;
}

import type { WorkflowStatus } from "../discovery/types";
export type { WorkflowStatus };

export interface DiscoverySource {
  artifact_id: string;
  conversion_id: string | null;
  content_hash: string;
  input_fingerprint: string;
  /** The user's original selection; absent in outcomes from older builds. */
  requested_range?: TextRange;
}

export interface DesktopDiscoveryView {
  version: "mathscape-desktop-discovery-view-1";
  status: WorkflowStatus;
  complete: boolean;
  source: DiscoverySource | null;
  units: DiscoveryUnitView[];
  references: DiscoveryReferenceView[];
  relations: DiscoveryRelationView[];
  issues: DiscoveryIssueView[];
  pending_tasks: { task_ref: string; owner: string | null }[];
  diagnostics: DiscoveryDiagnosticView[];
  attempts: { attempt: number; stage: string; repair: boolean; status: WorkflowStatus; error: string | null; total_tokens: number | null; elapsed_ms: number | null }[];
}

export interface DiscoveryRun {
  run_id: string;
  profile_id: string;
  provider: string;
  requested_model: string;
  admission: SelectionRequest["admission"];
  request_timeout_seconds: number | null;
  connect_timeout_seconds: number | null;
  attempts: number;
  input_tokens: number | null;
  output_tokens: number | null;
  total_tokens: number | null;
  elapsed_ms: number;
}

export type DiscoveryOutcome = {
  outcome: WorkflowStatus;
  view: DesktopDiscoveryView;
  review: SourceReviewProjection | null;
  run: DiscoveryRun;
};

/** Wire shape of `SourceReviewAnchor`: offsets live under `range`. */
export interface SourceReviewAnchor {
  purpose: string;
  range: TextRange;
  block_ids: string[];
  excerpt: string;
}

export interface SourceReviewUnit {
  unit_ref: string;
  unit_kind: string;
  display_title: string | null;
  source_title: { text: string; anchor: SourceReviewAnchor } | null;
  source_labels: { text: string; anchor: SourceReviewAnchor }[];
  parent: string | null;
  completeness: string;
  fact_status: string;
  acceptance: string;
  anchors: SourceReviewAnchor[];
  source_excerpt: string;
}

export interface SourceReviewProjection {
  version: "mathscape-source-review-projection-1";
  run_id: string;
  result_fingerprint: string;
  status: WorkflowStatus;
  complete: boolean;
  source: { artifact_id: string; conversion_id: string | null; content_hash: string; input_fingerprint: string };
  units: SourceReviewUnit[];
  references: { reference_ref: string; owner: string; text: string; anchor: SourceReviewAnchor; status: string; candidates: string[]; reason: string; origins: string[] }[];
  structure_relations: { relation_ref: string; parent: string; child: string }[];
  relations: { relation_ref: string; source: string; target: string; relation: string; evidence: SourceReviewAnchor[]; bases: string[]; origins: string[] }[];
  diagnostics: { diagnostic_ref: string; severity: string; stage: string; code: string; path: string; message: string; checked: boolean; related_units: string[] }[];
  pending_tasks: { task_ref: string; owner: string | null; reason: string | null }[];
  acceptance: { state: string; selected_units: string[]; required_units: string[]; missing_targets: string[]; blockers: string[]; allowed_scopes: unknown[] };
}

export interface DiscoveryAcceptanceOutcome {
  state: "registered" | "withdrawn" | "registration_review_required";
  registration_plan?: import("../discovery/registration-types").SourceRegistrationCommitPlan | null;
  decision_id: string;
  run_id: string;
  result_fingerprint: string;
  plan_fingerprint: string;
  selected_units: string[];
  partial: boolean;
  span_count: number;
  unit_count: number;
  reference_count: number;
  relation_count: number;
  pending_task_count: number;
  already_accepted: boolean;
  commit_id: string;
  created_object_count: number;
  reused_object_count: number;
  already_registered: boolean;
  reactivated_object_count?: number;
  revised_object_count?: number;
}

/** Envelope metadata from the run ledger; absent for a run that just finished. */
export interface ReviewRunMeta {
  title?: string;
  review_status?: string;
  created_at?: string;
  updated_at?: string;
  registrations?: import("../discovery/registration-types").RegistrationSummary[];
  disposition?: import("../discovery/types").DiscoveryDispositionReceipt|null;
  /**
   * Set when the workspace shows a Phase2 proposal (「提案」 data source)
   * instead of a discovery run. Units cannot be picked one by one: the
   * change set is accepted or discarded whole through `controller`.
   */
  proposal?: import("./proposal-model").ProposalWorkspaceMeta;
  /** Opens or keeps the workspace in Map-Linked split-drawer mode alongside the main Cytoscape map. */
  mapLinked?: boolean;
}
