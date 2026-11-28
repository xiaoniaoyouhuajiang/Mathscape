/**
 * GraphChoreographer — Extensible Map Posture & Transition Engine.
 *
 * Drives two kinds of motion on the Cytoscape canvas in a single batched
 * `requestAnimationFrame` loop (`core.batch()`):
 * 1. Bidirectional Posture Morphing (`OrganicLandscape` ⇄ `StudyOverview` ⇄ `StudySpine`,
 *    extensible to any future `MapPostureSpec`): smoothly interpolates entity
 *    nodes, hyperedge junctions, synthesized `ink-note-node` satellites, and
 *    the camera `(zoom, pan)` in lockstep (~420ms cubic ease-in-out).
 * 2. Incremental Bloom Transitions (~220ms): when filter checkboxes, presets,
 *    or semantic-edge toggles reveal new elements without changing posture,
 *    newly mounted elements bloom in smoothly at their exact deterministic
 *    slot coordinates without flashing the rest of the canvas.
 */
import type { Core, NodeCollection, NodeSingular } from "cytoscape";
import { reducedMotion } from "../core/motion";
import { INK_NOTE_NODE_CLASS } from "./ink-overlay";
import type { MapPoint } from "./layout";
import { ease, lerp, satelliteShift } from "./scene-clump";
import type { CameraTarget, MapPostureSpec } from "./study-posture";
import { layoutStudyLanes, type StudyLaneLabel } from "./study-viewport";
import { labelBox, refreshLabels, type LabelEle, type LabelSet } from "./label-hover";
import { mapLabelScale } from "./label-density";

export const POSTURE_MORPH_MS = 420;
export const INCREMENTAL_BLOOM_MS = 220;

export const STUDY_POSTURE_NODE_CLASSES = [
  "spine-header-node",
  "spine-unit-node",
  "spine-scope-node",
  "orbit-semantic-node",
  "orbit-outer-node",
  "study-status-mastered",
  "study-status-studying",
  "study-cat-aligned",
  "study-cat-diff",
  "study-cat-cross",
  "study-cat-unaligned",
  "is-study-signal-hit",
  "is-study-signal-muted",
] as const;

export const STUDY_POSTURE_EDGE_CLASSES = [
  "spine-arc-edge",
  "spine-tree-edge",
  "is-study-signal-hit",
  "is-study-signal-muted",
] as const;

const ALL_STUDY_NODE_CLASS_STR = STUDY_POSTURE_NODE_CLASSES.join(" ");
const ALL_STUDY_EDGE_CLASS_STR = STUDY_POSTURE_EDGE_CLASSES.join(" ");

interface ActiveTween {
  frame: number;
  finalize: () => void;
}

interface ViewportState {
  zoom: number;
  pan: { x: number; y: number };
}

function lerpNumber(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

export class GraphChoreographer {
  private lastPostureId: string = "organic";
  private tween: ActiveTween | null = null;
  private bloomTimer: number | null = null;
  /** Saved organic camera viewport before entering Study Posture so Local -> Global restores smoothly. */
  private savedOrganicViewport: ViewportState | null = null;
  private posture: MapPostureSpec | null = null;

  private settleStudy(core: Core): void {
    const posture = this.posture;
    requestAnimationFrame(() => requestAnimationFrame(() => {
      if (!core.destroyed() && this.posture === posture && !this.isAnimating()) this.refit(core, false, false);
    }));
  }

  refit(core: Core, resized = false, settle = true): void {
    if (!this.posture || this.posture.id === "organic") return;
    this.finishNow();
    // A pre-study camera belongs to the old canvas size. After a dock/window
    // resize, returning to organic must fit the current canvas instead.
    if (resized) this.savedOrganicViewport = null;
    this.layoutSpine(core, this.posture);
    core.batch(() => core.nodes().forEach(node => {
      const position = this.posture?.positions.get(node.id());
      if (position && !node.hasClass(INK_NOTE_NODE_CLASS)) node.position(position);
    }));
    const viewport = this.resolveCameraViewport(core, { kind: "target", camera: this.posture.camera, positions: this.posture.positions });
    if (viewport) core.viewport(viewport);
    if (settle) this.settleStudy(core);
  }

  private studyViewport(core: Core) {
    const pad = 20, container = core.container();
    const hud = container?.parentElement?.querySelector<HTMLElement>("#study-hud");
    const top = hud && !hud.hidden && container
      ? Math.max(pad, hud.getBoundingClientRect().bottom - container.getBoundingClientRect().top + pad)
      : pad;
    return { x1: pad, x2: core.width() - pad, y1: top, y2: core.height() - pad };
  }

  private layoutSpine(core: Core, posture: MapPostureSpec): void {
    if (!posture.id.startsWith("study:material:")) return;
    const viewport = this.studyViewport(core);
    const width = Math.max(300, viewport.x2 - viewport.x1), height = Math.max(120, viewport.y2 - viewport.y1);
    const nodes = core.nodes().filter(node => posture.inScopeNodeIds?.has(node.id()) === true && node.data("elementKind") !== "junction") as NodeCollection;
    const compact = width < 600;
    nodes.toggleClass("is-compact-study", compact);
    nodes.forEach(node => { node.data("studyFontSize", width < 480 ? 12 : 13); });
    const laneOf = (node: NodeSingular): 0 | 1 | 2 => node.hasClass("orbit-outer-node") ? 2 : node.hasClass("orbit-semantic-node") ? 1 : 0;
    const rows = nodes.map(node => {
      const p = posture.positions.get(node.id()) ?? node.position();
      const order = node.hasClass("spine-header-node") ? -2e6 : node.hasClass("spine-scope-node") ? -1e6 + p.y : p.y;
      const savedIndent = node.scratch("studyIndent") as { posture: string; value: number } | undefined;
      const indent = savedIndent?.posture === posture.id ? savedIndent.value : node.hasClass("spine-unit-node") ? Math.max(0, p.x) / 66 * 6 : 0;
      node.scratch("studyIndent", { posture: posture.id, value: indent });
      return { node, id: node.id(), lane: laneOf(node), order, indent };
    });
    const measure = (columns: readonly number[]) => {
      nodes.forEach(node => { node.data("studyLabelWidth", Math.max(64, columns[laneOf(node)] - (compact ? 30 : 42))); });
      // Measure safe wrapping at the actual reading width, independent of the
      // previous camera's font compensation. Never shrink study text to fit.
      refreshLabels(core, nodes as unknown as LabelSet);
      let horizontalOverflow = 0;
      const labels: StudyLaneLabel[] = rows.map(row => {
        const box = labelBox(row.node as unknown as LabelEle), scale = mapLabelScale(row.node);
        horizontalOverflow = Math.max(horizontalOverflow, box.w / scale - (columns[row.lane] - 20));
        return { ...row, height: Math.max(24, box.h / scale + 6) };
      });
      const heights = [0, 1, 2].map(lane => {
        const items = labels.filter(row => row.lane === lane);
        return items.reduce((sum, row) => sum + row.height, 0) + Math.max(0, items.length - 1) * 14;
      });
      const score = Math.max(0, ...heights.map(h => h - height)) + Math.max(0, horizontalOverflow) * 4;
      return { labels, score };
    };
    let columns = [width / 3, width / 3, width / 3];
    let best = measure(columns);
    // A dock-narrowed outline has many more rows than either orbit. Borrow
    // spare orbit width before allowing a caption to continue below the page.
    if (best.score > 0) {
      for (const ratios of [[0.4, 0.27, 0.33], [0.44, 0.25, 0.31], [0.48, 0.23, 0.29], [0.4, 0.2, 0.4], [0.42, 0.2, 0.38], [0.45, 0.2, 0.35], [0.5, 0.23, 0.27], [0.52, 0.23, 0.25], [0.36, 0.24, 0.4], [0.33, 0.24, 0.43], [0.3, 0.24, 0.46], [0.27, 0.23, 0.5]]) {
        const candidate = ratios.map(ratio => ratio * width), measured = measure(candidate);
        if (measured.score < best.score) { columns = candidate; best = measured; }
        if (best.score === 0) break;
      }
      best = measure(columns);
    }
    // Each measured row already includes 6px of caption padding; an 8px
    // compact row gap still leaves 14px between the actual captions.
    const minGap = best.score > 0 ? 8 : 14;
    for (const [id, position] of layoutStudyLanes(best.labels, width, height, { columns, minGap, compact })) posture.positions.set(id, position);
    core.nodes().filter(node => node.data("elementKind") === "junction").forEach(node => {
      const ends = node.connectedEdges().connectedNodes().not(node).toArray();
      const points = ends.map(end => posture.positions.get(end.id())).filter((p): p is MapPoint => p !== undefined);
      if (points.length) posture.positions.set(node.id(), { x: points.reduce((sum, p) => sum + p.x, 0) / points.length, y: points.reduce((sum, p) => sum + p.y, 0) / points.length });
    });
    // This is a readable page, not a thumbnail of the entire document. Longer
    // outlines continue below it and remain reachable by normal panning.
    posture.camera = { kind: "framing_box", box: { x1: 0, y1: 0, x2: width, y2: height }, maxZoom: 1 };
  }

  getPostureId(): string {
    return this.lastPostureId;
  }

  isAnimating(): boolean {
    return this.tween !== null;
  }

  /** Immediately settles any running posture morph to its final target coordinates. */
  finishNow(): void {
    if (this.tween) {
      cancelAnimationFrame(this.tween.frame);
      const fin = this.tween.finalize;
      this.tween = null;
      fin();
    }
    if (this.bloomTimer !== null) {
      window.clearTimeout(this.bloomTimer);
      this.bloomTimer = null;
    }
  }

  /** Captures current live positions of mounted Cytoscape nodes before a graph sync. */
  snapshotPositions(core: Core | null): Map<string, MapPoint> {
    const out = new Map<string, MapPoint>();
    if (!core || core.destroyed()) return out;
    core.nodes().forEach((node) => {
      if (node.hasClass(INK_NOTE_NODE_CLASS)) return;
      const p = node.position();
      out.set(node.id(), { x: p.x, y: p.y });
    });
    return out;
  }

  /**
   * Applies `posture` classes, spine arc geometry, node positions, and camera
   * transition after `syncCytoscapeGraph` & overlays have mounted elements.
   */
  applyPosture(
    core: Core,
    posture: MapPostureSpec,
    priorPositions: ReadonlyMap<string, MapPoint>,
    priorElementIds: ReadonlySet<string>,
    options: { animate?: boolean; forceSnap?: boolean; resetViewport?: boolean } = {},
  ): void {
    this.finishNow();

    this.posture = posture;

    const wasOrganic = this.lastPostureId === "organic";
    const isOrganic = posture.id === "organic";
    const postureChanged = posture.id !== this.lastPostureId;

    if (wasOrganic && !isOrganic && core.width() > 0 && core.height() > 0) {
      const p = core.pan();
      this.savedOrganicViewport = { zoom: core.zoom(), pan: { x: p.x, y: p.y } };
    }

    this.lastPostureId = posture.id;

    // 1. Apply semantic posture classes & spine Bezier arc attributes
    core.batch(() => {
      core.nodes().removeClass(`${ALL_STUDY_NODE_CLASS_STR} is-compact-study`);
      core.edges().removeClass(ALL_STUDY_EDGE_CLASS_STR);

      for (const [nodeId, clsList] of posture.nodeClasses) {
        if (clsList.length === 0) continue;
        const n = core.getElementById(nodeId);
        if (n.nonempty() && n.isNode()) {
          n.addClass(clsList.join(" "));
        }
      }

      core.edges().forEach((edge) => {
        const relId = edge.data("relationId");
        const key = typeof relId === "string" ? relId : edge.id();
        const arcSpec = posture.spineArcOffsets.get(key);
        if (arcSpec !== undefined) {
          const srcPos = posture.positions.get(edge.source().id()) ?? edge.source().position();
          const tgtPos = posture.positions.get(edge.target().id()) ?? edge.target().position();
          const dy = tgtPos.y - srcPos.y;
          const mag = Math.abs(arcSpec.magnitude);
          // Cytoscape `vectorNormInverse.x = -dy / len`:
          // For leftward bow: +mag when downward (dy >= 0), -mag when upward (dy < 0).
          // For rightward bow: opposite sign.
          const leftSign = dy >= 0 ? mag : -mag;
          edge.data("spineArcOffset", arcSpec.side === "left" ? leftSign : -leftSign);
          edge.addClass("spine-arc-edge");
        }
        if (posture.spineTreeRelationKeys.has(key)) {
          edge.addClass("spine-tree-edge");
        }
        if (posture.hitRelationKeys.has(key)) {
          edge.addClass("is-study-signal-hit");
        } else if (posture.mutedRelationKeys.has(key)) {
          edge.addClass("is-study-signal-muted");
        }
      });
    });

    this.layoutSpine(core, posture);

    const allowAnim =
      options.animate !== false &&
      !options.forceSnap &&
      !reducedMotion() &&
      core.width() > 0 &&
      core.height() > 0;

    // 2. If staying in the same posture (e.g. checkbox toggle, signal toggle, study mark update),
    //    keep camera untouched so the user's scroll position never jumps, bloom newly added elements,
    //    and gently nudge any orbit nodes whose relaxed slot shifted.
    if (!postureChanged && !options.forceSnap) {
      if (!isOrganic) {
        core.batch(() => {
          core.nodes().forEach((node) => {
            if (node.hasClass(INK_NOTE_NODE_CLASS) || node.grabbed()) return;
            const target = posture.positions.get(node.id());
            if (target) node.position({ x: target.x, y: target.y });
          });
        });
      }
      if (allowAnim && priorElementIds.size > 0) {
        this.bloomAddedElements(core, priorElementIds);
      }
      return;
    }

    // 3. Posture changed: compute moving nodes + satellites + target viewport and tween smoothly
    const moves: Array<{ node: NodeSingular; from: MapPoint; to: MapPoint }> = [];
    const deltaByNodeId = new Map<string, MapPoint>();

    core.nodes().forEach((node) => {
      if (node.hasClass(INK_NOTE_NODE_CLASS)) return;
      const id = node.id();
      const to = posture.positions.get(id);
      if (!to) return;
      const from = priorPositions.get(id) ?? to;
      if (Math.abs(from.x - to.x) > 0.5 || Math.abs(from.y - to.y) > 0.5) {
        moves.push({ node, from, to });
        deltaByNodeId.set(id, { x: to.x - from.x, y: to.y - from.y });
      } else {
        node.position({ x: to.x, y: to.y });
      }
    });

    // Synthesized ink-note satellites glide along with their host targets
    const inkMoves: Array<{ node: NodeSingular; from: MapPoint; to: MapPoint }> = [];
    if (deltaByNodeId.size > 0) {
      core.nodes(`.${INK_NOTE_NODE_CLASS}`).forEach((inkNode) => {
        const hosts = inkNode
          .connectedEdges(".ink-note-edge")
          .connectedNodes()
          .not(`.${INK_NOTE_NODE_CLASS}`)
          .toArray() as NodeSingular[];
        const shifts = hosts.map((h) => deltaByNodeId.get(h.id()) ?? null);
        if (!shifts.some((s) => s !== null)) return;
        const d = satelliteShift(shifts);
        const cur = inkNode.position();
        // `applyInkOverlay` already placed `inkNode` around the current (or target) host;
        // compute target relative to its host's target coordinate.
        const primaryHost = hosts[0];
        const hostTo = posture.positions.get(primaryHost.id());
        const hostCur = primaryHost.position();
        const offsetFromHost = { x: cur.x - hostCur.x, y: cur.y - hostCur.y };
        if (hostTo) {
          const hostFrom = priorPositions.get(primaryHost.id()) ?? hostTo;
          const from = { x: hostFrom.x + offsetFromHost.x, y: hostFrom.y + offsetFromHost.y };
          const to = { x: hostTo.x + offsetFromHost.x, y: hostTo.y + offsetFromHost.y };
          inkMoves.push({ node: inkNode, from, to });
        } else if (Math.hypot(d.x, d.y) > 0.5) {
          inkMoves.push({ node: inkNode, from: { x: cur.x - d.x, y: cur.y - d.y }, to: { x: cur.x, y: cur.y } });
        }
      });
    }

    const fromVp: ViewportState = {
      zoom: core.zoom(),
      pan: { x: core.pan().x, y: core.pan().y },
    };
    const toVp = this.resolveCameraViewport(
      core,
      isOrganic && this.savedOrganicViewport && !options.resetViewport
        ? { kind: "saved", vp: this.savedOrganicViewport }
        : {
            kind: "target",
            // A resized study canvas invalidates the saved organic camera.
            // Organic normally preserves the viewport; that must not preserve
            // the study page's 1:1 framing when returning to the source map.
            camera: isOrganic && !wasOrganic && !this.savedOrganicViewport
              ? { kind: "fit_elements", padding: 56, elementIds: posture.inScopeNodeIds ?? undefined }
              : posture.camera,
            positions: posture.positions,
          },
    );
    if (isOrganic) {
      this.savedOrganicViewport = null;
    }

    const applyFrame = (t: number) => {
      const k = t >= 1 ? 1 : ease(t);
      core.batch(() => {
        for (const m of moves) {
          if (m.node.removed() || m.node.grabbed()) continue;
          m.node.position(k >= 1 ? { x: m.to.x, y: m.to.y } : lerp(m.from, m.to, k));
        }
        for (const m of inkMoves) {
          if (m.node.removed() || m.node.grabbed()) continue;
          m.node.position(k >= 1 ? { x: m.to.x, y: m.to.y } : lerp(m.from, m.to, k));
        }
        if (toVp) {
          core.viewport({
            zoom: k >= 1 ? toVp.zoom : lerpNumber(fromVp.zoom, toVp.zoom, k),
            pan: k >= 1 ? { ...toVp.pan } : lerp(fromVp.pan, toVp.pan, k),
          });
        }
      });
      if (t >= 1 && !isOrganic) this.settleStudy(core);
    };

    if (!allowAnim || (moves.length === 0 && !toVp)) {
      applyFrame(1);
      return;
    }

    // Set `t = 0` start frame cleanly before first paint
    applyFrame(0);
    const start = performance.now();
    const finalize = () => applyFrame(1);
    const active: ActiveTween = { frame: 0, finalize };
    const step = (now: number) => {
      if (this.tween !== active || core.destroyed()) {
        this.tween = null;
        return;
      }
      const t = (now - start) / POSTURE_MORPH_MS;
      if (t >= 1) {
        this.tween = null;
        finalize();
        return;
      }
      applyFrame(t);
      active.frame = requestAnimationFrame(step);
    };
    this.tween = active;
    active.frame = requestAnimationFrame(step);
  }

  private bloomAddedElements(core: Core, priorElementIds: ReadonlySet<string>): void {
    const added = core.elements().filter((el) => !priorElementIds.has(el.id()));
    if (added.empty()) return;
    added.addClass("is-choreo-entering");
    requestAnimationFrame(() => {
      if (core.destroyed()) return;
      added.addClass("is-choreo-entered");
      this.bloomTimer = window.setTimeout(() => {
        this.bloomTimer = null;
        if (!core.destroyed()) {
          added.removeClass("is-choreo-entering is-choreo-entered");
        }
      }, INCREMENTAL_BLOOM_MS + 40);
    });
  }

  private resolveCameraViewport(
    core: Core,
    spec:
      | { kind: "saved"; vp: ViewportState }
      | { kind: "target"; camera: CameraTarget; positions: ReadonlyMap<string, MapPoint> },
  ): ViewportState | null {
    const w = core.width();
    const h = core.height();
    if (w <= 0 || h <= 0) return null;

    if (spec.kind === "saved") {
      return spec.vp;
    }

    const { camera, positions } = spec;
    if (camera.kind === "preserve") return null;

    if (camera.kind === "framing_box") {
      const viewport = this.studyViewport(core);
      return { zoom: 1, pan: { x: viewport.x1, y: viewport.y1 } };
    }

    // `fit_elements`: compute target bounding box from `positions`
    let x1 = Number.POSITIVE_INFINITY;
    let y1 = Number.POSITIVE_INFINITY;
    let x2 = Number.NEGATIVE_INFINITY;
    let y2 = Number.NEGATIVE_INFINITY;
    let count = 0;

    core.nodes().forEach((node) => {
      if (node.hasClass(INK_NOTE_NODE_CLASS) || node.data("elementKind") === "junction") return;
      if (camera.elementIds && !camera.elementIds.has(node.id())) return;
      const p = positions.get(node.id()) ?? node.position();
      x1 = Math.min(x1, p.x - 45);
      x2 = Math.max(x2, p.x + 45);
      y1 = Math.min(y1, p.y - 35);
      y2 = Math.max(y2, p.y + 35);
      count += 1;
    });

    if (count === 0) return null;
    const pad = camera.padding ?? 52;
    const bw = Math.max(160, x2 - x1);
    const bh = Math.max(160, y2 - y1);
    const availW = Math.max(120, w - pad * 2);
    // Reserve ~64px at top so floating HUD bar never overlaps top nodes
    const availH = Math.max(120, h - pad * 2 - 48);
    const zoom = Math.min(camera.maxZoom ?? 1.2, Math.max(0.22, Math.min(availW / bw, availH / bh)));
    const cx = (x1 + x2) / 2;
    const cy = (y1 + y2) / 2;
    return {
      zoom,
      pan: {
        x: w / 2 - cx * zoom,
        y: (h + 40) / 2 - cy * zoom,
      },
    };
  }
}
