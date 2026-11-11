/**
 * S1 dialogue shell (docs/sandbox-flow.md §4.2, §5 S1): a plain chat over one
 * sandbox session, backed by the local Codex with the 7 Mathscape MCP tools,
 * plus a per-turn process timeline (collapsible cards before the answer).
 *
 * A2 (A-PR2): the same shell renders in one of two dock modes. `sidebar`
 * (default) pins it to the right edge below the top bar and shrinks the map
 * pane so map and dialogue share the screen; `floating` is the S1 window
 * (draggable, corner-resizable). The header button switches modes; the
 * mode, sidebar width, and floating size/position are all remembered. When
 * collapsed it becomes a draggable float handle that reopens in the last
 * mode. A window too narrow for a sidebar renders floating. The status
 * line shows Codex availability; when the adapter supports selection a
 * model <select> and a reasoning-effort <select> (levels from the selected
 * model's catalog entry) let the user pick before the next round. Both are
 * disabled while a round runs.
 *
 * A-PR3: agent and user bubbles render Markdown + KaTeX (bubble.ts); ids in
 * bubbles and in process-card args/results become cite chips (cite.ts) that
 * select the node on the map through `setCiteContext().jump`.
 *
 * Composer chips: the input (composer.ts) shows the same chips for ids the
 * landscape or this session's tool results know; sending expands them back
 * to the full ids, so the agent receives exactly what was pasted or typed.
 *
 * Map Scene Player (first slice): every live tool_call / tool_result compiles
 * to a map scene (scene/compile.ts) played through `setCiteContext().scenes`;
 * process cards replay one step (▶ per step) or the whole turn (回放全部),
 * recompiled from the round's persisted events.
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
  effortOptionLabel,
  effortOptions,
  modelCatalogHint,
  modelOptionLabel,
  ROLE_LABEL,
  type AgentEventLike,
  type AgentStatusLike,
  type TranscriptRole,
} from "./format";
import type { IncidenceNode } from "../graph/projection";
import { renderBubbleHtml } from "./bubble";
import {
  buildCiteIndex,
  buildRoundCiteIndex,
  citeResolver,
  citeTextHtml,
  type CiteIndex,
  type CiteResolver,
  type CiteTarget,
} from "./cite";
import { ChipComposer } from "./composer";
import { mentionTargets } from "./composer-chips";
import {
  compileRoundScenes,
  compileToolScene,
  isParsingSummary,
  parsingScene,
  readPdfPages,
  resolveScene,
} from "./scene/compile";
import type { SceneDraft, SceneResolve, ToolScene } from "./scene/types";
import {
  clampSidebarWidth,
  effectiveDockMode,
  parseDockMode,
  parseSidebarWidth,
  type DockMode,
} from "./layout";
import {
  KIND_ICON,
  STATUS_LABEL,
  dialogueRows,
  type ToolStep,
  type TurnUsage,
  type TurnView,
} from "./steps";
import { openProposalReview, reviewBeforeLeaving } from "../review/change-sets";
import "../review/change-sets.css";

export const DIALOGUE_EVENT = "sandbox://dialogue";
export const DIALOGUE_SIZE_KEY = "mathscape:sandbox-dialogue-size";
export const DIALOGUE_POS_KEY = "mathscape:sandbox-dialogue-pos";
export const DIALOGUE_MODE_KEY = "mathscape:sandbox-dialogue-mode";
export const DIALOGUE_SIDEBAR_WIDTH_KEY = "mathscape:sandbox-dialogue-sidebar-width";
/** Set on `.shell` while the sidebar is showing; layout CSS reads the width var. */
export const SIDEBAR_SHELL_CLASS = "has-sandbox-sidebar";
const SIDEBAR_WIDTH_VAR = "--sandbox-sidebar-width";
const SIDEBAR_KEY_STEP = 24;

const DEFAULT_WIDTH = 460;
const DEFAULT_HEIGHT = 520;
const MIN_WIDTH = 320;
const MIN_HEIGHT = 280;
const HANDLE_SIZE = 48;

export interface TranscriptEntry {
  ts: string;
  role: TranscriptRole;
  text: string;
  round_id?: string;
  outcome?: string;
  /** Absent on lines written before dialogue threads existed (= "d1"). */
  thread_id?: string;
}

interface DialogueThread {
  thread_id: string;
  created_at: string;
}

interface SessionDialogue {
  sandboxSessionId: string;
  /** Every thread's lines; the dock filters by the viewed thread. */
  transcript: TranscriptEntry[];
  threads: DialogueThread[];
  activeThreadId: string;
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

/** How the dock reaches the map without importing main (A-PR3). */
export interface CiteContext {
  /** Landscape projection nodes currently on the map. */
  nodes: () => readonly IncidenceNode[];
  /** Select + reveal a projection node; the dock stays open. */
  jump: (nodeId: string) => void;
  /** Map Scene Player: live tool events and process-card replays. */
  scenes?: {
    live: (scene: ToolScene, part: "call" | "result") => void;
    replay: (scenes: ToolScene[]) => void;
    /** The round ended (or the session changed): forget in-flight calls. */
    idle?: () => void;
  };
  /**
   * Sync ensure (「解析中」): read a map node's current summary from a fresh
   * snapshot (re-rendering the map only if it changed). A long-running
   * scanned-PDF `material.read` polls this to tell parsing from reading.
   */
  probeSummary?: (nodeId: string) => Promise<string | null>;
}

/** First 「解析中」 probe after a scanned-page read starts, then every … */
/** The old textarea's maxLength, now checked on send (chips expand). */
const COMPOSER_MAX_LENGTH = 20_000;
/** Implicit first dialogue thread (mirrors `mathscape_sandbox::FIRST_THREAD_ID`). */
const FIRST_THREAD_ID = "d1";

export const PARSING_PROBE = { firstMs: 2500, everyMs: 3000 };

/** Cite resolver → scene resolver: unique map hits only (no wrong flash). */
function sceneResolve(resolve: CiteResolver): SceneResolve {
  return (id) => {
    const hit = resolve(id);
    return hit.status === "map" && hit.target ? { nodeId: hit.target.nodeId, label: hit.target.label } : null;
  };
}

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

function readPref(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function writePref(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
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
  private readonly effortRow: HTMLElement;
  private readonly effortSelect: HTMLSelectElement;
  private readonly log: HTMLElement;
  private readonly input: ChipComposer;
  private readonly sendButton: HTMLButtonElement;
  private readonly cancelButton: HTMLButtonElement;
  private readonly collapseButton: HTMLButtonElement;
  private readonly floatHandle: HTMLButtonElement;
  private readonly resizeHandle: HTMLElement;
  private readonly sideResizeHandle: HTMLElement;
  private readonly modeButton: HTMLButtonElement;
  private readonly threadRow: HTMLElement;
  private readonly threadSelect: HTMLSelectElement;
  private readonly newThreadButton: HTMLButtonElement;
  private readonly reviewButton: HTMLButtonElement;
  private readonly readonlyNote: HTMLElement;
  /** Element that gives the map pane room for the sidebar (`.shell`). */
  private readonly layoutHost: HTMLElement;

  private sessionId: string | null = null;
  private transcript: TranscriptEntry[] = [];
  private events: AgentEventLike[] = [];
  private activeRoundId: string | null = null;
  /** Dialogue threads on this session's one isolated copy (§3.3). */
  private threads: DialogueThread[] = [];
  private activeThreadId: string = FIRST_THREAD_ID;
  /** Thread on screen; anything but the active one is read-only history. */
  private viewThreadId: string = FIRST_THREAD_ID;
  private switchingThread = false;
  private sending = false;
  private unlisten: Promise<UnlistenFn> | null = null;
  /** User override for a round's process section. Absent means follow running/done. */
  private readonly processOpen = new Map<string, boolean>();
  /** Step cards the user has expanded, keyed by round + step id. */
  private readonly stepOpen = new Set<string>();
  private size = loadSize();
  private pos: PosPref = loadPos() ?? defaultPos(this.size);
  private mode: DockMode = parseDockMode(readPref(DIALOGUE_MODE_KEY));
  private sidebarWidth = parseSidebarWidth(readPref(DIALOGUE_SIDEBAR_WIDTH_KEY), window.innerWidth);
  /** Last sidebar footprint pushed to the shell; avoids resize feedback loops. */
  private layoutKey = "none";
  private cite: CiteContext | null = null;
  private citeIndexCache: { nodes: readonly IncidenceNode[]; index: CiteIndex } | null = null;
  /** Composer resolver: map index + every tool id this session has seen. */
  private mentionCache: { index: CiteIndex | null; targets: CiteTarget[] } | null = null;
  private composerCache: { index: CiteIndex; events: number; session: string | null; resolve: CiteResolver } | null = null;
  /** In-flight scanned-page `material.read` calls → probe timer. */
  private readonly parseWatch = new Map<string, ReturnType<typeof setTimeout>>();
  /** Calls the map currently shows as 「解析中」 (process card label). */
  private readonly parsingCalls = new Set<string>();

  constructor(
    host: HTMLElement,
    private readonly toast: Toast,
  ) {
    this.layoutHost = host.closest<HTMLElement>(".shell") ?? document.body;
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
    // Reasoning effort sits next to the model picker and is rebuilt from the
    // selected model's catalog entry whenever the model changes.
    this.effortRow = el("label", "sandbox-dock-model sandbox-dock-effort");
    this.effortRow.hidden = true;
    this.effortRow.append(el("span", "sandbox-dock-model-label", "推理强度"));
    this.effortSelect = el("select", "sandbox-dock-effort-select");
    this.effortSelect.setAttribute("aria-label", "选择推理强度");
    this.effortSelect.disabled = true;
    this.effortSelect.onchange = () => void this.onEffortChange();
    this.effortRow.append(this.effortSelect);
    // Dialogue threads: same sandbox copy, fresh agent context per thread.
    this.threadRow = el("div", "sandbox-dock-threads");
    const threadLabel = el("label", "sandbox-dock-model");
    threadLabel.append(el("span", "sandbox-dock-model-label", "对话"));
    this.threadSelect = el("select", "sandbox-dock-thread-select");
    this.threadSelect.setAttribute("aria-label", "查看本沙箱的对话");
    this.threadSelect.onchange = () => {
      this.viewThreadId = this.threadSelect.value;
      this.renderAll();
    };
    threadLabel.append(this.threadSelect);
    this.newThreadButton = el("button", "sandbox-btn sandbox-btn-quiet sandbox-dock-new-thread", "新对话");
    this.newThreadButton.type = "button";
    this.newThreadButton.title = "开一条新对话：Agent 上下文清空，沙箱副本里的状态保留";
    this.newThreadButton.onclick = () => void this.newThread();
    this.reviewButton = el("button", "sandbox-btn sandbox-btn-quiet sandbox-dock-review", "审核变更");
    this.reviewButton.type = "button";
    this.reviewButton.title = "审核当前对话调查的 staged / blocked 提案";
    this.reviewButton.onclick = () => void this.reviewChanges();
    this.threadRow.append(threadLabel, this.newThreadButton, this.reviewButton);
    const pickers = el("div", "sandbox-dock-pickers");
    pickers.append(this.modelRow, this.effortRow);
    this.modelHint = el("span", "sandbox-dock-model-hint");
    this.modelHint.hidden = true;
    heading.append(
      el("span", "settings-eyebrow", "SANDBOX · S1"),
      this.title,
      this.status,
      this.threadRow,
      pickers,
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
    this.modeButton = el("button", "sandbox-dock-icon sandbox-dock-mode");
    this.modeButton.type = "button";
    this.modeButton.onclick = () => this.setMode(this.mode === "sidebar" ? "floating" : "sidebar");
    header.append(heading, this.modeButton, this.collapseButton, close);
    this.bindDrag(header, () => ({ width: this.size.width, height: this.size.height }), false, () =>
      this.effectiveMode() === "floating",
    );

    const body = el("div", "sandbox-dock-body");
    this.log = el("ol", "sandbox-dock-log");
    this.log.setAttribute("aria-live", "polite");
    this.log.addEventListener("click", (event) => this.onLogClick(event));

    const composer = el("form", "sandbox-dock-composer");
    this.input = new ChipComposer({
      placeholder: "向沙箱里的 Agent 提问（Enter 发送，Shift+Enter 换行）",
      ariaLabel: "向沙箱里的 Agent 提问",
      resolver: () => this.composerResolver(),
      onJump: (nodeId) => this.cite?.jump(nodeId),
      onSubmit: () => void this.send(),
      mentions: () => this.mentionTargets(),
    });
    const actions = el("div", "sandbox-actions");
    this.cancelButton = el("button", "sandbox-btn sandbox-btn-quiet", "取消本轮");
    this.cancelButton.type = "button";
    this.cancelButton.onclick = () => void this.cancel();
    this.sendButton = el("button", "sandbox-btn sandbox-btn-primary", "发送");
    this.sendButton.type = "submit";
    actions.append(el("span", "sandbox-actions-spacer"), this.cancelButton, this.sendButton);
    this.readonlyNote = el("p", "sandbox-dock-readonly", "");
    this.readonlyNote.hidden = true;
    const backToActive = el("button", "sandbox-btn sandbox-btn-quiet", "回到当前对话");
    backToActive.type = "button";
    backToActive.onclick = () => {
      this.viewThreadId = this.activeThreadId;
      this.renderAll();
      this.input.focus();
    };
    this.readonlyNote.append(el("span", "", "这是只读的历史对话，不会再发给 Agent。"), backToActive);
    composer.append(this.readonlyNote, this.input.element, actions);
    composer.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.send();
    });

    body.append(this.log, composer);

    this.resizeHandle = el("div", "sandbox-dock-resize");
    this.resizeHandle.setAttribute("role", "separator");
    this.resizeHandle.setAttribute("aria-orientation", "both");
    this.resizeHandle.setAttribute("aria-label", "调整对话窗口大小");
    this.resizeHandle.title = "拖动调整大小";
    this.bindResize(this.resizeHandle);

    this.sideResizeHandle = el("div", "sandbox-dock-side-resize");
    this.sideResizeHandle.setAttribute("role", "separator");
    this.sideResizeHandle.setAttribute("aria-orientation", "vertical");
    this.sideResizeHandle.setAttribute("aria-label", "调整侧栏宽度");
    this.sideResizeHandle.tabIndex = 0;
    this.sideResizeHandle.title = "拖动调整侧栏宽度（←/→ 微调）";
    this.bindSideResize(this.sideResizeHandle);

    this.root.append(header, body, this.resizeHandle, this.sideResizeHandle);

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

  /** The persisted preference (what the user picked last). */
  get dockMode(): DockMode {
    return this.mode;
  }

  /** What is actually rendered: a too-narrow window falls back to floating. */
  private effectiveMode(): DockMode {
    return effectiveDockMode(this.mode, window.innerWidth);
  }

  setMode(mode: DockMode): void {
    if (mode !== this.mode) {
      this.mode = mode;
      writePref(DIALOGUE_MODE_KEY, mode);
    }
    this.applyGeometry();
  }

  get currentSessionId(): string | null {
    if (this.root.hidden && this.floatHandle.hidden) return null;
    return this.sessionId;
  }

  /**
   * Show `sessionId`. Switching away from another session first runs the
   * blocking batch review of that session's proposals; resolves `false`
   * when the user cancels the switch.
   */
  async open(sessionId: string): Promise<boolean> {
    const leaving = this.sessionId;
    if (leaving && leaving !== sessionId && !await reviewBeforeLeaving(leaving, "switch_session", this.toast)) return false;
    this.ensureListening();
    this.floatHandle.hidden = true;
    this.root.hidden = false;
    this.root.classList.remove("is-collapsed");
    if (this.sessionId !== sessionId) {
      for (const timer of this.parseWatch.values()) clearTimeout(timer);
      this.parseWatch.clear();
      this.parsingCalls.clear();
      this.cite?.scenes?.idle?.();
      this.sessionId = sessionId;
      this.transcript = [];
      this.events = [];
      this.activeRoundId = null;
      this.threads = [];
      this.activeThreadId = FIRST_THREAD_ID;
      this.viewThreadId = FIRST_THREAD_ID;
    }
    this.title.textContent = `沙箱对话 · ${sessionId}`;
    this.floatHandle.textContent = "对话";
    this.applyGeometry();
    void this.refreshStatus();
    try {
      const dialogue = await invoke<SessionDialogue>("sandbox_session_dialogue", { sandboxSessionId: sessionId });
      if (this.sessionId !== sessionId) return true;
      this.adopt(dialogue, true);
    } catch (error) {
      this.toast(errorMessage(error), "error");
    }
    this.renderAll();
    this.input.focus();
    return true;
  }

  close(): void {
    this.root.hidden = true;
    this.floatHandle.hidden = true;
    this.applyGeometry();
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
    const sidebar = this.effectiveMode() === "sidebar";
    this.root.classList.toggle("is-sidebar", sidebar);
    this.root.classList.toggle("is-floating", !sidebar);
    this.root.dataset.dockMode = sidebar ? "sidebar" : "floating";
    this.resizeHandle.hidden = sidebar;
    this.sideResizeHandle.hidden = !sidebar;
    this.modeButton.textContent = sidebar ? "浮动" : "侧栏";
    this.modeButton.title = sidebar ? "改为浮动窗口" : "停靠到右侧栏（与地图同屏）";
    this.modeButton.setAttribute("aria-label", this.modeButton.title);
    this.modeButton.setAttribute("aria-pressed", sidebar ? "true" : "false");
    if (sidebar) {
      this.sidebarWidth = clampSidebarWidth(this.sidebarWidth, window.innerWidth);
      this.root.style.width = `${this.sidebarWidth}px`;
      this.root.style.height = "auto";
      this.root.style.left = "auto";
      this.root.style.right = "0px";
      this.root.style.top = `${this.sidebarTop()}px`;
      this.root.style.bottom = "0px";
    } else {
      this.root.style.width = `${this.size.width}px`;
      this.root.style.height = `${this.size.height}px`;
      this.root.style.left = `${this.pos.left}px`;
      this.root.style.top = `${this.pos.top}px`;
      this.root.style.right = "auto";
      this.root.style.bottom = "auto";
    }
    this.floatHandle.style.left = `${this.pos.left}px`;
    this.floatHandle.style.top = `${this.pos.top}px`;
    this.floatHandle.style.right = "auto";
    this.floatHandle.style.bottom = "auto";
    this.syncShellLayout(sidebar && !this.root.hidden);
  }

  /** Sidebar starts under the top bar so the banner and controls stay reachable. */
  private sidebarTop(): number {
    const topbar = this.layoutHost.querySelector<HTMLElement>(".topbar");
    const bottom = topbar?.getBoundingClientRect().bottom ?? 0;
    return Math.max(0, Math.round(bottom));
  }

  /**
   * Give the map pane room for the sidebar (it shrinks rather than being
   * covered) and nudge listeners such as the graph canvas to re-measure.
   */
  private syncShellLayout(showing: boolean): void {
    const key = showing ? `sidebar:${this.sidebarWidth}` : "none";
    if (key === this.layoutKey) return;
    this.layoutKey = key;
    this.layoutHost.classList.toggle(SIDEBAR_SHELL_CLASS, showing);
    if (showing) this.layoutHost.style.setProperty(SIDEBAR_WIDTH_VAR, `${this.sidebarWidth}px`);
    else this.layoutHost.style.removeProperty(SIDEBAR_WIDTH_VAR);
    this.scheduleRelayout();
  }

  private relayoutFrame: number | null = null;

  private scheduleRelayout(): void {
    if (this.relayoutFrame !== null) return;
    this.relayoutFrame = requestAnimationFrame(() => {
      this.relayoutFrame = null;
      window.dispatchEvent(new Event("resize"));
    });
  }

  private bindSideResize(handle: HTMLElement): void {
    let dragging: number | undefined;
    let startX = 0;
    let origin = 0;
    handle.addEventListener("pointerdown", (event) => {
      if (event.button !== 0) return;
      event.preventDefault();
      event.stopPropagation();
      dragging = event.pointerId;
      startX = event.clientX;
      origin = this.sidebarWidth;
      try {
        handle.setPointerCapture(event.pointerId);
      } catch {
        /* Synthetic pointers. */
      }
      handle.classList.add("is-dragging");
      this.root.classList.add("is-resizing");
    });
    handle.addEventListener("pointermove", (event) => {
      if (event.pointerId !== dragging) return;
      // The handle is on the left edge: dragging left widens the sidebar.
      this.sidebarWidth = clampSidebarWidth(origin - (event.clientX - startX), window.innerWidth);
      this.applyGeometry();
    });
    const finish = (event: PointerEvent) => {
      if (event.pointerId !== dragging) return;
      dragging = undefined;
      handle.classList.remove("is-dragging");
      this.root.classList.remove("is-resizing");
      writePref(DIALOGUE_SIDEBAR_WIDTH_KEY, String(this.sidebarWidth));
    };
    handle.addEventListener("pointerup", finish);
    handle.addEventListener("pointercancel", finish);
    handle.addEventListener("keydown", (event) => {
      if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
      event.preventDefault();
      const delta = event.key === "ArrowLeft" ? SIDEBAR_KEY_STEP : -SIDEBAR_KEY_STEP;
      this.sidebarWidth = clampSidebarWidth(this.sidebarWidth + delta, window.innerWidth);
      this.applyGeometry();
      writePref(DIALOGUE_SIDEBAR_WIDTH_KEY, String(this.sidebarWidth));
    });
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
    enabled: () => boolean = () => true,
  ): void {
    let dragging: number | undefined;
    let startX = 0;
    let startY = 0;
    let origin: PosPref = { left: 0, top: 0 };
    let moved = false;
    target.addEventListener("pointerdown", (event) => {
      if (event.button !== 0 || !enabled()) return;
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
      this.effortRow.hidden = true;
      this.effortSelect.disabled = true;
    }
  }

  private applyAgentStatus(status: AgentStatusLike): void {
    this.status.textContent = agentStatusLine(status);
    this.status.classList.toggle("is-unavailable", !status.available);
    this.currentEffort = status.effort?.trim() || null;
    this.applyEffort(status);
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

  private currentEffort: string | null = null;

  /** Rebuild the effort <select> from the current model's advertised levels. */
  private applyEffort(status: AgentStatusLike): void {
    const options = effortOptions(status.efforts, status.effort);
    const canSelect = Boolean(status.effortSelection) && status.available && options.length > 0;
    this.effortSelect.replaceChildren();
    if (!canSelect) {
      this.effortRow.hidden = true;
      this.effortSelect.disabled = true;
      return;
    }
    const model = status.models?.find((m) => m.id === status.model?.trim());
    const fallback = model?.defaultReasoningEffort ?? null;
    const current = status.effort?.trim() || "";
    if (!current) {
      const placeholder = document.createElement("option");
      placeholder.value = "";
      placeholder.textContent = "（Codex 默认）";
      this.effortSelect.append(placeholder);
    }
    for (const effort of options) {
      const option = document.createElement("option");
      option.value = effort.id;
      option.textContent = effortOptionLabel(effort, fallback);
      if (effort.description) option.title = effort.description;
      this.effortSelect.append(option);
    }
    this.effortSelect.value = current;
    const selected = options.find((effort) => effort.id === current);
    this.effortSelect.title = selected?.description ?? `${options.length} 个推理强度`;
    this.effortSelect.disabled =
      Boolean(this.activeRoundId) || this.sending || options.length < 2;
    this.effortRow.hidden = false;
  }

  private async onEffortChange(): Promise<void> {
    const effort = this.effortSelect.value.trim();
    if (!effort || this.effortSelect.disabled) return;
    this.effortSelect.disabled = true;
    try {
      const status = await invoke<AgentStatusLike>("sandbox_set_effort", { effort });
      this.applyAgentStatus(status);
      this.toast(`已切换推理强度：${effort}`, "success");
    } catch (error) {
      this.toast(`切换推理强度失败：${errorMessage(error)}`, "error");
      await this.refreshStatus();
    }
  }

  private async onModelChange(): Promise<void> {
    const id = this.modelSelect.value.trim();
    if (!id || this.modelSelect.disabled) return;
    this.modelSelect.disabled = true;
    this.effortSelect.disabled = true;
    const previousEffort = this.currentEffort;
    try {
      const status = await invoke<AgentStatusLike>("sandbox_set_model", { modelId: id });
      this.applyAgentStatus(status);
      const effort = status.effort?.trim() || null;
      const note =
        effort && previousEffort && effort !== previousEffort
          ? `（推理强度 ${previousEffort} → ${effort}）`
          : "";
      this.toast(`已切换模型：${id}${note}`, "success");
    } catch (error) {
      this.toast(`切换模型失败：${errorMessage(error)}`, "error");
      await this.refreshStatus();
    }
  }

  /** Wire cite chips and echo to the map (main.ts, after the graph exists). */
  setCiteContext(context: CiteContext): void {
    this.cite = context;
    this.refreshCites();
  }

  /** The landscape changed: re-resolve chips against the new map. */
  refreshCites(): void {
    this.citeIndexCache = null;
    this.composerCache = null;
    this.input.refresh();
    if (!this.root.hidden && this.sessionId) this.renderLog(false);
  }

  private citeIndex(): CiteIndex | null {
    if (!this.cite) return null;
    const nodes = this.cite.nodes();
    if (this.citeIndexCache?.nodes !== nodes) this.citeIndexCache = { nodes, index: buildCiteIndex(nodes) };
    return this.citeIndexCache.index;
  }

  /** `@` menu source: the current landscape's nodes (same index as cite chips). */
  private mentionTargets(): CiteTarget[] {
    const index = this.citeIndex();
    if (this.mentionCache?.index !== index) this.mentionCache = { index, targets: mentionTargets(index) };
    return this.mentionCache.targets;
  }

  /** Composer chips: the map plus every round's tool ids and labels (the
   * user cites what the agent showed them anywhere in this session). */
  private composerResolver(): CiteResolver | undefined {
    const index = this.citeIndex();
    if (!index) return undefined;
    const cached = this.composerCache;
    if (cached && cached.index === index && cached.events === this.events.length && cached.session === this.sessionId) {
      return cached.resolve;
    }
    const resolve = citeResolver(index, buildRoundCiteIndex(this.events));
    this.composerCache = { index, events: this.events.length, session: this.sessionId, resolve };
    return resolve;
  }

  /** Resolver per round: the map plus that round's tool ids and labels. */
  private resolvers(): (roundId: string | undefined) => CiteResolver | undefined {
    const index = this.citeIndex();
    const cache = new Map<string, CiteResolver>();
    return (roundId) => {
      if (!index) return undefined;
      const key = roundId ?? "";
      let resolver = cache.get(key);
      if (!resolver) {
        const round = roundId ? buildRoundCiteIndex(this.events.filter((event) => event.round_id === roundId)) : undefined;
        resolver = citeResolver(index, round);
        cache.set(key, resolver);
      }
      return resolver;
    };
  }

  private onLogClick(event: MouseEvent): void {
    const target = event.target instanceof Element ? event.target : null;
    const chip = target?.closest<HTMLElement>("[data-cite-node]");
    if (!chip || !this.cite) return;
    event.preventDefault();
    event.stopPropagation();
    const nodeId = chip.dataset.citeNode;
    if (nodeId) this.cite.jump(nodeId);
  }

  /** Live tool events play as map scenes: call → seeds, result → layers. */
  private sceneEvent(event: AgentEventLike): void {
    const scenes = this.cite?.scenes;
    if (!scenes || (event.type !== "tool_call" && event.type !== "tool_result")) return;
    const callId = event.payload?.call_id;
    if (typeof callId !== "string") return;
    const step = this.events.filter((item) => item.round_id === event.round_id && item.payload?.call_id === callId);
    const call = step.find((item) => item.type === "tool_call");
    const draft = compileToolScene(call, event.type === "tool_result" ? event : undefined, this.events.slice(0, this.events.indexOf(event)));
    const resolve = this.resolvers()(event.round_id);
    if (!draft || !resolve) return;
    scenes.live(resolveScene(draft, sceneResolve(resolve)), event.type === "tool_call" ? "call" : "result");
    if (event.type === "tool_call" && readPdfPages(call)) this.watchParsing(callId);
    if (event.type === "tool_result") this.stopParsing(callId);
  }

  /**
   * A scanned-page `material.read` blocks while Marker parses (sync ensure).
   * Until its result arrives, probe the PDF node: once the map says 「解析中」
   * the call plays as a parsing block (HUD chip + process-card label).
   */
  private watchParsing(callId: string): void {
    if (!this.cite?.probeSummary || this.parseWatch.has(callId)) return;
    this.parseWatch.set(callId, setTimeout(() => void this.probeParsing(callId), PARSING_PROBE.firstMs));
  }

  private stopParsing(callId: string): void {
    clearTimeout(this.parseWatch.get(callId));
    this.parseWatch.delete(callId);
    if (this.parsingCalls.delete(callId)) {
      this.renderLog(false);
      // The lease is gone: let the map drop its 「解析中」 summary.
      const node = this.parsingNode(callId);
      if (node) void this.cite?.probeSummary?.(node).catch(() => null);
    }
  }

  private inFlightRead(callId: string): AgentEventLike | undefined {
    const step = this.events.filter((item) => item.payload?.call_id === callId);
    if (step.some((item) => item.type === "tool_result")) return undefined;
    return step.find((item) => item.type === "tool_call");
  }

  /** Map node of the PDF an in-flight read targets (its first seed). */
  private parsingNode(callId: string): string | undefined {
    const call = this.events.find((item) => item.type === "tool_call" && item.payload?.call_id === callId);
    const draft = compileToolScene(call);
    const resolve = this.resolvers()(call?.round_id);
    const seed = draft?.seeds[0];
    if (!seed || !resolve) return undefined;
    return sceneResolve(resolve)(seed)?.nodeId;
  }

  private async probeParsing(callId: string): Promise<void> {
    if (!this.parseWatch.has(callId)) return;
    const call = this.inFlightRead(callId);
    const node = this.parsingNode(callId);
    const probe = this.cite?.probeSummary;
    if (!call || !node || !probe) {
      this.stopParsing(callId);
      return;
    }
    let summary: string | null = null;
    try {
      summary = await probe(node);
    } catch {
      /* transient: try again next tick */
    }
    // The result may have landed (or the session changed) while probing.
    if (!this.parseWatch.has(callId) || !this.inFlightRead(callId)) return;
    const parsing = isParsingSummary(summary ?? undefined);
    if (parsing !== this.parsingCalls.has(callId)) {
      if (parsing) this.parsingCalls.add(callId);
      else this.parsingCalls.delete(callId);
      this.renderLog(false);
    }
    const draft = compileToolScene(call);
    const resolve = this.resolvers()(call.round_id);
    if (parsing && draft && resolve && this.cite?.scenes) {
      this.cite.scenes.live(resolveScene(parsingScene(draft, readPdfPages(call)), sceneResolve(resolve)), "call");
    }
    this.parseWatch.set(callId, setTimeout(() => void this.probeParsing(callId), PARSING_PROBE.everyMs));
  }

  private replay(drafts: SceneDraft[], resolve: CiteResolver | undefined): void {
    if (!this.cite?.scenes || !resolve || drafts.length === 0) return;
    const map = sceneResolve(resolve);
    this.cite.scenes.replay(drafts.map((draft) => resolveScene(draft, map)));
  }

  private apply(update: DialogueUpdate): void {
    if (update.sandbox_session_id !== this.sessionId) return;
    switch (update.kind) {
      case "agent_event":
        this.events.push(update.event);
        this.renderLog();
        this.sceneEvent(update.event);
        break;
      case "transcript":
        if (!this.transcript.some((entry) => sameEntry(entry, update.entry))) {
          this.transcript.push(update.entry);
          this.renderLog();
        }
        break;
      case "round_finished":
        if (this.activeRoundId === update.round_id) this.activeRoundId = null;
        this.cite?.scenes?.idle?.();
        this.syncControls();
        // Ids this round's tools returned can now chip in the composer.
        this.input.refresh();
        // Replay buttons unlock once the round is over.
        this.renderLog(false);
        break;
    }
  }

  private async send(): Promise<void> {
    // Chips expand to their full ids: the agent gets the exact id strings.
    const text = this.input.value.trim();
    const sessionId = this.sessionId;
    if (!text || !sessionId || this.sending || this.activeRoundId || this.switchingThread) return;
    if (this.viewThreadId !== this.activeThreadId) return;
    if (text.length > COMPOSER_MAX_LENGTH) {
      this.toast(`消息过长（${text.length} / ${COMPOSER_MAX_LENGTH} 字符）`, "error");
      return;
    }
    this.sending = true;
    this.syncControls();
    try {
      const outcome = await invoke<SendOutcome>("sandbox_send_message", { sandboxSessionId: sessionId, text });
      if (this.sessionId === sessionId) {
        this.activeRoundId = outcome.roundId;
        if (!this.transcript.some((entry) => sameEntry(entry, outcome.userEntry))) {
          this.transcript.push(outcome.userEntry);
        }
        this.input.clear();
      }
    } catch (error) {
      this.toast(errorMessage(error), "error");
      // A failed start still appends user + system lines on disk; reload them.
      if (this.sessionId === sessionId) {
        try {
          const dialogue = await invoke<SessionDialogue>("sandbox_session_dialogue", { sandboxSessionId: sessionId });
          if (this.sessionId === sessionId) this.adopt(dialogue, false);
        } catch {
          /* keep what we have */
        }
      }
    } finally {
      this.sending = false;
      this.renderAll();
    }
  }

  /** Take a backend snapshot; `follow` jumps the view to the active thread. */
  private adopt(dialogue: SessionDialogue, follow: boolean): void {
    const viewingActive = this.viewThreadId === this.activeThreadId;
    this.transcript = dialogue.transcript;
    this.events = dialogue.events;
    this.activeRoundId = dialogue.activeRoundId;
    this.threads = dialogue.threads ?? [];
    this.activeThreadId = dialogue.activeThreadId ?? FIRST_THREAD_ID;
    const known = this.threads.some((thread) => thread.thread_id === this.viewThreadId);
    if (follow || viewingActive || !known) this.viewThreadId = this.activeThreadId;
  }

  /** 「新对话」: fresh agent context on the same isolated copy. */
  private async newThread(): Promise<void> {
    const sessionId = this.sessionId;
    if (!sessionId || this.sending || this.activeRoundId || this.switchingThread) return;
    // A thread is metadata of the session's investigation: switching threads
    // keeps proposals staged and is not a review trigger.
    this.switchingThread = true;
    this.syncControls();
    try {
      const dialogue = await invoke<SessionDialogue>("sandbox_new_dialogue_thread", { sandboxSessionId: sessionId });
      if (this.sessionId === sessionId) {
        this.adopt(dialogue, true);
        this.input.clear();
      }
    } catch (error) {
      this.toast(errorMessage(error), "error");
    } finally {
      this.switchingThread = false;
      this.renderAll();
      this.input.focus();
    }
  }

  private async reviewChanges(): Promise<void> {
    if (!this.sessionId) return;
    try {
      await openProposalReview(this.sessionId, this.toast);
    } catch (error) {
      this.toast(`无法打开提案审核：${errorMessage(error)}`, "error");
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
    this.syncControls();
  }

  private renderLog(stick = true): void {
    const nearBottom = this.log.scrollHeight - this.log.scrollTop - this.log.clientHeight < 48;
    const view = threadView(this.transcript, this.events, this.viewThreadId, this.activeThreadId, this.activeRoundId);
    const rows = dialogueRows(view.transcript, view.events, view.activeRoundId, undefined, this.parsingCalls);
    if (rows.length === 0) {
      const continued = this.threads.length > 1 && this.viewThreadId === this.activeThreadId;
      this.log.replaceChildren(
        el(
          "li",
          "sandbox-empty",
          continued
            ? "新对话：Agent 不会看到之前对话的内容，但沙箱副本里的状态都还在。"
            : "还没有对话。Agent 只能读写这个沙箱的隔离副本；检查点不会被修改。",
        ),
      );
      return;
    }
    const resolverFor = this.resolvers();
    this.log.replaceChildren(
      ...rows.map((row) => {
        if (row.kind === "live") {
          return this.renderTurn("agent", null, row.process, resolverFor(row.process.roundId), "正在生成回复…");
        }
        const entry = row.entry;
        const resolve = resolverFor(row.process?.roundId ?? entry.round_id);
        if (!row.process) {
          const item = el("li", `sandbox-line role-${entry.role}`);
          item.append(el("span", "sandbox-line-role", ROLE_LABEL[entry.role] ?? entry.role), messageBody(entry.role, entry.text, resolve));
          return item;
        }
        return this.renderTurn(entry.role, entry.text, row.process, resolve);
      }),
    );
    if (stick && nearBottom) this.log.scrollTop = this.log.scrollHeight;
  }

  private renderTurn(
    role: TranscriptRole,
    text: string | null,
    process: TurnView,
    resolve: CiteResolver | undefined,
    pending?: string,
  ): HTMLLIElement {
    const item = el("li", `sandbox-line role-${role}${pending ? " is-live" : ""}`);
    item.append(el("span", "sandbox-line-role", ROLE_LABEL[role] ?? role));
    const status = el("div", `sandbox-turn-status tone-${process.headerTone}`, process.header);
    if (process.running) status.classList.add("is-running");
    item.append(status);
    item.append(this.renderProcess(process, resolve));
    if (text !== null) item.append(messageBody(role, text, resolve));
    else if (pending) item.append(el("div", "sandbox-line-pending", pending));
    return item;
  }

  private renderProcess(process: TurnView, resolve: CiteResolver | undefined): HTMLElement {
    const key = process.roundId ?? "live";
    const open = this.processOpen.has(key) ? this.processOpen.get(key)! : process.expandedByDefault;
    const section = el("section", "sandbox-process");
    const toggle = el("button", "sandbox-process-summary", `${open ? "▾" : "▸"} ${process.summary}`);
    toggle.type = "button";
    toggle.setAttribute("aria-expanded", open ? "true" : "false");
    toggle.onclick = () => {
      this.processOpen.set(key, !open);
      this.renderLog(false);
    };
    const drafts = this.cite?.scenes && process.roundId
      ? compileRoundScenes(this.events.filter((event) => event.round_id === process.roundId), this.events)
      : [];
    if (drafts.length > 0) {
      const head = el("div", "sandbox-process-head");
      const all = el("button", "sandbox-process-replay", "▶ 回放全部");
      all.type = "button";
      all.title = `在地图上连续回放本轮 ${drafts.length} 步`;
      all.disabled = process.running || this.activeRoundId !== null;
      all.onclick = () => this.replay(drafts, resolve);
      head.append(toggle, all);
      section.append(head);
    } else {
      section.append(toggle);
    }
    if (!open) return section;
    if (process.steps.length === 0) {
      section.append(el("p", "sandbox-process-live", process.running ? "正在处理…" : "这一轮没有工具步骤"));
    } else {
      const list = el("ol", "sandbox-steps");
      const byCall = new Map(drafts.map((draft) => [draft.callId, draft] as const));
      const locked = process.running || this.activeRoundId !== null;
      for (const step of process.steps) {
        const item = this.renderStep(key, step, resolve);
        const draft = byCall.get(step.id);
        if (draft) {
          const replay = el("button", "sandbox-step-replay", "▶");
          replay.type = "button";
          replay.title = "在地图上回放这一步";
          replay.setAttribute("aria-label", `回放 ${step.title}`);
          replay.disabled = locked;
          replay.onclick = () => this.replay([draft], resolve);
          item.classList.add("has-replay");
          item.firstElementChild?.after(replay);
        }
        list.append(item);
      }
      section.append(list);
    }
    if (process.usage) section.append(this.renderUsage(process.usage));
    return section;
  }

  private renderUsage(usage: TurnUsage): HTMLElement {
    const cached = usage.cachedInputTokens ? `（缓存 ${usage.cachedInputTokens}）` : "";
    const line = el(
      "p",
      "sandbox-process-usage",
      `用量：输入 ${usage.inputTokens}${cached} · 输出 ${usage.outputTokens}`,
    );
    line.title = usage.raw;
    return line;
  }

  private renderStep(roundKey: string, step: ToolStep, resolve: CiteResolver | undefined): HTMLLIElement {
    const openKey = `${roundKey}:${step.id}`;
    const open = this.stepOpen.has(openKey);
    const item = el("li", `sandbox-step is-${step.status}`);
    const head = el("button", "sandbox-step-head");
    head.type = "button";
    head.setAttribute("aria-expanded", open ? "true" : "false");
    head.append(
      el("span", "sandbox-step-kind", KIND_ICON[step.kind]),
      el("span", "sandbox-step-title", step.title),
      el("span", `sandbox-step-badge tone-${step.status}`, STATUS_LABEL[step.status]),
    );
    head.onclick = () => {
      if (open) this.stepOpen.delete(openKey);
      else this.stepOpen.add(openKey);
      this.renderLog(false);
    };
    item.append(head);
    if (open) item.append(this.renderStepDetail(step, resolve));
    return item;
  }

  private renderStepDetail(step: ToolStep, resolve: CiteResolver | undefined): HTMLElement {
    const detail = el("div", "sandbox-step-detail");
    const block = (label: string, value: string | undefined) => {
      if (!value) return;
      const pre = el("pre", "sandbox-step-pre");
      // citeTextHtml escapes every non-chip piece.
      pre.innerHTML = citeTextHtml(value, resolve);
      detail.append(el("div", "sandbox-step-label", label), pre);
    };
    block("参数", step.rawInput);
    block("结果", step.rawOutput);
    block("错误", step.error);
    if (!detail.childElementCount) detail.append(el("div", "sandbox-step-label", "没有更多细节"));
    return detail;
  }

  private renderThreadPicker(): void {
    const threads = this.threads.length ? this.threads : [{ thread_id: FIRST_THREAD_ID, created_at: "" }];
    const key = threads.map((t) => t.thread_id).join(",") + `|${this.activeThreadId}`;
    if (this.threadSelect.dataset.key !== key) {
      this.threadSelect.replaceChildren(
        ...threads.map((thread) => {
          const option = el(
            "option",
            "",
            thread.thread_id === this.activeThreadId ? `${threadLabel(thread.thread_id)}（当前）` : `${threadLabel(thread.thread_id)}（只读）`,
          );
          option.value = thread.thread_id;
          if (thread.created_at) option.title = `开始于 ${thread.created_at}`;
          return option;
        }),
      );
      this.threadSelect.dataset.key = key;
    }
    this.threadSelect.value = this.viewThreadId;
    this.threadSelect.disabled = threads.length < 2;
  }

  private syncControls(): void {
    const running = this.activeRoundId !== null;
    const readonly = this.viewThreadId !== this.activeThreadId;
    this.root.classList.toggle("is-running", running || this.sending);
    this.root.classList.toggle("is-readonly-thread", readonly);
    this.sendButton.disabled = running || this.sending || this.switchingThread || readonly || !this.sessionId;
    this.sendButton.textContent = running ? "进行中…" : "发送";
    this.cancelButton.hidden = !running;
    this.readonlyNote.hidden = !readonly;
    this.input.element.hidden = readonly;
    this.renderThreadPicker();
    this.newThreadButton.disabled = running || this.sending || this.switchingThread || !this.sessionId;
    this.reviewButton.disabled = running || this.sending || this.switchingThread || !this.sessionId;
    this.reviewButton.title = this.reviewButton.disabled
      ? (this.sessionId ? "本轮对话进行中，结束后才能审核提案" : "没有打开沙箱会话")
      : "审核当前对话调查的 staged / blocked 提案";
    if (!this.modelRow.hidden) {
      this.modelSelect.disabled = running || this.sending || this.modelSelect.options.length < 2;
    }
    if (!this.effortRow.hidden) {
      this.effortSelect.disabled = running || this.sending || this.effortSelect.options.length < 2;
    }
  }
}

/** Thread id of a transcript line (legacy lines belong to the first thread). */
export function threadOf(entry: { thread_id?: string }): string {
  return entry.thread_id ?? FIRST_THREAD_ID;
}

/** Display label: d1 → 对话 1. */
export function threadLabel(threadId: string): string {
  const n = /^d(\d+)$/.exec(threadId)?.[1];
  return n ? `对话 ${n}` : threadId;
}

/**
 * Lines + events of one thread. Events belong to the rounds whose lines are
 * in that thread; the live round (no reply line yet) only shows on the
 * active thread.
 */
export function threadView<E extends { round_id?: string }>(
  transcript: TranscriptEntry[],
  events: E[],
  viewThreadId: string,
  activeThreadId: string,
  activeRoundId: string | null,
): { transcript: TranscriptEntry[]; events: E[]; activeRoundId: string | null } {
  const lines = transcript.filter((entry) => threadOf(entry) === viewThreadId);
  const rounds = new Set(lines.map((entry) => entry.round_id).filter((id): id is string => Boolean(id)));
  const live = viewThreadId === activeThreadId ? activeRoundId : null;
  if (live) rounds.add(live);
  return {
    transcript: lines,
    events: events.filter((event) => event.round_id !== undefined && rounds.has(event.round_id)),
    activeRoundId: live,
  };
}

/** User/agent text as Markdown + KaTeX with cite chips; system lines and
 * process summaries stay plain text. */
function messageBody(role: TranscriptRole, text: string, resolve: CiteResolver | undefined): HTMLElement {
  if (role === "system") return el("div", "sandbox-line-text", text);
  const body = el("div", "sandbox-line-text markdown-body");
  body.innerHTML = renderBubbleHtml(text, resolve);
  return body;
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
