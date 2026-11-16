// Phase1d-b: the Rust renderer (crates/mathscape-ink-raster) ports
// perfect-freehand@1.2.3 and `projectInk`. This spec owns the shared fixture
// both sides replay: it rebuilds a deterministic document, projects it with
// the real frontend code and outlines every visible stroke exactly like
// `canvas.ts::drawInkStroke`. Regenerate with UPDATE_INK_RASTER_FIXTURE=1.
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { getStroke } from "perfect-freehand";
import { describe, expect, it } from "vitest";
import { parseInkDocument, projectInk, type InkDocument, type InkOperation, type InkPoint, type InkStroke } from "../../src/ink/model";

const FIXTURE = fileURLToPath(new URL("../../../testdata/ink/raster/projection-outlines.json", import.meta.url));

function mulberry32(seed: number): () => number {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function buildDocument(): InkDocument {
  const random = mulberry32(20261028);
  const pages = [{ id: "page-a", width: 1200, height: 1600 }, { id: "page-b", width: 800, height: 600 }];
  let time = 0;
  const point = (x: number, y: number, pressure: number | null): InkPoint =>
    ({ x, y, t_us: (time += 8000), pressure, tilt_x_deg: null, tilt_y_deg: null, buttons: 1 });
  const stroke = (id: string, page: string, points: InkPoint[], brush: Partial<InkStroke["brush"]> = {}): InkStroke => ({
    id, page_id: page, capture_segment_id: "segment-1", pointer_type: "pen", status: "completed",
    brush: { renderer: "perfect-freehand@1.2.3", color: "#1f2933", width: 4, smoothing: 0.35, streamline: 0.15, thinning: 0.4, simulate_pressure: false, ...brush },
    points,
  });
  const curve = (cx: number, cy: number, n: number, radius: number, pressure: boolean): InkPoint[] =>
    Array.from({ length: n }, (_, i) => point(
      cx + radius * Math.cos(i / 6) + random() * 3,
      cy + radius * 0.6 * Math.sin(i / 4) + i * 1.7,
      pressure ? 0.2 + 0.7 * random() : null,
    ));
  const zigzag = (x: number, y: number, n: number): InkPoint[] =>
    Array.from({ length: n }, (_, i) => point(x + i * 9, y + (i % 2 ? 40 : 0) + random() * 0.5, 0.5));
  const strokes: InkStroke[] = [
    stroke("dot", "page-a", [point(100, 100, null)]),
    stroke("dot-pressure", "page-a", [point(140, 100, 0.9)], { width: 9, thinning: 0.8 }),
    stroke("pair", "page-a", [point(180, 100, 0.3), point(230, 130, 0.7)]),
    stroke("triple", "page-a", [point(260, 100, null), point(262, 101, null), point(300, 160, null)]),
    stroke("curve", "page-a", curve(400, 300, 80, 120, false)),
    stroke("curve-pressure", "page-a", curve(700, 500, 120, 90, true), { width: 7, thinning: 0.6, color: "#c0392b" }),
    stroke("zigzag", "page-a", zigzag(120, 700, 24), { width: 5, thinning: 0, smoothing: 0, streamline: 0 }),
    stroke("negative-thinning", "page-a", curve(300, 1000, 60, 60, true), { width: 12, thinning: -0.5, smoothing: 1, streamline: 1 }),
    stroke("still", "page-a", [point(500, 900, 0.5), point(500, 900, 0.5), point(500, 900, 0.6), point(500.2, 900.1, 0.6)]),
    stroke("thin", "page-a", curve(900, 1200, 40, 30, false), { width: 0.5 }),
    stroke("wide", "page-b", curve(400, 300, 50, 150, true), { width: 40, thinning: 1, smoothing: 0.9, streamline: 0.9 }),
    stroke("erased", "page-a", curve(200, 1400, 20, 40, false)),
    stroke("split-source", "page-a", curve(800, 200, 60, 70, false)),
    stroke("undone", "page-a", curve(1000, 900, 20, 20, false)),
  ];
  const fragments = [
    stroke("fragment-1", "page-a", curve(760, 180, 25, 40, false)),
    stroke("fragment-2", "page-a", curve(860, 260, 30, 45, false)),
  ];
  const ops: Array<Record<string, unknown>> = [];
  const push = (kind: string, body: Record<string, unknown>) => {
    ops.push({ kind, seq: ops.length + 1, command_id: `cmd-${ops.length + 1}`, ...body });
    return `cmd-${ops.length}`;
  };
  for (const s of strokes.slice(0, 8)) push("stroke", { stroke: s });
  push("transform", { stroke_ids: ["curve", "pair"], matrix: { a: Math.cos(0.3), b: Math.sin(0.3), c: -Math.sin(0.3), d: Math.cos(0.3), e: 35.5, f: -12.25 } });
  push("restyle", { stroke_ids: ["curve", "zigzag"], color: "#2E86de" });
  push("page_add", { page: pages[1] });
  for (const s of strokes.slice(8)) push("stroke", { stroke: s });
  push("erase", { stroke_ids: ["erased"] });
  push("transform", { stroke_ids: ["erased", "wide"], matrix: { a: 1.25, b: 0, c: 0.1, d: 0.8, e: -20, f: 40 } });
  push("partial_erase", { source_stroke_id: "split-source", fragments });
  push("transform", { stroke_ids: ["fragment-2"], matrix: { a: 1, b: 0, c: 0, d: 1, e: 15, f: 5 } });
  const undone = push("restyle", { stroke_ids: ["undone"], color: "#00ff00" });
  push("undo", { target_command_id: undone });
  push("redo", { target_command_id: undone });
  const transformUndo = push("transform", { stroke_ids: ["undone"], matrix: { a: 2, b: 0, c: 0, d: 2, e: 0, f: 0 } });
  push("undo", { target_command_id: transformUndo });
  return parseInkDocument({
    version: "mathscape-ink-lab-1", document_id: "raster-fixture", title: "raster fixture",
    created_at: "2026-10-28T00:00:00.000Z", exported_at: "2026-10-28T00:00:00.000Z", pages,
    capture_segments: [{ id: "segment-1", started_at: "2026-10-28T00:00:00.000Z", user_agent: "fixture", secure_context: true,
      coalesced_events: true, pointer_raw_update: false, device_pixel_ratio: 2, time_origin_ms: 0,
      pressure_capability: "unknown", tilt_capability: "unknown" }],
    operations: ops as unknown as InkOperation[],
  });
}

function outline(stroke: InkStroke): number[][] {
  // Same call as canvas.ts::drawInkStroke for a settled stroke.
  return getStroke(stroke.points.map(p => [p.x, p.y, p.pressure ?? 0.5]), {
    size: stroke.brush.width, thinning: stroke.brush.thinning, smoothing: stroke.brush.smoothing,
    streamline: stroke.brush.streamline, simulatePressure: false, last: true,
  });
}

function fixture() {
  const document = buildDocument();
  const visible = projectInk(document).visible;
  return {
    generator: "frontend/tests/unit/ink-raster-fixture.spec.ts (perfect-freehand@1.2.3)",
    document,
    visible: visible.map(stroke => ({ id: stroke.id, page_id: stroke.page_id, color: stroke.brush.color, points: stroke.points.map(p => [p.x, p.y]) })),
    outlines: visible.map(stroke => ({ id: stroke.id, outline: outline(stroke) })),
  };
}

function expectFixtureClose(actual: unknown, expected: unknown, path = "fixture"): void {
  if (typeof actual === "number" && typeof expected === "number") {
    const tolerance = 1e-9 * Math.max(1, Math.abs(actual), Math.abs(expected));
    expect(Math.abs(actual - expected), path).toBeLessThanOrEqual(tolerance);
    return;
  }
  if (Array.isArray(actual) || Array.isArray(expected)) {
    expect(Array.isArray(actual), path).toBe(true);
    expect(Array.isArray(expected), path).toBe(true);
    const a = actual as unknown[];
    const e = expected as unknown[];
    expect(a.length, path).toBe(e.length);
    for (let i = 0; i < a.length; i++) expectFixtureClose(a[i], e[i], `${path}[${i}]`);
    return;
  }
  if (actual !== null && expected !== null && typeof actual === "object" && typeof expected === "object") {
    const a = actual as Record<string, unknown>;
    const e = expected as Record<string, unknown>;
    expect(Object.keys(a).sort(), path).toEqual(Object.keys(e).sort());
    for (const key of Object.keys(e)) expectFixtureClose(a[key], e[key], `${path}.${key}`);
    return;
  }
  expect(actual, path).toEqual(expected);
}

describe("Phase1d-b ink raster fixture", () => {
  it("matches the committed projection and perfect-freehand outlines", () => {
    const actual = JSON.parse(JSON.stringify(fixture()));
    if (process.env.UPDATE_INK_RASTER_FIXTURE === "1") writeFileSync(FIXTURE, `${JSON.stringify(actual)}\n`);
    expectFixtureClose(actual, JSON.parse(readFileSync(FIXTURE, "utf8")));
    expect(actual.visible.map((s: { id: string }) => s.id)).toEqual([
      "dot", "dot-pressure", "pair", "triple", "curve", "curve-pressure", "zigzag", "negative-thinning",
      "still", "thin", "wide", "undone", "fragment-1", "fragment-2",
    ]);
  });
});
