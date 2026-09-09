import { mockIPC } from "@tauri-apps/api/mocks";

import { LlmSettingsController } from "../src/llm/settings";

interface Profile {
  display_name: string;
  id: string;
  provider: string;
  wire_protocol: string;
  model: string;
  endpoint: string | null;
  is_default: boolean;
  has_credential: boolean;
}

let snapshot: { profiles: Profile[]; default_profile_id: string | null } = {
  profiles: [], default_profile_id: null,
};
const saves: unknown[] = [];
const workspaceSaves: unknown[] = [];
const toasts: unknown[] = [];
let workspaceSettings = {
  version: "mathscape-workspace-settings-1",
  discovery_admission: "review_before_publish",
  discovery_timeout_seconds: 600,
  connect_timeout_seconds: 15,
};

mockIPC((command, payload) => {
  if (command === "get_workspace_settings") return workspaceSettings;
  if (command === "save_workspace_settings") {
    const request = (payload as { request: Record<string, any> }).request;
    workspaceSaves.push(request);
    workspaceSettings = { version: "mathscape-workspace-settings-1", ...request } as typeof workspaceSettings;
    return workspaceSettings;
  }
  if (command === "get_llm_settings") return snapshot;
  if (command === "set_default_llm_profile") {
    snapshot.default_profile_id = (payload as any).profileId;
    snapshot.profiles.forEach(p => p.is_default = p.id === snapshot.default_profile_id);
    return snapshot;
  }
  if (command === "save_llm_profile") {
    const request = (payload as { request: Record<string, any> }).request;
    saves.push(request);
    const endpoint = request.endpoint && !request.endpoint.endsWith("/")
      ? request.endpoint + "/" : request.endpoint;
    const old = snapshot.profiles.find(p => p.id === request.profile_id);
    snapshot = {
      default_profile_id: snapshot.default_profile_id ?? request.profile_id,
      profiles: [...snapshot.profiles.filter(p => p.id !== request.profile_id), {
        display_name: request.display_name,
        id: request.profile_id,
        provider: request.provider,
        wire_protocol: request.wire_protocol,
        model: request.model,
        endpoint,
        is_default: (snapshot.default_profile_id ?? request.profile_id) === request.profile_id,
        has_credential: request.clear_api_key ? false
          : Boolean(request.api_key) || old?.has_credential === true,
      }],
    };
    return snapshot;
  }
  if (command === "test_llm_profile") return {
    profile_id: snapshot.default_profile_id,
    provider_model: "relay-model-resolved",
    elapsed_ms: 1250,
    response_preview: "MATHSCAPE_OK",
    input_tokens: 12,
    output_tokens: 4,
  };
  throw new Error("Unexpected IPC: " + command);
});

const controller = new LlmSettingsController(
  document.querySelector("#settings-host")!,
  document.querySelector("#settings-trigger")!,
  document.querySelector("#settings-label")!,
  (message, kind) => toasts.push({ message, kind }),
);
void controller.loadSummary().then(() => Object.assign(window, {
  harness: { controller, saves, workspaceSaves, toasts },
}));
