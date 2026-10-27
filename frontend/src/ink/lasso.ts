/** Lasso hit testing (M4), kept free of DOM so it can be unit tested. */
export type LassoPoint = { x: number; y: number };

/** Even-odd point-in-polygon test; the polygon is implicitly closed. */
export function polygonContains(p: LassoPoint, polygon: readonly LassoPoint[]): boolean {
  let inside = false;
  for (let i = 0, j = polygon.length - 1; i < polygon.length; j = i++) {
    const a = polygon[i], b = polygon[j];
    if ((a.y > p.y) !== (b.y > p.y) && p.x < ((b.x - a.x) * (p.y - a.y)) / (b.y - a.y) + a.x) inside = !inside;
  }
  return inside;
}

/** Share of a stroke's samples that lie inside the lasso (0 for empty input). */
export function lassoCoverage(points: readonly LassoPoint[], polygon: readonly LassoPoint[]): number {
  if (!points.length || polygon.length < 3) return 0;
  let inside = 0;
  for (const point of points) if (polygonContains(point, polygon)) inside++;
  return inside / points.length;
}

/**
 * M4: a stroke is lassoed when most of its samples (strictly more than half)
 * are inside. The old rule selected a stroke as soon as one sample was inside,
 * so a lasso around one formula also grabbed every long stroke crossing it.
 */
export function lassoSelects(points: readonly LassoPoint[], polygon: readonly LassoPoint[]): boolean {
  return lassoCoverage(points, polygon) > 0.5;
}
