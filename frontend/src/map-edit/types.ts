/**
 * TypeScript contracts matching the Rust DTOs in `mathscape-domain/src/map_edit.rs`.
 *
 * These types are the single source of truth for the frontend side of the
 * user-map-edit protocol. Field names use snake_case to match the JSON
 * serialization produced by serde on the Rust side.
 */

// ---------------------------------------------------------------------------
// FieldChange — three-state edit signal
// ---------------------------------------------------------------------------

export type FieldChange<T> =
  | { action: "keep" }
  | { action: "set"; value: T }
  | { action: "clear" };

export function keep<T>(): FieldChange<T> {
  return { action: "keep" };
}
export function set<T>(value: T): FieldChange<T> {
  return { action: "set", value };
}
export function clear<T>(): FieldChange<T> {
  return { action: "clear" };
}

// ---------------------------------------------------------------------------
// Drafts and changes
// ---------------------------------------------------------------------------

export interface SourceUnitDraft {
  kind: string;
  display_title: string | null;
  parent: string | null;
  source_spans: string[];
  completeness: UnitCompleteness;
}

export type UnitCompleteness =
  | "complete"
  | "left_truncated"
  | "right_truncated"
  | "internally_missing"
  | "uncertain";

export interface SourceUnitChanges {
  display_title?: FieldChange<string>;
  parent?: FieldChange<string>;
  completeness?: UnitCompleteness | null;
}

export interface SourceRelationDraft {
  relation: string;
  source: string;
  target: string;
  evidence_spans: string[];
}

export interface SourceRelationChanges {
  relation?: string | null;
  source?: string | null;
  target?: string | null;
  evidence_spans?: string[] | null;
}

// ---------------------------------------------------------------------------
// UserChangeRequest — the request envelope sent to the backend
// ---------------------------------------------------------------------------

export type UserChangeOperation =
  | {
      operation: "create_source_unit";
      object_id: string;
      draft: SourceUnitDraft;
    }
  | {
      operation: "create_source_relation";
      object_id: string;
      draft: SourceRelationDraft;
    }
  | {
      operation: "edit_source_unit";
      object_id: string;
      expected_revision: string;
      changes: SourceUnitChanges;
    }
  | {
      operation: "edit_source_relation";
      object_id: string;
      expected_revision: string;
      changes: SourceRelationChanges;
    }
  | {
      operation: "retire_object";
      object_id: string;
      expected_revision: string;
    }
  /** 用户专用：审阅一条对齐线（审核页 / Inspector 的对象级按钮）。 */
  | {
      operation: "set_alignment_review";
      object_id: string;
      expected_revision: string;
      status: string;
      note: string | null;
    }
  | {
      operation: "set_representation_binding_review";
      object_id: string;
      expected_revision: string;
      status: string;
      note: string | null;
    }
  /** 用户专用：确认已还清、免除（需理由）或重新打开一笔账。 */
  | {
      operation: "set_debt_settlement";
      object_id: string;
      expected_revision: string;
      settlement: "paid" | "waived" | "open";
      note: string | null;
    };

export interface UserChangeRequest {
  request_id: string;
  base_commit: string | null;
  reason: string;
  operations: UserChangeOperation[];
}

// ---------------------------------------------------------------------------
// EditContext — returned by get_map_object_edit_context
// ---------------------------------------------------------------------------

export interface EditOption {
  object_id: string;
  label: string;
}

export interface EditableField {
  name: string;
  field_type: string;
  value: unknown;
  constraints: string[];
}

export interface EditContext {
  version: string;
  base_commit: string | null;
  object_id: string;
  current_revision: string;
  lifecycle: string;
  kind: string;
  editable_fields: EditableField[];
  available_parents: EditOption[];
  related_relations: EditOption[];
  evidence_spans: EditOption[];
  can_retire: boolean;
}

// ---------------------------------------------------------------------------
// ImpactPlan — returned by plan_map_change
// ---------------------------------------------------------------------------

export type ImpactPlanStatus = "ready" | "blocked" | "conflicted";

export interface ImpactDiagnostic {
  code: string;
  object_id: string | null;
  message: string;
}

export interface ImpactItem {
  object_id: string;
  kind: string;
  action: string;
  reason: string;
  current_revision: string | null;
  proposed_revision: string | null;
  display_title?: string | null;
  reason_codes?: string[];
  related_objects?: { object_id: string; revision_id: string; kind: string; display_title: string | null; reference_kind: string }[];
  protected_by_decisions?: string[];
}

export interface ImpactPlan {
  version: string;
  request_id: string;
  base_commit: string | null;
  plan_fingerprint: string;
  status: ImpactPlanStatus;
  direct_changes: ImpactItem[];
  affected_objects: ImpactItem[];
  diagnostics: ImpactDiagnostic[];
}

// ---------------------------------------------------------------------------
// CommitReceipt — returned by commit_map_change
// ---------------------------------------------------------------------------

export interface ObjectRevisionReceipt {
  object_id: string;
  kind: string;
  previous_revision: string | null;
  current_revision: string;
  lifecycle: string;
  action: string;
}

export interface CommitReceipt {
  version: string;
  request_id: string;
  plan_fingerprint: string;
  base_commit: string | null;
  commit_id: string;
  committed_at: string;
  objects: ObjectRevisionReceipt[];
}
