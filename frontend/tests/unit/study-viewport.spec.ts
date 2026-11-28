import { describe, expect, it } from "vitest";
import { layoutStudyLanes, type StudyLaneLabel } from "../../src/graph/study-viewport";

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
