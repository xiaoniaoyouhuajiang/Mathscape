import type { Core, NodeSingular } from "cytoscape";
import { boxesOverlap, labelBox, LABEL_HOVER, type LabelBox, type LabelSet } from "./label-hover";

export const AUTO_LABEL_HIDDEN = "is-auto-label-hidden";
const SCALE_KEY = "mathscape.labelScale";
const DENSE_MAP_SIZE = 32;

export interface MapLabel {
  id: string;
  box: LabelBox;
  priority: number;
  /** Interaction targets remain legible even when they overlap. */
  protected?: boolean;
}

/** Stable, screen-space decluttering, never node or edge filtering. */
export function visibleMapLabels(labels: readonly MapLabel[], gap = 4): Set<string> {
  const occupied: LabelBox[] = [];
  const visible = new Set<string>();
  const sorted = [...labels].sort((a, b) =>
    Number(!!b.protected) - Number(!!a.protected) || b.priority - a.priority || a.id.localeCompare(b.id));
  for (const item of sorted) {
    const padded = { ...item.box, x1: item.box.x1 - gap / 2, y1: item.box.y1 - gap / 2, x2: item.box.x2 + gap / 2, y2: item.box.y2 + gap / 2 };
    if (!item.protected && occupied.some(box => boxesOverlap(box, padded))) continue;
    visible.add(item.id);
    occupied.push(padded);
  }
  return visible;
}

/** Do not shrink readable names to subpixel text when fitting a large map. */
export function mapLabelScale(node: NodeSingular): number {
  return Number(node.cy().scratch(SCALE_KEY) ?? 1);
}

function priority(node: NodeSingular): number {
  if (node.hasClass("is-staged") || node.hasClass("is-staged-conflict") || node.data("nodeType") === "debt") return 100;
  switch (node.data("nodeType")) {
    case "shared_entity": case "shared_theory": return 80;
    case "pdf": case "markdown": case "ink_snapshot": return 60;
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
      const nodes = core.nodes(':visible').filter(node => node.data("elementKind") !== "junction");
      const dense = enabled() && nodes.length >= DENSE_MAP_SIZE;
      const scale = dense ? Math.max(1, 1 / core.zoom()) : 1;
      if (core.scratch(SCALE_KEY) !== scale) {
        core.scratch(SCALE_KEY, scale);
        (core.nodes() as unknown as LabelSet).updateStyle();
      }
      if (!dense) {
        core.nodes().removeClass(AUTO_LABEL_HIDDEN);
        return;
      }
      const zoom = core.zoom(), pan = core.pan();
      const labels: MapLabel[] = [];
      nodes.forEach(node => {
        if (!node.data("label") || node.hasClass("is-out-of-focus") || node.hasClass("label-recess")) return;
        const box = labelBox(node);
        if (box.w <= 0 || box.h <= 0) return;
        const x1 = box.x1 * zoom + pan.x, x2 = box.x2 * zoom + pan.x;
        const y1 = box.y1 * zoom + pan.y, y2 = box.y2 * zoom + pan.y;
        if (x2 < 0 || y2 < 0 || x1 > core.width() || y1 > core.height()) return;
        labels.push({
          id: node.id(), box: { x1, x2, y1, y2, w: x2 - x1, h: y2 - y1 },
          priority: priority(node),
          protected: node.selected() || node.hasClass(LABEL_HOVER) || node.hasClass("is-selected-primary") || node.hasClass("is-focus-root"),
        });
      });
      const visible = visibleMapLabels(labels);
      core.batch(() => nodes.forEach(node => node.toggleClass(AUTO_LABEL_HIDDEN, !visible.has(node.id()))));
    } finally {
      applying = false;
    }
  };
  const schedule = () => {
    if (!applying && frame === null) frame = requestAnimationFrame(update);
  };
  core.on("zoom pan resize add remove data position select unselect class", schedule);
  core.one("destroy", () => { if (frame !== null) cancelAnimationFrame(frame); });
  schedule();
}
