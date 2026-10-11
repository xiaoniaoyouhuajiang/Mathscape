// P0 interchange contract. Rust's InkLabDocument owns the matching schema.
export const INK_LAB_VERSION = "mathscape-ink-lab-1";
export const MAX_INK_BYTES = 64 * 1024 * 1024;
export const MAX_INK_POINTS = 500_000;
export const MAX_INK_OPERATIONS = 20_000;
export interface InkPage { id: string; width: number; height: number }
export interface CaptureSegment {
  id: string; started_at: string; user_agent: string; secure_context: boolean;
  coalesced_events: boolean; pointer_raw_update: boolean; device_pixel_ratio: number;
  time_origin_ms: number; pressure_capability: "unknown"; tilt_capability: "unknown";
}
export interface InkPoint {
  x: number; y: number; t_us: number; pressure: number | null;
  tilt_x_deg: number | null; tilt_y_deg: number | null; buttons: number;
}
export interface InkBrush {
  renderer: "perfect-freehand@1.2.3"; color: string; width: number;
  smoothing: number; streamline: number; thinning: number; simulate_pressure: false;
}
export interface InkStroke {
  id: string; page_id: string; capture_segment_id: string;
  pointer_type: "pen" | "mouse" | "touch" | "unknown";
  status: "completed" | "cancelled" | "interrupted";
  brush: InkBrush; points: InkPoint[];
}
export interface InkTransform { a: number; b: number; c: number; d: number; e: number; f: number }
type Command = { seq: number; command_id: string };
export type InkOperation = Command & (
  { kind: "stroke"; stroke: InkStroke } |
  { kind: "erase"; stroke_ids: string[] } |
  { kind: "partial_erase"; source_stroke_id: string; fragments: InkStroke[] } |
  { kind: "transform"; stroke_ids: string[]; matrix: InkTransform } |
  { kind: "page_add"; page: InkPage } |
  { kind: "undo" | "redo"; target_command_id: string }
);
export type InkAppendCommand =
  | { kind: "stroke"; stroke: InkStroke }
  | { kind: "erase"; stroke_ids: string[] }
  | { kind: "partial_erase"; source_stroke_id: string; fragments: InkStroke[] }
  | { kind: "transform"; stroke_ids: string[]; matrix: InkTransform }
  | { kind: "page_add"; page: InkPage }
  | { kind: "undo" | "redo"; target_command_id: string };
export interface InkDocument {
  version: typeof INK_LAB_VERSION; document_id: string; title: string;
  created_at: string; exported_at: string; pages: InkPage[];
  capture_segments: CaptureSegment[]; operations: InkOperation[];
}
export type InkDocumentMetadata = Pick<InkDocument, "version" | "document_id" | "title" | "created_at" | "exported_at" | "pages" | "capture_segments">;
export function inkMetadata(document: InkDocument): InkDocumentMetadata {
  return {
    version: document.version,
    document_id: document.document_id,
    title: document.title,
    created_at: document.created_at,
    exported_at: document.exported_at,
    pages: structuredClone(document.pages),
    capture_segments: structuredClone(document.capture_segments),
  };
}
export interface InkProjection {
  strokes: InkStroke[]; visible: InkStroke[]; undo: string[]; redo: string[];
  pointCount: number;
}

export function inkId(): string {
  // getRandomValues is available on LAN HTTP, unlike randomUUID.
  return Array.from(crypto.getRandomValues(new Uint8Array(16)), b => b.toString(16).padStart(2, "0")).join("");
}
export function newDocument(title = "数学手稿"): InkDocument {
  const now = new Date().toISOString();
  return { version: INK_LAB_VERSION, document_id: inkId(), title, created_at: now,
    exported_at: now, pages: [{ id: inkId(), width: 1200, height: 1600 }],
    capture_segments: [], operations: [] };
}

function fail(message: string): never { throw new Error(`Ink 文件无效：${message}`); }
function object(value: unknown, keys: string[], at: string): Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) fail(`${at} 必须是对象`);
  const result = value as Record<string, unknown>;
  if (Object.keys(result).some(k => !keys.includes(k)) || keys.some(k => !(k in result))) fail(`${at} 字段不符合契约`);
  return result;
}
function text(value: unknown, at: string, max = 200): asserts value is string {
  if (typeof value !== "string" || !value.trim() || value.length > max || /[\x00-\x1f\x7f]/.test(value)) fail(`${at} 必须是有界非空文本`);
}
function number(value: unknown, min: number, max: number, at: string): asserts value is number {
  if (typeof value !== "number" || !Number.isFinite(value) || value < min || value > max) fail(`${at} 数值越界`);
}
function integer(value: unknown, min: number, max: number, at: string): asserts value is number {
  number(value, min, max, at); if (!Number.isSafeInteger(value)) fail(`${at} 必须是安全整数`);
}
function nullable(value: unknown, min: number, max: number, at: string): void {
  if (value !== null) number(value, min, max, at);
}
function array(value: unknown, min: number, max: number, at: string): asserts value is unknown[] {
  if (!Array.isArray(value) || value.length < min || value.length > max) fail(`${at} 长度越界`);
}
function uniqueId(value: unknown, seen: Set<string>, at: string): void {
  text(value, at); if (seen.has(value)) fail(`${at} 重复`); seen.add(value);
}
function validateTransform(value: unknown, at: string): InkTransform {
  const matrix = object(value, ["a", "b", "c", "d", "e", "f"], at);
  for (const key of ["a", "b", "c", "d", "e", "f"]) number(matrix[key], -1_000_000, 1_000_000, `${at}.${key}`);
  return matrix as unknown as InkTransform;
}
function validateStroke(value: unknown, pages: Set<string>, segments: Set<string>, strokes: Set<string>, total: { value: number }, at: string): InkStroke {
  const s = object(value, ["id", "page_id", "capture_segment_id", "pointer_type", "status", "brush", "points"], at);
  uniqueId(s.id, strokes, `${at}.id`);
  text(s.page_id, `${at}.page_id`); text(s.capture_segment_id, `${at}.capture_segment_id`);
  if (!pages.has(s.page_id) || !segments.has(s.capture_segment_id)) fail(`${at} 来源页或采集片段不存在`);
  if (typeof s.pointer_type !== "string" || !["pen", "mouse", "touch", "unknown"].includes(s.pointer_type)) fail(`${at}.pointer_type 无效`);
  if (typeof s.status !== "string" || !["completed", "cancelled", "interrupted"].includes(s.status)) fail(`${at}.status 无效`);
  const b = object(s.brush, ["renderer", "color", "width", "smoothing", "streamline", "thinning", "simulate_pressure"], `${at}.brush`);
  if (b.renderer !== "perfect-freehand@1.2.3" || b.simulate_pressure !== false) fail(`${at}.brush 渲染契约无效`);
  if (typeof b.color !== "string" || !/^#[0-9a-fA-F]{6}$/.test(b.color)) fail(`${at}.brush.color 无效`);
  number(b.width, 0.1, 100, `${at}.brush.width`); number(b.smoothing, 0, 1, `${at}.brush.smoothing`);
  number(b.streamline, 0, 1, `${at}.brush.streamline`); number(b.thinning, -1, 1, `${at}.brush.thinning`);
  array(s.points, 1, 100_000, `${at}.points`); total.value += s.points.length;
  if (total.value > MAX_INK_POINTS) fail("样本总量超过实验上限");
  let previous = -1;
  for (const value of s.points) {
    const p = object(value, ["x", "y", "t_us", "pressure", "tilt_x_deg", "tilt_y_deg", "buttons"], `${at}.point`);
    number(p.x, -1_000_000, 1_000_000, "point.x"); number(p.y, -1_000_000, 1_000_000, "point.y");
    integer(p.t_us, 0, Number.MAX_SAFE_INTEGER, "point.t_us"); if (p.t_us < previous) fail("采样时间倒退"); previous = p.t_us;
    nullable(p.pressure, 0, 1, "point.pressure"); nullable(p.tilt_x_deg, -90, 90, "point.tilt_x_deg");
    nullable(p.tilt_y_deg, -90, 90, "point.tilt_y_deg"); integer(p.buttons, 0, 65535, "point.buttons");
  }
  return s as unknown as InkStroke;
}

export function parseInkDocument(input: unknown): InkDocument {
  const d = object(input, ["version", "document_id", "title", "created_at", "exported_at", "pages", "capture_segments", "operations"], "document");
  if (d.version !== INK_LAB_VERSION) fail("不支持的版本");
  for (const k of ["document_id", "title", "created_at", "exported_at"]) text(d[k], k);
  array(d.pages, 1, 100, "pages"); array(d.capture_segments, 0, 1000, "capture_segments");
  array(d.operations, 0, MAX_INK_OPERATIONS, "operations");
  const pages = new Set<string>(), segments = new Set<string>(), strokes = new Set<string>(), commands = new Set<string>();
  for (const value of d.pages) {
    const p = object(value, ["id", "width", "height"], "page"); uniqueId(p.id, pages, "page.id");
    number(p.width, 1, 20_000, "page.width"); number(p.height, 1, 20_000, "page.height");
  }
  for (const value of d.capture_segments) {
    const s = object(value, ["id", "started_at", "user_agent", "secure_context", "coalesced_events", "pointer_raw_update", "device_pixel_ratio", "time_origin_ms", "pressure_capability", "tilt_capability"], "segment");
    uniqueId(s.id, segments, "segment.id"); text(s.started_at, "started_at"); text(s.user_agent, "user_agent", 2000);
    for (const k of ["secure_context", "coalesced_events", "pointer_raw_update"]) if (typeof s[k] !== "boolean") fail(`${k} 必须是布尔值`);
    number(s.device_pixel_ratio, 0.1, 20, "device_pixel_ratio"); number(s.time_origin_ms, 0, Number.MAX_SAFE_INTEGER, "time_origin_ms");
    if (s.pressure_capability !== "unknown" || s.tilt_capability !== "unknown") fail("P0 不能认证传感器能力");
  }
  const total = { value: 0 };
  for (const [index, value] of d.operations.entries()) {
    if (value === null || typeof value !== "object") fail("operation 必须是对象");
    const kind = (value as Record<string, unknown>).kind;
    if (typeof kind !== "string" || !["stroke", "erase", "partial_erase", "transform", "page_add", "undo", "redo"].includes(kind)) fail("未知 operation kind");
    const operationKeys = kind === "stroke" ? ["kind", "seq", "command_id", "stroke"] : kind === "erase" ? ["kind", "seq", "command_id", "stroke_ids"] : kind === "partial_erase" ? ["kind", "seq", "command_id", "source_stroke_id", "fragments"] : kind === "transform" ? ["kind", "seq", "command_id", "stroke_ids", "matrix"] : kind === "page_add" ? ["kind", "seq", "command_id", "page"] : ["kind", "seq", "command_id", "target_command_id"];
    const op = object(value, operationKeys, "operation");
    integer(op.seq, index + 1, index + 1, "operation.seq"); uniqueId(op.command_id, commands, "command_id");
    if (kind === "page_add") {
      const page = object(op.page, ["id", "width", "height"], "page_add.page"); text(page.id, "page_add.page.id");
      if (!pages.has(page.id)) fail("page_add.page 必须出现在 document.pages");
      number(page.width, 1, 20_000, "page_add.page.width"); number(page.height, 1, 20_000, "page_add.page.height");
      continue;
    }
    if (kind === "erase") {
      array(op.stroke_ids, 1, MAX_INK_OPERATIONS, "stroke_ids");
      for (const id of op.stroke_ids) text(id, "stroke_ids[]");
      continue;
    }
    if (kind === "partial_erase") {
      text(op.source_stroke_id, "partial_erase.source_stroke_id");
      if (!strokes.has(op.source_stroke_id)) fail("partial_erase 来源笔迹不存在");
      array(op.fragments, 1, 1000, "partial_erase.fragments");
      for (const fragment of op.fragments) validateStroke(fragment, pages, segments, strokes, total, "partial_erase.fragment");
      continue;
    }
    if (kind === "transform") {
      array(op.stroke_ids, 1, MAX_INK_OPERATIONS, "transform.stroke_ids");
      for (const id of op.stroke_ids) { text(id, "transform.stroke_ids[]"); if (!strokes.has(id)) fail("transform 来源笔迹不存在"); }
      validateTransform(op.matrix, "transform.matrix");
      continue;
    }
    if (kind === "undo" || kind === "redo") { text(op.target_command_id, "target_command_id"); continue; }
    validateStroke(op.stroke, pages, segments, strokes, total, "stroke");
  }
  const result = input as InkDocument;
  projectInk(result); // Check erase ownership and the complete undo/redo chain.
  return result;
}

export function projectInk(document: InkDocument): InkProjection {
  const base: InkOperation[] = [];
  const active = new Map<string, boolean>();
  const undo: string[] = [], redo: string[] = [];
  const knownStrokes = new Set<string>();
  let pointCount = 0;
  for (const op of document.operations) {
    if (op.kind === "stroke" || op.kind === "erase" || op.kind === "partial_erase" || op.kind === "transform" || op.kind === "page_add") {
      if (op.kind === "page_add") { base.push(op); active.set(op.command_id, true); continue; }
      if (op.kind === "stroke") { knownStrokes.add(op.stroke.id); pointCount += op.stroke.points.length; }
      else if (op.kind === "erase") {
        if (!op.stroke_ids.length || new Set(op.stroke_ids).size !== op.stroke_ids.length || op.stroke_ids.some(id => !knownStrokes.has(id))) fail("擦除引用不存在或重复的笔迹");
      } else if (op.kind === "partial_erase") {
        if (!knownStrokes.has(op.source_stroke_id)) fail("局部擦除来源不存在");
        for (const fragment of op.fragments) { knownStrokes.add(fragment.id); pointCount += fragment.points.length; }
      } else if (op.kind === "transform") {
        if (op.stroke_ids.some(id => !knownStrokes.has(id))) fail("变换引用不存在的笔迹");
      }
      base.push(op); active.set(op.command_id, true); undo.push(op.command_id); redo.length = 0;
    } else {
      const from = op.kind === "undo" ? undo : redo;
      const to = op.kind === "undo" ? redo : undo;
      if (from.at(-1) !== op.target_command_id) fail("撤销重做目标与历史不符");
      from.pop(); to.push(op.target_command_id); active.set(op.target_command_id, op.kind === "redo");
    }
  }
  const visible = new Map<string, InkStroke>(), strokes: InkStroke[] = [];
  for (const op of base) {
    if (op.kind === "stroke") strokes.push(op.stroke);
    if (!active.get(op.command_id)) continue;
    if (op.kind === "stroke") visible.set(op.stroke.id, op.stroke);
    else if (op.kind === "erase") for (const id of op.stroke_ids) visible.delete(id);
    else if (op.kind === "partial_erase") { visible.delete(op.source_stroke_id); for (const fragment of op.fragments) visible.set(fragment.id, fragment); }
    else if (op.kind === "transform") {
      const m = op.matrix;
      for (const id of op.stroke_ids) { const stroke = visible.get(id); if (!stroke) continue; visible.set(id, { ...stroke, points: stroke.points.map(p => ({ ...p, x: m.a * p.x + m.c * p.y + m.e, y: m.b * p.x + m.d * p.y + m.f })) }); }
    }
  }
  return { strokes, visible: [...visible.values()], undo, redo, pointCount };
}

export function appendInk(document: InkDocument, command: InkAppendCommand): void {
  if (document.operations.length >= MAX_INK_OPERATIONS) throw new Error("操作达到实验上限，请导出文件后新建画板。");
  document.operations.push({ ...command, seq: document.operations.length + 1, command_id: inkId() } as InkOperation);
}

export function encodeInk(document: InkDocument): string {
  const output = { ...document, exported_at: new Date().toISOString() };
  parseInkDocument(output);
  const json = JSON.stringify(output);
  if (new TextEncoder().encode(json).length > MAX_INK_BYTES) throw new Error("文件超过 64 MiB 实验上限。");
  return json;
}
