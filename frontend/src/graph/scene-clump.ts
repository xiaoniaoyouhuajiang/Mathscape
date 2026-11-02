/**
 * Map Scene Player — 临时收拢布局 (local clump), pure geometry.
 *
 * During a scene the focus nodes of a sparse cose cluster are pulled toward
 * their centroid so the few related nodes read as one compact topology in
 * the middle of the viewport. This module only computes target positions;
 * `scene-player.ts` snapshots the original coordinates, tweens to the
 * targets and tweens back on Settle. Nothing here ever reaches the
 * landscape graph or storage.
 *
 * Algorithm (predictable, no physics engine):
 * 1. Centroid `c` of the focus' original positions.
 * 2. Robust spread = median distance to `c`. When it already is within the
 *    target radius `R* = spacing·√n / 2` nothing moves (already compact).
 * 3. Otherwise every node moves radially: `r' = min(r·s, 2R*)` with
 *    `s = R* / median` — the shape (directions, order) is kept, outliers are
 *    pulled in harder.
 * 4. Each displacement is capped at `maxShift` (model units).
 * 5. A few rounds of pairwise separation push nodes closer than `minGap`
 *    apart (labels), re-capped against the original after each round.
 */

export interface Point {
  x: number;
  y: number;
}

export interface ClumpOptions {
  /** Target spacing between neighbours (model units). */
  spacing: number;
  /** Pairs closer than this are pushed apart (label room). */
  minGap: number;
  /** No node moves farther than this from its original position. */
  maxShift: number;
  /** Rounds of pairwise separation. */
  relaxRounds: number;
}

/**
 * Kill-switch and knobs. Flip `enabled` to turn clump off everywhere
 * (scenes then only dim + fit, as in the first slice).
 */
export const SCENE_CLUMP: ClumpOptions & { enabled: boolean; ms: number } = {
  enabled: true,
  /** Tween duration in and out (matches the camera fit). */
  ms: 420,
  spacing: 110,
  minGap: 72,
  maxShift: 520,
  relaxRounds: 12,
};

export function centroid(points: readonly Point[]): Point {
  if (points.length === 0) return { x: 0, y: 0 };
  let x = 0;
  let y = 0;
  for (const p of points) {
    x += p.x;
    y += p.y;
  }
  return { x: x / points.length, y: y / points.length };
}

/** `to`, or the point `max` along the way from `from` when it is farther. */
export function clampShift(from: Point, to: Point, max: number): Point {
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const d = Math.hypot(dx, dy);
  if (d <= max || d === 0) return { x: to.x, y: to.y };
  const k = max / d;
  return { x: from.x + dx * k, y: from.y + dy * k };
}

function median(values: number[]): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const mid = sorted.length >> 1;
  return sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
}

/** Target radius for `n` nodes at `spacing`. */
export function clumpRadius(n: number, spacing: number): number {
  return (spacing * Math.sqrt(Math.max(n, 1))) / 2;
}

/**
 * Target positions for the focus (`origin` = original model positions).
 * Returns only the nodes that move; an empty map means "leave as is"
 * (fewer than two nodes, or already compact).
 */
export function clumpTargets(
  origin: ReadonlyMap<string, Point>,
  options: ClumpOptions = SCENE_CLUMP,
): Map<string, Point> {
  const ids = [...origin.keys()];
  const out = new Map<string, Point>();
  if (ids.length < 2) return out;
  const points = ids.map((id) => origin.get(id)!);
  const c = centroid(points);
  const radii = points.map((p) => Math.hypot(p.x - c.x, p.y - c.y));
  const target = clumpRadius(ids.length, options.spacing);
  const spread = median(radii);
  if (spread <= target) return out;
  const s = target / spread;
  const limit = 2 * target;
  const next = points.map((p, index) => {
    const r = radii[index];
    if (r === 0) return { x: p.x, y: p.y };
    const r2 = Math.min(r * s, limit);
    const k = r2 / r;
    const to = { x: c.x + (p.x - c.x) * k, y: c.y + (p.y - c.y) * k };
    return clampShift(p, to, options.maxShift);
  });
  separate(next, points, options);
  ids.forEach((id, index) => {
    const p = next[index];
    const o = points[index];
    if (Math.abs(p.x - o.x) > 0.5 || Math.abs(p.y - o.y) > 0.5) out.set(id, p);
  });
  return out;
}

/** Push pairs closer than `minGap` apart; stay within `maxShift` of origin. */
function separate(points: Point[], origin: readonly Point[], options: ClumpOptions): void {
  const gap = options.minGap;
  for (let round = 0; round < options.relaxRounds; round += 1) {
    let moved = false;
    for (let i = 0; i < points.length; i += 1) {
      for (let j = i + 1; j < points.length; j += 1) {
        const a = points[i];
        const b = points[j];
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let d = Math.hypot(dx, dy);
        if (d >= gap) continue;
        if (d < 1e-6) {
          // Coincident: split along a fixed, index-derived direction.
          const angle = ((i * 7 + j * 13) % 16) * (Math.PI / 8);
          dx = Math.cos(angle);
          dy = Math.sin(angle);
          d = 1;
        }
        const push = (gap - Math.max(d, 1e-6)) / 2;
        const ux = dx / d;
        const uy = dy / d;
        a.x -= ux * push;
        a.y -= uy * push;
        b.x += ux * push;
        b.y += uy * push;
        moved = true;
      }
    }
    for (let i = 0; i < points.length; i += 1) {
      const p = clampShift(origin[i], points[i], options.maxShift);
      points[i].x = p.x;
      points[i].y = p.y;
    }
    if (!moved) break;
  }
}

/**
 * A satellite (semantic-mode junction, ink-note node) follows the average
 * displacement of its neighbours; neighbours that stay put count as zero.
 */
export function satelliteShift(neighbours: readonly (Point | null)[]): Point {
  if (neighbours.length === 0) return { x: 0, y: 0 };
  let x = 0;
  let y = 0;
  for (const d of neighbours) {
    if (!d) continue;
    x += d.x;
    y += d.y;
  }
  return { x: x / neighbours.length, y: y / neighbours.length };
}

/** Ease-in-out cubic on [0,1]. */
export function ease(t: number): number {
  const u = Math.min(1, Math.max(0, t));
  return u < 0.5 ? 4 * u * u * u : 1 - Math.pow(-2 * u + 2, 3) / 2;
}

export function lerp(a: Point, b: Point, t: number): Point {
  return { x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t };
}
