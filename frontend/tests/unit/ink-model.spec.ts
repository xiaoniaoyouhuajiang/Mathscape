import { describe, expect, it } from "vitest";
import trace from "../../../testdata/ink/p0/trace.json";
import expected from "../../../testdata/ink/p0/projection.json";
import invalid from "../../../testdata/ink/p0/invalid-cases.json";
import { parseInkDocument, projectInk } from "../../src/ink/model";

describe("P0 ink interchange", () => {
  it("replays the same visibility and history fixture as Rust", () => {
    const document = parseInkDocument(structuredClone(trace));
    const projection = projectInk(document);
    expect({ visible_stroke_ids: projection.visible.map(s => s.id), undo: projection.undo,
      redo: projection.redo, point_count: projection.pointCount }).toEqual(expected);
    expect(projection.strokes.map(s => s.status)).toEqual(["completed", "interrupted", "cancelled"]);
    expect(projection.strokes[1].points[0].pressure).toBeNull();
    expect(parseInkDocument(JSON.parse(JSON.stringify(document)))).toEqual(document);
  });
  it.each(invalid)("rejects shared invalid case: $name", testCase => {
    const value = structuredClone(trace) as unknown as Record<string, unknown>;
    const parent = (path: string) => {
      const keys = path.slice(1).split("/"), key = keys.pop()!;
      let object: Record<string, unknown> = value;
      for (const k of keys) object = object[k] as Record<string, unknown>;
      return { object, key };
    };
    for (const change of testCase.changes) { const { object, key } = parent(change.path); object[key] = change.value; }
    for (const path of testCase.removes) { const { object, key } = parent(path); delete object[key]; }
    expect(() => parseInkDocument(value)).toThrow();
  });
  it("rejects non-finite coordinates without changing the caller's input", () => {
    const value = structuredClone(trace); value.operations[0].stroke!.points[0].x = NaN;
    expect(() => parseInkDocument(value)).toThrow();
    expect(Number.isNaN(value.operations[0].stroke!.points[0].x)).toBe(true);
  });
});
