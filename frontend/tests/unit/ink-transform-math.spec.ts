import { describe, expect, it } from "vitest";
import { cornerDirections, cornerScale, handleScale, MAX_HANDLE_SCALE, MIN_HANDLE_SCALE } from "../../src/ink/transform-math";

// Selection bounds used below: x in [100, 300], y in [50, 150].
describe("handleScale (corner-handle resize)", () => {
  it("se handle: dragging outward doubles the size", () => {
    expect(handleScale(500, 100, 300)).toBeCloseTo(2);
    expect(handleScale(250, 50, 150)).toBeCloseTo(2);
  });

  it("nw handle: anchor at max corner, dragging outward doubles the size", () => {
    // start at minX=100, anchor maxX=300; pointer moves to -100 (span 400 vs 200)
    expect(handleScale(-100, 300, 100)).toBeCloseTo(2);
    expect(handleScale(-50, 150, 50)).toBeCloseTo(2);
  });

  it("nw/ne/sw handles: a tiny drag keeps the scale near 1 (regression for the x-hundreds blow-up)", () => {
    expect(handleScale(99, 300, 100)).toBeCloseTo(201 / 200);
    expect(handleScale(101, 300, 100)).toBeCloseTo(199 / 200);
  });

  it("dragging past the anchor mirrors the selection", () => {
    expect(handleScale(500, 300, 100)).toBeCloseTo(-1);
    expect(handleScale(-100, 100, 300)).toBeCloseTo(-1);
  });

  it("clamps magnitude to the allowed range", () => {
    expect(handleScale(300, 300, 100)).toBe(MIN_HANDLE_SCALE);
    expect(handleScale(100_000, 100, 300)).toBe(MAX_HANDLE_SCALE);
    expect(handleScale(-100_000, 300, 100)).toBe(MAX_HANDLE_SCALE);
  });

  it("degenerate zero-width selection does not divide by zero and keeps the drag direction", () => {
    expect(Number.isFinite(handleScale(105, 100, 100))).toBe(true);
    // Zero span: 1 + Δ / MIN_HANDLE_SPAN, i.e. 1 at the grab point.
    expect(handleScale(105, 100, 100)).toBeCloseTo(6);
    expect(handleScale(95, 100, 100)).toBeCloseTo(-4);
    expect(handleScale(100, 100, 100)).toBe(1);
  });
});

describe("cornerScale (H1, grab offset and handle direction)", () => {
  // Selection x in [100, 300]; each handle's edge and the opposite anchor.
  const corners = {
    nw: { edge: 100, anchor: 300, dir: -1 as const },
    se: { edge: 300, anchor: 100, dir: 1 as const },
  };

  it("is exactly 1 at the grab point wherever inside the handle the pointer went down", () => {
    for (const { edge, anchor, dir } of Object.values(corners)) {
      for (const offset of [-18, -6, 0, 6, 18]) expect(cornerScale(edge + offset, edge + offset, edge, anchor, dir)).toBeCloseTo(1);
    }
  });

  it("follows the pointer delta: 200 px outward doubles a 200 px selection on every corner", () => {
    expect(cornerScale(-110, 90, 100, 300, -1)).toBeCloseTo(2);
    expect(cornerScale(510, 310, 300, 100, 1)).toBeCloseTo(2);
    for (const [handle, expected] of [["nw", { x: -1, y: -1 }], ["ne", { x: 1, y: -1 }], ["sw", { x: -1, y: 1 }], ["se", { x: 1, y: 1 }]] as const) {
      expect(cornerDirections(handle)).toEqual(expected);
    }
  });

  it("a near-degenerate selection still grows when dragged outward from the far side of the anchor", () => {
    // 0.4-wide selection [100, 100.4]; the nw grab lands right of the anchor.
    const scale = cornerScale(80, 103, 100, 100.4, -1);
    expect(scale).toBeGreaterThan(1);
    expect(scale).toBeLessThanOrEqual(MAX_HANDLE_SCALE);
    // Same press with the old start-based ratio would have mirrored the selection.
    expect(handleScale(80, 100.4, 103)).toBeLessThan(0);
  });

  it("is exactly 1 at the grab point for zero, sub-unit and normal spans", () => {
    for (const dir of [-1, 1] as const) {
      for (const span of [0, 0.5, -0.5, 0.999]) {
        const anchor = 100;
        const edge = anchor + span;
        for (const start of [edge - 6, edge, edge + 6]) expect(cornerScale(start, start, edge, anchor, dir)).toBe(1);
      }
    }
    expect(cornerScale(310, 310, 300, 100, 1)).toBe(1);
    expect(cornerScale(90, 90, 100, 300, -1)).toBe(1);
  });

  it("sub-unit spans still scale smoothly with the drag, in the handle's direction", () => {
    // se-like handle on a 0.5-wide selection: outward grows, inward shrinks then mirrors.
    expect(cornerScale(100.6, 100.5, 100.5, 100, 1)).toBeCloseTo(1.1);
    expect(cornerScale(100.4, 100.5, 100.5, 100, 1)).toBeCloseTo(0.9);
    expect(cornerScale(98.5, 100.5, 100.5, 100, 1)).toBeCloseTo(-1);
    // nw-like handle on a zero-width selection: moving towards −x grows.
    expect(cornerScale(99.5, 100, 100, 100, -1)).toBeCloseTo(1.5);
    // Continuous at |span| === MIN_HANDLE_SPAN.
    expect(cornerScale(101.25, 101, 101, 100, 1)).toBeCloseTo(cornerScale(100.9999 + 0.25, 100.9999, 100.9999, 100, 1), 3);
  });

  it("mirrors past the anchor and keeps the clamp", () => {
    expect(cornerScale(500, 100, 100, 300, -1)).toBeCloseTo(-1);
    expect(cornerScale(1e6, 300, 300, 100, 1)).toBe(MAX_HANDLE_SCALE);
    expect(cornerScale(100, 300, 300, 100, 1)).toBe(MIN_HANDLE_SCALE);
    expect(cornerScale(Number.NaN, 0, 0, 0, 1)).toBe(1);
  });
});
