import { getDocument, GlobalWorkerOptions, type PDFDocumentProxy, type PDFPageProxy } from "pdfjs-dist/legacy/build/pdf.mjs";
import { PDF_ZOOM_DEFAULT, PDF_ZOOM_MAX, PDF_ZOOM_MIN, PDF_ZOOM_STEPS, anchoredScroll, canvasRatio, clampZoom, loadZoom, resolveZoom, saveZoom, stepZoom, zoomLabel, type PdfZoomMode } from "./pdf-zoom";
import { ArtifactPdfTransport } from "./pdf-transport";
import workerUrl from "pdfjs-dist/legacy/build/pdf.worker.min.mjs?url";
import { contentInfo, readRange, readSmallFile, resolveProductPath, type ContentInfo } from "./content";
import { getSourceBacklink } from "./backlink";
import { markdownRenderer } from "./markdown";
import { canonicalReaderProjection, gapLabel, pagesLabel, projectionSummary, segmentLabel, splitByPageMarkers, type CanonicalReaderSegment } from "./canonical";
import { isCanonicalMarkdownId } from "./openable";
import { markdownDocument, MarkdownSelection, type AnchoredMarkdown, type PatchReviewMode } from "./selection";
import { ReaderInkLinks } from "./ink-links";
import { showContextMenu } from "../graph/context-menu";
import { eventBus } from "../core/events";
import { openInkSnapshot } from "../ink/persistence";
import { mountInkSnapshotReader, type InkReaderFocus } from "../ink/snapshot-reader";
import "./reader.css";

GlobalWorkerOptions.workerSrc = workerUrl;

function element<K extends keyof HTMLElementTagNameMap>(tag: K, text = ""): HTMLElementTagNameMap[K] {
  const result = document.createElement(tag);
  result.textContent = text;
  return result;
}

export interface ReaderFocus { start: number; end: number; blockIds?: string[] }

export interface ReaderReturnTarget {
  id: string;
  title: string;
  options?: ReaderOpenOptions;
  label?: string;
}

export interface ReaderOpenOptions {
  focus?: ReaderFocus;
  /** Zero-based PDF page index used by provenance back-links. */
  pdfPageIndex?: number;
  /** Zero-based PDF page indices when an anchor spans or matches multiple pages. */
  candidatePages?: number[];
  /** Reason note shown when a PDF jump comes from a candidate multi-page mapping. */
  backlinkNote?: string;
  /** Optional round-trip target to jump right back to the Markdown block/view. */
  returnTo?: ReaderReturnTarget;
  /** Opens the Markdown patch review of a run instead of the latest pending one. */
  patch?: { runId: string; mode: PatchReviewMode };
  conversionId?: string | null;
  inkFocus?: InkReaderFocus;
}

export class ArtifactReader {
  private generation = 0;
  private disposeDocument: (() => void) | undefined;
  private urls: string[] = [];
  private inkOpen = false;
  private state: "empty" | "loading" | "ready" | "error" = "empty";
  private error: string | undefined;
  /** Last successfully opened artifact — kept across close() for 并排阅读 restore. */
  private lastOpen: { id: string; title: string; options: ReaderOpenOptions } | undefined;
  private overlayCloseHandler: (() => void) | undefined;
  constructor(private host: HTMLElement) {}

  setOverlayCloseHandler(handler: (() => void) | undefined): void {
    this.overlayCloseHandler = handler;
  }

  private revokeImages(): void {
    this.urls.forEach(url => URL.revokeObjectURL(url));
    this.urls = [];
  }

  close(): void {
    this.generation++;
    this.disposeDocument?.();
    this.disposeDocument = undefined;
    this.revokeImages();
    this.host.replaceChildren();
    this.inkOpen = false;
    this.state = "empty";
    this.error = undefined;
  }

  isInkOpen(): boolean { return this.inkOpen; }

  /** Loading headings and error messages are not successfully opened content. */
  hasContent(): boolean { return this.state === "ready"; }
  isLoading(): boolean { return this.state === "loading"; }
  lastError(): string | undefined { return this.error; }

  lastOpened(): { id: string; title: string; options: ReaderOpenOptions } | undefined {
    return this.lastOpen ? { ...this.lastOpen, options: { ...this.lastOpen.options } } : undefined;
  }

  /** False means the read failed or was superseded by another open/close. */
  async open(id: string, title: string, options: ReaderOpenOptions = {}): Promise<boolean> {
    this.close();
    this.state = "loading";
    eventBus.emit("reader-opened");
    const { focus, patch, conversionId } = options;
    const generation = this.generation;
    const heading = element("h2", title);
    const status = element("p", "正在读取…");
    if (this.overlayCloseHandler && this.host.parentElement?.id !== "ink-split-reader") {
      const headerRow = element("div");
      headerRow.className = "reader-heading-row";
      const closeBtn = element("button", "← 返回当前地图");
      closeBtn.type = "button";
      closeBtn.className = "reader-close-overlay-btn";
      closeBtn.onclick = () => this.overlayCloseHandler?.();
      headerRow.append(heading, closeBtn);
      this.host.append(headerRow, status);
    } else {
      this.host.append(heading, status);
    }
    try {
      if (isCanonicalMarkdownId(id)) {
        // Logical canonical id: a stitched read-only view, never content_info.
        await this.canonical(id, title, generation);
        if (generation !== this.generation) return false;
        status.remove();
        this.state = "ready";
        this.lastOpen = { id, title, options: { ...options } };
        return true;
      }
      const info = await contentInfo(id);
      if (generation !== this.generation) return false;
      if (info.ink_snapshot_id) {
        const snapshot = await openInkSnapshot(info.ink_snapshot_id);
        if (generation !== this.generation) return false;
        if (!snapshot || snapshot.artifact_id !== id) throw new Error("Ink 快照与所选 Artifact 不一致");
        this.disposeDocument = mountInkSnapshotReader(this.host,snapshot,options.inkFocus);
        this.inkOpen = true;
      } else if (info.media_type === "application/pdf") {
        await this.pdf(info, generation, options);
      } else if (info.media_type === "text/markdown") {
        await this.markdown(info, title, generation, focus, patch, conversionId);
      } else if (info.media_type === "application/json" || info.media_type.startsWith("text/")) {
        const bytes = await readSmallFile(info);
        if (generation !== this.generation) return false;
        const text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
        const pre = element("pre", info.media_type === "application/json" ? JSON.stringify(JSON.parse(text), null, 2) : text);
        pre.className = "reader-text";
        this.host.append(pre);
      } else if (["image/png", "image/jpeg", "image/webp", "image/gif"].includes(info.media_type)) {
        const bytes = await readSmallFile(info);
        if (generation !== this.generation) return false;
        const img = element("img");
        img.src = this.blobUrl(bytes, info.media_type);
        img.alt = title;
        this.host.append(img);
      } else {
        throw new Error("当前不支持此文件类型：" + info.media_type);
      }
      if (generation !== this.generation) return false;
      if (this.error) throw new Error(this.error);
      status.remove();
      this.state = "ready";
      this.lastOpen = { id, title, options: { ...options } };
      return true;
    } catch (error) {
      if (generation === this.generation) {
        this.state = "error";
        this.error = error instanceof Error ? error.message : String(error);
        status.textContent = this.error;
      }
      return false;
    }
  }

  private blobUrl(bytes: Uint8Array, media: string): string {
    const url = URL.createObjectURL(new Blob([bytes.slice().buffer], { type: media }));
    this.urls.push(url);
    return url;
  }

  private async pdf(info: ContentInfo, generation: number, pdfOptions: ReaderOpenOptions = {}): Promise<void> {
    const requestedPageIndex = pdfOptions.pdfPageIndex;
    const initial = await readRange(info.artifact_id, 0, Math.min(65536, info.byte_size));
    if (generation !== this.generation) return;
    const failure = element("p");
    let firstFailure: Error | undefined;
    let rejectRead!: (error: Error) => void;
    const readFailure = new Promise<never>((_, reject) => { rejectRead = reject; });
    // Failures can also occur after loading.promise has resolved.
    void readFailure.catch(() => {});
    const transport = new ArtifactPdfTransport(info.byte_size, initial,
      (begin, length) => readRange(info.artifact_id, begin, length),
      error => {
        if (generation !== this.generation || firstFailure) return;
        firstFailure = error;
        this.state = "error";
        this.error = error.message;
        failure.textContent = error.message;
        console.error("PDF range read failed", { artifactId: info.artifact_id, error });
        rejectRead(error);
        dispose();
      });
    const loading = getDocument({
      range: transport, disableAutoFetch: true, disableStream: true,
      cMapUrl: "/pdf-assets/cmaps/", cMapPacked: true,
      standardFontDataUrl: "/pdf-assets/standard_fonts/",
      wasmUrl: "/pdf-assets/wasm/",
    });
    let destroyed = false;
    const dispose = () => {
      transport.abort();
      if (destroyed) return;
      destroyed = true;
      void loading.destroy().catch(error => console.warn("PDF cleanup failed", error));
    };
    this.disposeDocument = dispose;
    this.host.append(failure);
    let pdf: PDFDocumentProxy;
    try {
      pdf = await Promise.race([loading.promise, readFailure]);
    } catch (error) {
      dispose();
      failure.remove();
      throw firstFailure ?? error;
    }
    if (generation !== this.generation || transport.stopped) return;
    const controls = element("div");
    controls.className = "reader-controls";
    const previous = element("button", "上一页");
    const next = element("button", "下一页");
    const pageInput = element("input");
    pageInput.type = "number"; pageInput.min = "1"; pageInput.max = String(pdf.numPages); pageInput.value = "1";
    pageInput.setAttribute("aria-label", "PDF 页码");
    // N3.0 zoom: fit width by default, presets, ±, Ctrl/Cmd +/-/0, Ctrl+wheel and pinch.
    const outlineToggle = element("button", "目录");
    outlineToggle.title = "显示或隐藏目录";
    const zoomOut = element("button", "−"); zoomOut.setAttribute("aria-label", "缩小"); zoomOut.title = "缩小（Ctrl/⌘ −）";
    const zoomIn = element("button", "＋"); zoomIn.setAttribute("aria-label", "放大"); zoomIn.title = "放大（Ctrl/⌘ +）";
    const zoom = element("select");
    zoom.setAttribute("aria-label", "缩放");
    zoom.title = "Ctrl/⌘ 0 恢复适合宽度；按住 Ctrl/⌘ 滚动或双指捏合可缩放";
    controls.append(previous, pageInput, element("span", "/ " + pdf.numPages + " 页"), next, outlineToggle, zoomOut, zoom, zoomIn);
    if (pdfOptions.returnTo) {
      const target = pdfOptions.returnTo;
      const back = element("button", `← ${target.label ?? "返回 Markdown 原文"}`);
      back.type = "button";
      back.className = "reader-return-button";
      back.onclick = () => void this.open(target.id, target.title, target.options);
      controls.prepend(back);
    }
    const candidateChipButtons: Array<{ pageNumber: number; button: HTMLButtonElement }> = [];
    if (pdfOptions.candidatePages && pdfOptions.candidatePages.length > 1) {
      const group = element("span");
      group.className = "reader-candidate-pages";
      group.append(element("span", "关联页："));
      for (const idx of pdfOptions.candidatePages) {
        const targetNum = idx + 1;
        if (targetNum < 1 || targetNum > pdf.numPages) continue;
        const chip = element("button", `第 ${targetNum} 页`);
        chip.type = "button";
        chip.className = "reader-page-chip";
        chip.onclick = () => void show(targetNum);
        candidateChipButtons.push({ pageNumber: targetNum, button: chip });
        group.append(chip);
      }
      if (candidateChipButtons.length > 1) controls.append(group);
    }
    const layout = element("div"); layout.className = "pdf-layout";
    const outline = element("nav"); outline.className = "pdf-outline"; outline.setAttribute("aria-label", "PDF 目录");
    const viewport = element("div"); viewport.className = "pdf-viewport";
    layout.append(outline, viewport);
    if (pdfOptions.backlinkNote) {
      const note = element("p", pdfOptions.backlinkNote);
      note.className = "reader-backlink-note";
      this.host.append(controls, note, layout);
    } else {
      this.host.append(controls, layout);
    }
    const outlineKey = "mathscape:pdf-outline";
    const setOutline = (visible: boolean): void => {
      outline.hidden = !visible;
      outlineToggle.setAttribute("aria-pressed", String(visible));
      try { localStorage.setItem(outlineKey, visible ? "1" : "0"); } catch { /* Optional preference. */ }
    };
    let outlineVisible = true;
    try { outlineVisible = localStorage.getItem(outlineKey) !== "0"; } catch { /* Optional preference. */ }
    setOutline(outlineVisible);
    outlineToggle.onclick = () => setOutline(outline.hidden === true);
    let pageNumber = Number.isInteger(requestedPageIndex)
      ? Math.min(Math.max((requestedPageIndex as number) + 1, 1), pdf.numPages)
      : 1;
    let renderVersion = 0;
    // Navigation has its own version: a zoom while the next page loads must
    // not discard that page (the zoom only bumps renderVersion).
    let navigationVersion = 0, navigating = false;
    let renderTask: ReturnType<PDFPageProxy["render"]> | undefined;
    let current: PDFPageProxy | undefined;
    let pageSize = { width: 1, height: 1 };
    let mode: PdfZoomMode = loadZoom(info.artifact_id);
    let scale = 1;
    let renderTimer: ReturnType<typeof setTimeout> | undefined;
    const available = () => ({
      width: Math.max(1, viewport.clientWidth - 2),
      height: Math.max(200, viewport.clientHeight || window.innerHeight - viewport.getBoundingClientRect().top - 24),
    });
    const presetValues = PDF_ZOOM_STEPS.filter(step => step >= 0.5 && step <= 3);
    const renderZoomControl = (): void => {
      const value = mode.kind === "scale" ? "s:" + mode.value : mode.kind;
      const options: Array<[string, string]> = [["fit-width", zoomLabel({ kind: "fit-width" }, mode.kind === "fit-width" ? scale : resolveZoom({ kind: "fit-width" }, pageSize, available()))],
        ["fit-page", zoomLabel({ kind: "fit-page" }, mode.kind === "fit-page" ? scale : resolveZoom({ kind: "fit-page" }, pageSize, available()))],
        ...presetValues.map(step => ["s:" + step, Math.round(step * 100) + "%"] as [string, string])];
      if (mode.kind === "scale" && !presetValues.includes(mode.value)) options.push([value, Math.round(mode.value * 100) + "%"]);
      zoom.replaceChildren(...options.map(([key, label]) => new Option(label, key)));
      zoom.value = value;
      zoomOut.disabled = scale <= PDF_ZOOM_MIN + 1e-3;
      zoomIn.disabled = scale >= PDF_ZOOM_MAX - 1e-3;
      viewport.dataset.scale = String(scale);
      viewport.dataset.zoomMode = mode.kind;
    };
    const draw = async (version: number): Promise<void> => {
      const page = current;
      if (!page || transport.stopped || version !== renderVersion) return;
      renderTask?.cancel();
      const view = page.getViewport({ scale });
      const canvas = element("canvas");
      const ratio = canvasRatio(window.devicePixelRatio || 1, view.width, view.height);
      canvas.width = Math.ceil(view.width * ratio); canvas.height = Math.ceil(view.height * ratio);
      canvas.style.width = view.width + "px"; canvas.style.height = view.height + "px";
      canvas.dataset.ratio = String(ratio);
      const swap = () => {
        const scrollLeft = viewport.scrollLeft, scrollTop = viewport.scrollTop;
        viewport.replaceChildren(canvas);
        viewport.scrollLeft = scrollLeft; viewport.scrollTop = scrollTop;
      };
      // A new page is shown at once; a re-zoom swaps only the finished bitmap.
      const replacing = viewport.querySelector("canvas") !== null && canvas.style.width === viewport.querySelector("canvas")!.style.width;
      if (!replacing) swap();
      try {
        renderTask = page.render({ canvas, viewport: view, transform: [ratio, 0, 0, ratio, 0, 0] });
        await renderTask.promise;
        if (replacing && version === renderVersion && generation === this.generation) swap();
      } catch (error) {
        if (!firstFailure && !transport.stopped && version === renderVersion && generation === this.generation && !(error instanceof Error && error.name === "RenderingCancelledException")) failure.textContent = String(error);
      }
    };
    /** Re-scale immediately (stretching the current bitmap so the anchor
     * stays put), then re-render crisply once the zoom settles. */
    const applyZoom = (next: PdfZoomMode, anchor?: { x: number; y: number }, persist = true): void => {
      mode = next;
      if (persist) saveZoom(info.artifact_id, mode);
      if (!current) { renderZoomControl(); return; }
      const nextScale = resolveZoom(mode, pageSize, available());
      if (navigating) {
        // The page being loaded is drawn at the latest mode once it arrives.
        scale = nextScale; renderZoomControl();
        return;
      }
      if (Math.abs(nextScale - scale) < 1e-4) { renderZoomControl(); return; }
      const x = anchor?.x ?? viewport.clientWidth / 2, y = anchor?.y ?? viewport.clientHeight / 2;
      const before = { width: pageSize.width * scale, height: pageSize.height * scale };
      const after = { width: pageSize.width * nextScale, height: pageSize.height * nextScale };
      const left = anchoredScroll(viewport.scrollLeft, x, before.width, after.width);
      const top = anchoredScroll(viewport.scrollTop, y, before.height, after.height);
      const canvas = viewport.querySelector("canvas");
      if (canvas) { canvas.style.width = after.width + "px"; canvas.style.height = after.height + "px"; }
      viewport.scrollLeft = left; viewport.scrollTop = top;
      scale = nextScale;
      renderZoomControl();
      const version = ++renderVersion;
      if (renderTimer) clearTimeout(renderTimer);
      renderTimer = setTimeout(() => { renderTimer = undefined; void draw(version); }, 120);
    };
    const oldDispose = this.disposeDocument;
    const cleanups: Array<() => void> = [];
    this.disposeDocument = () => { renderVersion++; navigationVersion++; if (renderTimer) clearTimeout(renderTimer); renderTask?.cancel(); cleanups.forEach(cleanup => cleanup()); oldDispose?.(); };
    const show = async (number: number): Promise<void> => {
      if (transport.stopped) return;
      if (!Number.isInteger(number) || number < 1 || number > pdf.numPages) { pageInput.value = String(pageNumber); return; }
      pageNumber = number; pageInput.value = String(number);
      previous.disabled = number === 1; next.disabled = number === pdf.numPages;
      for (const item of candidateChipButtons) {
        item.button.setAttribute("aria-pressed", String(item.pageNumber === number));
      }
      const navigation = ++navigationVersion;
      navigating = true;
      renderVersion++;
      if (renderTimer) { clearTimeout(renderTimer); renderTimer = undefined; }
      renderTask?.cancel();
      try {
        const page = await pdf.getPage(number);
        if (transport.stopped || generation !== this.generation || navigation !== navigationVersion) return;
        navigating = false;
        current = page;
        const base = page.getViewport({ scale: 1 });
        pageSize = { width: base.width, height: base.height };
        scale = resolveZoom(mode, pageSize, available());
        renderZoomControl();
        viewport.scrollTop = 0;
        await draw(++renderVersion);
      } catch (error) {
        if (navigation === navigationVersion) navigating = false;
        if (!firstFailure && !transport.stopped && navigation === navigationVersion && generation === this.generation) failure.textContent = String(error);
      }
    };
    previous.onclick = () => void show(pageNumber - 1);
    next.onclick = () => void show(pageNumber + 1);
    pageInput.onchange = () => void show(pageInput.valueAsNumber);
    zoom.onchange = () => {
      const value = zoom.value;
      applyZoom(value === "fit-width" || value === "fit-page" ? { kind: value } : { kind: "scale", value: clampZoom(Number(value.slice(2))) });
    };
    const stepBy = (direction: 1 | -1, anchor?: { x: number; y: number }) => applyZoom({ kind: "scale", value: stepZoom(scale, direction) }, anchor);
    zoomIn.onclick = () => stepBy(1);
    zoomOut.onclick = () => stepBy(-1);
    const visible = () => viewport.isConnected && viewport.getClientRects().length > 0;
    const onKey = (event: KeyboardEvent): void => {
      // M1: the split 手稿 pane handles its own Ctrl/⌘ zoom first (capture phase).
      if (event.defaultPrevented || !(event.ctrlKey || event.metaKey) || event.altKey || !visible()) return;
      if (event.key === "+" || event.key === "=") stepBy(1);
      else if (event.key === "-" || event.key === "_") stepBy(-1);
      else if (event.key === "0") applyZoom(PDF_ZOOM_DEFAULT);
      else return;
      event.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    cleanups.push(() => window.removeEventListener("keydown", onKey));
    const pointAt = (clientX: number, clientY: number) => {
      const rect = viewport.getBoundingClientRect();
      return { x: clientX - rect.left, y: clientY - rect.top };
    };
    viewport.addEventListener("wheel", event => {
      // Chromium and trackpad pinch both report Ctrl+wheel.
      if (!(event.ctrlKey || event.metaKey)) return;
      event.preventDefault();
      const delta = event.deltaY * (event.deltaMode === 1 ? 33 : event.deltaMode === 2 ? 400 : 1);
      applyZoom({ kind: "scale", value: clampZoom(scale * Math.exp(-delta * 0.0015)) }, pointAt(event.clientX, event.clientY));
    }, { passive: false });
    // WebKit (the macOS app) reports trackpad pinch as gesture events.
    let gestureStart = 1;
    type Gesture = Event & { scale: number; clientX: number; clientY: number };
    viewport.addEventListener("gesturestart", event => { event.preventDefault(); gestureStart = scale; });
    viewport.addEventListener("gesturechange", event => {
      event.preventDefault();
      const gesture = event as Gesture;
      applyZoom({ kind: "scale", value: clampZoom(gestureStart * gesture.scale) }, pointAt(gesture.clientX, gesture.clientY));
    });
    viewport.addEventListener("gestureend", event => event.preventDefault());
    // Fit modes follow the pane size (window resize, outline toggle, splitter).
    if (typeof ResizeObserver !== "undefined") {
      let lastWidth = viewport.clientWidth, lastHeight = viewport.clientHeight;
      const observer = new ResizeObserver(() => {
        if (viewport.clientWidth === lastWidth && viewport.clientHeight === lastHeight) return;
        lastWidth = viewport.clientWidth; lastHeight = viewport.clientHeight;
        if (mode.kind !== "scale") applyZoom(mode, { x: 0, y: 0 }, false);
      });
      observer.observe(viewport);
      cleanups.push(() => observer.disconnect());
    }
    await show(pageNumber);
    if (transport.stopped) return;
    const items = await pdf.getOutline().catch(error => {
      if (firstFailure || transport.stopped) return null;
      throw error;
    });
    if (generation !== this.generation) return;
    if (!items?.length) outline.append(element("p", "此 PDF 没有内置书签目录，可输入页码跳转。"));
    const add = (entries: NonNullable<typeof items>, parent: HTMLElement): void => {
      for (const item of entries) {
        const button = element("button", item.title);
        button.disabled = !item.dest;
        button.onclick = () => void (async () => {
          const destination = typeof item.dest === "string" ? await pdf.getDestination(item.dest) : item.dest;
          if (!destination) return;
          const index = typeof destination[0] === "number" ? destination[0] : await pdf.getPageIndex(destination[0]);
          if (generation === this.generation) await show(index + 1);
        })().catch(error => { if (!firstFailure && !transport.stopped && generation === this.generation) failure.textContent = String(error); });
        parent.append(button);
        if (item.items.length) { const child = element("div"); parent.append(child); add(item.items, child); }
      }
    };
    if (items) add(items, outline);
  }

  /**
   * 「规范解析正文」 unified view: every coverage segment in page order with
   * gaps marked. Read-only — block selection, discoveries and ink stay on the
   * physical snapshot, reachable through each segment's 「打开此快照」.
   */
  private async canonical(id: string, title: string, generation: number): Promise<void> {
    const projection = await canonicalReaderProjection(id);
    if (generation !== this.generation) return;
    const controls = element("div"); controls.className = "reader-controls";
    controls.append(element("span", projectionSummary(projection)));
    const openRootPdf = element("button", "打开原始 PDF");
    openRootPdf.type = "button";
    openRootPdf.onclick = () => void this.open(projection.source_pdf_artifact_id, `${title} · 原始 PDF`, {
      returnTo: { id, title, label: "返回规范解析正文" },
    });
    controls.append(openRootPdf);
    const article = element("article"); article.className = "markdown-body canonical-reader";
    article.dataset.canonicalId = projection.canonical_id;
    this.host.append(controls, article);
    const openCanonicalPdfPage = (pageIndex: number, candidatePages?: number[]) => {
      void this.open(projection.source_pdf_artifact_id, `${title} · PDF 原页`, {
        pdfPageIndex: pageIndex,
        candidatePages,
        returnTo: { id, title, label: "返回规范解析正文" },
      });
    };
    const md = markdownRenderer();
    const segments: Array<{ segment: CanonicalReaderSegment; body: HTMLElement }> = [];
    for (const piece of projection.pieces) {
      if (piece.kind === "gap") {
        const gap = element("div", gapLabel(piece));
        gap.className = "canonical-gap";
        gap.setAttribute("role", "note");
        gap.dataset.pages = `${piece.pages.from}-${piece.pages.to}`;
        article.append(gap);
        continue;
      }
      const section = element("section"); section.className = "canonical-segment";
      section.dataset.artifactId = piece.artifact_id;
      const header = element("div"); header.className = "canonical-segment-header";
      const headerActions = element("div"); headerActions.className = "canonical-segment-actions";
      if (piece.pages) {
        const jumpPdf = element("button", `查看 PDF ${pagesLabel(piece.pages)}`);
        jumpPdf.type = "button";
        jumpPdf.title = "打开此分段对应的原始 PDF 页面";
        const spanPages = piece.pages.to - piece.pages.from <= 7
          ? Array.from({ length: piece.pages.to - piece.pages.from + 1 }, (_, k) => piece.pages!.from - 1 + k)
          : undefined;
        jumpPdf.onclick = () => openCanonicalPdfPage(piece.pages!.from - 1, spanPages);
        headerActions.append(jumpPdf);
      }
      const open = element("button", "打开此快照");
      open.type = "button";
      open.title = "以物理快照打开，可选择原文块、记录发现或手写批注";
      open.onclick = () => void this.open(piece.artifact_id, title + " · " + segmentLabel(piece));
      headerActions.append(open);
      header.append(element("span", segmentLabel(piece)), headerActions);
      const body = element("div");
      let activePageIndex: number | null = piece.pages ? piece.pages.from - 1 : null;
      for (const chunk of splitByPageMarkers(piece.markdown)) {
        if (chunk.kind === "page") {
          activePageIndex = chunk.pageId;
          const marker = element("div"); marker.className = "source-block source-page-marker";
          marker.append(element("span", `PDF 第 ${chunk.pageId + 1} 页`));
          const jump = element("button", "查看原页 ↗");
          jump.type = "button";
          jump.className = "page-marker-jump";
          const targetIdx = chunk.pageId;
          jump.onclick = (event) => { event.stopPropagation(); openCanonicalPdfPage(targetIdx); };
          marker.append(jump);
          body.append(marker);
          continue;
        }
        const block = element("div"); block.className = "source-block";
        if (activePageIndex !== null) block.dataset.pdfPageIndex = String(activePageIndex);
        block.dataset.segmentArtifactId = piece.artifact_id;
        block.dataset.segmentLabel = segmentLabel(piece);
        block.innerHTML = md.render(chunk.text, {});
        if (!block.innerHTML.trim()) block.append(element("pre", chunk.text));
        body.append(block);
      }
      section.append(header, body);
      article.append(section);
      segments.push({ segment: piece, body });
    }
    const onCanonicalContextMenu = (event: MouseEvent): void => {
      const block = (event.target as Element).closest<HTMLElement>(".source-block:not(.source-page-marker)");
      if (!block || !article.contains(block)) return;
      const pageAttr = block.dataset.pdfPageIndex;
      const segArt = block.dataset.segmentArtifactId;
      const segLbl = block.dataset.segmentLabel ?? "";
      if (pageAttr === undefined && !segArt) return;
      event.preventDefault();
      block.classList.add("is-ink-menu-target");
      const clearHighlight = () => block.classList.remove("is-ink-menu-target");
      const items: Array<{ label: string; onSelect: () => void }> = [];
      if (pageAttr !== undefined) {
        const idx = Number(pageAttr);
        items.push({
          label: `定位对应 PDF 原页（第 ${idx + 1} 页）`,
          onSelect: () => { clearHighlight(); openCanonicalPdfPage(idx); },
        });
      }
      if (segArt) {
        items.push({
          label: "打开所属物理快照",
          onSelect: () => { clearHighlight(); void this.open(segArt, `${title} · ${segLbl}`); },
        });
      }
      showContextMenu(event.clientX, event.clientY, items, clearHighlight);
    };
    article.addEventListener("contextmenu", onCanonicalContextMenu);
    this.disposeDocument = () => article.removeEventListener("contextmenu", onCanonicalContextMenu);
    if (!projection.pieces.length) article.append(element("p", "规范解析正文暂无可读快照。"));
    article.querySelectorAll("a").forEach(link => { link.removeAttribute("href"); link.title = "链接导航尚未启用"; });
    for (const { segment, body } of segments) {
      const placeholders = body.querySelectorAll<HTMLElement>("[data-image-ref]");
      if (!placeholders.length) continue;
      let info: ContentInfo | undefined;
      try { info = await contentInfo(segment.artifact_id); } catch { /* shown per image */ }
      if (generation !== this.generation) return;
      const context = info?.contexts.find(c => c.conversion_id === segment.conversion_id) ?? info?.contexts[0];
      const bases = context?.paths.filter(p => p.artifact_id === segment.artifact_id) ?? [];
      for (const placeholder of placeholders) {
        try {
          const candidates = new Set<string>();
          for (const base of bases) {
            const path = resolveProductPath(base.path, placeholder.dataset.imageRef ?? "");
            context?.paths.filter(p => p.path === path).forEach(p => candidates.add(p.artifact_id));
          }
          if (candidates.size !== 1) throw new Error("图片映射缺失或不明确");
          const imageInfo = await contentInfo([...candidates][0]);
          if (!["image/png", "image/jpeg", "image/webp", "image/gif"].includes(imageInfo.media_type)) throw new Error("不支持的图片类型");
          const data = await readSmallFile(imageInfo);
          if (generation !== this.generation) return;
          const img = element("img"); img.alt = placeholder.textContent ?? "";
          img.src = this.blobUrl(data, imageInfo.media_type);
          placeholder.replaceWith(img);
        } catch (error) {
          placeholder.textContent = "[" + String(error) + "]";
        }
      }
    }
  }

  private async markdown(info: ContentInfo, title: string, generation: number, focus?: ReaderFocus, patch?: ReaderOpenOptions["patch"], conversionId?: string | null): Promise<void> {
    let document = await markdownDocument(info.artifact_id, conversionId ?? info.contexts[0]?.conversion_id ?? null);
    if (generation !== this.generation) return;
    const controls = element("div"); controls.className = "reader-controls";
    const raw = element("button", "查看原文");
    const contextSelect = element("select");
    contextSelect.setAttribute("aria-label", "转换来源");
    info.contexts.forEach((context, i) => contextSelect.add(new Option("转换来源 " + (i + 1), String(i))));
    if (conversionId) {
      const idx = info.contexts.findIndex(c => c.conversion_id === conversionId);
      if (idx >= 0) contextSelect.value = String(idx);
    }
    controls.append(raw);
    if (info.contexts.length > 1) controls.append(contextSelect);
    const article = element("article"); article.className = "markdown-body";
    this.host.append(controls, article);
    let rawMode = false, version = 0;
    let inkLinks: ReaderInkLinks | undefined;
    const currentConversionId = (): string | null =>
      info.contexts[Number(contextSelect.value) || 0]?.conversion_id ?? conversionId ?? null;
    const resolveFocusedBlockRange = (doc: AnchoredMarkdown): { first: number; last: number } | null => {
      if (!focus) return null;
      const matched: number[] = [];
      doc.blocks.forEach((block, index) => {
        if (block.kind === "page_marker") return;
        const byId = Boolean(focus.blockIds?.length && focus.blockIds.includes(block.id));
        const byBytes = !focus.blockIds?.length && focus.end > focus.start && block.range.start < focus.end && block.range.end > focus.start;
        if (byId || byBytes) matched.push(index);
      });
      return matched.length ? { first: matched[0], last: matched[matched.length - 1] } : null;
    };
    const handwrite = (range: { first: number; last: number }, anchor: HTMLElement) => {
      const bounds = anchor.getBoundingClientRect();
      inkLinks?.create(range, bounds.left, bounds.bottom + 4);
    };
    let selection!: MarkdownSelection;
    const openPdfForRange = async (range: { first: number; last: number }, preferredPageIndex?: number): Promise<void> => {
      const contentBlocks = document.blocks.slice(range.first, range.last + 1).filter(b => b.kind !== "page_marker");
      if (!contentBlocks.length) {
        selection.notifyFeedback("所选范围仅包含分页标记，请选择具体正文块。");
        return;
      }
      const start = contentBlocks[0].range.start;
      const end = contentBlocks[contentBlocks.length - 1].range.end;
      const blockIds = contentBlocks.map(b => b.id);
      try {
        const backlink = await getSourceBacklink({
          artifact_id: info.artifact_id,
          conversion_id: currentConversionId(),
          start,
          end,
          block_ids: blockIds,
        });
        if (generation !== this.generation) return;
        if (backlink.kind === "markdown_only" || !backlink.pdf_artifact_id || !backlink.pdf_page_indices?.length) {
          selection.notifyFeedback(backlink.reason ?? "当前段落没有对应的 PDF 页映射。");
          return;
        }
        const pages = backlink.pdf_page_indices;
        const targetPage = preferredPageIndex !== undefined && pages.includes(preferredPageIndex)
          ? preferredPageIndex
          : pages[0];
        await this.open(backlink.pdf_artifact_id, `${title} · PDF 原页`, {
          pdfPageIndex: targetPage,
          candidatePages: pages,
          backlinkNote: backlink.kind === "candidate_pdf_pages" ? backlink.reason : undefined,
          returnTo: {
            id: info.artifact_id,
            title,
            options: {
              conversionId: currentConversionId(),
              focus: { start, end, blockIds },
            },
            label: "返回 Markdown 原文",
          },
        });
      } catch (error) {
        if (generation === this.generation) {
          selection.notifyFeedback("PDF 回链查询失败：" + (error instanceof Error ? error.message : String(error)));
        }
      }
    };
    selection = new MarkdownSelection(
      document,
      article,
      patch,
      handwrite,
      (range) => void openPdfForRange(range),
      resolveFocusedBlockRange(document),
    );
    controls.after(selection.panel);
    inkLinks = new ReaderInkLinks(
      document,
      article,
      () => selection,
      () => rawMode,
      (range) => void openPdfForRange(range),
    );
    selection.setOnSelectionChange(() => inkLinks?.syncActionPill());
    controls.after(inkLinks.hint);
    this.disposeDocument = () => { selection.dispose(); inkLinks?.dispose(); };
    const render = async (): Promise<void> => {
      const current = ++version;
      this.revokeImages();
      if (rawMode) { article.replaceChildren(element("pre", document.source)); inkLinks?.decorate(); return; }
      const md = markdownRenderer();
      const environment = {};
      // Parse the full source once to retain reference-style link/image definitions.
      md.parse(document.source, environment);
      const sourceBytes = new TextEncoder().encode(document.source);
      const decoder = new TextDecoder("utf-8", { fatal: true });
      article.replaceChildren();
      document.blocks.forEach((block, i) => {
        const wrapper = element("div");
        wrapper.className = "source-block";
        wrapper.dataset.sourceBlock = block.id;
        const isFocused = Boolean(
          focus && (
            focus.blockIds?.length
              ? focus.blockIds.includes(block.id)
              : focus.end > focus.start && block.kind !== "page_marker" && block.range.start < focus.end && block.range.end > focus.start
          ),
        );
        if (isFocused) wrapper.classList.add("source-block-focus");
        wrapper.tabIndex = 0;
        wrapper.setAttribute("aria-label", `原文块 ${i + 1}`);
        wrapper.title = `第 ${i + 1} 块 · 单击选择，按住 Shift 单击可连选多块，右键手写或反查 PDF`;
        const text = decoder.decode(sourceBytes.subarray(block.range.start, block.range.end));
        if (block.kind === "page_marker") {
          const page = /^\s*\{(\d+)\}-{16,}\s*$/.exec(text)?.[1];
          wrapper.classList.add("source-page-marker");
          wrapper.setAttribute("aria-label", page ? `PDF 第 ${Number(page) + 1} 页` : "PDF 页边界");
          wrapper.title = page ? `PDF 第 ${Number(page) + 1} 页（Marker page_id=${page}）` : "PDF 页边界";
          wrapper.append(element("span", page ? `PDF 第 ${Number(page) + 1} 页` : "PDF 页边界"));
          if (page !== undefined && info.contexts.length > 0) {
            const jump = element("button", "查看原页 ↗");
            jump.type = "button";
            jump.className = "page-marker-jump";
            jump.onclick = (event) => {
              event.stopPropagation();
              let adj = -1;
              for (let j = i + 1; j < document.blocks.length; j++) {
                if (document.blocks[j].kind !== "page_marker") { adj = j; break; }
              }
              if (adj < 0) {
                for (let j = i - 1; j >= 0; j--) {
                  if (document.blocks[j].kind !== "page_marker") { adj = j; break; }
                }
              }
              if (adj >= 0) void openPdfForRange({ first: adj, last: adj }, Number(page));
              else selection.notifyFeedback("当前文档没有可用于验证页映射的正文块。");
            };
            wrapper.append(jump);
          }
          article.append(wrapper);
          return;
        }
        wrapper.innerHTML = md.render(text, environment);
        if (!wrapper.innerHTML.trim()) wrapper.append(element("pre", text));
        article.append(wrapper);
      });
      if (focus) (article.querySelector<HTMLElement>(".source-block-focus") as HTMLElement | null)?.scrollIntoView({ behavior: "smooth", block: "center" });
      selection.refresh();
      inkLinks?.decorate();
      article.querySelectorAll("a").forEach(link => {
        link.removeAttribute("href");
        link.title = "链接导航尚未启用";
      });
      const context = info.contexts[Number(contextSelect.value) || 0];
      const bases = context?.paths.filter(p => p.artifact_id === info.artifact_id) ?? [];
      const placeholders = article.querySelectorAll<HTMLElement>("[data-image-ref]");
      for (const placeholder of placeholders) {
        if (generation !== this.generation || current !== version) return;
        try {
          const candidates = new Set<string>();
          for (const base of bases) {
            const path = resolveProductPath(base.path, placeholder.dataset.imageRef ?? "");
            context?.paths.filter(p => p.path === path).forEach(p => candidates.add(p.artifact_id));
          }
          if (candidates.size !== 1) throw new Error("图片映射缺失或不明确");
          const imageInfo = await contentInfo([...candidates][0]);
          if (!["image/png", "image/jpeg", "image/webp", "image/gif"].includes(imageInfo.media_type)) throw new Error("不支持的图片类型");
          const data = await readSmallFile(imageInfo);
          if (generation !== this.generation || current !== version) return;
          const img = element("img"); img.alt = placeholder.textContent ?? "";
          img.src = this.blobUrl(data, imageInfo.media_type);
          placeholder.replaceWith(img);
        } catch (error) {
          placeholder.textContent = "[" + String(error) + "]";
        }
      }
    };
    raw.onclick = () => { rawMode = !rawMode; raw.textContent = rawMode ? "查看渲染" : "查看原文"; void render(); };
    contextSelect.onchange = () => {
      const current = ++version;
      selection.dispose();
      this.revokeImages();
      article.textContent = "正在切换转换来源…";
      raw.disabled = true;
      void markdownDocument(info.artifact_id, info.contexts[Number(contextSelect.value)]?.conversion_id ?? null).then(data => {
        if (generation !== this.generation || current !== version) return;
        document = data;
        selection = new MarkdownSelection(
          document,
          article,
          patch,
          handwrite,
          (range) => void openPdfForRange(range),
          resolveFocusedBlockRange(document),
        );
        controls.after(selection.panel);
        selection.setOnSelectionChange(() => inkLinks?.syncActionPill());
        inkLinks?.setDocument(document);
        raw.disabled = false;
        return render();
      }).catch(error => {
        if (generation === this.generation && current === version) article.textContent = String(error);
      });
    };
    await render();
  }
}
