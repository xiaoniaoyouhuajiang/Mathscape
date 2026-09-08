import { mockIPC } from "@tauri-apps/api/mocks";

import { LlmSettingsController } from "../src/llm/settings";

interface Profile {
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
const toasts: unknown[] = [];

mockIPC((command, payload) => {
  if (command === "get_llm_settings") return snapshot;
  if (command === "save_llm_profile") {
    const request = (payload as { request: Record<string, any> }).request;
    saves.push(request);
    const endpoint = request.endpoint && !request.endpoint.endsWith("/")
      ? request.endpoint + "/" : request.endpoint;
    snapshot = {
      default_profile_id: request.profile_id,
      profiles: [{
        id: request.profile_id,
        provider: request.provider,
        wire_protocol: request.wire_protocol,
        model: request.model,
        endpoint,
        is_default: true,
        has_credential: request.clear_api_key ? false
          : Boolean(request.api_key) || snapshot.profiles[0]?.has_credential === true,
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
  harness: { controller, saves, toasts },
}));
