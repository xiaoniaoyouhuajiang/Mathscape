/** Smallest and largest per-axis scale factor a corner-handle drag may produce. */
export const MIN_HANDLE_SCALE = 0.05;
export const MAX_HANDLE_SCALE = 20;
/** Smallest span (page units) used as the denominator of a corner-handle ratio. */
export const MIN_HANDLE_SPAN = 1;

/** Outward direction of a corner handle on one axis (nw grows towards −x and −y). */
export type HandleDirection = -1 | 1;
export function cornerDirections(handle: "nw" | "ne" | "sw" | "se"): { x: HandleDirection; y: HandleDirection } {
  return { x: handle === "nw" || handle === "sw" ? -1 : 1, y: handle === "nw" || handle === "ne" ? -1 : 1 };
}

function clampScale(ratio: number): number {
  if (!Number.isFinite(ratio)) return 1;
  const magnitude = Math.min(MAX_HANDLE_SCALE, Math.max(MIN_HANDLE_SCALE, Math.abs(ratio)));
  return ratio < 0 ? -magnitude : magnitude;
}

/**
 * Per-axis scale factor for a corner-handle drag (H1).
 *
 * `edge` is the selection edge the handle belongs to, `anchor` the opposite
 * edge that stays fixed, `start` where the pointer went down and `point` where
 * it is now, all on the same axis. The pointer delta moves the edge, so the
 * scale is exactly 1 at the grab point no matter where inside the handle's hit
 * area the user pressed. The denominator keeps the handle's outward sign (the
 * old `Math.max(1, start - anchor)` collapsed every negative span to 1, which
 * blew nw/ne/sw drags up hundreds of times). When |edge − anchor| is below
 * MIN_HANDLE_SPAN the ratio is `1 + direction·Δ / MIN_HANDLE_SPAN`, so the
 * scale is still exactly 1 at the grab point. Dragging past the anchor mirrors; the magnitude is
 * clamped to [MIN_HANDLE_SCALE, MAX_HANDLE_SCALE].
 */
export function cornerScale(point: number, start: number, edge: number, anchor: number, direction: HandleDirection): number {
  const span = edge - anchor;
  if (Math.abs(span) >= MIN_HANDLE_SPAN) return clampScale((span + point - start) / (direction * Math.abs(span)));
  // Near-degenerate selection: measure the drag against MIN_HANDLE_SPAN in the
  // handle's outward direction, so the grab point is still exactly 1 (a 0 or
  // sub-unit span must not snap to MIN_HANDLE_SCALE on press). Continuous with
  // the branch above at |span| === MIN_HANDLE_SPAN.
  return clampScale(1 + (direction * (point - start)) / MIN_HANDLE_SPAN);
}

/**
 * Same ratio when the grab point is taken as the edge (`start === edge`). The
 * sign of `start - anchor` gives the direction unless one is passed.
 */
export function handleScale(point: number, anchor: number, start: number, direction?: HandleDirection): number {
  const span = start - anchor;
  return cornerScale(point, start, start, anchor, direction ?? (span < 0 ? -1 : 1));
}
