/**
 * Map Scene Player (first slice): performs `ToolScene`s on the Cytoscape map.
 *
 * One scene segment = Announce (HUD) → Isolate (existing `is-out-of-focus`
 * dim, nothing is hidden or re-laid out) → fit the camera to the focus →
 * light seeds, then result layers one after another (逐层亮). When the queue
 * has been idle for a moment the player Settles: scene classes go, the
 * controller's own focus scope is re-applied and the camera returns to where
 * the user left it. While a session runs the map does not take pointer
 * input (the dialogue sidebar and typing stay live); the HUD has 停止.
 *
 * Second slice (Act): a scene may carry an Act overlay. `workspace.search`
 * types its query in a HUD line before its hits land (a result segment that
 * follows a typed call shows the full query at once); `pdf.outline` and
 * `ink.page_image` / `pdf.page_image` open a temporary mini-window beside the focused node (or
 * at the map stage's corner when nothing is on the map). The window closes
 * with the next segment and on Settle / 停止; nothing is persisted.
 * Ink pad open = the whole scene is skipped (same `canPlay` rule), so the page
 * image never sits on top of the pad.
 *
 * Third slice (临时收拢布局 / clump): a multi-node focus is pulled toward its
 * centroid (`scene-clump.ts`) so a sparse cluster reads as one topology.
 * Order per segment: snapshot original positions → clump tween and camera
 * fit run together (same duration; the camera aims at the *clumped* box) →
 * seeds / layers light up → Act window opens once both have landed. The next
 * segment re-clumps from the original positions (nodes that left the focus
 * glide home in the same tween). Settle / 停止 / ink pad tween every moved
 * node back to its snapshot, then the camera returns; a cite jump
 * (`cancel(false)`) and any graph re-render restore instantly. Junctions and
 * ink-note satellites follow their endpoints; edges follow on their own.
 * Single-node focus: no-op. Kill-switch: `SCENE_CLUMP.enabled`.
 *
 * Parsing (sync ensure): a `material.read` that waits on Marker carries a
 * `parsing` act. While `running`, the HUD shows a 「解析中」 chip, the PDF
 * node gets `scene-parsing`, and the segment holds (up to `parsingHoldMs`,
 * ending early once the next segment — normally the result — is queued).
 * Repeated running segments for one call replace each other in the queue.
 *
 * Fourth slice: `propose_changes` plays on the map — the proposal's real
 * staged nodes light up when the map already shows them (staged overlay),
 * otherwise transient dashed ghost nodes appear beside their anchors;
 * relations draw one by one, blocked items flash red with their code, a
 * superseded version's nodes fade, and the HUD ends with the `staged_ids`
 * counts. Ghosts are Cytoscape elements that live only inside the segment
 * (removed on the next segment, Settle, 停止 and before any graph sync).
 * `mathlib.search` / `lean.check` / `material.locate` open Act windows
 * (call: running state; result: candidates / diagnostics / matched quote).
 *
 * Long-running tools: any in-flight call older than `RUNNING_SHOW_MS` shows
 * the running pill with its elapsed time (Loogle 启动中 / Lean 编译中 /
 * Marker 解析中 …), independent of the scene queue.
 *
 * Pacing (live only; a replay plays every step): a newer frame of a queued
 * call replaces it, consecutive same-tool segments coalesce, the queue is
 * capped, and when the head is far behind live progress the player skips to
 * the latest (proposals are never dropped). Every segment has a time budget.
 * 「减少动效」 (`host.reducedMotion`): no clump, camera glide, typewriter or
 * one-by-one reveal — states switch instantly and are held to be read.
 *
 * Animation state only: the landscape graph is never touched; clumped
 * coordinates live in Cytoscape for the duration of the scene and are never
 * written to the graph model or storage.
 */
import type { BoundingBox12, Core, NodeCollection, NodeSingular } from "cytoscape";
import { OUT_OF_FOCUS_CLASS, applyFocusScope } from "./cytoscape";
import { SCENE_CLUMP, clumpTargets, ease, lerp, satelliteShift, type Point } from "./scene-clump";
import {
  SCENE_PACING,
  capLayers,
  catchUpLive,
  elapsedLabel,
  enqueueLive,
  runningLabel,
  type ScenePart,
  type Segment,
} from "./scene-pacing";
import type {
  LeanAct,
  LocateAct,
  MathlibAct,
  OutlineAct,
  PageImageAct,
  ProposeAct,
  ProposeItem,
  ProposeRelation,
  SceneAct,
  ToolScene,
} from "../sandbox/scene/types";
import { PROPOSE_STEP_CAP } from "../sandbox/scene/types";
import { renderBubbleHtml } from "../sandbox/bubble";
import { t } from "../i18n";

export { SCENE_PACING, type ScenePart } from "./scene-pacing";

interface Session {
  zoom: number;
  pan: { x: number; y: number };
  panning: boolean;
  zooming: boolean;
  ungrabify: boolean;
}

interface Visual {
  focus: Set<string>;
  seeds: string[];
  layers: string[][];
  /** Seeds being parsed (sync ensure): `scene-parsing`. */
  parsing?: boolean;
  /** Extra classes by element id (proposal: hits, ghost edges, blocked, retired). */
  marks?: Map<string, string[]>;
}

type WindowAct = Extract<SceneAct, { kind: "outline" | "page_image" | "mathlib" | "lean" | "locate" }>;
/** Acts whose window also opens on the call (running state). */
const RUNNING_WINDOWS = new Set<SceneAct["kind"]>(["mathlib", "lean"]);

const isParsingRun = (scene: ToolScene): boolean => scene.act?.kind === "parsing" && scene.act.phase === "running";

export interface ScenePlayerHost {
  core: () => Core | null;
  /** Where the HUD lives (positioned ancestor of the map canvas). */
  hudHost: HTMLElement;
  /** The map canvas element; gets `is-scene-playing` while locked. */
  canvas: HTMLElement;
  /** False while the ink pad / 手稿 layer is open: skip, do not queue. */
  canPlay: () => boolean;
  /** Put the map's own focus scope (focus mode) back after a scene. */
  restoreFocusScope: () => void;
  /** 「减少动效」: instant state changes (read per segment). */
  reducedMotion?: () => boolean;
}

/** In-flight calls older than this show the running pill. */
export const RUNNING_SHOW_MS = 2000;
/** Forget an in-flight call after this long (its result never came). */
const RUNNING_FORGET_MS = 15 * 60_000;

export const SCENE_TIMING = {
  fitMs: 420,
  seedMs: 900,
  layerMs: 700,
  holdMs: 900,
  hudOnlyMs: 1100,
  /** Live: wait this long for the next segment (the result) before Settle. */
  settleIdleMs: 1500,
  settleMs: 420,
  /** Typewriter: per character, clamped so a query types in ≲1.2s. */
  typeCharMinMs: 35,
  typeCharMaxMs: 90,
  typeTotalMs: 1200,
  /** After the query is typed on a call segment (hits come with the result). */
  typeHoldMs: 450,
  /** How long an Act mini-window (目录 / 页图) stays up. */
  actMs: 2200,
  /** A running parse segment holds this long unless something is queued. */
  parsingHoldMs: 4500,
  /** A running Lean / Loogle window holds this long unless something is queued. */
  runningHoldMs: 2400,
  /** Proposal: per ghost / staged node, per relation. */
  proposeItemMs: 220,
  proposeRelationMs: 280,
  /** Blocked flash: on/off phase, number of flashes. */
  flashMs: 200,
  flashes: 3,
  /** Mathlib candidates / Lean diagnostics: one row every … */
  rowMs: 160,
};
const ACT_GAP = 18;
const ACT_MARGIN = 8;
const FIT_PADDING = 64;
const FIT_MAX_ZOOM = 1.6;
/** Nodes that hang off entities and follow them when they move. */
const SATELLITE_SELECTOR = '[elementKind = "junction"], .ink-note-node';
const MARK_CLASSES = "scene-propose-hit scene-propose-edge scene-propose-semantic-hit scene-propose-theory-hit scene-propose-concept-hit scene-propose-declaration-hit scene-propose-problem-schema-hit scene-propose-representation-core-hit scene-propose-debt-hit scene-propose-dialect-scope-hit scene-propose-alignment-edge scene-propose-theory-edge scene-propose-uses-edge scene-propose-debt-edge scene-propose-scope-edge scene-blocked scene-blocked-flash scene-retired";
const SCENE_CLASSES = `sandbox-echo-seed sandbox-echo-hit sandbox-echo-fade sandbox-echo-edge scene-parsing ${MARK_CLASSES}`;
const GHOST_SELECTOR = ".scene-ghost, .scene-ghost-edge";
const GHOST_SIZE = 34;

interface Inflight {
  tool: string;
  label: string;
  since: number;
}

function visualKindClass(visualKind: string): string {
  return visualKind.replace(/_/g, "-");
}

export function proposeItemMarkClass(item: ProposeItem): string {
  if (item.kind !== "semantic" || !item.visualKind) return "";
  return `scene-propose-semantic-hit scene-propose-${visualKindClass(item.visualKind)}-hit`;
}

export function proposeGhostClass(item: ProposeItem): string {
  const classes = ["scene-ghost"];
  if (item.kind === "span") classes.push("scene-ghost-span");
  if (item.kind === "semantic" && item.visualKind) classes.push("scene-ghost-semantic", `scene-ghost-${visualKindClass(item.visualKind)}`);
  return classes.join(" ");
}

export function proposeRelationMarkClass(relation: ProposeRelation): string {
  switch (relation.relationKind) {
    case "alignment":
    case "binds_representation":
      return "scene-propose-alignment-edge";
    case "in_theory": return "scene-propose-theory-edge";
    case "uses_entity": return "scene-propose-uses-edge";
    case "debt_on": return "scene-propose-debt-edge";
    case "scope_of": return "scene-propose-scope-edge";
    default: return "";
  }
}

export function proposeRelationGhostClass(relation: ProposeRelation): string {
  const extra = proposeRelationMarkClass(relation).replace("scene-propose-", "scene-ghost-");
  return extra ? `scene-ghost-edge ${extra}` : "scene-ghost-edge";
}

export class ScenePlayer {
  private queue: Segment[] = [];
  private running = false;
  private token = 0;
  private session: Session | null = null;
  private visual: Visual | null = null;
  private settleTimer: ReturnType<typeof setTimeout> | undefined;
  /** Camera a Settle animation is still heading to (a new session starts there). */
  private restoring: { zoom: number; pan: { x: number; y: number } } | null = null;
  private readonly hud: HTMLElement;
  private readonly hudTitle: HTMLElement;
  private readonly hudSummary: HTMLElement;
  private readonly hudStep: HTMLElement;
  private readonly hudType: HTMLElement;
  private readonly hudTypeText: HTMLElement;
  private readonly hudParsing: HTMLElement;
  /** Clump: original model positions of every node moved this session. */
  private origin = new Map<string, Point>();
  /** Clump: the focus the current segment clumped (re-applied after render). */
  private clumpFocus: string[] = [];
  /** Running position tween (clump in / restore out). */
  private tween: { frame: number; done: () => void } | null = null;
  /** The open Act mini-window, if any. */
  private actWindow: HTMLElement | null = null;
  /** Long-running calls (live) and the pill that shows them. */
  private inflight = new Map<string, Inflight>();
  private ticker: ReturnType<typeof setInterval> | undefined;
  private readonly runningPill: HTMLElement;
  private readonly hudElapsed: HTMLElement;
  /** Call id of the segment on the HUD. */
  private hudCall: string | null = null;
  /** 「减少动效」 for the segment being performed. */
  private reduced = false;
  /** Time budget of the segment being performed (epoch ms). */
  private deadline = Infinity;
  /** Notified when a session starts/ends (tests, dock). */
  onChange?: (playing: boolean) => void;

  constructor(private readonly host: ScenePlayerHost) {
    this.hud = document.createElement("div");
    this.hud.className = "scene-hud";
    this.hud.setAttribute("role", "status");
    this.hud.setAttribute("aria-live", "polite");
    this.hud.hidden = true;
    const tag = document.createElement("span");
    tag.className = "scene-hud-tag";
    tag.textContent = "Agent";
    this.hudTitle = document.createElement("strong");
    this.hudTitle.className = "scene-hud-title";
    this.hudSummary = document.createElement("span");
    this.hudSummary.className = "scene-hud-summary";
    this.hudStep = document.createElement("span");
    this.hudStep.className = "scene-hud-step";
    this.hudType = document.createElement("div");
    this.hudType.className = "scene-hud-type";
    this.hudType.hidden = true;
    this.hudTypeText = document.createElement("div");
    this.hudTypeText.className = "scene-hud-type-text";
    const caret = document.createElement("span");
    caret.className = "scene-hud-caret";
    caret.setAttribute("aria-hidden", "true");
    this.hudType.append(this.hudTypeText, caret);
    const stop = document.createElement("button");
    stop.type = "button";
    stop.className = "scene-hud-stop";
    stop.textContent = "停止";
    stop.title = "停止回放并恢复视角";
    stop.onclick = () => this.cancel();
    this.hudParsing = document.createElement("span");
    this.hudParsing.className = "scene-hud-parsing";
    this.hudParsing.textContent = "解析中";
    this.hudParsing.hidden = true;
    this.hudElapsed = document.createElement("span");
    this.hudElapsed.className = "scene-hud-elapsed";
    this.hudElapsed.hidden = true;
    this.hud.append(tag, this.hudTitle, this.hudStep, this.hudParsing, this.hudElapsed, this.hudType, this.hudSummary, stop);
    host.hudHost.append(this.hud);
    this.runningPill = document.createElement("div");
    this.runningPill.className = "scene-running";
    this.runningPill.setAttribute("role", "status");
    this.runningPill.setAttribute("aria-live", "off");
    this.runningPill.hidden = true;
    host.hudHost.append(this.runningPill);
  }

  isPlaying(): boolean {
    return this.session !== null;
  }

  /** Live tool event: queue behind whatever is playing (paced). */
  live(scene: ToolScene, part: "call" | "result"): void {
    if (!this.host.canPlay()) return;
    this.track(scene, part);
    this.enqueue({ scene, part, enqueuedAt: Date.now(), merged: new Set([scene.callId]) });
    void this.pump();
  }

  /** The round ended / the session changed: nothing is in flight any more. */
  clearRunning(): void {
    this.inflight.clear();
    this.renderRunning();
  }

  /** Test hook: call ids the running pill tracks. */
  runningCalls(): string[] {
    return [...this.inflight.keys()];
  }

  private enqueue(segment: Segment): void {
    this.queue = enqueueLive(this.queue, segment);
  }

  private catchUp(): void {
    this.queue = catchUpLive(this.queue, Date.now());
  }

  /** Replay one step or a whole turn: replaces whatever is playing (never paced). */
  replay(scenes: readonly ToolScene[]): void {
    if (!this.host.canPlay() || scenes.length === 0) return;
    this.token += 1;
    this.queue = scenes.map((scene, index) => ({
      scene,
      part: "full",
      ...(scenes.length > 1 ? { step: index + 1, total: scenes.length } : {}),
    }));
    this.running = false;
    void this.pump();
  }

  /** Running pill: track in-flight calls by their call / result events. */
  private track(scene: ToolScene, part: "call" | "result"): void {
    if (part === "result" || scene.status !== "running") {
      this.inflight.delete(scene.callId);
    } else {
      const known = this.inflight.get(scene.callId);
      this.inflight.set(scene.callId, {
        tool: scene.tool,
        label: runningLabel(scene),
        since: known?.since ?? scene.startedAt ?? Date.now(),
      });
    }
    this.renderRunning();
  }

  private renderRunning(): void {
    const now = Date.now();
    for (const [id, entry] of this.inflight) if (now - entry.since > RUNNING_FORGET_MS) this.inflight.delete(id);
    if (this.inflight.size === 0) {
      clearInterval(this.ticker);
      this.ticker = undefined;
    } else if (!this.ticker) {
      this.ticker = setInterval(() => this.renderRunning(), 250);
    }
    const shown = [...this.inflight.entries()].filter(([, entry]) => now - entry.since >= RUNNING_SHOW_MS);
    const oldest = shown.sort(([, a], [, b]) => a.since - b.since)[0];
    this.runningPill.hidden = !oldest;
    if (oldest) {
      const [callId, entry] = oldest;
      this.runningPill.dataset.tool = entry.tool;
      this.runningPill.dataset.call = callId;
      this.runningPill.textContent = `${entry.label} · ${elapsedLabel(now - entry.since)}${shown.length > 1 ? ` · 另有 ${shown.length - 1} 个` : ""}`;
    } else {
      delete this.runningPill.dataset.tool;
      delete this.runningPill.dataset.call;
    }
    // The HUD / Act window of an in-flight call count up too.
    const hud = this.hudCall ? this.inflight.get(this.hudCall) : undefined;
    this.hudElapsed.hidden = !hud || this.hud.hidden;
    if (hud) this.hudElapsed.textContent = elapsedLabel(now - hud.since);
    const clock = this.actWindow?.querySelector<HTMLElement>(".scene-act-elapsed[data-running]");
    const owner = this.actWindow?.dataset.call ? this.inflight.get(this.actWindow.dataset.call) : undefined;
    if (clock && owner) clock.textContent = elapsedLabel(now - owner.since);
  }

  /** Stop now; Settle (camera back unless `restoreCamera` is false). */
  cancel(restoreCamera = true): void {
    this.token += 1;
    this.queue = [];
    this.running = false;
    this.settle(restoreCamera, false);
  }

  /** The graph re-rendered (class/scope reset): re-apply the current frame. */
  reapply(): void {
    if (!this.session) return;
    if (this.visual) this.paint(this.visual);
    const core = this.host.core();
    if (core && this.clumpFocus.length > 0) this.clump(core, this.clumpFocus, false);
  }

  /**
   * About to re-render / sync the graph: put every moved node back at its
   * original position now, so the sync (and ink-note placement) never sees
   * clumped coordinates. `reapply()` clumps again afterwards.
   */
  beforeGraphSync(): void {
    this.restorePositions(false);
    this.clearGhosts();
  }

  /** Test hook: ids currently displaced by the clump. */
  clumpedIds(): string[] {
    return [...this.origin.keys()];
  }

  private async pump(): Promise<void> {
    if (this.running) return;
    this.running = true;
    const token = this.token;
    clearTimeout(this.settleTimer);
    while (this.queue.length > 0 && token === this.token) {
      if (!this.host.canPlay()) {
        this.cancel();
        return;
      }
      this.catchUp();
      const segment = this.queue.shift()!;
      this.begin();
      const budget = segment.scene.act?.kind === "propose" ? SCENE_PACING.proposeMaxMs : SCENE_PACING.segmentMaxMs;
      this.deadline = Date.now() + budget;
      await this.perform(segment, token);
      this.deadline = Infinity;
    }
    if (token !== this.token) return;
    this.running = false;
    clearTimeout(this.settleTimer);
    this.settleTimer = setTimeout(() => {
      if (token === this.token && this.queue.length === 0) this.settle(true, true);
    }, SCENE_TIMING.settleIdleMs);
  }

  private begin(): void {
    if (this.session) return;
    const core = this.host.core();
    if (!core) return;
    const camera = this.restoring ?? { zoom: core.zoom(), pan: { ...core.pan() } };
    this.restoring = null;
    this.session = {
      zoom: camera.zoom,
      pan: { ...camera.pan },
      panning: core.userPanningEnabled(),
      zooming: core.userZoomingEnabled(),
      ungrabify: core.autoungrabify(),
    };
    core.userPanningEnabled(false);
    core.userZoomingEnabled(false);
    core.autoungrabify(true);
    this.host.canvas.classList.add("is-scene-playing");
    this.onChange?.(true);
  }

  private async perform(segment: Segment, token: number): Promise<void> {
    const { scene, part } = segment;
    this.reduced = this.host.reducedMotion?.() ?? false;
    this.host.hudHost.classList.toggle("is-reduced-motion", this.reduced);
    this.announce(segment);
    this.closeAct();
    this.clearGhosts();
    const act = scene.act;
    if (act?.kind === "propose" && part !== "call") {
      await this.performPropose(scene, act, token);
      return;
    }
    const typed = act?.kind === "typewriter" ? act.text : act?.kind === "locate" ? act.quote : undefined;
    // 目录 / 页图 / 定位 windows need the result; Lean / Loogle also show their running state.
    const windowAct: WindowAct | undefined =
      act && isWindowAct(act) && (part !== "call" || RUNNING_WINDOWS.has(act.kind)) ? act : undefined;
    const parsing = !!scene.act && isParsingRun(scene);
    const running = scene.status === "running" && !!windowAct;
    // Locate quotes are reading content: render complete Markdown/math, never
    // a 60-character or mid-typing source fragment. Search keeps its typewriter.
    if (act?.kind === "locate") {
      this.setTyped(act.quote, true);
      this.hudType.classList.add("is-locate-quote");
      this.hudTypeText.innerHTML = renderBubbleHtml(act.quote);
    } else if (typed === undefined) this.setTyped(null);
    else if (part === "result" || this.reduced) this.setTyped(typed, true);
    else if (!(await this.typewrite(typed, token))) return;
    const core = this.host.core();
    const seeds = scene.seeds;
    const layers = part === "call" ? [] : capLayers(scene.layers, SCENE_PACING.layerCap);
    const all = [...seeds, ...layers.flat()];
    const nodes = core && all.length > 0 ? collect(core, all) : null;
    if (!core || !nodes || nodes.empty()) {
      this.visual = null;
      this.clearPaint();
      this.clumpFocus = [];
      if (core) this.clump(core, [], !this.reduced);
      if (windowAct) {
        await this.showWindow(windowAct, null, scene, token);
        if (running) await this.holdRunning(token);
        else await this.wait(Math.max(SCENE_TIMING.hudOnlyMs, SCENE_TIMING.actMs), token);
        return;
      }
      if (parsing) {
        await this.holdParsing(token);
        return;
      }
      await this.wait(typed !== undefined && part === "call" ? SCENE_TIMING.typeHoldMs : SCENE_TIMING.hudOnlyMs, token);
      return;
    }
    // Isolate the seeds first; each result layer then joins the focus.
    const visual: Visual = { focus: new Set(seeds), seeds, layers: [], ...(parsing ? { parsing } : {}) };
    if (seeds.length > 0 || layers.length === 0) this.paint(visual);
    // Clump and fit move together; the camera aims at where nodes will be.
    // Reduced motion: nothing moves, the camera jumps.
    this.clumpFocus = this.reduced ? [] : nodes.map((node) => node.id());
    const box = this.clump(core, this.clumpFocus, !this.reduced);
    this.fit(core, box ?? nodes.boundingBox({}));
    if (this.reduced) {
      if (!(await this.wait(80, token))) return;
    } else if (seeds.length > 0) {
      if (!(await this.wait(part === "call" ? SCENE_TIMING.seedMs : SCENE_TIMING.layerMs, token))) return;
    } else if (!(await this.wait(Math.max(SCENE_TIMING.fitMs, SCENE_CLUMP.ms) + 40, token))) return;
    // The camera has landed: the window can sit beside its node.
    const shown = windowAct ? this.showWindow(windowAct, scene.anchor ?? null, scene, token) : Promise.resolve(true);
    if (this.reduced && layers.length > 0) {
      visual.layers = [...layers];
      for (const id of layers.flat()) visual.focus.add(id);
      this.paint(visual);
    } else {
      for (const layer of layers) {
        visual.layers = [...visual.layers, layer];
        for (const id of layer) visual.focus.add(id);
        this.paint(visual);
        if (!(await this.wait(SCENE_TIMING.layerMs, token))) return;
      }
    }
    if (!(await shown)) return;
    if (running) await this.holdRunning(token);
    else if (windowAct) await this.wait(SCENE_TIMING.actMs, token);
    else if (parsing) await this.holdParsing(token);
    else if (layers.length > 0) await this.wait(SCENE_TIMING.holdMs, token);
  }

  /** Open an Act window and reveal its rows one by one (all at once when reduced). */
  private async showWindow(act: WindowAct, anchor: string | null, scene: ToolScene, token: number): Promise<boolean> {
    const win = this.openAct(act, anchor, scene);
    const rows = [...win.querySelectorAll<HTMLElement>(".is-pending")];
    if (this.reduced) {
      rows.forEach((row) => row.classList.remove("is-pending"));
      return true;
    }
    for (const row of rows) {
      if (!(await this.wait(SCENE_TIMING.rowMs, token))) return false;
      row.classList.remove("is-pending");
    }
    return true;
  }

  /** A running Lean / Loogle window: hold until the next segment (or the hold ends). */
  private async holdRunning(token: number): Promise<void> {
    const until = Date.now() + SCENE_TIMING.runningHoldMs;
    while (Date.now() < until && this.queue.length === 0) {
      if (!(await this.wait(Math.min(200, until - Date.now()), token))) return;
    }
  }

  // -------------------------------------------------------------------------

  // propose_changes on the main map

  private async performPropose(scene: ToolScene, act: ProposeAct, token: number): Promise<void> {
    this.setTyped(null);
    const core = this.host.core();
    this.setCounts(act);
    if (!core) {
      await this.wait(SCENE_TIMING.hudOnlyMs, token);
      return;
    }
    const reduced = this.reduced;
    const exists = (id: string | undefined): id is string => !!id && core.getElementById(id).nonempty();
    // Nothing moves in a proposal scene: send earlier clumps home.
    this.clumpFocus = [];
    this.clump(core, [], !reduced);
    const anchors = scene.seeds.filter(exists);
    const retired = (act.retiredNodes ?? []).filter(exists);
    // Real staged nodes (staged overlay) are animated as they are; the rest get ghosts.
    const items = act.items.map((item) => ({ item, id: exists(item.node) ? item.node : ghostId(item.localRef), ghost: !exists(item.node) }));
    const spots = ghostSpots(core, items.filter((entry) => entry.ghost).map((entry) => entry.item), [
      ...anchors,
      ...items.filter((entry) => !entry.ghost).map((entry) => entry.id),
    ]);
    const nodeOf = (end: ProposeAct["relations"][number]["source"]): string | undefined => {
      if ("local" in end) return items.find((entry) => entry.item.localRef === end.local)?.id;
      return exists(end.node) ? end.node : undefined;
    };
    const relations = act.relations.map((relation) => ({
      relation,
      source: nodeOf(relation.source),
      target: nodeOf(relation.target),
      real: relation.stagedId ? relationElements(core, relation.stagedId) : core.collection(),
      targetRelation: "id" in relation.target ? relationElements(core, relation.target.id) : core.collection(),
    }));
    const marks = new Map<string, string[]>();
    const mark = (id: string, cls: string) => marks.set(id, [...(marks.get(id) ?? []), cls]);
    const visual: Visual = { focus: new Set([...anchors, ...retired]), seeds: anchors, layers: [], marks };
    for (const id of retired) mark(id, "scene-retired");
    for (const entry of relations) {
      for (const id of [entry.source, entry.target]) if (id && !id.startsWith("scene-ghost")) visual.focus.add(id);
      entry.targetRelation.connectedNodes().forEach((node) => { visual.focus.add(node.id()); });
    }
    for (const entry of items) if (!entry.ghost) visual.focus.add(entry.id);
    this.paint(visual);
    // The camera frames anchors, staged nodes and where the ghosts will appear.
    const box = boxOf(core, [...visual.focus], [...spots.values()]);
    if (box) this.fit(core, box);
    if (!(await this.wait(reduced ? 80 : SCENE_TIMING.fitMs + 80, token))) return;
    if (retired.length > 0 && !reduced && !(await this.wait(SCENE_TIMING.layerMs, token))) return;
    // Units appear one by one (capped), then relations draw in one after another.
    const itemStep = reduced ? 0 : SCENE_TIMING.proposeItemMs;
    for (const [index, entry] of items.entries()) {
      if (entry.ghost) {
        const at = spots.get(entry.item.localRef);
        core.add({
          group: "nodes",
          data: { id: entry.id, label: entry.item.label, elementKind: "scene-ghost", localRef: entry.item.localRef, visualKind: entry.item.visualKind ?? null },
          position: at ?? { x: 0, y: 0 },
          classes: proposeGhostClass(entry.item),
        });
      } else {
        mark(entry.id, "scene-propose-hit");
        const semanticMark = proposeItemMarkClass(entry.item);
        if (semanticMark) mark(entry.id, semanticMark);
      }
      visual.focus.add(entry.id);
      if (entry.item.blocked.length > 0) mark(entry.id, "scene-blocked");
      this.paint(visual);
      if (itemStep && index < PROPOSE_STEP_CAP && !(await this.wait(itemStep, token))) return;
    }
    const relationStep = reduced ? 0 : SCENE_TIMING.proposeRelationMs;
    for (const [index, entry] of relations.entries()) {
      const blocked = entry.relation.blocked.length > 0;
      if (entry.real.nonempty()) {
        const semanticClass = proposeRelationMarkClass(entry.relation);
        entry.real.forEach((element) => {
          mark(element.id(), "scene-propose-edge");
          if (semanticClass) mark(element.id(), semanticClass);
          if (blocked) mark(element.id(), "scene-blocked");
          if (element.isNode()) visual.focus.add(element.id());
        });
      } else if (entry.source && entry.target) {
        const id = `scene-ghost-edge:${entry.relation.localRef}`;
        core.add({
          group: "edges",
          data: { id, source: entry.source, target: entry.target, role: entry.relation.label, label: entry.relation.label, relationKind: entry.relation.relationKind ?? null },
          classes: proposeRelationGhostClass(entry.relation),
        });
        if (blocked) mark(id, "scene-blocked");
      } else if (entry.source && entry.targetRelation.nonempty()) {
        // A Debt attached to an existing Alignment has no node endpoint in semantic mode;
        // pulse the holder edge while the debt node is shown separately.
        const semanticClass = proposeRelationMarkClass(entry.relation);
        entry.targetRelation.forEach((element) => {
          mark(element.id(), "scene-propose-edge");
          if (semanticClass) mark(element.id(), semanticClass);
          if (blocked) mark(element.id(), "scene-blocked");
        });
        if (blocked) mark(entry.source, "scene-blocked");
      } else {
        continue;
      }
      this.paint(visual);
      if (relationStep && index < PROPOSE_STEP_CAP && !(await this.wait(relationStep, token))) return;
    }
    // Blocked: flash red with the diagnostic code (on the ghost label and in the window).
    const blockedIds = [...marks].filter(([, classes]) => classes.includes("scene-blocked")).map(([id]) => id);
    for (const id of blockedIds) {
      const node = core.getElementById(id);
      const entry = items.find((candidate) => candidate.id === id);
      if (node.nonempty() && node.isNode() && entry?.ghost) node.data("label", `⚠ ${entry.item.blocked[0].code} · ${entry.item.label}`);
    }
    if (act.blocks.length > 0) this.openAct(act, null, scene);
    if (blockedIds.length > 0 && !reduced) {
      let flash = core.collection();
      for (const id of blockedIds) flash = flash.union(core.getElementById(id));
      for (let round = 0; round < SCENE_TIMING.flashes * 2; round += 1) {
        flash.toggleClass("scene-blocked-flash", round % 2 === 0);
        if (!(await this.wait(SCENE_TIMING.flashMs, token))) return;
      }
      flash.removeClass("scene-blocked-flash");
    }
    await this.wait(SCENE_TIMING.holdMs + (act.blocks.length > 0 ? SCENE_TIMING.actMs : 0), token);
  }

  /** HUD data of a proposal: the `staged_ids` counts (= review overview totals). */
  private setCounts(act: ProposeAct | null): void {
    if (!act) {
      delete this.hud.dataset.units;
      delete this.hud.dataset.relations;
      delete this.hud.dataset.spans;
      delete this.hud.dataset.semantic;
      delete this.hud.dataset.blocked;
      return;
    }
    this.hud.dataset.units = String(act.counts.units);
    this.hud.dataset.relations = String(act.counts.relations);
    this.hud.dataset.spans = String(act.counts.spans);
    this.hud.dataset.semantic = String(act.counts.semantic);
    this.hud.dataset.blocked = String(act.blocks.length);
  }

  /** Remove transient ghost nodes / edges (proposal scenes). */
  private clearGhosts(): void {
    const core = this.host.core();
    if (!core) return;
    const ghosts = core.elements(GHOST_SELECTOR);
    if (ghosts.nonempty()) core.remove(ghosts);
  }

  /** Test hook: ids of the ghost elements on the map right now. */
  ghostIds(): string[] {
    return this.host.core()?.elements(GHOST_SELECTOR).map((element) => element.id()) ?? [];
  }

  /** Hold a 「解析中」 frame until the next segment arrives (or the hold ends). */
  private async holdParsing(token: number): Promise<void> {
    const until = Date.now() + SCENE_TIMING.parsingHoldMs;
    while (Date.now() < until && this.queue.length === 0) {
      if (!(await this.wait(Math.min(200, until - Date.now()), token))) return;
    }
  }

  /** HUD typewriter line: `null` hides it; `done` shows the text at once. */
  private setTyped(text: string | null, done = false): void {
    this.hudType.hidden = text === null;
    this.hudType.classList.remove("is-locate-quote");
    this.hudTypeText.textContent = text ?? "";
    this.hudType.classList.toggle("is-typing", text !== null && !done);
    if (text !== null) this.hudType.dataset.full = text;
  }

  /** Type `text` character by character; false when cancelled/replaced. */
  private async typewrite(text: string, token: number): Promise<boolean> {
    const chars = [...text];
    const step = clamp(SCENE_TIMING.typeTotalMs / Math.max(chars.length, 1), SCENE_TIMING.typeCharMinMs, SCENE_TIMING.typeCharMaxMs);
    const perTick = Math.max(1, Math.ceil((chars.length * step) / SCENE_TIMING.typeTotalMs));
    this.setTyped("");
    this.hudType.dataset.full = text;
    for (let at = 0; at < chars.length; at += perTick) {
      this.hudTypeText.textContent = chars.slice(0, at + perTick).join("");
      if (!(await this.wait(step, token))) return false;
    }
    this.setTyped(text, true);
    return true;
  }

  /** Open the Act mini-window beside `anchor` (else the stage corner). */
  private openAct(act: WindowAct | ProposeAct, anchor: string | null, scene?: ToolScene): HTMLElement {
    this.closeAct();
    const win = document.createElement("section");
    win.className = "scene-act";
    win.dataset.act = act.kind;
    if (scene) win.dataset.call = scene.callId;
    win.setAttribute("role", "dialog");
    win.setAttribute("aria-label", ACT_LABEL[act.kind]);
    const elapsed = scene && scene.status !== "running" ? scene.durationMs : undefined;
    if (act.kind === "outline") fillOutline(win, act);
    else if (act.kind === "page_image") fillPageImage(win, act);
    else if (act.kind === "mathlib") fillMathlib(win, act, elapsed);
    else if (act.kind === "lean") fillLean(win, act, elapsed);
    else if (act.kind === "locate") fillLocate(win, act);
    else fillBlocked(win, act);
    this.host.hudHost.append(win);
    this.actWindow = win;
    if (act.kind === "locate") {
      // Long reading content must scroll below the replay controls, not cover
      // them. Measure before placement so the bounded height participates.
      const top = Math.max(ACT_MARGIN, this.hud.getBoundingClientRect().bottom - this.host.hudHost.getBoundingClientRect().top + ACT_MARGIN);
      win.style.setProperty("--scene-locate-top", `${top}px`);
    }
    if (!this.placeBeside(win, anchor)) win.classList.add("is-stage");
    this.renderRunning();
    return win;
  }

  private placeBeside(win: HTMLElement, anchor: string | null): boolean {
    const core = this.host.core();
    if (!core || !anchor) return false;
    const node = core.getElementById(anchor);
    if (node.empty() || !node.isNode()) return false;
    const hostRect = this.host.hudHost.getBoundingClientRect();
    const canvasRect = this.host.canvas.getBoundingClientRect();
    const pos = node.renderedPosition();
    const half = node.renderedOuterWidth() / 2;
    const x = canvasRect.left - hostRect.left + pos.x;
    const y = canvasRect.top - hostRect.top + pos.y;
    const width = win.offsetWidth;
    const height = win.offsetHeight;
    const hostWidth = this.host.hudHost.clientWidth;
    const hostHeight = this.host.hudHost.clientHeight;
    const minTop = Number.parseFloat(win.style.getPropertyValue("--scene-locate-top")) || ACT_MARGIN;
    if (hostWidth <= 0 || hostHeight <= 0) return false;
    const halfH = node.renderedOuterHeight() / 2;
    // Beside the node: right, left, below, above — the first that fits whole;
    // else the right side clamped into the stage.
    const candidates = [
      { left: x + half + ACT_GAP, top: y - height / 2 },
      { left: x - half - ACT_GAP - width, top: y - height / 2 },
      { left: x - width / 2, top: y + halfH + ACT_GAP },
      { left: x - width / 2, top: y - halfH - ACT_GAP - height },
    ];
    const fits = (c: { left: number; top: number }) =>
      c.left >= ACT_MARGIN && c.top >= minTop && c.left + width <= hostWidth - ACT_MARGIN && c.top + height <= hostHeight - ACT_MARGIN;
    const spot = candidates.find(fits) ?? candidates[0];
    const left = clamp(spot.left, ACT_MARGIN, Math.max(ACT_MARGIN, hostWidth - width - ACT_MARGIN));
    const top = clamp(spot.top, minTop, Math.max(minTop, hostHeight - height - ACT_MARGIN));
    win.style.left = `${Math.round(left)}px`;
    win.style.top = `${Math.round(top)}px`;
    // Technical details can grow after placement; keep their lower edge inside
    // the stage as well, without changing any other tool's window layout.
    if (win.dataset.act === "locate") win.style.setProperty("--scene-locate-top", `${Math.round(top)}px`);
    win.classList.add("is-anchored");
    win.dataset.anchor = anchor;
    return true;
  }

  private closeAct(): void {
    this.actWindow?.remove();
    this.actWindow = null;
  }

  private announce(segment: Segment): void {
    const { scene } = segment;
    this.hud.hidden = false;
    this.hud.dataset.tool = scene.tool;
    this.hud.dataset.status = scene.status;
    this.hud.classList.toggle("is-error", scene.status === "error");
    this.hudTitle.textContent = scene.title;
    this.hudSummary.textContent = scene.summary;
    // Replay: `i/N`. Live pacing: merged / skipped segments say so.
    const merged = scene.coalesced ? `合并 ${scene.coalesced + 1} 次` : "";
    const skipped = segment.skipped ? `跳过 ${segment.skipped} 步` : "";
    this.hudStep.textContent = segment.total ? `${segment.step}/${segment.total}` : [merged, skipped].filter(Boolean).join(" · ");
    this.hudStep.hidden = !this.hudStep.textContent;
    if (merged || skipped) this.hudStep.dataset.pacing = skipped ? "skip" : "merge";
    else delete this.hudStep.dataset.pacing;
    this.hudCall = scene.callId;
    this.setCounts(scene.act?.kind === "propose" && segment.part !== "call" ? scene.act : null);
    if (scene.act && scene.act.kind !== "typewriter" && scene.act.kind !== "parsing") this.hud.dataset.act = scene.act.kind;
    const parsing = isParsingRun(scene);
    this.hudParsing.hidden = !parsing;
    this.hud.classList.toggle("is-parsing", parsing);
    if (parsing) this.hud.dataset.act = "parsing";
    else if (!scene.act || scene.act.kind === "typewriter" || scene.act.kind === "parsing") delete this.hud.dataset.act;
    this.renderRunning();
  }

  /** Dim out-of-focus elements, mark seeds / layers / edges between them. */
  private paint(visual: Visual): void {
    const core = this.host.core();
    if (!core) return;
    this.visual = visual;
    core.batch(() => {
      core.elements().removeClass(SCENE_CLASSES);
      applyFocusScope(core, visual.focus);
      const hit = new Set(visual.layers.flat());
      collect(core, visual.seeds.filter((id) => !hit.has(id))).addClass("sandbox-echo-seed");
      visual.layers.forEach((layer, index) => {
        const nodes = collect(core, layer).addClass("sandbox-echo-hit");
        if (index < visual.layers.length - 1) nodes.addClass("sandbox-echo-fade");
      });
      if (visual.parsing) collect(core, visual.seeds).addClass("scene-parsing");
      // Edges (and semantic-mode junction edges) whose ends are both in focus.
      core.edges().not(`.${OUT_OF_FOCUS_CLASS}`).not(".scene-ghost-edge").addClass("sandbox-echo-edge");
      for (const [id, classes] of visual.marks ?? []) {
        const element = core.getElementById(id);
        if (element.nonempty()) element.addClass(classes.join(" "));
      }
    });
  }

  private clearPaint(): void {
    const core = this.host.core();
    if (!core) return;
    core.elements().removeClass(SCENE_CLASSES);
    this.host.restoreFocusScope();
  }

  /** Fit the focus with padding; never zoom past FIT_MAX_ZOOM (single node). */
  private fit(core: Core, box: BoundingBox12 & { w: number; h: number }): void {
    const width = core.width();
    const height = core.height();
    if (width <= 0 || height <= 0 || !Number.isFinite(box.w)) return;
    const pad = Math.min(FIT_PADDING, width / 4, height / 4);
    const zoomFit = Math.min((width - 2 * pad) / Math.max(box.w, 1), (height - 2 * pad) / Math.max(box.h, 1));
    const zoom = clamp(Math.min(zoomFit, FIT_MAX_ZOOM), core.minZoom(), core.maxZoom());
    const cx = (box.x1 + box.x2) / 2;
    const cy = (box.y1 + box.y2) / 2;
    core.stop(true, false);
    const pan = { x: width / 2 - zoom * cx, y: height / 2 - zoom * cy };
    if (this.reduced) core.viewport({ zoom, pan });
    else core.animate({ zoom, pan }, { duration: SCENE_TIMING.fitMs });
  }

  /**
   * Clump `focusIds` (from their original positions) and send every other
   * node moved earlier this session home, in one tween (or at once).
   * Returns the focus bounding box at the target positions, for the fit.
   */
  private clump(core: Core, focusIds: readonly string[], animate: boolean): (BoundingBox12 & { w: number; h: number }) | null {
    const focus = collect(core, focusIds).filter((node) => !node.is(SATELLITE_SELECTOR));
    const targets = new Map<string, Point>();
    if (SCENE_CLUMP.enabled && focus.length >= 2) {
      const origin = new Map<string, Point>();
      focus.forEach((node) => {
        origin.set(node.id(), this.originOf(node));
      });
      for (const [id, to] of clumpTargets(origin)) targets.set(id, to);
      // Satellites follow the average shift of their neighbours.
      const shift = (node: NodeSingular): Point | null => {
        const to = targets.get(node.id());
        if (!to) return null;
        const from = origin.get(node.id())!;
        return { x: to.x - from.x, y: to.y - from.y };
      };
      if (targets.size > 0) {
        core.nodes(SATELLITE_SELECTOR).forEach((sat) => {
          const neighbours = sat.neighborhood().nodes().not(SATELLITE_SELECTOR).toArray() as NodeSingular[];
          if (!neighbours.some((n) => targets.has(n.id()))) return;
          const d = satelliteShift(neighbours.map(shift));
          const from = this.originOf(sat);
          targets.set(sat.id(), { x: from.x + d.x, y: from.y + d.y });
        });
      }
    }
    // Everything moved before and not targeted now goes back home.
    for (const [id, home] of this.origin) if (!targets.has(id)) targets.set(id, home);
    if (targets.size === 0) {
      this.origin.clear();
      return null;
    }
    let box: (BoundingBox12 & { w: number; h: number }) | null = null;
    focus.forEach((node) => {
      const to = targets.get(node.id());
      const at = node.position();
      const b = node.boundingBox({});
      const dx = to ? to.x - at.x : 0;
      const dy = to ? to.y - at.y : 0;
      const x1 = b.x1 + dx, x2 = b.x2 + dx, y1 = b.y1 + dy, y2 = b.y2 + dy;
      box = box
        ? { x1: Math.min(box.x1, x1), y1: Math.min(box.y1, y1), x2: Math.max(box.x2, x2), y2: Math.max(box.y2, y2), w: 0, h: 0 }
        : { x1, y1, x2, y2, w: 0, h: 0 };
    });
    if (box) {
      const b = box as BoundingBox12 & { w: number; h: number };
      b.w = b.x2 - b.x1;
      b.h = b.y2 - b.y1;
    }
    this.moveTo(core, targets, animate, () => {
      // Nodes that are back home need no snapshot any more.
      for (const [id, home] of [...this.origin]) {
        const node = core.getElementById(id);
        if (node.empty()) {
          this.origin.delete(id);
          continue;
        }
        const at = node.position();
        if (Math.abs(at.x - home.x) < 0.5 && Math.abs(at.y - home.y) < 0.5) this.origin.delete(id);
      }
    });
    return box;
  }

  /** Original position of `node` (snapshotted on first move). */
  private originOf(node: NodeSingular): Point {
    let home = this.origin.get(node.id());
    if (!home) {
      const at = node.position();
      home = { x: at.x, y: at.y };
      this.origin.set(node.id(), home);
    }
    return home;
  }

  /** Every moved node back to its snapshot (tweened or at once). */
  private restorePositions(animate: boolean): void {
    const core = this.host.core();
    if (!core || this.origin.size === 0) {
      this.stopTween();
      this.origin.clear();
      return;
    }
    const targets = new Map(this.origin);
    this.moveTo(core, targets, animate, () => {
      for (const [id, home] of targets) if (this.origin.get(id) === home) this.origin.delete(id);
    });
  }

  private stopTween(): void {
    if (!this.tween) return;
    cancelAnimationFrame(this.tween.frame);
    this.tween = null;
  }

  /** Tween nodes to `targets`; a new tween replaces the running one. */
  private moveTo(core: Core, targets: ReadonlyMap<string, Point>, animate: boolean, done: () => void): void {
    this.stopTween();
    const moves: { node: NodeSingular; from: Point; to: Point }[] = [];
    for (const [id, to] of targets) {
      const node = core.getElementById(id);
      if (node.empty() || !node.isNode()) continue;
      const at = node.position();
      moves.push({ node, from: { x: at.x, y: at.y }, to });
    }
    const place = (t: number) => {
      core.batch(() => {
        for (const move of moves) {
          if (move.node.removed() || move.node.grabbed()) continue;
          move.node.position(t >= 1 ? { ...move.to } : lerp(move.from, move.to, ease(t)));
        }
      });
    };
    if (!animate || SCENE_CLUMP.ms <= 0) {
      place(1);
      done();
      return;
    }
    const start = performance.now();
    const tween = { frame: 0, done };
    const step = (now: number) => {
      if (this.tween !== tween) return;
      const t = (now - start) / SCENE_CLUMP.ms;
      place(t);
      if (t >= 1) {
        this.tween = null;
        done();
        return;
      }
      tween.frame = requestAnimationFrame(step);
    };
    this.tween = tween;
    tween.frame = requestAnimationFrame(step);
  }

  private settle(restoreCamera: boolean, animate: boolean): void {
    clearTimeout(this.settleTimer);
    const session = this.session;
    this.session = null;
    this.visual = null;
    this.hud.hidden = true;
    this.hudParsing.hidden = true;
    this.hudElapsed.hidden = true;
    this.hudCall = null;
    this.setCounts(null);
    this.hud.classList.remove("is-parsing");
    this.setTyped(null);
    this.closeAct();
    this.clearGhosts();
    this.host.canvas.classList.remove("is-scene-playing");
    this.clumpFocus = [];
    const reduced = this.host.reducedMotion?.() ?? false;
    if (reduced) animate = false;
    // Nodes glide home (instantly when a jump follows or motion is reduced).
    this.restorePositions(restoreCamera && !reduced);
    const core = this.host.core();
    if (!core) return;
    core.elements().removeClass(SCENE_CLASSES);
    this.host.restoreFocusScope();
    if (!session) return;
    core.userPanningEnabled(session.panning);
    core.userZoomingEnabled(session.zooming);
    core.autoungrabify(session.ungrabify);
    if (restoreCamera) {
      core.stop(true, false);
      if (animate) {
        const target = { zoom: session.zoom, pan: { ...session.pan } };
        this.restoring = target;
        core.animate(target, {
          duration: SCENE_TIMING.settleMs,
          complete: () => {
            if (this.restoring === target) this.restoring = null;
          },
        });
      } else {
        this.restoring = null;
        core.viewport({ zoom: session.zoom, pan: session.pan });
      }
    }
    this.onChange?.(false);
  }

  /** Sleep (cut to the segment's time budget); false when cancelled/replaced. */
  private wait(ms: number, token: number): Promise<boolean> {
    const capped = Math.max(0, Math.min(ms, this.deadline - Date.now()));
    return new Promise((resolve) => setTimeout(() => resolve(token === this.token), capped));
  }
}

function element<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function actHead(win: HTMLElement, title: string, meta?: string): void {
  const head = element("header", "scene-act-head");
  head.append(element("strong", "scene-act-title", title));
  if (meta) head.append(element("span", "scene-act-meta", meta));
  win.append(head);
}

/** 目录 window: indented rows, 1-based pages, empty / truncated notes. */
function fillOutline(win: HTMLElement, act: OutlineAct): void {
  const meta = [act.rows.length > 0 ? `${act.total} 条` : "", act.totalPages !== undefined ? `${act.totalPages} 页` : ""]
    .filter(Boolean)
    .join(" · ");
  actHead(win, "PDF 目录", meta || undefined);
  if (act.rows.length === 0) {
    win.append(
      element(
        "p",
        "scene-act-empty",
        act.empty === "unreadable" ? "目录无法读取（书签树损坏或过大）" : "这份 PDF 没有书签目录",
      ),
    );
    return;
  }
  const list = element("ol", "scene-act-outline");
  for (const row of act.rows) {
    const item = element("li", "scene-act-row");
    item.dataset.level = String(row.level);
    item.style.paddingLeft = `${(row.level - 1) * 12 + 4}px`;
    item.append(element("span", "scene-act-row-title", row.title));
    if (row.page !== undefined) item.append(element("span", "scene-act-row-page", `第 ${row.page} 页`));
    list.append(item);
  }
  win.append(list);
  if (act.truncated || act.total > act.rows.length) {
    win.append(element("p", "scene-act-foot", `仅显示前 ${act.rows.length} 条，共 ${act.total} 条`));
  }
}

/** 页图 window: the PNG when returned, else text meta only. */
function fillPageImage(win: HTMLElement, act: PageImageAct): void {
  actHead(win, act.noteTitle ?? "页图", act.pageLabel);
  const empty = () => element("p", "scene-act-empty", "未取得页图（仅元数据）");
  if (!act.src) {
    win.append(empty());
    return;
  }
  const img = element("img", "scene-act-image");
  img.alt = [act.noteTitle ?? "页图", act.pageLabel].filter(Boolean).join(" · ");
  if (act.width && act.height) {
    img.width = act.width;
    img.height = act.height;
  }
  img.onerror = () => img.replaceWith(empty());
  img.src = act.src;
  win.append(img);
}

const ACT_LABEL: Record<WindowAct["kind"] | "propose", string> = {
  outline: "PDF 目录",
  page_image: "手稿页图",
  mathlib: "Mathlib 检索",
  lean: "Lean 检查",
  locate: "原文定位",
  propose: "提案诊断",
};

const isWindowAct = (act: SceneAct): act is WindowAct =>
  act.kind === "outline" || act.kind === "page_image" || act.kind === "mathlib" || act.kind === "lean" || act.kind === "locate";

function badge(className: string, text: string, data?: Record<string, string>): HTMLElement {
  const node = element("span", `scene-act-badge ${className}`, text);
  for (const [key, value] of Object.entries(data ?? {})) node.dataset[key] = value;
  return node;
}

/** Running row with a live clock (the player's ticker fills it). */
function runningRow(label: string): HTMLElement {
  const row = element("p", "scene-act-running");
  row.append(element("span", "scene-act-spinner"), element("span", "scene-act-running-label", label));
  const clock = element("span", "scene-act-elapsed", "");
  clock.dataset.running = "true";
  row.append(clock);
  return row;
}

function elapsedRow(ms: number | undefined): HTMLElement | null {
  return ms === undefined ? null : element("p", "scene-act-foot scene-act-took", `用时 ${elapsedLabel(ms)}`);
}

/** Loogle window: query, backend badge, candidates (revealed one by one). */
function fillMathlib(win: HTMLElement, act: MathlibAct, elapsed?: number): void {
  actHead(win, "Mathlib · Loogle", act.total !== undefined ? `${act.total} 个候选` : undefined);
  const meta = element("div", "scene-act-badges");
  if (act.mode) meta.append(badge("scene-act-mode", act.mode === "resident" ? "常驻" : "一次性", { mode: act.mode }));
  if (act.fallback) {
    const flag = badge("scene-act-fallback", "已回退", { fallback: act.fallback });
    flag.title = act.fallback;
    meta.append(flag);
  }
  if (meta.childElementCount > 0) win.append(meta);
  win.append(element("p", "scene-act-query", act.query || "（空查询）"));
  if (act.phase === "running") {
    win.append(runningRow("Loogle 启动中…"));
    return;
  }
  if (act.error) {
    win.append(element("p", "scene-act-error", act.error));
  } else if (act.noMatches) {
    win.append(element("p", "scene-act-empty", "没有匹配的声明"));
  } else {
    const list = element("ol", "scene-act-candidates");
    for (const candidate of act.candidates) {
      const row = element("li", "scene-act-candidate is-pending");
      row.append(element("code", "scene-act-candidate-name", candidate.name));
      if (candidate.type) row.append(element("span", "scene-act-candidate-type", candidate.type));
      list.append(row);
    }
    win.append(list);
    if (act.truncated) win.append(element("p", "scene-act-foot", `仅显示前 ${act.candidates.length} 个`));
  }
  if (act.suggestions.length > 0) {
    const tips = element("p", "scene-act-suggestions", "你可能想找：");
    for (const tip of act.suggestions) tips.append(element("code", "scene-act-suggestion", tip));
    win.append(tips);
  }
  if (act.fallback) win.append(element("p", "scene-act-foot scene-act-fallback-note", `常驻 Loogle 未使用：${act.fallback}`));
  const took = elapsedRow(elapsed);
  if (took) win.append(took);
}

/** Lean window: the snippet, running clock, then ✓ or the diagnostics. */
function fillLean(win: HTMLElement, act: LeanAct, elapsed?: number): void {
  actHead(win, "Lean 检查", act.codeLines ? `${act.codeLines} 行` : undefined);
  if (act.code) {
    const pre = element("pre", "scene-act-code");
    pre.textContent = act.code + (act.codeLines > act.code.split("\n").length ? "\n…" : "");
    win.append(pre);
  }
  if (act.phase === "running") {
    win.append(runningRow("Lean 编译中…"));
    return;
  }
  if (act.phase === "error") {
    win.append(element("p", "scene-act-error", act.error ?? "调用失败"));
  } else if (act.phase === "checked") {
    const verdict = element("p", "scene-act-verdict", "✓ 编译通过");
    verdict.dataset.verdict = "checked";
    win.append(verdict, element("p", "scene-act-note", "编译通过 ≠ 证明忠实：还需核对陈述与原文一致"));
  } else {
    const verdict = element("p", "scene-act-verdict", act.diagnostics.length ? `✗ ${act.diagnostics.length} 条诊断` : "✗ 未通过");
    verdict.dataset.verdict = "failed";
    win.append(verdict);
    const list = element("ul", "scene-act-diagnostics");
    for (const item of act.diagnostics) {
      const row = element("li", "scene-act-diagnostic is-pending");
      row.dataset.severity = item.severity;
      const where = item.line !== undefined ? `${item.line}:${item.column ?? 0}` : "";
      if (where) row.append(element("span", "scene-act-diagnostic-where", where));
      row.append(element("span", "scene-act-diagnostic-message", item.message));
      list.append(row);
    }
    win.append(list);
    if (act.diagnosticsTruncated) win.append(element("p", "scene-act-foot", "诊断已截断"));
  }
  const took = elapsedRow(act.elapsedMs ?? elapsed);
  if (took) win.append(took);
}

/** Locate window: the quote (highlighted when it matched), page, or why not. */
function fillLocate(win: HTMLElement, act: LocateAct): void {
  actHead(win, "原文定位", act.page !== undefined ? `第 ${act.page} 页` : undefined);
  const quote = element("blockquote", "scene-act-quote scene-locate-quote");
  // Keep complete Markdown/math blocks: clipping source can cut a TeX delimiter.
  // The bounded preview scrolls instead; raw HTML and external links stay inert.
  quote.innerHTML = renderBubbleHtml(act.quote);
  if (act.phase === "ok") quote.classList.add("scene-act-mark");
  quote.dataset.phase = act.phase;
  win.append(quote);
  if (act.phase === "ok") {
    const verdict = element("p", "scene-act-verdict");
    verdict.append(element("span", "scene-locate-match", act.match ?? t("scene.locate.located")), " · ",
      element("span", "scene-locate-page", act.page !== undefined ? t("scene.locate.page", { page: act.page }) : t("scene.locate.pageUnknown")));
    verdict.dataset.verdict = "ok";
    win.append(verdict);
  } else if (act.phase !== "running") {
    const verdict = element("p", "scene-act-reason", act.reason ?? "未定位");
    verdict.dataset.verdict = act.phase;
    win.append(verdict);
  }
  if (act.diagnostics?.length) {
    const list = element("ul", "scene-locate-diagnostics");
    const labels = {
      page: "scene.locate.category.page", match: "scene.locate.category.match", source: "scene.locate.category.source",
      candidates: "scene.locate.category.candidates", other: "scene.locate.category.other",
    } as const;
    for (const diagnostic of act.diagnostics) {
      const item = element("li", "scene-locate-diagnostic");
      item.dataset.category = diagnostic.category;
      item.append(element("span", "scene-locate-category", t(labels[diagnostic.category])),
        element("span", "scene-locate-message", diagnostic.message));
      list.append(item);
    }
    win.append(list);
  }
  if (act.artifactId || act.diagnostics?.length) {
    const details = document.createElement("details");
    details.className = "scene-locate-details";
    details.append(element("summary", "", t("scene.locate.technicalDetails")));
    if (act.artifactId) details.append(element("p", "scene-act-note", t("scene.locate.snapshotId")), element("code", "scene-locate-artifact", act.artifactId));
    for (const diagnostic of act.diagnostics ?? []) {
      details.append(element("code", "scene-locate-code", diagnostic.code), element("p", "scene-locate-original", diagnostic.original));
    }
    win.append(details);
  }
}

/** Blocked proposal: its diagnostics (code + message). */
function fillBlocked(win: HTMLElement, act: ProposeAct): void {
  actHead(win, "提案被阻塞", `${act.blocks.length} 个问题`);
  const list = element("ul", "scene-act-diagnostics");
  for (const block of act.blocks.slice(0, 8)) {
    const row = element("li", "scene-act-diagnostic");
    row.dataset.severity = "error";
    row.append(element("code", "scene-act-diagnostic-code", block.code));
    if (block.localRef) row.append(element("span", "scene-act-diagnostic-where", block.localRef));
    row.append(element("span", "scene-act-diagnostic-message", block.message));
    list.append(row);
  }
  win.append(list);
  if (act.blocks.length > 8) win.append(element("p", "scene-act-foot", `另有 ${act.blocks.length - 8} 个问题`));
}

const ghostId = (localRef: string): string => `scene-ghost:${localRef}`;

/** Real map elements of a staged relation: its edges or its junction (and the junction's edges). */
function relationElements(core: Core, relationId: string) {
  const own = core.elements().filter((element) => element.data("relationId") === relationId || element.data("domainId") === relationId);
  return own.union(own.nodes().connectedEdges());
}

/**
 * Where each ghost appears: a golden-angle spiral around its anchors'
 * centroid; anchorless ghosts spiral around the scene's known nodes (else
 * the viewport centre).
 */
function ghostSpots(core: Core, items: readonly ProposeAct["items"][number][], known: readonly string[]): Map<string, Point> {
  const spots = new Map<string, Point>();
  const centroid = (ids: readonly string[]): Point | null => {
    const nodes = collect(core, ids);
    if (nodes.empty()) return null;
    let x = 0;
    let y = 0;
    nodes.forEach((node) => {
      x += node.position("x");
      y += node.position("y");
    });
    return { x: x / nodes.length, y: y / nodes.length };
  };
  const pan = core.pan();
  const zoom = core.zoom() || 1;
  const fallback = centroid(known) ?? { x: (core.width() / 2 - pan.x) / zoom, y: (core.height() / 2 - pan.y) / zoom };
  const groups = new Map<string, { center: Point; count: number }>();
  for (const item of items) {
    const anchors = item.anchorNodes ?? [];
    const key = anchors.length > 0 ? [...anchors].sort().join("|") : "";
    let group = groups.get(key);
    if (!group) {
      group = { center: (anchors.length > 0 ? centroid(anchors) : null) ?? fallback, count: 0 };
      groups.set(key, group);
    }
    const index = group.count;
    group.count += 1;
    const angle = index * 2.39996 - Math.PI / 2;
    const radius = 96 + 30 * Math.sqrt(index);
    spots.set(item.localRef, { x: group.center.x + radius * Math.cos(angle), y: group.center.y + radius * Math.sin(angle) });
  }
  return spots;
}

/** Bounding box of `ids` plus ghost spots (model coordinates). */
function boxOf(core: Core, ids: readonly string[], points: readonly Point[]): (BoundingBox12 & { w: number; h: number }) | null {
  let x1 = Infinity;
  let y1 = Infinity;
  let x2 = -Infinity;
  let y2 = -Infinity;
  const nodes = collect(core, ids);
  if (nodes.nonempty()) {
    const b = nodes.boundingBox({});
    x1 = b.x1;
    y1 = b.y1;
    x2 = b.x2;
    y2 = b.y2;
  }
  for (const point of points) {
    x1 = Math.min(x1, point.x - GHOST_SIZE);
    y1 = Math.min(y1, point.y - GHOST_SIZE);
    x2 = Math.max(x2, point.x + GHOST_SIZE);
    y2 = Math.max(y2, point.y + GHOST_SIZE);
  }
  if (!Number.isFinite(x1)) return null;
  return { x1, y1, x2, y2, w: x2 - x1, h: y2 - y1 };
}

function collect(core: Core, ids: readonly string[]): NodeCollection {
  const wanted = new Set(ids);
  return core.nodes().filter((node) => wanted.has(node.id()));
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}
