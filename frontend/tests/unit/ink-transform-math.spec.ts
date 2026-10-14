import { describe, expect, it } from "vitest";
import { handleScale, MAX_HANDLE_SCALE, MIN_HANDLE_SCALE } from "../../src/ink/transform-math";

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
    expect(handleScale(105, 100, 100)).toBeCloseTo(5);
    expect(handleScale(95, 100, 100)).toBeCloseTo(-5);
  });
});
