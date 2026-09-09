import { invoke } from "@tauri-apps/api/core";

export type DiscoveryAdmissionPreference =
  | "review_before_publish"
  | "automatic_candidate"
  | "ask_each_run";

export interface WorkspaceSettings {
  version: "mathscape-workspace-settings-1";
  discovery_admission: DiscoveryAdmissionPreference;
  discovery_timeout_seconds: number;
  connect_timeout_seconds: number;
}

export interface SaveWorkspaceSettingsRequest {
  discovery_admission: DiscoveryAdmissionPreference;
  discovery_timeout_seconds: number;
  connect_timeout_seconds: number;
}

export const WORKSPACE_SETTINGS_CHANGED = "mathscape:workspace-settings-changed";

export function getWorkspaceSettings(): Promise<WorkspaceSettings> {
  return invoke("get_workspace_settings");
}

export function saveWorkspaceSettings(
  request: SaveWorkspaceSettingsRequest,
): Promise<WorkspaceSettings> {
  return invoke("save_workspace_settings", { request });
}

export function announceWorkspaceSettings(settings: WorkspaceSettings): void {
  window.dispatchEvent(new CustomEvent<WorkspaceSettings>(WORKSPACE_SETTINGS_CHANGED, {
    detail: settings,
  }));
}
