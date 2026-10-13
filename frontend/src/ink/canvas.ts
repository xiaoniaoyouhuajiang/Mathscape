import { getStroke } from "perfect-freehand";
import { inkId, MAX_INK_POINTS, projectInk, type InkBrush, type InkDocument, type InkPage, type InkPoint, type InkProjection, type InkStroke, type InkTransform } from "./model";

export type InkTool = "pen" | "erase" | "partial_erase" | "select";
export interface InkCanvasOptions {
  document: () => InkDocument;
  page: () => InkPage;
  segmentId: () => string;
  brush: () => InkBrush;
  tool: () => InkTool;
  fingerWriting: () => boolean;
  readOnly?: boolean;
  onStroke: (stroke: InkStroke) => void;
  onErase: (ids: string[]) => void;
  onPartialErase: (sourceStrokeId: string, fragments: InkStroke[]) => void;
  onTransform: (strokeIds: string[], matrix: InkTransform) => void;
  onSelection: (strokeIds: string[]) => void;
  onMessage: (message: string) => void;
  onMetrics: (metrics: CanvasMetrics) => void;
}
export interface CanvasMetrics {
  pointerEvents: number;
  coalescedSamples: number;
  cancelled: number;
  lastPointer: string;
  frameMs: number;
  activePoints: number;
  candidateStrokes?: number;
  previewStrokes?: number;
}

const paths = new WeakMap<InkStroke, Path2D>();
export function drawInkStroke(ctx: CanvasRenderingContext2D, stroke: InkStroke, complete = true): void {
  const cached = complete ? paths.get(stroke) : undefined;
  if (cached) {
    ctx.fillStyle = stroke.brush.color;
    ctx.fill(cached);
    return;
  }
  const outline = getStroke(stroke.points.map(p => [p.x, p.y, p.pressure ?? 0.5]), {
    size: stroke.brush.width,
    thinning: stroke.brush.thinning,
    smoothing: stroke.brush.smoothing,
    streamline: stroke.brush.streamline,
    simulatePressure: false,
    last: complete,
  });
  if (!outline.length) return;
  const path = new Path2D();
  path.moveTo(outline[0][0], outline[0][1]);
  for (let i = 1; i < outline.length; i++) path.lineTo(outline[i][0], outline[i][1]);
  path.closePath();
  ctx.fillStyle = stroke.brush.color;
  ctx.fill(path);
  if (complete) paths.set(stroke, path);
}

type Point = { x: number; y: number };
type Bounds = { minX: number; minY: number; maxX: number; maxY: number };
type TransformHandle = "move" | "nw" | "ne" | "sw" | "se" | "rotate";
type TransformState = { mode: "drag" | "scale" | "rotate"; handle: TransformHandle; start: Point; bounds: Bounds; matrix: InkTransform };
type InteractionTarget = "canvas" | TransformHandle;
type StrokeIndex = { key: string; strokes: Map<string, InkStroke>; bounds: Map<string, Bounds>; cells: Map<string, Set<string>> };

const identity: InkTransform = { a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 };
const distance = (a: Point, b: Point) => Math.hypot(a.x - b.x, a.y - b.y);
const sameMatrix = (a: InkTransform, b: InkTransform) => a.a === b.a && a.b === b.b && a.c === b.c && a.d === b.d && a.e === b.e && a.f === b.f;
const apply = (m: InkTransform, p: Point): Point => ({ x: m.a * p.x + m.c * p.y + m.e, y: m.b * p.x + m.d * p.y + m.f });
const pointSegmentDistance = (p: Point, a: Point, b: Point) => {
  const dx = b.x - a.x, dy = b.y - a.y;
  const t = Math.max(0, Math.min(1, ((p.x - a.x) * dx + (p.y - a.y) * dy) / (dx * dx + dy * dy || 1)));
  return Math.hypot(p.x - a.x - t * dx, p.y - a.y - t * dy);
};
const polygonContains = (p: Point, polygon: Point[]) => {
  let inside = false;
  for (let i = 0, j = polygon.length - 1; i < polygon.length; j = i++) {
    const a = polygon[i], b = polygon[j];
    if ((a.y > p.y) !== (b.y > p.y) && p.x < ((b.x - a.x) * (p.y - a.y)) / (b.y - a.y) + a.x) inside = !inside;
  }
  return inside;
};
function boundsFor(strokes: InkStroke[]): Bounds | undefined {
  const points = strokes.flatMap(s => s.points);
  if (!points.length) return undefined;
  return { minX: Math.min(...points.map(p => p.x)), minY: Math.min(...points.map(p => p.y)), maxX: Math.max(...points.map(p => p.x)), maxY: Math.max(...points.map(p => p.y)) };
}
function strokeBounds(stroke: InkStroke): Bounds | undefined {
  const bounds = boundsFor([stroke]);
  if (!bounds) return undefined;
  const pad = stroke.brush.width / 2;
  return { minX: bounds.minX - pad, minY: bounds.minY - pad, maxX: bounds.maxX + pad, maxY: bounds.maxY + pad };
}
function transformedStroke(stroke: InkStroke, matrix: InkTransform): InkStroke {
  return { ...stroke, points: stroke.points.map(point => ({ ...point, ...apply(matrix, point) })) };
}
function transformedBounds(strokes: InkStroke[], matrix: InkTransform): Bounds | undefined {
  return boundsFor(strokes.map(stroke => transformedStroke(stroke, matrix)));
}
function intersects(a: Bounds, b: Bounds): boolean {
  return a.minX <= b.maxX && a.maxX >= b.minX && a.minY <= b.maxY && a.maxY >= b.minY;
}

export class InkCanvas {
  private scale = 1;
  private offset = { x: 0, y: 0 };
  private width = 1;
  private height = 1;
  private dpr = 1;
  private active: InkStroke | undefined;
  private activePointer: number | undefined;
  private erasePath: Point[] = [];
  private eraserPoint: Point | undefined;
  private wholeErase = new Set<string>();
  private selected = new Set<string>();
  private lasso: Point[] = [];
  private transform: TransformState | undefined;
  private partialPreview = new Map<string, InkStroke[]>();
  private touches = new Map<number, Point>();
  private navigation: { id: number; x: number; y: number } | undefined;
  private hoverTarget: InteractionTarget = "canvas";
  private activeTarget: InteractionTarget = "canvas";
  private frame = 0;
  private committedPoints = 0;
  private cachedDocument: InkDocument | undefined;
  private cachedOperationCount = -1;
  private cachedPageId = "";
  private cachedProjection: InkProjection | undefined;
  private spatialIndex: StrokeIndex | undefined;
  private metrics: CanvasMetrics = { pointerEvents: 0, coalescedSamples: 0, cancelled: 0, lastPointer: "—", frameMs: 0, activePoints: 0, candidateStrokes: 0, previewStrokes: 0 };
  private readonly settledCanvas = document.createElement("canvas");
  private readonly previewCanvas = document.createElement("canvas");
  private readonly input = document.createElement("canvas");
  private readonly observer: ResizeObserver;
  private readonly blur = () => {
    if (this.active) this.finish("interrupted");
    else this.cancelTransientInteraction();
  };

  constructor(private readonly host: HTMLElement, private readonly options: InkCanvasOptions) {
    this.settledCanvas.className = "ink-canvas ink-settled";
    this.previewCanvas.className = "ink-canvas ink-preview";
    this.input.className = "ink-canvas ink-input";
    this.input.setAttribute("aria-label", "手写画板");
    if (options.readOnly) this.input.style.pointerEvents = "none";
    this.input.dataset.cursor = "crosshair";
    host.append(this.settledCanvas, this.previewCanvas, this.input);
    this.input.addEventListener("pointerdown", e => this.down(e));
    this.input.addEventListener("pointermove", e => this.move(e));
    this.input.addEventListener("pointerup", e => this.up(e, "completed"));
    this.input.addEventListener("pointercancel", e => this.up(e, "cancelled"));
    this.input.addEventListener("lostpointercapture", e => {
      if (this.activePointer === e.pointerId) {
        if (this.active) this.finish("interrupted");
        else this.cancelTransientInteraction();
      }
      this.touches.delete(e.pointerId);
      if (this.navigation?.id === e.pointerId) this.navigation = undefined;
    });
    this.input.addEventListener("contextmenu", e => e.preventDefault());
    this.input.addEventListener("wheel", e => {
      e.preventDefault();
      if (this.activePointer === undefined) this.zoom(Math.exp(-e.deltaY * 0.002), this.local(e.clientX, e.clientY));
    }, { passive: false });
    window.addEventListener("blur", this.blur);
    document.addEventListener("visibilitychange", () => { if (document.hidden) this.blur(); });
    let first = true;
    this.observer = new ResizeObserver(() => {
      this.width = Math.max(1, host.clientWidth);
      this.height = Math.max(1, host.clientHeight);
      this.dpr = Math.min(window.devicePixelRatio || 1, 2);
      for (const canvas of [this.settledCanvas, this.previewCanvas, this.input]) {
        canvas.width = Math.round(this.width * this.dpr);
        canvas.height = Math.round(this.height * this.dpr);
      }
      if (first) { first = false; this.fit(); } else this.refresh();
    });
    this.observer.observe(host);
  }

  private local(x: number, y: number): Point {
    const rect = this.input.getBoundingClientRect();
    return { x: x - rect.left, y: y - rect.top };
  }
  private world(point: Point): Point { return { x: (point.x - this.offset.x) / this.scale, y: (point.y - this.offset.y) / this.scale }; }
  private projection(): InkProjection {
    const documentState = this.options.document();
    const page = this.options.page();
    if (!this.cachedProjection || this.cachedDocument !== documentState || this.cachedOperationCount !== documentState.operations.length || this.cachedPageId !== page.id) {
      this.cachedDocument = documentState;
      this.cachedOperationCount = documentState.operations.length;
      this.cachedPageId = page.id;
      this.cachedProjection = projectInk(documentState);
      this.spatialIndex = undefined;
    }
    return this.cachedProjection;
  }
  private visible(): InkStroke[] { return this.projection().visible.filter(stroke => stroke.page_id === this.options.page().id); }
  private selectedStrokes(): InkStroke[] {
    const byId = new Map(this.visible().map(stroke => [stroke.id, stroke]));
    return [...this.selected].map(id => byId.get(id)).filter((stroke): stroke is InkStroke => Boolean(stroke));
  }
  setSelection(ids: string[]): void {
    const visible = new Set(this.visible().map(stroke => stroke.id));
    this.selected = new Set(ids.filter(id => visible.has(id)));
    this.schedule();
  }
  /** Clear only transient gesture state; the selected ids remain available when re-entering lasso mode. */
  resetTransientInteraction(): void {
    if (this.active) this.finish("interrupted");
    else this.cancelTransientInteraction();
  }
  fit(): void {
    if (this.activePointer !== undefined) return;
    const page = this.options.page();
    this.scale = Math.max(0.05, Math.min((this.width - 32) / page.width, (this.height - 32) / page.height));
    this.offset = { x: (this.width - page.width * this.scale) / 2, y: (this.height - page.height * this.scale) / 2 };
    this.refresh();
  }
  zoom(factor: number, center = { x: this.width / 2, y: this.height / 2 }): void {
    if (this.activePointer !== undefined) return;
    const world = this.world(center), next = Math.max(0.05, Math.min(8, this.scale * factor));
    this.offset = { x: center.x - world.x * next, y: center.y - world.y * next };
    this.scale = next;
    this.refresh();
  }
  private setup(canvas: HTMLCanvasElement, clear: boolean): CanvasRenderingContext2D {
    const ctx = canvas.getContext("2d")!;
    if (clear) { ctx.setTransform(1, 0, 0, 1, 0, 0); ctx.clearRect(0, 0, canvas.width, canvas.height); }
    ctx.setTransform(this.dpr * this.scale, 0, 0, this.dpr * this.scale, this.offset.x * this.dpr, this.offset.y * this.dpr);
    return ctx;
  }
  private pageClip(ctx: CanvasRenderingContext2D, page: InkPage): void {
    ctx.save();
    ctx.beginPath();
    ctx.rect(0, 0, page.width, page.height);
    ctx.clip();
  }
  refresh(): void {
    const page = this.options.page();
    const visible = this.visible();
    const settled = this.setup(this.settledCanvas, true);
    settled.fillStyle = "#fff";
    settled.fillRect(0, 0, page.width, page.height);
    settled.strokeStyle = "#e8edf4";
    settled.lineWidth = 1 / this.scale;
    settled.beginPath();
    for (let y = 40; y < page.height; y += 40) { settled.moveTo(0, y); settled.lineTo(page.width, y); }
    settled.stroke();
    this.pageClip(settled, page);
    const selected = this.transform ? this.selected : new Set<string>();
    for (const stroke of visible) {
      if (this.wholeErase.has(stroke.id) || this.partialPreview.has(stroke.id) || selected.has(stroke.id)) continue;
      drawInkStroke(settled, stroke);
    }
    settled.restore();

    this.drawPreviewLayer(page, visible);
    this.committedPoints = this.projection().pointCount;
    this.schedule();
  }
  private drawPreviewLayer(page: InkPage, visible: InkStroke[]): void {
    const preview = this.setup(this.previewCanvas, true);
    this.pageClip(preview, page);
    for (const stroke of visible) {
      if (this.wholeErase.has(stroke.id)) continue;
      const fragments = this.partialPreview.get(stroke.id);
      if (fragments) {
        for (const fragment of fragments) drawInkStroke(preview, fragment);
        continue;
      }
      if (this.transform && this.selected.has(stroke.id)) drawInkStroke(preview, transformedStroke(stroke, this.transform.matrix));
    }
    preview.restore();
  }
  private refreshPreviewOnly(): void {
    this.drawPreviewLayer(this.options.page(), this.visible());
    this.schedule();
  }
  private schedule(): void {
    if (this.frame) return;
    this.frame = requestAnimationFrame(() => {
      this.frame = 0;
      const started = performance.now();
      const ctx = this.setup(this.input, true), page = this.options.page();
      this.pageClip(ctx, page);
      if (this.active) drawInkStroke(ctx, this.active, false);
      this.drawOverlay(ctx);
      ctx.restore();
      this.metrics.activePoints = this.active?.points.length ?? 0;
      this.metrics.previewStrokes = this.partialPreview.size + (this.transform ? this.selected.size : 0);
      this.metrics.frameMs = performance.now() - started;
      this.options.onMetrics({ ...this.metrics });
    });
  }
  private drawOverlay(ctx: CanvasRenderingContext2D): void {
    if (this.options.tool() === "select") {
      if (this.lasso.length > 1) {
        ctx.strokeStyle = "#2c8976";
        ctx.lineWidth = 2 / this.scale;
        ctx.setLineDash([8 / this.scale, 5 / this.scale]);
        ctx.beginPath();
        ctx.moveTo(this.lasso[0].x, this.lasso[0].y);
        for (const point of this.lasso.slice(1)) ctx.lineTo(point.x, point.y);
        ctx.stroke();
        ctx.setLineDash([]);
      }
      const bounds = this.transform ? transformedBounds(this.selectedStrokes(), this.transform.matrix) : boundsFor(this.selectedStrokes());
      if (bounds) this.drawSelectionOverlay(ctx, bounds);
    }
    if ((this.options.tool() === "erase" || this.options.tool() === "partial_erase") && this.eraserPoint) {
      const radius = this.options.brush().width / 2 + 10 / this.scale;
      ctx.fillStyle = this.options.tool() === "partial_erase" ? "#d8873e22" : "#d8873e1d";
      ctx.strokeStyle = this.options.tool() === "partial_erase" ? "#bb6d28" : "#9a5c3f";
      ctx.lineWidth = 1.5 / this.scale;
      ctx.beginPath(); ctx.arc(this.eraserPoint.x, this.eraserPoint.y, radius, 0, Math.PI * 2); ctx.fill(); ctx.stroke();
    }
  }
  private drawSelectionOverlay(ctx: CanvasRenderingContext2D, bounds: Bounds): void {
    const pad = 10 / this.scale;
    const corners: Array<[TransformHandle, Point]> = [["nw", { x: bounds.minX - pad, y: bounds.minY - pad }], ["ne", { x: bounds.maxX + pad, y: bounds.minY - pad }], ["sw", { x: bounds.minX - pad, y: bounds.maxY + pad }], ["se", { x: bounds.maxX + pad, y: bounds.maxY + pad }]];
    ctx.strokeStyle = "#2c8976";
    ctx.lineWidth = 2 / this.scale;
    ctx.strokeRect(bounds.minX - pad, bounds.minY - pad, bounds.maxX - bounds.minX + pad * 2, bounds.maxY - bounds.minY + pad * 2);
    ctx.fillStyle = "#fff";
    for (const [handle, point] of corners) {
      const active = this.activeTarget === handle || this.hoverTarget === handle;
      ctx.fillStyle = active ? "#cde9de" : "#fff";
      ctx.fillRect(point.x - 7 / this.scale, point.y - 7 / this.scale, 14 / this.scale, 14 / this.scale);
      ctx.strokeRect(point.x - 7 / this.scale, point.y - 7 / this.scale, 14 / this.scale, 14 / this.scale);
    }
    const center = { x: (bounds.minX + bounds.maxX) / 2, y: bounds.minY - 55 / this.scale };
    ctx.beginPath(); ctx.moveTo((bounds.minX + bounds.maxX) / 2, bounds.minY - pad); ctx.lineTo((bounds.minX + bounds.maxX) / 2, bounds.minY - 45 / this.scale); ctx.stroke();
    ctx.fillStyle = this.activeTarget === "rotate" || this.hoverTarget === "rotate" ? "#cde9de" : "#fff";
    ctx.beginPath(); ctx.arc(center.x, center.y, 8 / this.scale, 0, Math.PI * 2); ctx.fill(); ctx.stroke();
  }
  private cursorFor(target: InteractionTarget): string {
    if (this.options.tool() === "partial_erase" || this.options.tool() === "erase") return "crosshair";
    if (target === "move") return this.activePointer === undefined ? "grab" : "grabbing";
    if (target === "rotate") return "crosshair";
    if (target === "nw" || target === "se") return "nwse-resize";
    if (target === "ne" || target === "sw") return "nesw-resize";
    return this.options.tool() === "select" ? "cell" : "crosshair";
  }
  private setCursor(target: InteractionTarget): void {
    this.hoverTarget = target;
    this.input.dataset.cursor = this.cursorFor(target);
    this.input.style.cursor = this.cursorFor(target);
    this.schedule();
  }
  private boundsForSelection(): Bounds | undefined { return boundsFor(this.selectedStrokes()); }
  private handleAt(point: Point, bounds: Bounds, pad: number): TransformHandle | undefined {
    const corners: Array<[TransformHandle, Point]> = [["nw", { x: bounds.minX - pad, y: bounds.minY - pad }], ["ne", { x: bounds.maxX + pad, y: bounds.minY - pad }], ["sw", { x: bounds.minX - pad, y: bounds.maxY + pad }], ["se", { x: bounds.maxX + pad, y: bounds.maxY + pad }], ["rotate", { x: (bounds.minX + bounds.maxX) / 2, y: bounds.minY - 55 / this.scale }]];
    for (const [handle, position] of corners) if (distance(point, position) <= 18 / this.scale) return handle;
    if (point.x >= bounds.minX - pad && point.x <= bounds.maxX + pad && point.y >= bounds.minY - pad && point.y <= bounds.maxY + pad) return "move";
    return undefined;
  }
  private targetAt(point: Point): InteractionTarget {
    const bounds = this.boundsForSelection();
    if (!bounds) return "canvas";
    return this.handleAt(point, bounds, 12 / this.scale) ?? "canvas";
  }
  private down(e: PointerEvent): void {
    e.preventDefault();
    if (this.options.readOnly) return;
    if (this.activePointer !== undefined) return;
    const position = this.local(e.clientX, e.clientY), world = this.world(position);
    if (e.pointerType === "touch" && !this.options.fingerWriting()) { this.touches.set(e.pointerId, position); return; }
    if (e.pointerType === "mouse" && e.button !== 0) { if (e.button === 1) this.navigation = { id: e.pointerId, ...position }; return; }
    const page = this.options.page();
    if (world.x < 0 || world.y < 0 || world.x > page.width || world.y > page.height) return;
    if (this.options.document().operations.length >= 19_990) { this.options.onMessage("操作接近实验上限，请先导出并新建画板。"); return; }
    this.input.setPointerCapture(e.pointerId);
    this.activePointer = e.pointerId;
    this.metrics.lastPointer = e.pointerType || "unknown";
    this.touches.clear();
    const tool = this.options.tool();
    if (tool === "select") { this.beginSelect(world); return; }
    if (tool === "erase" || tool === "partial_erase") {
      this.erasePath = [world];
      this.eraserPoint = world;
      this.wholeErase.clear();
      this.partialPreview.clear();
      if (tool === "erase") this.erase(world);
      else this.updatePartialPreview();
      this.setCursor("canvas");
      return;
    }
    if (this.projection().pointCount >= MAX_INK_POINTS) { this.activePointer = undefined; this.options.onMessage("样本达到实验上限，请导出并新建画板。"); return; }
    const pointerType = ["pen", "mouse", "touch"].includes(e.pointerType) ? e.pointerType as "pen" | "mouse" | "touch" : "unknown";
    let segmentId: string;
    try { segmentId = this.options.segmentId(); }
    catch (error) { this.activePointer = undefined; this.options.onMessage(error instanceof Error ? error.message : String(error)); return; }
    this.active = { id: inkId(), page_id: page.id, capture_segment_id: segmentId, pointer_type: pointerType, status: "completed", brush: { ...this.options.brush() }, points: [] };
    this.samples(e);
    this.schedule();
  }
  private beginSelect(point: Point): void {
    const bounds = this.boundsForSelection();
    const target = bounds ? this.handleAt(point, bounds, 12 / this.scale) : undefined;
    this.activeTarget = target ?? "canvas";
    this.setCursor(this.activeTarget);
    if (target) {
      this.transform = { mode: target === "rotate" ? "rotate" : target === "move" ? "drag" : "scale", handle: target, start: point, bounds: bounds!, matrix: { ...identity } };
      this.refresh();
      return;
    }
    this.lasso = [point];
    this.selected.clear();
    this.options.onSelection([]);
  }
  private move(e: PointerEvent): void {
    const position = this.local(e.clientX, e.clientY);
    if (e.pointerId === this.activePointer) {
      e.preventDefault();
      const world = this.world(position), tool = this.options.tool();
      if (this.active) this.samples(e);
      else if (tool === "erase" || tool === "partial_erase") {
        this.erasePath.push(world);
        this.eraserPoint = world;
        if (tool === "erase") this.erase(world); else this.updatePartialPreview();
      } else if (tool === "select") {
        if (this.transform) this.updateTransform(world); else this.lasso.push(world);
      }
      this.schedule();
      return;
    }
    if (this.navigation?.id === e.pointerId) {
      this.offset.x += position.x - this.navigation.x;
      this.offset.y += position.y - this.navigation.y;
      this.navigation = { id: e.pointerId, ...position };
      this.refresh();
      return;
    }
    if (this.activePointer !== undefined || !this.touches.has(e.pointerId)) {
      if (this.activePointer === undefined) {
        const world = this.world(position), tool = this.options.tool();
        if (tool === "select") this.setCursor(this.targetAt(world));
        else if (tool === "erase" || tool === "partial_erase") { this.eraserPoint = world; this.setCursor("canvas"); }
      }
      return;
    }
    const before = [...this.touches.values()];
    this.touches.set(e.pointerId, position);
    const after = [...this.touches.values()];
    if (before.length === 1) {
      this.offset.x += after[0].x - before[0].x;
      this.offset.y += after[0].y - before[0].y;
    } else {
      const center = (list: Point[]) => ({ x: (list[0].x + list[1].x) / 2, y: (list[0].y + list[1].y) / 2 });
      const length = (list: Point[]) => Math.hypot(list[1].x - list[0].x, list[1].y - list[0].y);
      const previous = center(before), next = center(after), world = this.world(previous);
      this.scale = Math.max(0.05, Math.min(8, this.scale * length(after) / Math.max(1, length(before))));
      this.offset = { x: next.x - world.x * this.scale, y: next.y - world.y * this.scale };
    }
    this.refresh();
  }
  private updateTransform(point: Point): void {
    const state = this.transform!;
    const bounds = state.bounds;
    if (state.mode === "drag") state.matrix = { ...identity, e: point.x - state.start.x, f: point.y - state.start.y };
    else if (state.mode === "rotate") {
      const center = { x: (bounds.minX + bounds.maxX) / 2, y: (bounds.minY + bounds.maxY) / 2 };
      const startAngle = Math.atan2(state.start.y - center.y, state.start.x - center.x), nextAngle = Math.atan2(point.y - center.y, point.x - center.x);
      const cos = Math.cos(nextAngle - startAngle), sin = Math.sin(nextAngle - startAngle);
      state.matrix = { a: cos, b: sin, c: -sin, d: cos, e: center.x - cos * center.x + sin * center.y, f: center.y - sin * center.x - cos * center.y };
    } else {
      const anchor = state.handle === "nw" ? { x: bounds.maxX, y: bounds.maxY } : state.handle === "ne" ? { x: bounds.minX, y: bounds.maxY } : state.handle === "sw" ? { x: bounds.maxX, y: bounds.minY } : { x: bounds.minX, y: bounds.minY };
      const sx = Math.max(0.05, Math.abs((point.x - anchor.x) / Math.max(1, state.start.x - anchor.x))) * Math.sign(point.x - anchor.x || 1) * Math.sign(state.start.x - anchor.x || 1);
      const sy = Math.max(0.05, Math.abs((point.y - anchor.y) / Math.max(1, state.start.y - anchor.y))) * Math.sign(point.y - anchor.y || 1) * Math.sign(state.start.y - anchor.y || 1);
      state.matrix = { a: sx, b: 0, c: 0, d: sy, e: anchor.x - sx * anchor.x, f: anchor.y - sy * anchor.y };
    }
    this.refreshPreviewOnly();
  }
  private samples(e: PointerEvent): void {
    if (!this.active) return;
    this.metrics.pointerEvents++;
    let events: PointerEvent[] = [];
    try { events = e.getCoalescedEvents?.() ?? []; } catch { /* unavailable on LAN HTTP */ }
    if (events.length) this.metrics.coalescedSamples += events.length; else events = [e];
    for (const sample of events) {
      if (!this.active) break;
      if (this.active.points.length >= 100_000 || this.active.points.length + this.committedPoints >= MAX_INK_POINTS) { this.finish("interrupted"); this.options.onMessage("采样达到实验上限；已保留当前笔迹，请导出。"); break; }
      const point = this.world(this.local(sample.clientX, sample.clientY)), pen = this.active.pointer_type === "pen";
      const next: InkPoint = { ...point, t_us: Math.max(0, Math.round(sample.timeStamp * 1000)), pressure: pen && Number.isFinite(sample.pressure) ? sample.pressure : null, tilt_x_deg: pen && Number.isFinite(sample.tiltX) ? sample.tiltX : null, tilt_y_deg: pen && Number.isFinite(sample.tiltY) ? sample.tiltY : null, buttons: sample.buttons };
      const previous = this.active.points.at(-1);
      if (previous && next.t_us < previous.t_us) { this.finish("interrupted"); this.options.onMessage("采样时间倒退，当前笔迹已标记为中断。"); break; }
      if (!previous || Object.keys(next).some(key => next[key as keyof InkPoint] !== previous[key as keyof InkPoint])) this.active.points.push(next);
    }
  }
  private ensureIndex(): StrokeIndex {
    const key = `${this.cachedOperationCount}:${this.cachedPageId}`;
    if (this.spatialIndex?.key === key) return this.spatialIndex;
    const strokes = new Map(this.visible().map(stroke => [stroke.id, stroke]));
    const bounds = new Map<string, Bounds>(), cells = new Map<string, Set<string>>(), cellSize = 160;
    const add = (cell: string, id: string) => { const ids = cells.get(cell) ?? new Set<string>(); ids.add(id); cells.set(cell, ids); };
    for (const stroke of strokes.values()) {
      const box = strokeBounds(stroke); if (!box) continue;
      bounds.set(stroke.id, box);
      for (let x = Math.floor(box.minX / cellSize); x <= Math.floor(box.maxX / cellSize); x++) for (let y = Math.floor(box.minY / cellSize); y <= Math.floor(box.maxY / cellSize); y++) add(`${x}:${y}`, stroke.id);
    }
    this.spatialIndex = { key, strokes, bounds, cells };
    return this.spatialIndex;
  }
  private candidates(path: Point[], radius: number): InkStroke[] {
    if (!path.length) return [];
    const index = this.ensureIndex(), cellSize = 160;
    const minX = Math.min(...path.map(point => point.x)) - radius, maxX = Math.max(...path.map(point => point.x)) + radius;
    const minY = Math.min(...path.map(point => point.y)) - radius, maxY = Math.max(...path.map(point => point.y)) + radius;
    const query = { minX, minY, maxX, maxY }, ids = new Set<string>();
    for (let x = Math.floor(minX / cellSize); x <= Math.floor(maxX / cellSize); x++) for (let y = Math.floor(minY / cellSize); y <= Math.floor(maxY / cellSize); y++) for (const id of index.cells.get(`${x}:${y}`) ?? []) ids.add(id);
    const result: InkStroke[] = [];
    for (const id of ids) { const box = index.bounds.get(id); const stroke = index.strokes.get(id); if (stroke && box && intersects(box, query)) result.push(stroke); }
    this.metrics.candidateStrokes = result.length;
    return result;
  }
  private distanceToErasePath(point: Point): number {
    let best = Number.POSITIVE_INFINITY;
    for (let i = 0; i < this.erasePath.length; i++) best = Math.min(best, pointSegmentDistance(point, this.erasePath[i], this.erasePath[i + 1] ?? this.erasePath[i]));
    return best;
  }
  private erase(point: Point): void {
    const radius = this.options.brush().width / 2 + 10 / this.scale;
    for (const stroke of this.candidates([point], radius)) if (stroke.points.some((sample, index) => pointSegmentDistance(point, sample, stroke.points[index + 1] ?? sample) <= radius)) this.wholeErase.add(stroke.id);
    this.refresh();
  }
  private fragmentsFor(stroke: InkStroke, preview = false): InkStroke[] {
    const radius = stroke.brush.width / 2 + 10 / this.scale, kept = stroke.points.map(point => this.distanceToErasePath(point) > radius), fragments: InkStroke[] = [];
    let points: InkPoint[] = [];
    const flush = () => { if (points.length) { fragments.push({ ...stroke, id: preview ? `${stroke.id}:preview:${fragments.length}` : inkId(), points }); points = []; } };
    stroke.points.forEach((point, index) => { if (kept[index]) points.push(point); else flush(); });
    flush();
    return fragments;
  }
  private updatePartialPreview(): void {
    const previousIds = [...this.partialPreview.keys()].sort().join(",");
    const next = new Map<string, InkStroke[]>(), candidates = this.candidates(this.erasePath, this.options.brush().width / 2 + 10 / this.scale);
    for (const stroke of candidates) {
      const fragments = this.fragmentsFor(stroke, true);
      if (fragments.length !== 1 || fragments[0].points.length !== stroke.points.length) next.set(stroke.id, fragments);
    }
    this.partialPreview = next;
    const nextIds = [...next.keys()].sort().join(",");
    if (previousIds !== nextIds) this.refresh(); else this.refreshPreviewOnly();
  }
  private finishPartial(): void {
    const candidates = this.candidates(this.erasePath, this.options.brush().width / 2 + 10 / this.scale);
    for (const stroke of candidates) {
      const fragments = this.fragmentsFor(stroke);
      if (fragments.length !== 1 || fragments[0].points.length !== stroke.points.length) this.options.onPartialErase(stroke.id, fragments);
    }
  }
  private selectLasso(): void {
    if (this.lasso.length < 3) return;
    const ids = this.visible().filter(stroke => stroke.points.some(point => polygonContains(point, this.lasso))).map(stroke => stroke.id);
    this.selected = new Set(ids);
    this.options.onSelection(ids);
  }
  private completeGesture(status: InkStroke["status"]): void {
    if (status !== "completed") { this.cancelTransientInteraction(); return; }
    const tool = this.options.tool();
    if (tool === "select") {
      if (this.transform) {
        const ids = [...this.selected];
        if (!sameMatrix(this.transform.matrix, identity)) this.options.onTransform(ids, this.transform.matrix);
      } else this.selectLasso();
    } else if (tool === "erase" && this.wholeErase.size) this.options.onErase([...this.wholeErase]);
    else if (tool === "partial_erase") this.finishPartial();
    this.clearTransient();
  }
  private up(e: PointerEvent, status: InkStroke["status"]): void {
    if (e.pointerId === this.activePointer) {
      if (status === "completed" && this.active) this.samples(e);
      const stroke = this.active;
      if (stroke) {
        this.active = undefined;
        this.completeGesture(status);
        if (stroke.points.length) { stroke.status = status; if (status === "cancelled") this.metrics.cancelled++; this.options.onStroke(stroke); this.refresh(); }
        else this.refresh();
      } else this.completeGesture(status);
    }
    this.touches.delete(e.pointerId);
    if (this.navigation?.id === e.pointerId) this.navigation = undefined;
  }
  private clearTransient(): void {
    if (this.activePointer !== undefined) {
      try { this.input.releasePointerCapture(this.activePointer); } catch { /* synthetic test pointers */ }
    }
    this.activePointer = undefined;
    this.erasePath = [];
    this.eraserPoint = undefined;
    this.wholeErase.clear();
    this.partialPreview.clear();
    this.transform = undefined;
    this.lasso = [];
    this.activeTarget = "canvas";
    this.setCursor("canvas");
    this.refresh();
  }
  private cancelTransientInteraction(): void {
    this.active = undefined;
    this.clearTransient();
  }
  finish(status: InkStroke["status"] = "interrupted"): void {
    const stroke = this.active;
    const erased = [...this.wholeErase];
    this.active = undefined;
    this.clearTransient();
    if (stroke?.points.length) { stroke.status = status; if (status === "cancelled") this.metrics.cancelled++; this.options.onStroke(stroke); }
    else if (erased.length) this.options.onErase(erased);
    this.refresh();
  }
  png(): HTMLCanvasElement {
    this.finish();
    const page = this.options.page(), canvas = document.createElement("canvas"), width = Math.min(2400, Math.round(page.width)), height = Math.max(1, Math.round(page.height * width / page.width));
    if (height > 4800) throw new Error("页面尺寸过大，无法生成 PNG；请导出 Ink JSON。");
    canvas.width = width; canvas.height = height;
    const ctx = canvas.getContext("2d")!;
    ctx.scale(canvas.width / page.width, canvas.height / page.height);
    ctx.fillStyle = "#fff"; ctx.fillRect(0, 0, page.width, page.height);
    for (const stroke of this.visible()) drawInkStroke(ctx, stroke);
    return canvas;
  }
  dispose(): void {
    this.observer.disconnect();
    window.removeEventListener("blur", this.blur);
    cancelAnimationFrame(this.frame);
    this.settledCanvas.remove();
    this.previewCanvas.remove();
    this.input.remove();
  }
}
