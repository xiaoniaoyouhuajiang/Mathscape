/** Smallest and largest per-axis scale factor a corner-handle drag may produce. */
export const MIN_HANDLE_SCALE = 0.05;
export const MAX_HANDLE_SCALE = 20;

/**
 * Per-axis scale factor for a corner-handle drag.
 *
 * `start` is where the drag began (the grabbed corner), `anchor` the opposite
 * corner that stays fixed and `point` the current pointer position, all on the
 * same axis. The ratio keeps its sign so dragging past the anchor mirrors the
 * selection. The denominator is guarded against zero while keeping its sign
 * (the old `Math.max(1, start - anchor)` collapsed negative spans to 1, which
 * blew the selection up hundreds of times when dragging the nw/ne/sw handles).
 */
export function handleScale(point: number, anchor: number, start: number): number {
  const span = start - anchor;
  const denominator = Math.abs(span) < 1 ? (span < 0 ? -1 : 1) : span;
  const ratio = (point - anchor) / denominator;
  const magnitude = Math.min(MAX_HANDLE_SCALE, Math.max(MIN_HANDLE_SCALE, Math.abs(ratio)));
  return ratio < 0 ? -magnitude : magnitude;
}
