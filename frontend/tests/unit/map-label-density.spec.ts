import { describe, expect, it } from "vitest";
import { visibleMapLabels, type MapLabel } from "../../src/graph/label-density";

function label(id: string, x: number, priority = 0, protectedLabel = false): MapLabel {
  return { id, priority, protected: protectedLabel, box: { x1: x, x2: x + 60, y1: 0, y2: 12, w: 60, h: 12 } };
}

describe("large-map names", () => {
  it("keeps interaction targets ahead of higher-priority overlapping names", () => {
    const labels = [label("candidate", 0, 100), label("hovered", 20, 0, true), label("distant", 150)];
    expect([...visibleMapLabels(labels)]).toEqual(["hovered", "distant"]);
  });

  it("gives state and material names space without discarding distant unit names", () => {
    const labels = [label("unit", 0, 40), label("debt", 20, 100), label("material", 120, 60), label("other-unit", 230, 40)];
    expect(visibleMapLabels(labels)).toEqual(new Set(["debt", "material", "other-unit"]));
  });

  it("is deterministic when projection insertion order changes", () => {
    const labels = [label("b", 0), label("a", 10), label("c", 180)];
    expect(visibleMapLabels(labels)).toEqual(visibleMapLabels([...labels].reverse()));
    expect(visibleMapLabels(labels)).toEqual(new Set(["a", "c"]));
  });

  it("uses a screen-space breathing gap and restores names when spread out", () => {
    expect(visibleMapLabels([label("a", 0), label("b", 63)])).toEqual(new Set(["a"]));
    expect(visibleMapLabels([label("a", 0), label("b", 66)])).toEqual(new Set(["a", "b"]));
  });
});
