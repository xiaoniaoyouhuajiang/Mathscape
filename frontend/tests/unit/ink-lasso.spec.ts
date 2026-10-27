import { describe, expect, it } from "vitest";
import { lassoCoverage, lassoSelects, polygonContains } from "../../src/ink/lasso";

const square = [{ x: 0, y: 0 }, { x: 100, y: 0 }, { x: 100, y: 100 }, { x: 0, y: 100 }];
const line = (from: number, to: number, y = 50, n = 21) => Array.from({ length: n }, (_, i) => ({ x: from + ((to - from) * i) / (n - 1), y }));

describe("lasso majority rule (M4)", () => {
  it("polygonContains is an even-odd test on the closed polygon", () => {
    expect(polygonContains({ x: 50, y: 50 }, square)).toBe(true);
    expect(polygonContains({ x: 150, y: 50 }, square)).toBe(false);
    expect(polygonContains({ x: 50, y: 50 }, square.slice(0, 2))).toBe(false);
  });

  it("selects a stroke only when more than half of its samples are inside", () => {
    expect(lassoSelects(line(10, 90), square)).toBe(true);
    // 30% inside: a long stroke crossing the lasso is not grabbed any more.
    expect(lassoCoverage(line(70, 170), square)).toBeCloseTo(6 / 21);
    expect(lassoSelects(line(70, 170), square)).toBe(false);
    // 70% inside.
    expect(lassoSelects(line(-30, 70), square)).toBe(true);
    // Exactly half is not a majority.
    expect(lassoSelects([{ x: 50, y: 50 }, { x: 150, y: 50 }], square)).toBe(false);
  });

  it("is false for empty strokes and degenerate lassos", () => {
    expect(lassoSelects([], square)).toBe(false);
    expect(lassoSelects(line(10, 90), square.slice(0, 2))).toBe(false);
  });
});
