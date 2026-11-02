import { describe, expect, it } from "vitest";
import {
  SCENE_CLUMP,
  centroid,
  clampShift,
  clumpRadius,
  clumpTargets,
  ease,
  lerp,
  satelliteShift,
  type Point,
} from "../../src/graph/scene-clump";

const map = (entries: Record<string, Point>) => new Map(Object.entries(entries));
const dist = (a: Point, b: Point) => Math.hypot(a.x - b.x, a.y - b.y);
const opts = { spacing: 100, minGap: 60, maxShift: 400, relaxRounds: 12 };

describe("scene clump geometry", () => {
  it("centroid averages; empty is the origin", () => {
    expect(centroid([])).toEqual({ x: 0, y: 0 });
    expect(centroid([{ x: 0, y: 0 }, { x: 4, y: 2 }, { x: 2, y: 4 }])).toEqual({ x: 2, y: 2 });
  });

  it("clampShift caps the displacement along the same direction", () => {
    expect(clampShift({ x: 0, y: 0 }, { x: 3, y: 4 }, 10)).toEqual({ x: 3, y: 4 });
    expect(clampShift({ x: 0, y: 0 }, { x: 30, y: 40 }, 10)).toEqual({ x: 6, y: 8 });
    expect(clampShift({ x: 1, y: 1 }, { x: 1, y: 1 }, 0)).toEqual({ x: 1, y: 1 });
  });

  it("single node and an already compact focus do not move", () => {
    expect(clumpTargets(map({ a: { x: 900, y: 900 } }), opts).size).toBe(0);
    // Two nodes 100 apart: spread 50 ≤ R* (≈70.7).
    expect(clumpTargets(map({ a: { x: 0, y: 0 }, b: { x: 100, y: 0 } }), opts).size).toBe(0);
    expect(clumpRadius(4, 100)).toBe(100);
  });

  it("a sparse focus is pulled toward its centroid, shape kept, shift capped", () => {
    const origin = map({ a: { x: -1000, y: 0 }, b: { x: 1000, y: 0 }, c: { x: 0, y: 800 } });
    const c0 = centroid([...origin.values()]);
    const out = clumpTargets(origin, opts);
    expect(out.size).toBe(3);
    for (const [id, to] of out) {
      const from = origin.get(id)!;
      expect(dist(from, to)).toBeLessThanOrEqual(opts.maxShift + 1e-6);
      expect(dist(to, c0)).toBeLessThan(dist(from, c0));
    }
    // Left stays left of right (directions kept).
    expect(out.get("a")!.x).toBeLessThan(out.get("b")!.x);
  });

  it("without the cap the focus ends within 2·R* of the centroid", () => {
    const origin = map({ a: { x: -2000, y: 0 }, b: { x: 2000, y: 0 }, c: { x: 0, y: 2000 }, d: { x: 0, y: -2000 } });
    const out = clumpTargets(origin, { ...opts, maxShift: 1e9 });
    const limit = 2 * clumpRadius(4, opts.spacing);
    for (const to of out.values()) expect(dist(to, { x: 0, y: 0 })).toBeLessThanOrEqual(limit + 1e-6);
  });

  it("pairs keep at least minGap apart after clumping (label room)", () => {
    // A tight trio and one far outlier: the outlier drives the pull, the trio must not collapse.
    const origin = map({
      a: { x: 0, y: 0 },
      b: { x: 30, y: 0 },
      c: { x: 0, y: 30 },
      d: { x: 3000, y: 3000 },
      e: { x: -3000, y: 3000 },
    });
    const out = clumpTargets(origin, { ...opts, maxShift: 1e9 });
    const final = new Map([...origin].map(([id, p]) => [id, out.get(id) ?? p]));
    const pts = [...final.values()];
    for (let i = 0; i < pts.length; i += 1) {
      for (let j = i + 1; j < pts.length; j += 1) expect(dist(pts[i], pts[j])).toBeGreaterThanOrEqual(opts.minGap - 0.5);
    }
  });

  it("coincident nodes are split deterministically", () => {
    const origin = map({ a: { x: 0, y: 0 }, b: { x: 0, y: 0 }, c: { x: 5000, y: 0 } });
    const one = clumpTargets(origin, opts);
    const two = clumpTargets(origin, opts);
    expect([...one]).toEqual([...two]);
    const a = one.get("a") ?? { x: 0, y: 0 };
    const b = one.get("b") ?? { x: 0, y: 0 };
    expect(dist(a, b)).toBeGreaterThan(1);
  });

  it("satellites follow the average neighbour shift; still neighbours count as zero", () => {
    expect(satelliteShift([])).toEqual({ x: 0, y: 0 });
    expect(satelliteShift([{ x: 10, y: 0 }, { x: 0, y: 10 }])).toEqual({ x: 5, y: 5 });
    expect(satelliteShift([{ x: 10, y: 0 }, null])).toEqual({ x: 5, y: 0 });
  });

  it("ease / lerp are bounded and hit their ends", () => {
    expect(ease(0)).toBe(0);
    expect(ease(1)).toBe(1);
    expect(ease(2)).toBe(1);
    expect(ease(0.5)).toBeCloseTo(0.5);
    expect(lerp({ x: 0, y: 0 }, { x: 10, y: 20 }, 0.5)).toEqual({ x: 5, y: 10 });
  });

  it("is on by default and in step with the camera fit", () => {
    expect(SCENE_CLUMP.enabled).toBe(true);
    expect(SCENE_CLUMP.ms).toBeGreaterThan(0);
  });
});
