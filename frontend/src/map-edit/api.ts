/**
 * Typed Tauri IPC wrappers for the map-edit commands.
 */
import { invoke } from "@tauri-apps/api/core";
import type {
  CommitReceipt,
  EditContext,
  ImpactPlan,
  UserChangeRequest,
} from "./types";

export async function getEditContext(objectId: string): Promise<EditContext> {
  return invoke<EditContext>("get_map_object_edit_context", {
    objectId,
  });
}

export async function planMapChange(
  request: UserChangeRequest,
): Promise<ImpactPlan> {
  return invoke<ImpactPlan>("plan_map_change", { request });
}

export async function commitMapChange(
  request: UserChangeRequest,
  planFingerprint: string,
): Promise<CommitReceipt> {
  return invoke<CommitReceipt>("commit_map_change", {
    request,
    planFingerprint,
  });
}

/** StableId-shaped idempotency key. A bare UUID is rejected by the planner. */
export function generateRequestId(): string {
  return `mathscape:map-change/${crypto.randomUUID()}`;
}
