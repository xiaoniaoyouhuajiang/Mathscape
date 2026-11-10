import { invoke } from "@tauri-apps/api/core";
import { eventBus } from "./core/events";

export type DiscoveryAdmissionPreference =
  | "review_before_publish"
  | "automatic_candidate"
  | "ask_each_run";

export interface WorkspaceSettings {
  version: "mathscape-workspace-settings-1";
  discovery_admission: DiscoveryAdmissionPreference;
  discovery_timeout_seconds: number;
  connect_timeout_seconds: number;
  /** 「Loogle 常驻模式」；旧版本保存的设置没有这个字段，按打开处理。 */
  loogle_resident?: boolean;
}

export interface SaveWorkspaceSettingsRequest {
  discovery_admission: DiscoveryAdmissionPreference;
  discovery_timeout_seconds: number;
  connect_timeout_seconds: number;
  loogle_resident: boolean;
}

/** Desktop 侧常驻 Loogle 的可用性（不是进程状态）。 */
export interface LoogleResidentStatus {
  available: boolean;
  forced_off: boolean;
}

export const WORKSPACE_SETTINGS_CHANGED = "mathscape:workspace-settings-changed";

export function getWorkspaceSettings(): Promise<WorkspaceSettings> {
  return invoke("get_workspace_settings");
}

export async function getLoogleResidentStatus(): Promise<LoogleResidentStatus | null> {
  const status = await invoke<LoogleResidentStatus | undefined>("get_loogle_resident_status");
  return status ?? null;
}

export function saveWorkspaceSettings(
  request: SaveWorkspaceSettingsRequest,
): Promise<WorkspaceSettings> {
  return invoke("save_workspace_settings", { request });
}
export function announceWorkspaceSettings(settings: WorkspaceSettings): void {
  eventBus.emit("workspace-settings-changed", settings);
}
