import { THEMES, THEME_CHANGED, getTheme, setTheme, isTheme, themeLabel } from "../shell/theme-manager";
import { MAP_STYLES, MAP_STYLE_CHANGED, getMapStyle, setMapStyle, isMapStyle } from "../shell/map-style";
import { invoke } from "@tauri-apps/api/core";
import { eventBus } from "../core/events";
import { motionPreference, setMotionPreference, type MotionPreference } from "../core/motion";

import "./settings.css";
import {
  announceWorkspaceSettings,
  getLoogleResidentStatus,
  getWorkspaceSettings,
  saveWorkspaceSettings,
  type DiscoveryAdmissionPreference,
  type WorkspaceSettings,
} from "../workspace-settings";

const LOOGLE_RESIDENT_NOTE =
  "打开后，mathlib.search 第一次查询会启动一个常驻 Loogle（约 11–36 秒，常驻约 5.5GB 内存），之后每次查询通常不到 1 秒；空闲 15 分钟自动关闭。关闭后每次查询单独启动 Loogle（约 11 秒）。";

type WireProtocol =
  | "open_ai_chat_completions"
  | "open_ai_responses"
  | "anthropic_messages"
  | "gemini_generate_content"
  | "ollama_native";

interface LlmProfileView {
  display_name?: string | null;
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
  private readonly workspaceForm: HTMLFormElement;
  private readonly admission: HTMLSelectElement;
  private readonly discoveryTimeout: HTMLInputElement;
  private readonly connectTimeout: HTMLInputElement;
  private readonly loogleResident: HTMLInputElement;
  private readonly motion: HTMLSelectElement;
  private readonly loogleResidentNote: HTMLElement;
  private readonly workspaceResult: HTMLElement;
  private readonly workspaceSaveButton: HTMLButtonElement;
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
  private profiles: LlmProfileView[] = [];
  private defaultId: string | null = null;
  private draftId = "desktop-default";
  private profileSelect!: HTMLSelectElement;
  private profileName!: HTMLInputElement;

  constructor(
    host: HTMLElement,
    private readonly trigger: HTMLButtonElement,
    private readonly triggerLabel: HTMLElement,
    private readonly toast: Toast,
  ) {
    host.innerHTML = `
      <div class="settings-overlay" hidden>
        <section class="settings-dialog" role="dialog" aria-modal="true" aria-labelledby="mathscape-settings-title">
          <header class="settings-header">
            <div>
              <span class="settings-eyebrow">WORKSPACE CONTROL</span>
              <h2 id="mathscape-settings-title">Mathscape 设置</h2>
              <p>运行策略随当前 Workspace 保存；API key 仅存入系统钥匙串。</p>
            </div>
            <button class="settings-close" type="button" aria-label="关闭">×</button>
          </header>
          <section class="motion-settings settings-section">
            <div class="settings-section-heading">
              <div>
                <span class="settings-eyebrow">DISPLAY</span>
                <h3>界面</h3>
              </div>
              <span class="settings-scope">本机</span>
            </div>
            <label class="settings-field">显示主题
              <select name="theme" aria-label="显示主题">${(Object.keys(THEMES) as (keyof typeof THEMES)[]).map(value => `<option value="${value}">${themeLabel(value)}</option>`).join("")}</select>
              <small>立即生效，自动记住本机的选择。</small>
            </label>
            <label class="settings-field">地图风格
              <select name="map-style" aria-label="地图风格">${Object.entries(MAP_STYLES).map(([value, label]) => `<option value="${value}">${label}${value === "constellation" ? "（默认）" : ""}</option>`).join("")}</select>
              <small>星座：轻量星核与疏排标签；原版：较大节点、标签常显。独立于显示主题，立即生效并自动记住。</small>
            </label>
            <label class="settings-field">
              <span>减少动效</span>
              <select name="reduce-motion">
                <option value="system">跟随系统（默认）</option>
                <option value="on">开启：状态直接切换</option>
                <option value="off">关闭：播放完整动画</option>
              </select>
              <small>影响沙箱会话在地图上的工具场景：开启后不做收拢、镜头平移、打字机与逐条出现，但仍显示每一步的状态。立即生效。</small>
            </label>
          </section>
          <form class="workspace-settings-form settings-section">
            <div class="settings-section-heading">
              <div>
                <span class="settings-eyebrow">MAP CONSTRUCTION</span>
                <h3>内容发现运行策略</h3>
              </div>
              <span class="settings-scope">Workspace</span>
            </div>
            <label class="settings-field">
              <span>默认审核方式</span>
              <select name="discovery-admission">
                <option value="review_before_publish">始终先人工审核（推荐）</option>
                <option value="automatic_candidate">后续登记：验证后自动 candidate</option>
                <option value="ask_each_run">每次运行时选择</option>
              </select>
              <small>本阶段记录审核偏好但仍只展示提案；未来登记实现后，自动模式仍必须通过确定性验证且不会直接产生 canonical 实体。</small>
            </label>
            <label class="settings-field">
              <span>内容发现请求总超时（秒）</span>
              <input name="discovery-timeout" type="number" min="30" max="1800" step="1" required />
              <small>默认 600 秒。超时不会自动重试，以免产生不可见的重复费用。</small>
            </label>
            <details class="settings-advanced">
              <summary>高级连接设置</summary>
              <label class="settings-field">
                <span>连接超时（秒）</span>
                <input name="connect-timeout" type="number" min="5" max="60" step="1" required />
                <small>只限制建立网络连接的时间，不限制模型推理时长。</small>
              </label>
            </details>
            <div class="settings-field loogle-resident-setting">
              <label class="clear-key-field">
                <input name="loogle-resident" type="checkbox" />
                <span>Loogle 常驻模式</span>
              </label>
              <small class="loogle-resident-note">${LOOGLE_RESIDENT_NOTE}</small>
            </div>
            <div class="workspace-settings-result settings-result" aria-live="polite" hidden></div>
            <footer class="settings-actions compact-actions">
              <button class="settings-primary save-workspace-settings" type="submit">保存运行策略</button>
            </footer>
          </form>
          <form class="llm-settings-form">
            <div class="settings-section-heading">
              <div>
                <span class="settings-eyebrow">MODEL RUNTIME</span>
                <h3>模型连接</h3>
              </div>
              <span class="settings-scope">Workspace + Keychain</span>
            </div>
            <label class="settings-field"><span>已有配置（选择以编辑）</span><select name="saved-profile"></select></label>
            <div class="settings-actions"><button type="button" class="new-profile">新增配置</button><button type="button" class="default-profile">将所选配置设为默认</button></div>
            <label class="settings-field"><span>配置名称</span><input name="profile-name" maxlength="120" required placeholder="例如：APINOMA · 主力模型" /></label>
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
            <div class="llm-settings-result settings-result" aria-live="polite" hidden></div>
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
    const theme = required<HTMLSelectElement>(host, "select[name=theme]");
    const syncTheme = () => { theme.value = getTheme(); };
    syncTheme();
    theme.onchange = () => { if (isTheme(theme.value)) setTheme(theme.value); };
    window.addEventListener(THEME_CHANGED, syncTheme);
    const mapStyle = required<HTMLSelectElement>(host, "select[name=map-style]");
    const syncMapStyle = () => { mapStyle.value = getMapStyle(); };
    syncMapStyle();
    mapStyle.onchange = () => { if (isMapStyle(mapStyle.value)) setMapStyle(mapStyle.value); };
    window.addEventListener(MAP_STYLE_CHANGED, syncMapStyle);
    this.form = required<HTMLFormElement>(host, ".llm-settings-form");
    this.workspaceForm = required<HTMLFormElement>(host, ".workspace-settings-form");
    this.admission = required<HTMLSelectElement>(host, "select[name=discovery-admission]");
    this.discoveryTimeout = required<HTMLInputElement>(host, "input[name=discovery-timeout]");
    this.connectTimeout = required<HTMLInputElement>(host, "input[name=connect-timeout]");
    this.loogleResident = required<HTMLInputElement>(host, "input[name=loogle-resident]");
    this.loogleResidentNote = required<HTMLElement>(host, ".loogle-resident-note");
    this.workspaceResult = required<HTMLElement>(host, ".workspace-settings-result");
    this.workspaceSaveButton = required<HTMLButtonElement>(host, ".save-workspace-settings");
    this.provider = required<HTMLSelectElement>(host, "select[name=provider]");
    this.protocol = required<HTMLSelectElement>(host, "select[name=protocol]");
    this.model = required<HTMLInputElement>(host, "input[name=model]");
    this.endpoint = required<HTMLInputElement>(host, "input[name=endpoint]");
    this.apiKey = required<HTMLInputElement>(host, "input[name=api-key]");
    this.clearKey = required<HTMLInputElement>(host, "input[name=clear-key]");
    this.credentialState = required<HTMLElement>(host, ".credential-state");
    this.endpointNote = required<HTMLElement>(host, ".endpoint-note");
    this.result = required<HTMLElement>(host, ".llm-settings-result");
    this.saveButton = required<HTMLButtonElement>(host, ".save-settings");
    this.testButton = required<HTMLButtonElement>(host, ".test-settings");

    const motion = required<HTMLSelectElement>(host, "select[name=reduce-motion]");
    motion.value = motionPreference();
    motion.onchange = () => setMotionPreference(motion.value as MotionPreference);
    this.motion = motion;
    this.trigger.onclick = () => void this.open();
    eventBus.on("open-settings", () => void this.open());
    this.profileSelect = required<HTMLSelectElement>(host, "select[name=saved-profile]");
    this.profileName = required<HTMLInputElement>(host, "input[name=profile-name]");
    this.profileSelect.onchange = () => {
      this.current = this.profiles.find(p => p.id === this.profileSelect.value) ?? null;
      this.draftId = this.current?.id ?? `profile-${crypto.randomUUID()}`;
      this.populateForm();
    };
    required<HTMLButtonElement>(host, ".new-profile").onclick = () => {
      this.current = null;
      this.draftId = `profile-${crypto.randomUUID()}`;
      this.profileSelect.value = "";
      this.populateForm();
      this.profileName.focus();
    };
    required<HTMLButtonElement>(host, ".default-profile").onclick = async () => {
      if (!this.current) { this.showResult("请先保存新配置。", false); return; }
      this.setBusy(true, "正在切换默认配置…");
      try {
        this.applySnapshot(await invoke<LlmSettingsSnapshot>("set_default_llm_profile", { profileId: this.current.id }));
        this.showResult("默认模型已切换，仅影响后续新任务。", true);
      } catch (error) { this.showResult(errorMessage(error), false); }
      finally { this.setBusy(false); }
    };
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
    this.workspaceForm.onsubmit = (event) => {
      event.preventDefault();
      void this.saveWorkspacePolicy();
    };
    this.testButton.onclick = () => void this.save(true);
  }

  async loadSummary(): Promise<void> {
    try {
      const snapshot = await invoke<LlmSettingsSnapshot>("get_llm_settings");
      this.applySnapshot(snapshot);
      this.trigger.classList.toggle("is-configured", this.current !== null);
      this.triggerLabel.textContent = "设置";
    } catch {
      this.triggerLabel.textContent = "设置";
      this.trigger.classList.remove("is-configured");
    }
  }

  private async open(): Promise<void> {
    await this.loadSummary();
    this.populateForm();
    this.result.hidden = true;
    this.workspaceResult.hidden = true;
    this.motion.value = motionPreference();
    this.overlay.hidden = false;
    this.admission.focus();
    try {
      this.populateWorkspaceForm(await getWorkspaceSettings());
    } catch (error: unknown) {
      this.showWorkspaceResult(errorMessage(error), false);
    }
    await this.loadLoogleResidentStatus();
  }

  private async loadLoogleResidentStatus(): Promise<void> {
    let status = null;
    try {
      status = await getLoogleResidentStatus();
    } catch {
      status = null;
    }
    const forcedOff = status?.forced_off === true;
    const unavailable = status?.available === false;
    this.loogleResident.disabled = forcedOff || unavailable;
    this.loogleResidentNote.textContent = forcedOff
      ? "已被环境变量 MATHSCAPE_LOOGLE_RESIDENT 关闭，mathlib.search 每次单独启动 Loogle。"
      : unavailable
        ? "当前平台不支持常驻 Loogle，mathlib.search 每次单独启动 Loogle。"
        : LOOGLE_RESIDENT_NOTE;
  }

  private close(): void {
    this.overlay.hidden = true;
    this.apiKey.value = "";
  }

  private applySnapshot(snapshot: LlmSettingsSnapshot): void {
    this.profiles = snapshot.profiles;
    this.defaultId = snapshot.default_profile_id;
    this.current = snapshot.profiles.find(p => p.id === this.draftId) ?? snapshot.profiles.find(
      (profile) => profile.id === snapshot.default_profile_id,
    ) ?? snapshot.profiles[0] ?? null;
    this.draftId = this.current?.id ?? this.draftId;
    this.profileSelect.replaceChildren(new Option("新增配置…", ""), ...this.profiles.map(p => new Option(`${p.display_name ?? p.model}${p.id === this.defaultId ? " · 默认" : ""}`, p.id)));
    this.profileSelect.value = this.current?.id ?? "";
  }

  private populateForm(): void {
    const profile = this.current;
    this.profileName.value = profile?.display_name ?? profile?.model ?? "";
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

  private populateWorkspaceForm(settings: WorkspaceSettings): void {
    this.admission.value = settings.discovery_admission;
    this.discoveryTimeout.value = String(settings.discovery_timeout_seconds);
    this.connectTimeout.value = String(settings.connect_timeout_seconds);
    this.loogleResident.checked = settings.loogle_resident !== false;
  }

  private async saveWorkspacePolicy(): Promise<void> {
    if (!this.workspaceForm.reportValidity()) return;
    this.workspaceSaveButton.disabled = true;
    this.workspaceSaveButton.textContent = "正在保存…";
    this.showWorkspaceResult("正在保存 Workspace 运行策略…", true);
    try {
      const settings = await saveWorkspaceSettings({
        discovery_admission: this.admission.value as DiscoveryAdmissionPreference,
        discovery_timeout_seconds: this.discoveryTimeout.valueAsNumber,
        connect_timeout_seconds: this.connectTimeout.valueAsNumber,
        loogle_resident: this.loogleResident.checked,
      });
      this.populateWorkspaceForm(settings);
      announceWorkspaceSettings(settings);
      this.showWorkspaceResult("运行策略已保存，后续内容发现任务将使用新超时。", true);
      this.toast("Workspace 运行策略已保存。", "success");
    } catch (error: unknown) {
      const message = errorMessage(error);
      this.showWorkspaceResult(message, false);
      this.toast(message, "error");
    } finally {
      this.workspaceSaveButton.disabled = false;
      this.workspaceSaveButton.textContent = "保存运行策略";
    }
  }

  private showWorkspaceResult(message: string, success: boolean): void {
    this.workspaceResult.hidden = false;
    this.workspaceResult.dataset.kind = success ? "success" : "error";
    this.workspaceResult.textContent = message;
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
          profile_id: this.draftId,
          display_name: this.profileName.value.trim(),
          provider: this.provider.value,
          wire_protocol: this.protocol.value,
          model: this.model.value,
          endpoint: this.endpoint.value.trim() || null,
          api_key: this.apiKey.value || null,
          clear_api_key: this.clearKey.checked,
          make_default: false,
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
    this.form.querySelectorAll<HTMLInputElement | HTMLSelectElement | HTMLButtonElement>("input, select, button").forEach(element => element.disabled = busy);
    if (!busy) this.apiKey.disabled = this.clearKey.checked;
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
