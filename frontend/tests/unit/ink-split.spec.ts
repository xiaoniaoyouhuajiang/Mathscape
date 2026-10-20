import { describe, expect, it } from "vitest";
import { clampInkSplitWidth, INK_SPLIT_MIN, INK_PANE_MIN } from "../../src/shell/ink-split";

describe("N3c ink split clamp", () => {
  it("keeps the 手稿 pane usable and clamps the reader", () => {
    expect(clampInkSplitWidth(500, 1200)).toBe(500);
    expect(clampInkSplitWidth(50, 1200)).toBe(INK_SPLIT_MIN);
    expect(clampInkSplitWidth(2000, 1200)).toBe(1200 - INK_PANE_MIN);
    expect(clampInkSplitWidth(700, 900)).toBe(900 - INK_PANE_MIN);
  });
});
