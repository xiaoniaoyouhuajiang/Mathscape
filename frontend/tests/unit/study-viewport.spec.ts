import { describe, expect, it } from "vitest";
import { fitStudyViewport, layoutStudyLanes, type StudyLaneLabel } from "../../src/graph/study-viewport";

describe("measured study reading lanes", () => {
  it("separates long debt captions, preserves row order and uses the available height", () => {
    const rows: StudyLaneLabel[] = [
      { id: "scope", lane: 0, order: -1, height: 38 },
      { id: "definition", lane: 0, order: 0, height: 44 },
      { id: "debt-a", lane: 2, order: 0, height: 100 },
      { id: "theory", lane: 2, order: 1, height: 24 },
      { id: "debt-b", lane: 2, order: 2, height: 110 },
    ];
    const positions = layoutStudyLanes(rows, 690, 460);
    for (const lane of [0, 2]) {
      const items = rows.filter(row => row.lane === lane);
      expect(positions.get(items[0].id)!.y - items[0].height / 2).toBe(0);
      const last = items.at(-1)!;
      expect(positions.get(last.id)!.y + last.height / 2).toBe(460);
      for (let i = 1; i < items.length; i++) {
        const previous = items[i - 1], current = items[i];
        expect(positions.get(current.id)!.y - current.height / 2 - positions.get(previous.id)!.y - previous.height / 2).toBeGreaterThanOrEqual(14);
      }
    }
  });
  it("continues a long outline below the page instead of shrinking text or colliding", () => {
    const rows: StudyLaneLabel[] = Array.from({ length: 15 }, (_, i) => ({ id: String(i), lane: 0, order: i, height: 50 }));
    const positions = layoutStudyLanes(rows, 690, 460);
    expect(positions.get("14")!.y).toBeGreaterThan(460);
    expect(positions.get("1")!.y - positions.get("0")!.y).toBe(64);
  });
});

describe("study camera with readable screen-space captions", () => {
  const box = { x1: -325, x2: 900, y1: -130, y2: 490 };
  const labels = [
    { x: -230, y: 20, left: 75, right: 75, top: 10, bottom: 40 },
    { x: 740, y: 240, left: 10, right: 210, top: 10, bottom: 50 },
  ];
  it("fits long right-orbit names on a narrow canvas without a clipping zoom floor", () => {
    const viewport = { x1: 20, x2: 600, y1: 130, y2: 580 };
    const { zoom, pan } = fitStudyViewport(box, labels, viewport);
    expect(zoom).toBeLessThan(0.78);
    for (const label of labels) {
      expect(label.x * zoom + pan.x - label.left).toBeGreaterThanOrEqual(viewport.x1 - 0.01);
      expect(label.x * zoom + pan.x + label.right).toBeLessThanOrEqual(viewport.x2 + 0.01);
    }
    expect(box.y1 * zoom + pan.y).toBeGreaterThanOrEqual(viewport.y1 - 0.01);
    expect(box.y2 * zoom + pan.y).toBeLessThanOrEqual(viewport.y2 + 0.01);
  });
  it("reserves a wrapped HUD's actual height and refits when the canvas grows", () => {
    const narrow = fitStudyViewport(box, labels, { x1: 20, x2: 600, y1: 190, y2: 500 });
    const wide = fitStudyViewport(box, labels, { x1: 20, x2: 1200, y1: 80, y2: 800 });
    expect(wide.zoom).toBeGreaterThan(narrow.zoom);
    expect(box.y1 * narrow.zoom + narrow.pan.y).toBeGreaterThanOrEqual(190 - 0.01);
    expect(wide.zoom).toBeLessThanOrEqual(1.04);
  });
});
