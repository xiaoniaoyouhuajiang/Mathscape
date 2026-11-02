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
 * `ink.page_image` open a temporary mini-window beside the focused node (or
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
 * Animation state only: the landscape graph is never touched; clumped
 * coordinates live in Cytoscape for the duration of the scene and are never
 * written to the graph model or storage.
 */
import type { BoundingBox12, Core, NodeCollection, NodeSingular } from "cytoscape";
import { OUT_OF_FOCUS_CLASS, applyFocusScope } from "./cytoscape";
import { SCENE_CLUMP, clumpTargets, ease, lerp, satelliteShift, type Point } from "./scene-clump";
import type { OutlineAct, PageImageAct, SceneAct, ToolScene } from "../sandbox/scene/types";

export type ScenePart = "call" | "result" | "full";

interface Segment {
  scene: ToolScene;
  part: ScenePart;
  /** 1-based position in a turn replay. */
  step?: number;
  total?: number;
}

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
}

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
}

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
};
const ACT_GAP = 18;
const ACT_MARGIN = 8;
const FIT_PADDING = 64;
const FIT_MAX_ZOOM = 1.6;
/** Nodes that hang off entities and follow them when they move. */
const SATELLITE_SELECTOR = '[elementKind = "junction"], .ink-note-node';
const SCENE_CLASSES = "sandbox-echo-seed sandbox-echo-hit sandbox-echo-fade sandbox-echo-edge";

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
  /** Clump: original model positions of every node moved this session. */
  private origin = new Map<string, Point>();
  /** Clump: the focus the current segment clumped (re-applied after render). */
  private clumpFocus: string[] = [];
  /** Running position tween (clump in / restore out). */
  private tween: { frame: number; done: () => void } | null = null;
  /** The open Act mini-window, if any. */
  private actWindow: HTMLElement | null = null;
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
    this.hudType = document.createElement("span");
    this.hudType.className = "scene-hud-type";
    this.hudType.hidden = true;
    this.hudTypeText = document.createElement("span");
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
    this.hud.append(tag, this.hudTitle, this.hudStep, this.hudType, this.hudSummary, stop);
    host.hudHost.append(this.hud);
  }

  isPlaying(): boolean {
    return this.session !== null;
  }

  /** Live tool event: queue behind whatever is playing. */
  live(scene: ToolScene, part: "call" | "result"): void {
    if (!this.host.canPlay()) return;
    this.queue.push({ scene, part });
    void this.pump();
  }

  /** Replay one step or a whole turn: replaces whatever is playing. */
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
      const segment = this.queue.shift()!;
      this.begin();
      await this.perform(segment, token);
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
    this.announce(segment);
    this.closeAct();
    const act = scene.act;
    const typed = act?.kind === "typewriter" ? act.text : undefined;
    // 目录 / 页图 windows need the result: never on a bare call segment.
    const windowAct = act && act.kind !== "typewriter" && part !== "call" ? act : undefined;
    if (typed === undefined) this.setTyped(null);
    else if (part === "result") this.setTyped(typed, true);
    else if (!(await this.typewrite(typed, token))) return;
    const core = this.host.core();
    const seeds = scene.seeds;
    const layers = part === "call" ? [] : scene.layers;
    const all = [...seeds, ...layers.flat()];
    const nodes = core && all.length > 0 ? collect(core, all) : null;
    if (!core || !nodes || nodes.empty()) {
      this.visual = null;
      this.clearPaint();
      this.clumpFocus = [];
      if (core) this.clump(core, [], true);
      if (windowAct) {
        this.openAct(windowAct, null);
        await this.wait(Math.max(SCENE_TIMING.hudOnlyMs, SCENE_TIMING.actMs), token);
        return;
      }
      await this.wait(typed !== undefined && part === "call" ? SCENE_TIMING.typeHoldMs : SCENE_TIMING.hudOnlyMs, token);
      return;
    }
    // Isolate the seeds first; each result layer then joins the focus.
    const visual: Visual = { focus: new Set(seeds), seeds, layers: [] };
    if (seeds.length > 0 || layers.length === 0) this.paint(visual);
    // Clump and fit move together; the camera aims at where nodes will be.
    this.clumpFocus = nodes.map((node) => node.id());
    const box = this.clump(core, this.clumpFocus, true);
    this.fit(core, box ?? nodes.boundingBox({}));
    if (seeds.length > 0) {
      if (!(await this.wait(part === "call" ? SCENE_TIMING.seedMs : SCENE_TIMING.layerMs, token))) return;
    } else if (!(await this.wait(Math.max(SCENE_TIMING.fitMs, SCENE_CLUMP.ms) + 40, token))) return;
    // The camera has landed: the window can sit beside its node.
    if (windowAct) this.openAct(windowAct, scene.anchor ?? null);
    for (const layer of layers) {
      visual.layers = [...visual.layers, layer];
      for (const id of layer) visual.focus.add(id);
      this.paint(visual);
      if (!(await this.wait(SCENE_TIMING.layerMs, token))) return;
    }
    if (windowAct) await this.wait(SCENE_TIMING.actMs, token);
    else if (layers.length > 0) await this.wait(SCENE_TIMING.holdMs, token);
  }

  /** HUD typewriter line: `null` hides it; `done` shows the text at once. */
  private setTyped(text: string | null, done = false): void {
    this.hudType.hidden = text === null;
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
  private openAct(act: Exclude<SceneAct, { kind: "typewriter" }>, anchor: string | null): void {
    this.closeAct();
    const win = document.createElement("section");
    win.className = "scene-act";
    win.dataset.act = act.kind;
    win.setAttribute("role", "dialog");
    win.setAttribute("aria-label", act.kind === "outline" ? "PDF 目录" : "手稿页图");
    if (act.kind === "outline") fillOutline(win, act);
    else fillPageImage(win, act);
    this.host.hudHost.append(win);
    this.actWindow = win;
    if (!this.placeBeside(win, anchor)) win.classList.add("is-stage");
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
      c.left >= ACT_MARGIN && c.top >= ACT_MARGIN && c.left + width <= hostWidth - ACT_MARGIN && c.top + height <= hostHeight - ACT_MARGIN;
    const spot = candidates.find(fits) ?? candidates[0];
    const left = clamp(spot.left, ACT_MARGIN, Math.max(ACT_MARGIN, hostWidth - width - ACT_MARGIN));
    const top = clamp(spot.top, ACT_MARGIN, Math.max(ACT_MARGIN, hostHeight - height - ACT_MARGIN));
    win.style.left = `${Math.round(left)}px`;
    win.style.top = `${Math.round(top)}px`;
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
    this.hudStep.textContent = segment.total ? `${segment.step}/${segment.total}` : "";
    this.hudStep.hidden = !segment.total;
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
      // Edges (and semantic-mode junction edges) whose ends are both in focus.
      core.edges().not(`.${OUT_OF_FOCUS_CLASS}`).addClass("sandbox-echo-edge");
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
    core.animate({ zoom, pan: { x: width / 2 - zoom * cx, y: height / 2 - zoom * cy } }, { duration: SCENE_TIMING.fitMs });
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
    this.setTyped(null);
    this.closeAct();
    this.host.canvas.classList.remove("is-scene-playing");
    this.clumpFocus = [];
    // Nodes glide home (instantly when a jump follows: restoreCamera false).
    this.restorePositions(restoreCamera);
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

  private wait(ms: number, token: number): Promise<boolean> {
    return new Promise((resolve) => setTimeout(() => resolve(token === this.token), ms));
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
  actHead(win, act.noteTitle ?? "手稿页图", act.pageLabel);
  const empty = () => element("p", "scene-act-empty", "未取得页图（仅元数据）");
  if (!act.src) {
    win.append(empty());
    return;
  }
  const img = element("img", "scene-act-image");
  img.alt = [act.noteTitle ?? "手稿页图", act.pageLabel].filter(Boolean).join(" · ");
  if (act.width && act.height) {
    img.width = act.width;
    img.height = act.height;
  }
  img.onerror = () => img.replaceWith(empty());
  img.src = act.src;
  win.append(img);
}

function collect(core: Core, ids: readonly string[]): NodeCollection {
  const wanted = new Set(ids);
  return core.nodes().filter((node) => wanted.has(node.id()));
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}
