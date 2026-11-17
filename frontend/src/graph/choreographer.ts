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
import type { Core, NodeSingular } from "cytoscape";
import { reducedMotion } from "../core/motion";
import { INK_NOTE_NODE_CLASS } from "./ink-overlay";
import type { MapPoint } from "./layout";
import { ease, lerp, satelliteShift } from "./scene-clump";
import type { CameraTarget, MapPostureSpec } from "./study-posture";

export const POSTURE_MORPH_MS = 420;
export const INCREMENTAL_BLOOM_MS = 220;

export const STUDY_POSTURE_NODE_CLASSES = [
  "spine-header-node",
  "spine-unit-node",
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
    options: { animate?: boolean; forceSnap?: boolean } = {},
  ): void {
    this.finishNow();

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
      core.nodes().removeClass(ALL_STUDY_NODE_CLASS_STR);
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

    const allowAnim =
      options.animate !== false &&
      !options.forceSnap &&
      !reducedMotion() &&
      core.width() > 0 &&
      core.height() > 0;

    // 2. If staying in the same posture (e.g. checkbox toggle, signal toggle, study mark update),
    //    keep camera untouched so the user's scroll position never jumps, bloom newly added elements,
    //    and gently nudge any orbit nodes whose relaxed slot shifted.
    if (!postureChanged) {
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
      isOrganic && this.savedOrganicViewport
        ? { kind: "saved", vp: this.savedOrganicViewport }
        : { kind: "target", camera: posture.camera, positions: posture.positions },
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
      const pad = camera.padding ?? 40;
      const bw = Math.max(120, camera.box.x2 - camera.box.x1);
      const bh = Math.max(120, camera.box.y2 - camera.box.y1);
      const availW = Math.max(120, w - pad * 2);
      const availH = Math.max(120, h - pad * 2);
      const rawZoom = Math.min(availW / bw, availH / bh);
      const zoom = Math.min(
        camera.maxZoom ?? 1.05,
        Math.max(camera.minZoom ?? 0.75, rawZoom),
      );
      const cx = (camera.box.x1 + camera.box.x2) / 2;
      // Measure actual `#study-hud` height dynamically so the spine header always breathes cleanly below it
      const hudEl = core.container()?.parentElement?.querySelector<HTMLElement>("#study-hud");
      const hudHeight = hudEl && !hudEl.hidden ? hudEl.offsetHeight : 44;
      const topScreenOffset = Math.max(96, hudHeight + 48);
      const panX = w / 2 - cx * zoom;
      const panY = topScreenOffset - camera.box.y1 * zoom;
      return { zoom, pan: { x: panX, y: panY } };
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
