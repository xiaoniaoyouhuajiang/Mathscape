import { getStroke } from "perfect-freehand";
import { inkId, MAX_INK_POINTS, projectInk, type InkBrush, type InkDocument, type InkPage, type InkPoint, type InkStroke } from "./model";

export interface InkCanvasOptions {
  document: () => InkDocument;
  page: () => InkPage;
  segmentId: () => string;
  brush: () => InkBrush;
  tool: () => "pen" | "erase";
  fingerWriting: () => boolean;
  onStroke: (stroke: InkStroke) => void;
  onErase: (ids: string[]) => void;
  onMessage: (message: string) => void;
  onMetrics: (metrics: CanvasMetrics) => void;
}
export interface CanvasMetrics {
  pointerEvents: number; coalescedSamples: number; cancelled: number;
  lastPointer: string; frameMs: number; activePoints: number;
}
const paths = new WeakMap<InkStroke, Path2D>();
export function drawInkStroke(ctx: CanvasRenderingContext2D, stroke: InkStroke, complete = true): void {
  const cached = complete ? paths.get(stroke) : undefined;
  if (cached) { ctx.fillStyle = stroke.brush.color; ctx.fill(cached); return; }
  const outline = getStroke(stroke.points.map(p => [p.x, p.y, p.pressure ?? 0.5]), {
    size: stroke.brush.width, thinning: stroke.brush.thinning,
    smoothing: stroke.brush.smoothing, streamline: stroke.brush.streamline,
    simulatePressure: false, last: complete,
  });
  if (!outline.length) return;
  ctx.fillStyle = stroke.brush.color;
  const path = new Path2D(); path.moveTo(outline[0][0], outline[0][1]);
  for (let i = 1; i < outline.length; i++) path.lineTo(outline[i][0], outline[i][1]);
  path.closePath(); ctx.fill(path); if (complete) paths.set(stroke, path);
}

export class InkCanvas {
  private scale = 1;
  private offset = { x: 0, y: 0 };
  private width = 1;
  private height = 1;
  private dpr = 1;
  private active: InkStroke | undefined;
  private activePointer: number | undefined;
  private erased = new Set<string>();
  private touches = new Map<number, { x: number; y: number }>();
  private navigation: { id: number; x: number; y: number } | undefined;
  private frame = 0;
  private committedPoints = 0;
  private metrics: CanvasMetrics = { pointerEvents: 0, coalescedSamples: 0, cancelled: 0, lastPointer: "—", frameMs: 0, activePoints: 0 };
  private readonly staticCanvas = document.createElement("canvas");
  private readonly input = document.createElement("canvas");
  private readonly observer: ResizeObserver;
  private readonly blur = () => this.finish("interrupted");
  constructor(private readonly host: HTMLElement, private readonly options: InkCanvasOptions) {
    this.staticCanvas.className = "ink-canvas ink-settled";
    this.input.className = "ink-canvas ink-input";
    this.input.setAttribute("aria-label", "手写画板");
    host.append(this.staticCanvas, this.input);
    this.input.addEventListener("pointerdown", e => this.down(e));
    this.input.addEventListener("pointermove", e => this.move(e));
    this.input.addEventListener("pointerup", e => this.up(e, "completed"));
    this.input.addEventListener("pointercancel", e => this.up(e, "cancelled"));
    this.input.addEventListener("lostpointercapture", e => {
      if (this.activePointer === e.pointerId) this.finish("interrupted");
      this.touches.delete(e.pointerId);
      if (this.navigation?.id === e.pointerId) this.navigation = undefined;
    });
    this.input.addEventListener("contextmenu", e => e.preventDefault());
    this.input.addEventListener("wheel", e => {
      e.preventDefault(); if (this.activePointer !== undefined) return;
      const point = this.local(e.clientX, e.clientY); this.zoom(Math.exp(-e.deltaY * 0.002), point);
    }, { passive: false });
    window.addEventListener("blur", this.blur);
    document.addEventListener("visibilitychange", () => { if (document.hidden) this.finish("interrupted"); });
    let first = true;
    this.observer = new ResizeObserver(() => {
      this.width = Math.max(1, host.clientWidth); this.height = Math.max(1, host.clientHeight);
      this.dpr = Math.min(window.devicePixelRatio || 1, 2);
      for (const c of [this.staticCanvas, this.input]) { c.width = Math.round(this.width * this.dpr); c.height = Math.round(this.height * this.dpr); }
      if (first) { first = false; this.fit(); } else this.refresh();
    });
    this.observer.observe(host);
  }
  private local(clientX: number, clientY: number): { x: number; y: number } {
    const rect = this.input.getBoundingClientRect(); return { x: clientX - rect.left, y: clientY - rect.top };
  }
  private world(point: { x: number; y: number }): { x: number; y: number } {
    return { x: (point.x - this.offset.x) / this.scale, y: (point.y - this.offset.y) / this.scale };
  }
  fit(): void {
    if (this.activePointer !== undefined) return;
    const page = this.options.page();
    this.scale = Math.min((this.width - 32) / page.width, (this.height - 32) / page.height);
    this.scale = Math.max(0.05, this.scale);
    this.offset = { x: (this.width - page.width * this.scale) / 2, y: (this.height - page.height * this.scale) / 2 };
    this.refresh();
  }
  zoom(factor: number, center = { x: this.width / 2, y: this.height / 2 }): void {
    if (this.activePointer !== undefined) return;
    const world = this.world(center), next = Math.max(0.05, Math.min(8, this.scale * factor));
    this.offset = { x: center.x - world.x * next, y: center.y - world.y * next }; this.scale = next; this.refresh();
  }
  private setup(canvas: HTMLCanvasElement, clear: boolean): CanvasRenderingContext2D {
    const ctx = canvas.getContext("2d")!;
    if (clear) { ctx.setTransform(1, 0, 0, 1, 0, 0); ctx.clearRect(0, 0, canvas.width, canvas.height); }
    ctx.setTransform(this.dpr * this.scale, 0, 0, this.dpr * this.scale, this.offset.x * this.dpr, this.offset.y * this.dpr);
    return ctx;
  }
  refresh(): void {
    const page = this.options.page(), ctx = this.setup(this.staticCanvas, true);
    ctx.fillStyle = "#ffffff"; ctx.fillRect(0, 0, page.width, page.height);
    ctx.strokeStyle = "#e8edf4"; ctx.lineWidth = 1 / this.scale;
    ctx.beginPath();
    for (let y = 40; y < page.height; y += 40) { ctx.moveTo(0, y); ctx.lineTo(page.width, y); }
    ctx.stroke();
    ctx.save(); ctx.beginPath(); ctx.rect(0, 0, page.width, page.height); ctx.clip();
    const projection = projectInk(this.options.document()); this.committedPoints = projection.pointCount;
    for (const stroke of projection.visible) if (stroke.page_id === page.id && !this.erased.has(stroke.id)) drawInkStroke(ctx, stroke);
    ctx.restore(); this.schedule();
  }
  private schedule(): void {
    if (this.frame) return;
    this.frame = requestAnimationFrame(() => {
      this.frame = 0; const started = performance.now();
      const ctx = this.setup(this.input, true);
      const page = this.options.page();
      ctx.save(); ctx.beginPath(); ctx.rect(0, 0, page.width, page.height); ctx.clip();
      if (this.active) drawInkStroke(ctx, this.active, false);
      ctx.restore();
      this.metrics.activePoints = this.active?.points.length ?? 0;
      this.metrics.frameMs = performance.now() - started;
      this.options.onMetrics({ ...this.metrics });
    });
  }
  private down(e: PointerEvent): void {
    e.preventDefault();
    if (this.activePointer !== undefined) return; // Pen takes precedence over palm/touch input.
    this.input.setPointerCapture(e.pointerId);
    const position = this.local(e.clientX, e.clientY);
    if (e.pointerType === "touch" && !this.options.fingerWriting()) { this.touches.set(e.pointerId, position); return; }
    if (e.pointerType === "mouse" && e.button !== 0) {
      if (e.button === 1) this.navigation = { id: e.pointerId, ...position };
      return;
    }
    const world = this.world(position), page = this.options.page();
    if (world.x < 0 || world.y < 0 || world.x > page.width || world.y > page.height) return;
    if (this.options.document().operations.length >= 19_990) { this.options.onMessage("操作接近实验上限，请先导出并新建画板。"); return; }
    this.touches.clear(); this.activePointer = e.pointerId; this.metrics.lastPointer = e.pointerType || "unknown";
    if (this.options.tool() === "erase") { this.erased.clear(); this.erase(world); return; }
    if (projectInk(this.options.document()).pointCount >= MAX_INK_POINTS) { this.activePointer = undefined; this.options.onMessage("样本达到实验上限，请导出并新建画板。"); return; }
    const pointerType = ["pen", "mouse", "touch"].includes(e.pointerType) ? e.pointerType as "pen" | "mouse" | "touch" : "unknown";
    let segmentId: string;
    try { segmentId = this.options.segmentId(); }
    catch (error) { this.activePointer = undefined; this.options.onMessage(error instanceof Error ? error.message : String(error)); return; }
    this.active = { id: inkId(), page_id: page.id, capture_segment_id: segmentId,
      pointer_type: pointerType, status: "completed", brush: { ...this.options.brush() }, points: [] };
    this.samples(e); this.schedule();
  }
  private move(e: PointerEvent): void {
    if (e.pointerId === this.activePointer) {
      e.preventDefault();
      if (this.active) { this.samples(e); this.schedule(); }
      else this.erase(this.world(this.local(e.clientX, e.clientY)));
      return;
    }
    if (this.navigation?.id === e.pointerId) {
      const p = this.local(e.clientX, e.clientY);
      this.offset.x += p.x - this.navigation.x; this.offset.y += p.y - this.navigation.y;
      this.navigation = { id: e.pointerId, ...p }; this.refresh(); return;
    }
    if (this.activePointer !== undefined || !this.touches.has(e.pointerId)) return;
    const before = [...this.touches.values()];
    this.touches.set(e.pointerId, this.local(e.clientX, e.clientY));
    const after = [...this.touches.values()];
    if (before.length === 1) { this.offset.x += after[0].x - before[0].x; this.offset.y += after[0].y - before[0].y; }
    else {
      const center = (list: { x: number; y: number }[]) => ({ x: (list[0].x + list[1].x) / 2, y: (list[0].y + list[1].y) / 2 });
      const distance = (list: { x: number; y: number }[]) => Math.hypot(list[1].x - list[0].x, list[1].y - list[0].y);
      const previous = center(before), next = center(after), world = this.world(previous);
      this.scale = Math.max(0.05, Math.min(8, this.scale * distance(after) / Math.max(1, distance(before))));
      this.offset = { x: next.x - world.x * this.scale, y: next.y - world.y * this.scale };
    }
    this.refresh();
  }
  private samples(e: PointerEvent): void {
    if (!this.active) return;
    this.metrics.pointerEvents++;
    let events: PointerEvent[] = [];
    try { events = e.getCoalescedEvents?.() ?? []; } catch { /* LAN HTTP can lack this API. */ }
    if (events.length) this.metrics.coalescedSamples += events.length;
    else events = [e];
    for (const sample of events) {
      if (!this.active) break;
      if (this.active.points.length >= 100_000 || this.active.points.length + this.committedPoints >= MAX_INK_POINTS) {
        this.finish("interrupted"); this.options.onMessage("采样达到实验上限；已保留当前笔迹，请导出。"); break;
      }
      const world = this.world(this.local(sample.clientX, sample.clientY));
      const pen = this.active.pointer_type === "pen";
      const point: InkPoint = { ...world, t_us: Math.max(0, Math.round(sample.timeStamp * 1000)),
        pressure: pen && Number.isFinite(sample.pressure) ? sample.pressure : null,
        tilt_x_deg: pen && Number.isFinite(sample.tiltX) ? sample.tiltX : null,
        tilt_y_deg: pen && Number.isFinite(sample.tiltY) ? sample.tiltY : null, buttons: sample.buttons };
      const previous = this.active.points.at(-1);
      if (previous && point.t_us < previous.t_us) { this.finish("interrupted"); this.options.onMessage("采样时间倒退，当前笔迹已标记为中断。"); break; }
      if (!previous || Object.keys(point).some(k => point[k as keyof InkPoint] !== previous[k as keyof InkPoint])) this.active.points.push(point);
    }
  }
  private erase(point: { x: number; y: number }): void {
    for (const stroke of projectInk(this.options.document()).visible) {
      if (stroke.page_id !== this.options.page().id || this.erased.has(stroke.id)) continue;
      const radius = stroke.brush.width / 2 + 10 / this.scale;
      for (let i = 0; i < stroke.points.length; i++) {
        const a = stroke.points[Math.max(0, i - 1)], b = stroke.points[i];
        const dx = b.x - a.x, dy = b.y - a.y;
        const t = Math.max(0, Math.min(1, ((point.x - a.x) * dx + (point.y - a.y) * dy) / (dx * dx + dy * dy || 1)));
        if (Math.hypot(point.x - a.x - t * dx, point.y - a.y - t * dy) <= radius) { this.erased.add(stroke.id); break; }
      }
    }
    this.refresh();
  }
  private up(e: PointerEvent, status: InkStroke["status"]): void {
    if (e.pointerId === this.activePointer) {
      if (status === "completed" && this.active) this.samples(e);
      this.finish(status);
    }
    this.touches.delete(e.pointerId);
    if (this.navigation?.id === e.pointerId) this.navigation = undefined;
  }
  finish(status: InkStroke["status"] = "interrupted"): void {
    const stroke = this.active, erased = [...this.erased];
    this.active = undefined; this.activePointer = undefined; this.erased.clear();
    if (stroke?.points.length) { stroke.status = status; if (status === "cancelled") this.metrics.cancelled++; this.options.onStroke(stroke); }
    else if (erased.length) this.options.onErase(erased);
    this.refresh();
  }
  png(): HTMLCanvasElement {
    this.finish();
    const page = this.options.page(), canvas = document.createElement("canvas");
    const width = Math.min(2400, Math.round(page.width)), height = Math.max(1, Math.round(page.height * width / page.width));
    if (height > 4800) throw new Error("页面尺寸过大，无法生成 PNG；请导出 Ink JSON。");
    canvas.width = width; canvas.height = height;
    const ctx = canvas.getContext("2d")!; ctx.scale(canvas.width / page.width, canvas.height / page.height);
    ctx.fillStyle = "#ffffff"; ctx.fillRect(0, 0, page.width, page.height);
    for (const stroke of projectInk(this.options.document()).visible) if (stroke.page_id === page.id) drawInkStroke(ctx, stroke);
    return canvas;
  }
  dispose(): void {
    this.observer.disconnect(); window.removeEventListener("blur", this.blur);
    cancelAnimationFrame(this.frame);
  }
}
