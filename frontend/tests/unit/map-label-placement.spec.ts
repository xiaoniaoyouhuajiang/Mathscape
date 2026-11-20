import { describe, expect, it } from "vitest";
import { crossesLabel, placeMapLabels, type MapLabel } from "../../src/graph/label-placement";

const box = (x: number, y: number) => ({ x1: x, y1: y, x2: x + 60, y2: y + 16, w: 60, h: 16 });
describe("label placement without moving graph objects", () => {
  it("moves a title above its cluster instead of printing over a glyph", () => {
    const item: MapLabel = { id: "file", priority: 60, box: box(0, 0), alternatives: [box(0, -40)] };
    expect(placeMapLabels([item], [box(20, 5)]).get("file")).toEqual(box(0, -40));
  });
  it("hides a low priority name when every adjacent slot contains a glyph", () => {
    expect(placeMapLabels([{ id: "span", priority: 10, box: box(0, 0) }], [box(0, 0)]).has("span")).toBe(false);
  });
  it("prefers an adjacent slot that is not crossed by a connector", () => {
    const item: MapLabel = { id: "file", priority: 60, box: box(0, 0), alternatives: [box(0, -40)] };
    expect(placeMapLabels([item], [], [{ a: { x: -10, y: 8 }, b: { x: 80, y: 8 } }]).get("file")).toEqual(box(0, -40));
  });
  it("distinguishes crossing, parallel and exterior line segments", () => {
    expect(crossesLabel(box(0, 0), { a: { x: 20, y: -8 }, b: { x: 20, y: 30 } })).toBe(true);
    expect(crossesLabel(box(0, 0), { a: { x: -10, y: 25 }, b: { x: 80, y: 25 } })).toBe(false);
    expect(crossesLabel(box(0, 0), { a: { x: 80, y: 0 }, b: { x: 100, y: 10 } })).toBe(false);
  });
});
