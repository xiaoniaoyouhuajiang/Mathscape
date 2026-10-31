/**
 * S1 dialogue shell (docs/sandbox-flow.md §4.2, §5 S1): a plain chat over one
 * sandbox session, backed by the local Codex with the 7 Mathscape MCP tools,
 * plus an event strip that shows the live `agent_events v0` stream.
 *
 * The expanded window is resizable and remembers its last size; when collapsed
 * it becomes a draggable float handle (not a fixed corner dock). The status
 * line shows Codex availability; when the adapter supports selection a
 * model <select> lets the user pick before the next round.
 *
 * Everything the agent reads or writes is the session's isolated copy; the
 * shell never touches the checkpoint. The reply is only shown once the round
 * has an event stream — no fake "connected" state.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { errorMessage } from "../core/ipc";
import {
  agentStatusLine,
  eventChip,
  latestRoundEvents,
  modelCatalogHint,
  modelOptionLabel,
  ROLE_LABEL,
  type AgentEventLike,
  type AgentStatusLike,
  type TranscriptRole,
} from "./format";

export const DIALOGUE_EVENT = "sandbox://dialogue";
export const DIALOGUE_SIZE_KEY = "mathscape:sandbox-dialogue-size";
export const DIALOGUE_POS_KEY = "mathscape:sandbox-dialogue-pos";

const DEFAULT_WIDTH = 460;
const DEFAULT_HEIGHT = 520;
const MIN_WIDTH = 320;
const MIN_HEIGHT = 280;
const HANDLE_SIZE = 48;

interface TranscriptEntry {
  ts: string;
  role: TranscriptRole;
  text: string;
  round_id?: string;
  outcome?: string;
}

interface SessionDialogue {
  sandboxSessionId: string;
  transcript: TranscriptEntry[];
  events: AgentEventLike[];
  activeRoundId: string | null;
}

interface SendOutcome {
  roundId: string;
  userEntry: TranscriptEntry;
}

type DialogueUpdate =
  | { kind: "agent_event"; sandbox_session_id: string; event: AgentEventLike }
  | { kind: "transcript"; sandbox_session_id: string; entry: TranscriptEntry }
  | { kind: "round_finished"; sandbox_session_id: string; round_id: string };

type Toast = (message: string, kind: "success" | "error") => void;

interface SizePref {
  width: number;
  height: number;
}

interface PosPref {
  left: number;
  top: number;
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className = "", text = ""): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text) node.textContent = text;
  return node;
}

function loadSize(): SizePref {
  try {
    const raw = localStorage.getItem(DIALOGUE_SIZE_KEY);
    if (!raw) return { width: DEFAULT_WIDTH, height: DEFAULT_HEIGHT };
    const parsed = JSON.parse(raw) as Partial<SizePref>;
    return {
      width: clamp(Number(parsed.width) || DEFAULT_WIDTH, MIN_WIDTH, maxWidth()),
      height: clamp(Number(parsed.height) || DEFAULT_HEIGHT, MIN_HEIGHT, maxHeight()),
    };
  } catch {
    return { width: DEFAULT_WIDTH, height: DEFAULT_HEIGHT };
  }
}

function saveSize(size: SizePref): void {
  try {
    localStorage.setItem(DIALOGUE_SIZE_KEY, JSON.stringify(size));
  } catch {
    /* Optional preference. */
  }
}

function loadPos(): PosPref | null {
  try {
    const raw = localStorage.getItem(DIALOGUE_POS_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<PosPref>;
    const left = Number(parsed.left);
    const top = Number(parsed.top);
    if (!Number.isFinite(left) || !Number.isFinite(top)) return null;
    return { left, top };
  } catch {
    return null;
  }
}

function savePos(pos: PosPref): void {
  try {
    localStorage.setItem(DIALOGUE_POS_KEY, JSON.stringify(pos));
  } catch {
    /* Optional preference. */
  }
}

function clamp(value: number, min: number, max: number): number {
  return Math.round(Math.min(max, Math.max(min, value)));
}

function maxWidth(): number {
  return Math.max(MIN_WIDTH, window.innerWidth - 24);
}

function maxHeight(): number {
  return Math.max(MIN_HEIGHT, window.innerHeight - 24);
}

function defaultPos(size: SizePref): PosPref {
  return {
    left: Math.max(12, window.innerWidth - size.width - 20),
    top: Math.max(12, window.innerHeight - size.height - 20),
  };
}

export class SandboxDialogueDock {
  private readonly root: HTMLElement;
  private readonly title: HTMLElement;
  private readonly status: HTMLElement;
  private readonly modelRow: HTMLElement;
  private readonly modelSelect: HTMLSelectElement;
  private readonly modelHint: HTMLElement;
  private readonly log: HTMLElement;
  private readonly strip: HTMLElement;
  private readonly input: HTMLTextAreaElement;
  private readonly sendButton: HTMLButtonElement;
  private readonly cancelButton: HTMLButtonElement;
  private readonly collapseButton: HTMLButtonElement;
  private readonly floatHandle: HTMLButtonElement;
  private readonly resizeHandle: HTMLElement;

  private sessionId: string | null = null;
  private transcript: TranscriptEntry[] = [];
  private events: AgentEventLike[] = [];
  private activeRoundId: string | null = null;
  private sending = false;
  private unlisten: Promise<UnlistenFn> | null = null;
  private size = loadSize();
  private pos: PosPref = loadPos() ?? defaultPos(this.size);

  constructor(
    host: HTMLElement,
    private readonly toast: Toast,
  ) {
    this.root = el("aside", "sandbox-dock");
    this.root.hidden = true;
    this.root.setAttribute("aria-label", "沙箱对话");

    const header = el("header", "sandbox-dock-header");
    const heading = el("div", "sandbox-dock-heading");
    this.title = el("strong", "", "沙箱对话");
    this.status = el("span", "sandbox-dock-status", "正在检测本机 Codex…");
    this.modelRow = el("label", "sandbox-dock-model");
    this.modelRow.hidden = true;
    this.modelRow.append(el("span", "sandbox-dock-model-label", "模型"));
    this.modelSelect = el("select", "sandbox-dock-model-select");
    this.modelSelect.setAttribute("aria-label", "选择 Codex 模型");
    this.modelSelect.disabled = true;
    this.modelSelect.onchange = () => void this.onModelChange();
    this.modelRow.append(this.modelSelect);
    this.modelHint = el("span", "sandbox-dock-model-hint");
    this.modelHint.hidden = true;
    heading.append(
      el("span", "settings-eyebrow", "SANDBOX · S1"),
      this.title,
      this.status,
      this.modelRow,
      this.modelHint,
    );
    this.collapseButton = el("button", "sandbox-dock-icon", "–");
    this.collapseButton.type = "button";
    this.collapseButton.title = "收起为浮动入口";
    this.collapseButton.onclick = () => this.collapseToFloat();
    const close = el("button", "sandbox-dock-icon", "×");
    close.type = "button";
    close.title = "关闭对话（不会取消正在进行的一轮）";
    close.setAttribute("aria-label", "关闭对话");
    close.onclick = () => this.close();
    header.append(heading, this.collapseButton, close);
    this.bindDrag(header, () => ({ width: this.size.width, height: this.size.height }));

    const body = el("div", "sandbox-dock-body");
    this.log = el("ol", "sandbox-dock-log");
    this.log.setAttribute("aria-live", "polite");
    this.strip = el("div", "sandbox-dock-strip");
    this.strip.setAttribute("aria-label", "事件条");

    const composer = el("form", "sandbox-dock-composer");
    this.input = el("textarea", "sandbox-input sandbox-dock-input");
    this.input.rows = 3;
    this.input.maxLength = 20_000;
    this.input.placeholder = "向沙箱里的 Agent 提问（Enter 发送，Shift+Enter 换行）";
    this.input.addEventListener("keydown", (event) => {
      if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
        event.preventDefault();
        void this.send();
      }
    });
    const actions = el("div", "sandbox-actions");
    this.cancelButton = el("button", "sandbox-btn sandbox-btn-quiet", "取消本轮");
    this.cancelButton.type = "button";
    this.cancelButton.onclick = () => void this.cancel();
    this.sendButton = el("button", "sandbox-btn sandbox-btn-primary", "发送");
    this.sendButton.type = "submit";
    actions.append(el("span", "sandbox-actions-spacer"), this.cancelButton, this.sendButton);
    composer.append(this.input, actions);
    composer.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.send();
    });

    body.append(this.log, this.strip, composer);

    this.resizeHandle = el("div", "sandbox-dock-resize");
    this.resizeHandle.setAttribute("role", "separator");
    this.resizeHandle.setAttribute("aria-orientation", "both");
    this.resizeHandle.setAttribute("aria-label", "调整对话窗口大小");
    this.resizeHandle.title = "拖动调整大小";
    this.bindResize(this.resizeHandle);

    this.root.append(header, body, this.resizeHandle);

    this.floatHandle = el("button", "sandbox-dock-float", "对话");
    this.floatHandle.type = "button";
    this.floatHandle.hidden = true;
    this.floatHandle.title = "展开沙箱对话（可拖动）";
    this.floatHandle.setAttribute("aria-label", "展开沙箱对话");
    this.floatHandle.addEventListener("click", (event) => {
      if (this.floatHandle.dataset.dragged === "1") {
        event.preventDefault();
        this.floatHandle.dataset.dragged = "0";
        return;
      }
      this.expandFromFloat();
    });
    this.bindDrag(this.floatHandle, () => ({ width: HANDLE_SIZE, height: HANDLE_SIZE }), true);

    host.append(this.root, this.floatHandle);
    this.applyGeometry();
    this.syncControls();
    window.addEventListener("resize", () => this.clampIntoViewport());
  }

  get currentSessionId(): string | null {
    if (this.root.hidden && this.floatHandle.hidden) return null;
    return this.sessionId;
  }

  async open(sessionId: string): Promise<void> {
    this.ensureListening();
    this.floatHandle.hidden = true;
    this.root.hidden = false;
    this.root.classList.remove("is-collapsed");
    if (this.sessionId !== sessionId) {
      this.sessionId = sessionId;
      this.transcript = [];
      this.events = [];
      this.activeRoundId = null;
    }
    this.title.textContent = `沙箱对话 · ${sessionId}`;
    this.floatHandle.textContent = "对话";
    this.applyGeometry();
    void this.refreshStatus();
    try {
      const dialogue = await invoke<SessionDialogue>("sandbox_session_dialogue", { sandboxSessionId: sessionId });
      if (this.sessionId !== sessionId) return;
      this.transcript = dialogue.transcript;
      this.events = dialogue.events;
      this.activeRoundId = dialogue.activeRoundId;
    } catch (error) {
      this.toast(errorMessage(error), "error");
    }
    this.renderAll();
    this.input.focus();
  }

  close(): void {
    this.root.hidden = true;
    this.floatHandle.hidden = true;
  }

  /** Close if showing `sessionId` (e.g. after it was discarded). */
  forget(sessionId: string): void {
    if (this.sessionId === sessionId) {
      this.close();
      this.sessionId = null;
    }
  }

  private collapseToFloat(): void {
    if (!this.sessionId) {
      this.close();
      return;
    }
    this.root.hidden = true;
    this.floatHandle.hidden = false;
    this.floatHandle.textContent = "对话";
    this.applyGeometry();
  }

  private expandFromFloat(): void {
    if (!this.sessionId) {
      this.close();
      return;
    }
    void this.open(this.sessionId);
  }

  private applyGeometry(): void {
    this.size = {
      width: clamp(this.size.width, MIN_WIDTH, maxWidth()),
      height: clamp(this.size.height, MIN_HEIGHT, maxHeight()),
    };
    this.pos = this.clampPos(this.pos, this.root.hidden && !this.floatHandle.hidden
      ? { width: HANDLE_SIZE, height: HANDLE_SIZE }
      : this.size);
    this.root.style.width = `${this.size.width}px`;
    this.root.style.height = `${this.size.height}px`;
    this.root.style.left = `${this.pos.left}px`;
    this.root.style.top = `${this.pos.top}px`;
    this.root.style.right = "auto";
    this.root.style.bottom = "auto";
    this.floatHandle.style.left = `${this.pos.left}px`;
    this.floatHandle.style.top = `${this.pos.top}px`;
    this.floatHandle.style.right = "auto";
    this.floatHandle.style.bottom = "auto";
  }

  private clampIntoViewport(): void {
    this.applyGeometry();
  }

  private clampPos(pos: PosPref, box: { width: number; height: number }): PosPref {
    return {
      left: clamp(pos.left, 8, Math.max(8, window.innerWidth - box.width - 8)),
      top: clamp(pos.top, 8, Math.max(8, window.innerHeight - box.height - 8)),
    };
  }

  private bindDrag(
    target: HTMLElement,
    box: () => { width: number; height: number },
    markClickSuppress = false,
  ): void {
    let dragging: number | undefined;
    let startX = 0;
    let startY = 0;
    let origin: PosPref = { left: 0, top: 0 };
    let moved = false;
    target.addEventListener("pointerdown", (event) => {
      if (event.button !== 0) return;
      // Interactive controls inside the drag handle (the model <select> lives in
      // the header) must keep their native pointer behaviour: preventDefault +
      // pointer capture here suppresses the compat mousedown that opens the
      // native picker, so clicking the model bar showed no options.
      if (event.target !== target && isInteractive(event.target)) return;
      event.preventDefault();
      dragging = event.pointerId;
      moved = false;
      startX = event.clientX;
      startY = event.clientY;
      origin = { ...this.pos };
      try {
        target.setPointerCapture(event.pointerId);
      } catch {
        /* Synthetic pointers. */
      }
      target.classList.add("is-dragging");
    });
    target.addEventListener("pointermove", (event) => {
      if (event.pointerId !== dragging) return;
      const dx = event.clientX - startX;
      const dy = event.clientY - startY;
      if (Math.abs(dx) + Math.abs(dy) > 3) moved = true;
      this.pos = this.clampPos({ left: origin.left + dx, top: origin.top + dy }, box());
      this.applyGeometry();
    });
    const finish = (event: PointerEvent) => {
      if (event.pointerId !== dragging) return;
      dragging = undefined;
      target.classList.remove("is-dragging");
      savePos(this.pos);
      if (markClickSuppress && moved) target.dataset.dragged = "1";
    };
    target.addEventListener("pointerup", finish);
    target.addEventListener("pointercancel", finish);
  }

  private bindResize(handle: HTMLElement): void {
    let dragging: number | undefined;
    let startX = 0;
    let startY = 0;
    let origin: SizePref = { width: 0, height: 0 };
    handle.addEventListener("pointerdown", (event) => {
      if (event.button !== 0) return;
      event.preventDefault();
      event.stopPropagation();
      dragging = event.pointerId;
      startX = event.clientX;
      startY = event.clientY;
      origin = { ...this.size };
      try {
        handle.setPointerCapture(event.pointerId);
      } catch {
        /* Synthetic pointers. */
      }
      handle.classList.add("is-dragging");
    });
    handle.addEventListener("pointermove", (event) => {
      if (event.pointerId !== dragging) return;
      this.size = {
        width: clamp(origin.width + (event.clientX - startX), MIN_WIDTH, maxWidth()),
        height: clamp(origin.height + (event.clientY - startY), MIN_HEIGHT, maxHeight()),
      };
      this.applyGeometry();
    });
    const finish = (event: PointerEvent) => {
      if (event.pointerId !== dragging) return;
      dragging = undefined;
      handle.classList.remove("is-dragging");
      saveSize(this.size);
      savePos(this.pos);
    };
    handle.addEventListener("pointerup", finish);
    handle.addEventListener("pointercancel", finish);
  }

  private ensureListening(): void {
    if (this.unlisten) return;
    this.unlisten = listen<DialogueUpdate>(DIALOGUE_EVENT, (message) => this.apply(message.payload));
    this.unlisten.catch((error: unknown) => {
      this.unlisten = null;
      this.toast(`无法订阅对话事件流：${errorMessage(error)}`, "error");
    });
  }

  private async refreshStatus(): Promise<void> {
    try {
      const status = await invoke<AgentStatusLike>("sandbox_agent_status");
      this.applyAgentStatus(status);
    } catch (error) {
      this.status.textContent = errorMessage(error);
      this.status.classList.add("is-unavailable");
      this.modelRow.hidden = true;
      this.modelSelect.disabled = true;
    }
  }

  private applyAgentStatus(status: AgentStatusLike): void {
    this.status.textContent = agentStatusLine(status);
    this.status.classList.toggle("is-unavailable", !status.available);
    const canSelect = Boolean(status.modelSelection) && status.available;
    if (!canSelect) {
      this.modelRow.hidden = true;
      this.modelHint.hidden = true;
      this.modelSelect.disabled = true;
      return;
    }
    const models = [...(status.models ?? [])];
    const current = status.model?.trim() || "";
    if (current && !models.some((model) => model.id === current)) {
      models.unshift({ id: current });
    }
    this.modelSelect.replaceChildren();
    const hint = modelCatalogHint(models, current);
    this.modelHint.textContent = hint ?? "";
    this.modelHint.hidden = hint === null;
    if (models.length === 0) {
      const empty = document.createElement("option");
      empty.value = "";
      empty.textContent = "（未设置）";
      this.modelSelect.append(empty);
      this.modelSelect.disabled = true;
      this.modelRow.hidden = false;
      return;
    }
    for (const model of models) {
      const option = document.createElement("option");
      option.value = model.id;
      option.textContent = modelOptionLabel(model);
      this.modelSelect.append(option);
    }
    this.modelSelect.value = current && models.some((m) => m.id === current) ? current : models[0].id;
    this.modelSelect.title = `${models.length} 个可选模型`;
    this.modelSelect.disabled =
      Boolean(this.activeRoundId) || this.sending || models.length < 2;
    this.modelRow.hidden = false;
  }

  private async onModelChange(): Promise<void> {
    const id = this.modelSelect.value.trim();
    if (!id || this.modelSelect.disabled) return;
    this.modelSelect.disabled = true;
    try {
      const status = await invoke<AgentStatusLike>("sandbox_set_model", { modelId: id });
      this.applyAgentStatus(status);
      this.toast(`已切换模型：${id}`, "success");
    } catch (error) {
      this.toast(`切换模型失败：${errorMessage(error)}`, "error");
      await this.refreshStatus();
    }
  }

  private apply(update: DialogueUpdate): void {
    if (update.sandbox_session_id !== this.sessionId) return;
    switch (update.kind) {
      case "agent_event":
        this.events.push(update.event);
        this.renderStrip();
        break;
      case "transcript":
        if (!this.transcript.some((entry) => sameEntry(entry, update.entry))) {
          this.transcript.push(update.entry);
          this.renderLog();
        }
        break;
      case "round_finished":
        if (this.activeRoundId === update.round_id) this.activeRoundId = null;
        this.syncControls();
        break;
    }
  }

  private async send(): Promise<void> {
    const text = this.input.value.trim();
    const sessionId = this.sessionId;
    if (!text || !sessionId || this.sending || this.activeRoundId) return;
    this.sending = true;
    this.syncControls();
    try {
      const outcome = await invoke<SendOutcome>("sandbox_send_message", { sandboxSessionId: sessionId, text });
      if (this.sessionId === sessionId) {
        this.activeRoundId = outcome.roundId;
        if (!this.transcript.some((entry) => sameEntry(entry, outcome.userEntry))) {
          this.transcript.push(outcome.userEntry);
        }
        this.input.value = "";
      }
    } catch (error) {
      this.toast(errorMessage(error), "error");
      // A failed start still appends user + system lines on disk; reload them.
      if (this.sessionId === sessionId) {
        try {
          const dialogue = await invoke<SessionDialogue>("sandbox_session_dialogue", { sandboxSessionId: sessionId });
          this.transcript = dialogue.transcript;
          this.events = dialogue.events;
          this.activeRoundId = dialogue.activeRoundId;
        } catch {
          /* keep what we have */
        }
      }
    } finally {
      this.sending = false;
      this.renderAll();
    }
  }

  private async cancel(): Promise<void> {
    if (!this.sessionId || !this.activeRoundId) return;
    try {
      await invoke<boolean>("sandbox_cancel_round", { sandboxSessionId: this.sessionId });
    } catch (error) {
      this.toast(errorMessage(error), "error");
    }
  }

  private renderAll(): void {
    this.renderLog();
    this.renderStrip();
    this.syncControls();
  }

  private renderLog(): void {
    if (this.transcript.length === 0) {
      this.log.replaceChildren(
        el("li", "sandbox-empty", "还没有对话。Agent 只能读写这个沙箱的隔离副本；检查点不会被修改。"),
      );
      return;
    }
    this.log.replaceChildren(
      ...this.transcript.map((entry) => {
        const item = el("li", `sandbox-line role-${entry.role}`);
        item.append(el("span", "sandbox-line-role", ROLE_LABEL[entry.role] ?? entry.role), el("div", "sandbox-line-text", entry.text));
        return item;
      }),
    );
    this.log.scrollTop = this.log.scrollHeight;
  }

  private renderStrip(): void {
    const round = latestRoundEvents(this.events);
    if (round.length === 0) {
      this.strip.replaceChildren(el("span", "sandbox-strip-empty", "事件条：尚无事件"));
      return;
    }
    this.strip.replaceChildren(
      ...round.map((event) => {
        const chip = eventChip(event);
        const node = el("span", `sandbox-chip tone-${chip.tone}`, chip.label);
        node.title = chip.title;
        return node;
      }),
    );
    this.strip.scrollLeft = this.strip.scrollWidth;
  }

  private syncControls(): void {
    const running = this.activeRoundId !== null;
    this.root.classList.toggle("is-running", running || this.sending);
    this.sendButton.disabled = running || this.sending || !this.sessionId;
    this.sendButton.textContent = running ? "进行中…" : "发送";
    this.cancelButton.hidden = !running;
    if (!this.modelRow.hidden) {
      this.modelSelect.disabled = running || this.sending || this.modelSelect.options.length < 2;
    }
  }
}

/** Form controls that must not start a dock drag. */
function isInteractive(target: EventTarget | null): boolean {
  return (
    target instanceof Element &&
    target.closest("button, select, option, input, textarea, a, label, [contenteditable], [data-no-drag]") !==
      null
  );
}

function sameEntry(a: TranscriptEntry, b: TranscriptEntry): boolean {
  return a.ts === b.ts && a.role === b.role && a.text === b.text;
}
