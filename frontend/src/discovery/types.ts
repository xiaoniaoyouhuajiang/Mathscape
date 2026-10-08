import type { DiscoveryOutcome } from "../review/types";
import type { RegistrationSummary } from "./registration-types";

export type WorkflowStatus =
  | "running"
  | "awaiting_patch_review"
  | "completed"
  | "partial"
  | "needs_attention"
  | "failed"
  | "cancelled";

export type ReviewStatus =
  | "processing"
  | "reviewable"
  | "accepted"
  | "registered"
  | "withdrawn"
  | "discarded"
  | "diagnostic"
  | "terminal";

export interface DiscoveryRunSummaryView {
  run_id: string;
  artifact_id: string;
  artifact_name: string | null;
  conversion_id: string | null;
  input_fingerprint: string;
  result_fingerprint: string | null;
  pipeline_version: string;
  status: string;
  review_status: ReviewStatus;
  profile_id: string;
  unit_count: number;
  fact_count: number;
  pending_task_count: number;
  unresolved_reference_count: number;
  relation_count: number;
  created_at: string;
  updated_at: string;
  registrations?: RegistrationSummary[];
}

export interface DiscoveryRunAttemptView {
  ordinal: number;
  stage: string;
  repair: boolean;
  status: string;
  error: string | null;
  response_received: boolean;
  elapsed_ms: number | null;
}

export interface DiscoveryDiagnosticItemView {
  report: number;
  code: string;
  path: string;
  message: string;
  checked: boolean;
}

export interface DiscoveryRunDiagnosticsView {
  run_id: string;
  status: string;
  review_status: string;
  result_fingerprint: string | null;
  message: string;
  attempts: DiscoveryRunAttemptView[];
  diagnostics: DiscoveryDiagnosticItemView[];
  events: string[];
}

export interface DiscoveryReviewEnvelopeView {
  run_id: string;
  review_status: string;
  created_at: string;
  updated_at: string;
  outcome: DiscoveryOutcome;
  registrations?: RegistrationSummary[];
}

export interface WorkflowEvent {
  patches?: number;
  applied?: boolean;
  rebuild_units?: boolean;
  kind: string;
  attempt?: number;
  stage?: string;
  batch_index?: number | null;
  batch_total?: number | null;
  repair?: boolean;
  units?: number;
  batches?: number;
  tasks?: number;
  completed_tasks?: number;
  total_tasks?: number;
  status?: WorkflowStatus;
}

export type DiscoveryProgress =
  | { kind: "started"; profile: string; model: string }
  | { kind: "workflow_event"; event: WorkflowEvent }
  | {
      kind: "stream_delta" | "response_completed";
      attempt: number;
      stage: string;
      batch_index: number | null;
      batch_total: number | null;
      stream?: string;
      text?: string;
    };
