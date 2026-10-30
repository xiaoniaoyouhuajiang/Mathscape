/**
 * S1 dialogue dock (docs/sandbox-flow.md §4.2, §5 S1): a plain chat over one
 * sandbox session, backed by the local Codex with the 7 Mathscape MCP tools,
 * plus an event strip that shows the live `agent_events v0` stream.
 *
 * Everything the agent reads or writes is the session's isolated copy; the
 * dock never touches the checkpoint. The reply is only shown once the round
 * has an event stream — no fake "connected" state.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { errorMessage } from "../core/ipc";
import {
  agentStatusLine,
  eventChip,
  latestRoundEvents,
  ROLE_LABEL,
  type AgentEventLike,
  type AgentStatusLike,
  type TranscriptRole,
} from "./format";

export const DIALOGUE_EVENT = "sandbox://dialogue";

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

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className = "", text = ""): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text) node.textContent = text;
  return node;
}

export class SandboxDialogueDock {
  private readonly root: HTMLElement;
  private readonly title: HTMLElement;
  private readonly status: HTMLElement;
  private readonly log: HTMLElement;
  private readonly strip: HTMLElement;
  private readonly input: HTMLTextAreaElement;
  private readonly sendButton: HTMLButtonElement;
  private readonly cancelButton: HTMLButtonElement;
  private readonly collapseButton: HTMLButtonElement;

  private sessionId: string | null = null;
  private transcript: TranscriptEntry[] = [];
  private events: AgentEventLike[] = [];
  private activeRoundId: string | null = null;
  private sending = false;
  private unlisten: Promise<UnlistenFn> | null = null;

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
    heading.append(el("span", "settings-eyebrow", "SANDBOX · S1"), this.title, this.status);
    this.collapseButton = el("button", "sandbox-dock-icon", "–");
    this.collapseButton.type = "button";
    this.collapseButton.title = "折叠";
    this.collapseButton.onclick = () => this.toggleCollapsed();
    const close = el("button", "sandbox-dock-icon", "×");
    close.type = "button";
    close.title = "关闭对话（不会取消正在进行的一轮）";
    close.setAttribute("aria-label", "关闭对话");
    close.onclick = () => this.close();
    header.append(heading, this.collapseButton, close);

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
    this.root.append(header, body);
    host.append(this.root);
    this.syncControls();
  }

  get currentSessionId(): string | null {
    return this.root.hidden ? null : this.sessionId;
  }

  async open(sessionId: string): Promise<void> {
    this.ensureListening();
    this.root.hidden = false;
    this.root.classList.remove("is-collapsed");
    if (this.sessionId !== sessionId) {
      this.sessionId = sessionId;
      this.transcript = [];
      this.events = [];
      this.activeRoundId = null;
    }
    this.title.textContent = `沙箱对话 · ${sessionId}`;
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
  }

  /** Close if showing `sessionId` (e.g. after it was discarded). */
  forget(sessionId: string): void {
    if (this.sessionId === sessionId) {
      this.close();
      this.sessionId = null;
    }
  }

  private toggleCollapsed(): void {
    const collapsed = this.root.classList.toggle("is-collapsed");
    this.collapseButton.textContent = collapsed ? "+" : "–";
    this.collapseButton.title = collapsed ? "展开" : "折叠";
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
      this.status.textContent = agentStatusLine(status);
      this.status.classList.toggle("is-unavailable", !status.available);
    } catch (error) {
      this.status.textContent = errorMessage(error);
      this.status.classList.add("is-unavailable");
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
  }
}

function sameEntry(a: TranscriptEntry, b: TranscriptEntry): boolean {
  return a.ts === b.ts && a.role === b.role && a.text === b.text;
}
