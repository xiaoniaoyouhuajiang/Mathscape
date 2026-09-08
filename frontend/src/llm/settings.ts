import { invoke } from "@tauri-apps/api/core";

import "./settings.css";

type WireProtocol =
  | "open_ai_chat_completions"
  | "open_ai_responses"
  | "anthropic_messages"
  | "gemini_generate_content"
  | "ollama_native";

interface LlmProfileView {
  id: string;
  provider: string;
  wire_protocol: WireProtocol;
  model: string;
  endpoint: string | null;
  is_default: boolean;
  has_credential: boolean;
}

interface LlmSettingsSnapshot {
  profiles: LlmProfileView[];
  default_profile_id: string | null;
}

interface LlmConnectionTest {
  profile_id: string;
  provider_model: string;
  elapsed_ms: number;
  response_preview: string;
  input_tokens: number | null;
  output_tokens: number | null;
}

type Toast = (message: string, kind: "success" | "error") => void;

const presets: Record<string, { protocol: WireProtocol; needsEndpoint: boolean }> = {
  openai: { protocol: "open_ai_chat_completions", needsEndpoint: false },
  anthropic: { protocol: "anthropic_messages", needsEndpoint: false },
  gemini: { protocol: "gemini_generate_content", needsEndpoint: false },
  deepseek: { protocol: "open_ai_chat_completions", needsEndpoint: false },
  groq: { protocol: "open_ai_chat_completions", needsEndpoint: false },
  openrouter: { protocol: "open_ai_chat_completions", needsEndpoint: false },
  moonshot: { protocol: "open_ai_chat_completions", needsEndpoint: false },
  aliyun: { protocol: "open_ai_chat_completions", needsEndpoint: false },
  zai: { protocol: "open_ai_chat_completions", needsEndpoint: false },
  xai: { protocol: "open_ai_chat_completions", needsEndpoint: false },
  together: { protocol: "open_ai_chat_completions", needsEndpoint: false },
  fireworks: { protocol: "open_ai_chat_completions", needsEndpoint: false },
  aihubmix: { protocol: "open_ai_chat_completions", needsEndpoint: false },
  minimax: { protocol: "anthropic_messages", needsEndpoint: false },
  ollama: { protocol: "ollama_native", needsEndpoint: false },
  "custom-relay": { protocol: "open_ai_chat_completions", needsEndpoint: true },
};

export class LlmSettingsController {
  private readonly overlay: HTMLElement;
  private readonly form: HTMLFormElement;
  private readonly provider: HTMLSelectElement;
  private readonly protocol: HTMLSelectElement;
  private readonly model: HTMLInputElement;
  private readonly endpoint: HTMLInputElement;
  private readonly apiKey: HTMLInputElement;
  private readonly clearKey: HTMLInputElement;
  private readonly credentialState: HTMLElement;
  private readonly endpointNote: HTMLElement;
  private readonly result: HTMLElement;
  private readonly saveButton: HTMLButtonElement;
  private readonly testButton: HTMLButtonElement;
  private current: LlmProfileView | null = null;

  constructor(
    host: HTMLElement,
    private readonly trigger: HTMLButtonElement,
    private readonly triggerLabel: HTMLElement,
    private readonly toast: Toast,
  ) {
    host.innerHTML = `
      <div class="settings-overlay" hidden>
        <section class="settings-dialog" role="dialog" aria-modal="true" aria-labelledby="llm-settings-title">
          <header class="settings-header">
            <div>
              <span class="settings-eyebrow">MODEL RUNTIME</span>
              <h2 id="llm-settings-title">LLM 配置</h2>
              <p>配置保存于当前 Workspace；API key 仅存入系统钥匙串。</p>
            </div>
            <button class="settings-close" type="button" aria-label="关闭">×</button>
          </header>
          <form class="llm-settings-form">
            <div class="settings-grid">
              <label class="settings-field">
                <span>供应商</span>
                <select name="provider">
                  <option value="openai">OpenAI</option>
                  <option value="anthropic">Anthropic</option>
                  <option value="gemini">Google Gemini</option>
                  <option value="deepseek">DeepSeek</option>
                  <option value="groq">Groq</option>
                  <option value="openrouter">OpenRouter</option>
                  <option value="moonshot">Moonshot</option>
                  <option value="aliyun">阿里云百炼</option>
                  <option value="zai">智谱 Z.AI</option>
                  <option value="xai">xAI</option>
                  <option value="together">Together AI</option>
                  <option value="fireworks">Fireworks AI</option>
                  <option value="aihubmix">AIHubMix</option>
                  <option value="minimax">MiniMax</option>
                  <option value="ollama">Ollama（本地）</option>
                  <option value="custom-relay">OpenAI-compatible 中转站</option>
                </select>
              </label>
              <label class="settings-field">
                <span>通信协议</span>
                <select name="protocol">
                  <option value="open_ai_chat_completions">OpenAI Chat Completions</option>
                  <option value="open_ai_responses">OpenAI Responses</option>
                  <option value="anthropic_messages">Anthropic Messages</option>
                  <option value="gemini_generate_content">Gemini GenerateContent</option>
                  <option value="ollama_native">Ollama Native</option>
                </select>
              </label>
            </div>
            <label class="settings-field">
              <span>模型 ID</span>
              <input name="model" type="text" autocomplete="off" required
                placeholder="填写供应商或中转站提供的精确模型 ID" />
            </label>
            <label class="settings-field">
              <span>Base URL（可选）</span>
              <input name="endpoint" type="url" inputmode="url" autocomplete="off"
                placeholder="https://relay.example/v1/" />
              <small class="endpoint-note">官方供应商可留空。</small>
            </label>
            <label class="settings-field">
              <span>API key</span>
              <input name="api-key" type="password" autocomplete="new-password"
                placeholder="输入新密钥；留空则保留已保存的密钥" />
              <small class="credential-state">尚未保存密钥</small>
            </label>
            <label class="clear-key-field">
              <input name="clear-key" type="checkbox" />
              <span>清除当前 profile 的已保存密钥</span>
            </label>
            <p class="connection-test-note">“保存并测试”会向所选模型发送一个最多 128 output tokens 的短请求。</p>
            <div class="settings-result" aria-live="polite" hidden></div>
            <footer class="settings-actions">
              <button class="settings-secondary cancel-settings" type="button">取消</button>
              <button class="settings-secondary test-settings" type="button">保存并测试</button>
              <button class="settings-primary save-settings" type="submit">保存配置</button>
            </footer>
          </form>
        </section>
      </div>
    `;
    this.overlay = required<HTMLElement>(host, ".settings-overlay");
    this.form = required<HTMLFormElement>(host, ".llm-settings-form");
    this.provider = required<HTMLSelectElement>(host, "select[name=provider]");
    this.protocol = required<HTMLSelectElement>(host, "select[name=protocol]");
    this.model = required<HTMLInputElement>(host, "input[name=model]");
    this.endpoint = required<HTMLInputElement>(host, "input[name=endpoint]");
    this.apiKey = required<HTMLInputElement>(host, "input[name=api-key]");
    this.clearKey = required<HTMLInputElement>(host, "input[name=clear-key]");
    this.credentialState = required<HTMLElement>(host, ".credential-state");
    this.endpointNote = required<HTMLElement>(host, ".endpoint-note");
    this.result = required<HTMLElement>(host, ".settings-result");
    this.saveButton = required<HTMLButtonElement>(host, ".save-settings");
    this.testButton = required<HTMLButtonElement>(host, ".test-settings");

    this.trigger.onclick = () => void this.open();
    required<HTMLButtonElement>(host, ".settings-close").onclick = () => this.close();
    required<HTMLButtonElement>(host, ".cancel-settings").onclick = () => this.close();
    this.overlay.addEventListener("click", (event) => {
      if (event.target === this.overlay) this.close();
    });
    this.provider.onchange = () => this.applyProviderPreset();
    this.clearKey.onchange = () => {
      this.apiKey.disabled = this.clearKey.checked;
      if (this.clearKey.checked) this.apiKey.value = "";
    };
    this.form.onsubmit = (event) => {
      event.preventDefault();
      void this.save(false);
    };
    this.testButton.onclick = () => void this.save(true);
  }

  async loadSummary(): Promise<void> {
    try {
      const snapshot = await invoke<LlmSettingsSnapshot>("get_llm_settings");
      this.applySnapshot(snapshot);
      this.trigger.classList.toggle("is-configured", this.current !== null);
      this.triggerLabel.textContent = this.current === null
        ? "LLM 未配置"
        : `${providerLabel(this.current.provider)} · ${this.current.model}`;
    } catch {
      this.triggerLabel.textContent = "LLM 配置不可用";
      this.trigger.classList.remove("is-configured");
    }
  }

  private async open(): Promise<void> {
    await this.loadSummary();
    this.populateForm();
    this.result.hidden = true;
    this.overlay.hidden = false;
    this.model.focus();
  }

  private close(): void {
    this.overlay.hidden = true;
    this.apiKey.value = "";
  }

  private applySnapshot(snapshot: LlmSettingsSnapshot): void {
    this.current = snapshot.profiles.find(
      (profile) => profile.id === snapshot.default_profile_id,
    ) ?? snapshot.profiles[0] ?? null;
  }

  private populateForm(): void {
    const profile = this.current;
    this.provider.value = profile?.provider ?? "openai";
    if (!(this.provider.value in presets)) this.provider.value = "custom-relay";
    this.protocol.value = profile?.wire_protocol ?? presets[this.provider.value].protocol;
    this.model.value = profile?.model ?? "";
    this.endpoint.value = profile?.endpoint ?? "";
    this.apiKey.value = "";
    this.clearKey.checked = false;
    this.apiKey.disabled = false;
    this.credentialState.textContent = profile?.has_credential
      ? "系统钥匙串中已有密钥引用；留空保持不变。"
      : "尚未保存密钥；Ollama 等无鉴权服务可留空。";
    this.applyProviderPreset(false);
  }

  private applyProviderPreset(changeProtocol = true): void {
    const preset = presets[this.provider.value] ?? presets["custom-relay"];
    if (changeProtocol) this.protocol.value = preset.protocol;
    this.endpoint.required = preset.needsEndpoint;
    this.endpointNote.textContent = preset.needsEndpoint
      ? "中转站必须填写完整 API 前缀；结尾的 / 会自动补齐。"
      : "使用官方地址时留空；也可填写兼容服务的完整 API 前缀。";
  }

  private async save(testAfterSave: boolean): Promise<void> {
    if (!this.form.reportValidity()) return;
    this.setBusy(true, testAfterSave ? "正在保存并连接模型…" : "正在保存配置…");
    try {
      const snapshot = await invoke<LlmSettingsSnapshot>("save_llm_profile", {
        request: {
          profile_id: this.current?.id ?? "desktop-default",
          provider: this.provider.value,
          wire_protocol: this.protocol.value,
          model: this.model.value,
          endpoint: this.endpoint.value.trim() || null,
          api_key: this.apiKey.value || null,
          clear_api_key: this.clearKey.checked,
          make_default: true,
        },
      });
      this.applySnapshot(snapshot);
      this.apiKey.value = "";
      this.clearKey.checked = false;
      this.apiKey.disabled = false;
      await this.loadSummary();
      if (testAfterSave) {
        const tested = await invoke<LlmConnectionTest>("test_llm_profile", {
          profileId: this.current?.id ?? null,
        });
        this.showResult(
          `连接成功 · ${tested.provider_model} · ${formatDuration(tested.elapsed_ms)}\n${tested.response_preview}`,
          true,
        );
        this.toast("LLM 配置已保存，连接测试成功。", "success");
      } else {
        this.showResult("配置已保存。API key 未写入 Workspace 数据库。", true);
        this.toast("LLM 配置已保存。", "success");
      }
      this.populateCredentialState();
    } catch (error: unknown) {
      const message = errorMessage(error);
      this.showResult(message, false);
      this.toast(message, "error");
    } finally {
      this.setBusy(false);
    }
  }

  private populateCredentialState(): void {
    this.credentialState.textContent = this.current?.has_credential
      ? "系统钥匙串中已有密钥引用；留空保持不变。"
      : "尚未保存密钥；Ollama 等无鉴权服务可留空。";
  }

  private showResult(message: string, success: boolean): void {
    this.result.hidden = false;
    this.result.dataset.kind = success ? "success" : "error";
    this.result.textContent = message;
  }

  private setBusy(busy: boolean, message = ""): void {
    this.saveButton.disabled = busy;
    this.testButton.disabled = busy;
    this.saveButton.textContent = busy ? "请稍候…" : "保存配置";
    if (busy) this.showResult(message, true);
  }
}

function required<T extends Element>(host: ParentNode, selector: string): T {
  const element = host.querySelector<T>(selector);
  if (element === null) throw new Error(`required LLM settings element is missing: ${selector}`);
  return element;
}

function providerLabel(provider: string): string {
  return {
    openai: "OpenAI",
    anthropic: "Anthropic",
    gemini: "Gemini",
    deepseek: "DeepSeek",
    groq: "Groq",
    openrouter: "OpenRouter",
    moonshot: "Moonshot",
    aliyun: "阿里云",
    zai: "Z.AI",
    xai: "xAI",
    together: "Together",
    fireworks: "Fireworks",
    aihubmix: "AIHubMix",
    minimax: "MiniMax",
    ollama: "Ollama",
    "custom-relay": "中转站",
  }[provider] ?? provider;
}

function errorMessage(error: unknown): string {
  return typeof error === "string" ? error : "LLM 配置操作失败";
}

function formatDuration(milliseconds: number): string {
  return milliseconds < 1_000
    ? `${milliseconds} ms`
    : `${(milliseconds / 1_000).toFixed(1)} s`;
}
