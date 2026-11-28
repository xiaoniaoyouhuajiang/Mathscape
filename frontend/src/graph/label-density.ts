import type { Core, EdgeSingular, NodeCollection, NodeSingular } from "cytoscape";
import { labelBox, LABEL_HOVER, refreshLabels, type LabelBox, type LabelEle, type LabelSet } from "./label-hover";
import { placeMapLabels, type MapLabel, type LabelSegment } from "./label-placement";
export type { MapLabel } from "./label-placement";

export const AUTO_LABEL_HIDDEN = "is-auto-label-hidden";
const SCALE_KEY = "mathscape.labelScale";
const OFFSET_KEY = "mathscape.labelOffset";
const ZERO_OFFSET = { x: 0, y: 0 };

export function mapLabelOffset(node: NodeSingular): { x: number; y: number } {
  return node.scratch(OFFSET_KEY) ?? ZERO_OFFSET;
}

/** Clear only generated presentation state before repainting either style. */
export function resetMapLabelDensity(core: Core): void {
  core.scratch(SCALE_KEY, 1);
  core.nodes().forEach(node => { node.scratch(OFFSET_KEY, ZERO_OFFSET); });
  core.nodes().removeClass(AUTO_LABEL_HIDDEN);
}

/** Stable, screen-space decluttering, never node or edge filtering. */
export function visibleMapLabels(labels: readonly MapLabel[], gap = 4): Set<string> {
  return new Set(placeMapLabels(labels, [], [], gap).keys());
}

/** Do not shrink readable names to subpixel text when fitting a large map. */
export function mapLabelScale(node: NodeSingular): number {
  return Number(node.cy().scratch(SCALE_KEY) ?? 1);
}

function priority(node: NodeSingular): number {
  if (node.hasClass("is-staged") || node.hasClass("is-staged-conflict") || node.data("nodeType") === "debt") return 100;
  switch (node.data("nodeType")) {
    case "shared_entity": case "shared_theory": return 95;
    case "pdf": return 70;
    case "markdown": case "ink_snapshot": return 60;
    case "semantic_unit": return 40;
    default: return 10;
  }
}

/** Declutter small maps too: a few long theorem names can fill a narrow canvas. */
export function bindMapLabelDensity(core: Core, enabled: () => boolean): void {
  let frame: number | null = null;
  let applying = false;
  const update = () => {
    frame = null;
    if (core.destroyed()) return;
    applying = true;
    try {
      const nodes = core.nodes(':visible').filter(node => node.data("elementKind") !== "junction") as NodeCollection;
      const organic = enabled();
      const scale = Math.max(1, 1 / core.zoom());
      if (core.scratch(SCALE_KEY) !== scale) {
        core.scratch(SCALE_KEY, scale);
        // Font mappings alone leave Cytoscape's label/wrap textures at their
        // previous size. Measure only after both render frames have settled.
        refreshLabels(core, core.nodes() as unknown as LabelSet, schedule);
        return;
      }
      const zoom = core.zoom(), pan = core.pan();
      const container = core.container();
      const hud = container?.parentElement?.querySelector<HTMLElement>("#study-hud");
      const top = !organic && hud && !hud.hidden && container
        ? Math.max(12, hud.getBoundingClientRect().bottom - container.getBoundingClientRect().top + 12)
        : 12;
      const viewport = { x1: 12, y1: top, x2: core.width() - 12, y2: core.height() - 12, w: core.width() - 24, h: core.height() - top - 12 };
      if (!organic) {
        // The spine/orbits already allocate a reading lane for every title.
        // Do not let the organic priority filter hide outline rows or move
        // their names to a distant cluster perimeter.
        let shifted = core.collection();
        core.batch(() => {
          core.nodes().removeClass(AUTO_LABEL_HIDDEN);
          nodes.forEach(node => {
            const old = mapLabelOffset(node), box = labelBox(node as unknown as LabelEle);
            const left = box.x1 * zoom + pan.x - old.x, width = box.w * zoom;
            const x = node.hasClass("is-out-of-focus") ? 0 : Math.max(viewport.x1, Math.min(left, viewport.x2 - width)) - left;
            if (Math.abs(x - old.x) < 0.1 && old.y === 0) return;
            node.scratch(OFFSET_KEY, { x, y: 0 });
            shifted = shifted.union(node);
          });
        });
        if (shifted.nonempty()) refreshLabels(core, shifted as unknown as LabelSet, schedule);
        return;
      }
      const labels: MapLabel[] = [];
      const baseBoxes = new Map<string, LabelBox>();
      const bodies = nodes.map(node => {
        const p = node.renderedPosition(), w = node.renderedOuterWidth(), h = node.renderedOuterHeight();
        return { x1: p.x - w / 2, x2: p.x + w / 2, y1: p.y - h / 2, y2: p.y + h / 2, w, h };
      });
      const edges: LabelSegment[] = core.edges(':visible').filter(edge => Number(edge.style("opacity")) > 0.15).map(edge => ({ a: (edge as EdgeSingular).renderedSourceEndpoint(), b: (edge as EdgeSingular).renderedTargetEndpoint() }));
      const clusterBounds = new Map<string, LabelBox>();
      const heads = new Set<string>();
      // Provenance links do not define the visible constellation's clusters.
      const structure = core.nodes(':visible').union(core.edges(':visible').not('.evidence-edge'));
      for (const component of structure.components()) {
        const members = component.nodes().filter(n => n.data("elementKind") !== "junction") as NodeCollection;
        if (members.empty()) continue;
        const bb = members.renderedBoundingBox({ includeLabels: false, includeOverlays: false, includeUnderlays: false });
        members.forEach(n => { clusterBounds.set(n.id(), bb); });
        const ranked = members.toArray().filter(n => priority(n) >= 40).sort((a, b) =>
          priority(b) - priority(a) || b.connectedEdges().not('.evidence-edge').length - a.connectedEdges().not('.evidence-edge').length || a.id().localeCompare(b.id()));
        // Isolated exercises must not take the only open title slot of a
        // substantial neighbouring constellation.
        if (ranked[0] && members.length >= 3) heads.add(ranked[0].id());
      }
      nodes.forEach(node => {
        if (!node.data("label") || node.hasClass("is-out-of-focus") || node.hasClass("label-recess")) return;
        const protectedLabel = node.selected() || node.hasClass(LABEL_HOVER) || node.hasClass("is-selected-primary") || node.hasClass("is-focus-root");
        // Provenance belongs to close inspection, not the overview's title layer.
        // Keep every glyph/hit target; hover and selection still reveal its name.
        if (!protectedLabel && zoom < 1.2 && priority(node) < 40) return;
        const box = labelBox(node as unknown as LabelEle);
        if (box.w <= 0 || box.h <= 0) return;
        const offset = mapLabelOffset(node);
        const x1 = box.x1 * zoom + pan.x - offset.x, x2 = box.x2 * zoom + pan.x - offset.x;
        const y1 = box.y1 * zoom + pan.y - offset.y, y2 = box.y2 * zoom + pan.y - offset.y;
        if (x2 < 0 || y2 < 0 || x1 > core.width() || y1 > core.height()) return;
        const base = { x1, x2, y1, y2, w: x2 - x1, h: y2 - y1 };
        baseBoxes.set(node.id(), base);
        const p = node.renderedPosition(), hw = node.renderedOuterWidth() / 2, hh = node.renderedOuterHeight() / 2;
        const at = (x: number, y: number): LabelBox => ({ x1: x, x2: x + base.w, y1: y, y2: y + base.h, w: base.w, h: base.h });
        const alternatives = [at(p.x - base.w / 2, p.y - hh - 12 - base.h), at(p.x - hw - 12 - base.w, p.y - base.h / 2), at(p.x + hw + 12, p.y - base.h / 2)];
        // Keep captions near the canvas edge in an inward slot.
        alternatives.push(at(Math.max(viewport.x1, Math.min(base.x1, viewport.x2 - base.w)), Math.max(viewport.y1, Math.min(base.y1, viewport.y2 - base.h))));
        // Keep a readable identity for every constellation. A short annotation
        // leader ties perimeter captions to their actual glyph (not a new edge).
        const cluster = clusterBounds.get(node.id());
        if (cluster && (heads.has(node.id()) || priority(node) >= 60 || protectedLabel)) {
          for (const gap of [12, 30, 48, 66, 84, 102]) {
            for (const x of [p.x - base.w / 2, cluster.x1, cluster.x2 - base.w]) alternatives.push(
              at(x, cluster.y1 - gap - base.h), at(x, cluster.y2 + gap),
            );
            for (const y of [p.y - base.h / 2, cluster.y1, cluster.y2 - base.h]) alternatives.push(
              at(cluster.x1 - gap - base.w, y), at(cluster.x2 + gap, y),
            );
          }
        }
        labels.push({
          id: node.id(), box: base,
          alternatives: alternatives
            .filter(b => b.x1 >= 2 && b.y1 >= 2 && b.x2 <= core.width() - 2 && b.y2 <= core.height() - 2),
          priority: heads.has(node.id()) ? Math.max(node.data("nodeType") === "semantic_unit" ? 90 : 85, priority(node)) : priority(node),
          protected: protectedLabel,
        });
      });
      const placements = placeMapLabels(labels, bodies, edges, 12, viewport);
      let shifted = core.collection();
      core.batch(() => { nodes.forEach(node => {
        const placed = placements.get(node.id()), base = baseBoxes.get(node.id());
        node.toggleClass(AUTO_LABEL_HIDDEN, !placed);
        if (!placed || !base) return;
        const next = { x: placed.x1 - base.x1, y: placed.y1 - base.y1 }, old = mapLabelOffset(node);
        if (Math.abs(next.x - old.x) < 0.1 && Math.abs(next.y - old.y) < 0.1) return;
        node.scratch(OFFSET_KEY, next);
        shifted = shifted.union(node);
      }); });
      if (shifted.nonempty()) refreshLabels(core, shifted as unknown as LabelSet, schedule);
    } finally {
      applying = false;
    }
  };
  const schedule = () => {
    if (!applying && frame === null) frame = requestAnimationFrame(update);
  };
  core.on("zoom pan resize add remove data position select unselect class style", schedule);
  core.one("destroy", () => { if (frame !== null) cancelAnimationFrame(frame); });
  schedule();
}
