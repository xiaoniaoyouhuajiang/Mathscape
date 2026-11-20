import { boxesOverlap, type LabelBox } from "./label-hover";

export interface MapLabel {
  id: string;
  box: LabelBox;
  priority: number;
  protected?: boolean;
  alternatives?: LabelBox[];
}
export interface LabelSegment { a: { x: number; y: number }; b: { x: number; y: number } }

function padded(box: LabelBox, gap: number): LabelBox {
  return { x1: box.x1 - gap / 2, x2: box.x2 + gap / 2, y1: box.y1 - gap / 2, y2: box.y2 + gap / 2, w: box.w + gap, h: box.h + gap };
}

/** Liang–Barsky clipping: true if a visible connector crosses the title. */
export function crossesLabel(box: LabelBox, { a, b }: LabelSegment): boolean {
  let lo = 0, hi = 1;
  const dx = b.x - a.x, dy = b.y - a.y;
  for (const [p, q] of [[-dx, a.x - box.x1], [dx, box.x2 - a.x], [-dy, a.y - box.y1], [dy, box.y2 - a.y]]) {
    if (p === 0) { if (q < 0) return false; continue; }
    const t = q / p;
    if (p < 0) lo = Math.max(lo, t); else hi = Math.min(hi, t);
    if (lo > hi) return false;
  }
  return true;
}

/** Place titles, never graph objects. Prefer open space over connector strokes. */
export function placeMapLabels(labels: readonly MapLabel[], bodies: readonly LabelBox[] = [], edges: readonly LabelSegment[] = [], gap = 4): Map<string, LabelBox> {
  const occupied: LabelBox[] = [];
  const placed = new Map<string, LabelBox>();
  const sorted = [...labels].sort((a, b) => Number(!!b.protected) - Number(!!a.protected) || b.priority - a.priority || a.id.localeCompare(b.id));
  for (const item of sorted) {
    const candidates = [item.box, ...(item.alternatives ?? [])].filter(box => {
      const room = padded(box, gap);
      return !occupied.some(other => boxesOverlap(other, room)) && !bodies.some(body => boxesOverlap(body, room));
    });
    // A label is never allowed to cover a glyph just to preserve the default
    // below-node anchor. Hover/selection remain readable even in a packed area.
    const chosen = candidates.find(box => !edges.some(edge => crossesLabel(padded(box, 2), edge))) ?? candidates[0] ?? (item.protected ? item.box : undefined);
    if (!chosen) continue;
    placed.set(item.id, chosen);
    occupied.push(padded(chosen, gap));
  }
  return placed;
}
