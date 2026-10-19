import { describe, expect, it } from "vitest";
import trace from "../../../testdata/ink/p0/trace.json";
import expected from "../../../testdata/ink/p0/projection.json";
import invalid from "../../../testdata/ink/p0/invalid-cases.json";
import snapshot from "../../../testdata/ink/p2/snapshot.json";
import legacy from "../../../testdata/ink/p2/legacy-fragments.json";
import { appendInk, inkId, parseInkDocument, parseInkSnapshotManifest, projectInk } from "../../src/ink/model";

describe("P0 ink interchange", () => {
  it("reads the same snapshot prefix and refuses a false visible-state summary", () => {
    const manifest = parseInkSnapshotManifest(structuredClone(snapshot));
    expect(projectInk(manifest.document).visible.map(stroke => stroke.id)).toEqual(["stroke-a","stroke-b"]);
    expect(() => parseInkSnapshotManifest({...structuredClone(snapshot),visible_stroke_ids:["stroke-a"]})).toThrow("可见笔迹摘要不一致");
    expect(() => parseInkSnapshotManifest({...structuredClone(snapshot),point_count:999})).toThrow("可见笔迹摘要不一致");
  });
  it("keeps legacy fragment snapshots readable without weakening version 2 summaries", () => {
    const manifest = parseInkSnapshotManifest(structuredClone(legacy));
    expect(projectInk(manifest.document).visible.map(stroke => stroke.id)).toEqual(["fragment-a"]);
    expect(manifest.visible_stroke_ids).toEqual([]);
    expect(() => parseInkSnapshotManifest({...structuredClone(legacy),version:"mathscape-ink-snapshot-2"})).toThrow("可见笔迹摘要不一致");
  });
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
  it("replays page creation, transform and partial erase as editable operations", () => {
    const document = parseInkDocument(structuredClone(trace));
    const page = { id: inkId(), width: 1200, height: 1600 };
    document.pages.push(page); appendInk(document, { kind: "page_add", page });
    appendInk(document, { kind: "transform", stroke_ids: ["stroke-a"], matrix: { a: 1, b: 0, c: 0, d: 1, e: 20, f: 30 } });
    const sourceOp = document.operations.find(op => op.kind === "stroke" && op.stroke.id === "stroke-a");
    const source = sourceOp?.kind === "stroke" ? sourceOp.stroke : undefined;
    expect(source).toBeDefined();
    const fragment = { ...source!, id: inkId(), points: source!.points.slice(1) };
    appendInk(document, { kind: "partial_erase", source_stroke_id: source!.id, fragments: [fragment] });
    const parsed = parseInkDocument(JSON.parse(JSON.stringify(document)));
    const projection = projectInk(parsed);
    expect(parsed.pages).toHaveLength(3);
    expect(projection.visible.map(stroke => stroke.id)).toContain(fragment.id);
    expect(projection.visible.map(stroke => stroke.id)).not.toContain(source!.id);
  });
});

describe("N3a restyle", () => {
  it("changes the colour of selected strokes and one undo restores it", () => {
    const document = parseInkDocument(structuredClone(trace));
    const before = projectInk(document).visible.find(stroke => stroke.id === "stroke-a")!.brush.color;
    appendInk(document, { kind: "restyle", stroke_ids: ["stroke-a", "stroke-c"], color: "#c62828" });
    const after = projectInk(document);
    expect(after.visible.map(stroke => stroke.brush.color)).toEqual(["#c62828", "#c62828"]);
    appendInk(document, { kind: "undo", target_command_id: document.operations.at(-1)!.command_id });
    expect(projectInk(document).visible.find(stroke => stroke.id === "stroke-a")!.brush.color).toBe(before);
  });

  it("lets a later partial erase inherit the restyled colour", () => {
    const document = parseInkDocument(structuredClone(trace));
    appendInk(document, { kind: "restyle", stroke_ids: ["stroke-a"], color: "#2e7d32" });
    const source = projectInk(document).visible.find(stroke => stroke.id === "stroke-a")!;
    const fragment = { ...structuredClone(source), id: "fragment-restyle", brush: { ...source.brush }, points: source.points.slice(0, Math.max(1, source.points.length - 1)) };
    appendInk(document, { kind: "partial_erase", source_stroke_id: "stroke-a", fragments: [fragment] });
    const visible = projectInk(document).visible;
    expect(visible.some(stroke => stroke.id === "stroke-a")).toBe(false);
    expect(visible.find(stroke => stroke.id === "fragment-restyle")!.brush.color).toBe("#2e7d32");
  });

  it("rejects width, bad colour, duplicates and unknown strokes", () => {
    const document = parseInkDocument(structuredClone(trace));
    expect(() => appendInk(document, { kind: "restyle", stroke_ids: ["stroke-a"], color: "#c62828", width: 4 } as never)).not.toThrow();
    // appendInk does not validate; projectInk / parseInkDocument do.
    expect(() => projectInk(document)).toThrow(/restyle\.width/);
    const again = parseInkDocument(structuredClone(trace));
    appendInk(again, { kind: "restyle", stroke_ids: ["stroke-a"], color: "red" });
    expect(() => projectInk(again)).toThrow(/restyle\.color/);
    const third = parseInkDocument(structuredClone(trace));
    appendInk(third, { kind: "restyle", stroke_ids: ["stroke-a", "stroke-a"], color: "#c62828" });
    expect(() => projectInk(third)).toThrow(/restyle/);
    const fourth = parseInkDocument(structuredClone(trace));
    appendInk(fourth, { kind: "restyle", stroke_ids: ["missing"], color: "#c62828" });
    expect(() => projectInk(fourth)).toThrow(/restyle/);
  });
});
