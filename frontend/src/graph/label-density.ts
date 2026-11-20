import type { Core, EdgeSingular, NodeCollection, NodeSingular } from "cytoscape";
import { labelBox, LABEL_HOVER, refreshLabels, type LabelBox, type LabelEle, type LabelSet } from "./label-hover";
import { placeMapLabels, type MapLabel, type LabelSegment } from "./label-placement";
export type { MapLabel } from "./label-placement";

export const AUTO_LABEL_HIDDEN = "is-auto-label-hidden";
const SCALE_KEY = "mathscape.labelScale";
const DENSE_MAP_SIZE = 32;
const OFFSET_KEY = "mathscape.labelOffset";
const ZERO_OFFSET = { x: 0, y: 0 };

export function mapLabelOffset(node: NodeSingular): { x: number; y: number } {
  return node.scratch(OFFSET_KEY) ?? ZERO_OFFSET;
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
    case "shared_entity": case "shared_theory": return 80;
    case "pdf": return 70;
    case "markdown": case "ink_snapshot": return 60;
    case "semantic_unit": return 40;
    default: return 10;
  }
}

/** The study outline already has a designed label hierarchy; leave it alone. */
export function bindMapLabelDensity(core: Core, enabled: () => boolean): void {
  let frame: number | null = null;
  let applying = false;
  const update = () => {
    frame = null;
    if (core.destroyed()) return;
    applying = true;
    try {
      const nodes = core.nodes(':visible').filter(node => node.data("elementKind") !== "junction") as NodeCollection;
      const dense = enabled() && nodes.length >= DENSE_MAP_SIZE;
      const scale = dense ? Math.max(1, 1 / core.zoom()) : 1;
      if (core.scratch(SCALE_KEY) !== scale) {
        core.scratch(SCALE_KEY, scale);
        // Font mappings alone leave Cytoscape's label/wrap textures at their
        // previous size. Measure only after both render frames have settled.
        refreshLabels(core, core.nodes() as unknown as LabelSet, schedule);
        return;
      }
      if (!dense) {
        core.nodes().removeClass(AUTO_LABEL_HIDDEN);
        const shifted = core.nodes().filter(node => mapLabelOffset(node).x !== 0 || mapLabelOffset(node).y !== 0);
        shifted.forEach(node => { node.scratch(OFFSET_KEY, { x: 0, y: 0 }); });
        if (shifted.nonempty()) refreshLabels(core, shifted as unknown as LabelSet);
        return;
      }
      const zoom = core.zoom(), pan = core.pan();
      const labels: MapLabel[] = [];
      const baseBoxes = new Map<string, LabelBox>();
      const bodies = nodes.map(node => {
        const p = node.renderedPosition(), w = node.renderedOuterWidth(), h = node.renderedOuterHeight();
        return { x1: p.x - w / 2, x2: p.x + w / 2, y1: p.y - h / 2, y2: p.y + h / 2, w, h };
      });
      const edges: LabelSegment[] = core.edges(':visible').filter(edge => Number(edge.style("opacity")) > 0.15).map(edge => ({ a: (edge as EdgeSingular).renderedSourceEndpoint(), b: (edge as EdgeSingular).renderedTargetEndpoint() }));
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
        // A file bundle can occupy every immediately adjacent slot. Allow its
        // material caption to sit just outside that cluster, never over glyphs.
        const clearances = priority(node) >= 60 || protectedLabel ? [12, 36, 60, 84] : [12];
        labels.push({
          id: node.id(), box: base,
          alternatives: clearances.flatMap(gap => [at(p.x - base.w / 2, p.y - hh - gap - base.h), at(p.x - hw - gap - base.w, p.y - base.h / 2), at(p.x + hw + gap, p.y - base.h / 2), at(p.x - base.w / 2, p.y + hh + gap)])
            .filter(b => b.x1 >= 2 && b.y1 >= 2 && b.x2 <= core.width() - 2 && b.y2 <= core.height() - 2),
          priority: priority(node),
          protected: protectedLabel,
        });
      });
      const placements = placeMapLabels(labels, bodies, edges, 12);
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
