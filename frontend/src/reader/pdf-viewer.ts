import { getDocument, GlobalWorkerOptions, type PDFDocumentProxy, type PDFPageProxy } from "pdfjs-dist/legacy/build/pdf.mjs";
import workerUrl from "pdfjs-dist/legacy/build/pdf.worker.min.mjs?url";
import {
  PDF_ZOOM_DEFAULT,
  PDF_ZOOM_MAX,
  PDF_ZOOM_MIN,
  PDF_ZOOM_STEPS,
  anchoredScroll,
  canvasRatio,
  clampZoom,
  loadZoom,
  resolveZoom,
  saveZoom,
  stepZoom,
  zoomLabel,
  type PdfZoomMode,
} from "./pdf-zoom";
import { ArtifactPdfTransport } from "./pdf-transport";
import { readRange, type ContentInfo } from "./content";
import { extractAnchorPhrase, mountPdfTextLayer, PdfFindController, type MountedPdfTextLayer } from "./pdf-text-find";

GlobalWorkerOptions.workerSrc = workerUrl;

function element<K extends keyof HTMLElementTagNameMap>(tag: K, text = ""): HTMLElementTagNameMap[K] {
  const result = document.createElement(tag);
  result.textContent = text;
  return result;
}

export interface PdfReturnTargetLike {
  id: string;
  title: string;
  label?: string;
  options?: unknown;
}

export interface PdfViewerMountOptions {
  /** Zero-based PDF page index to land on initially. */
  pdfPageIndex?: number;
  /** Zero-based candidate pages when backlink spans multiple pages. */
  candidatePages?: number[];
  /** Optional note displayed above the viewport (e.g. candidate page explanation). */
  backlinkNote?: string;
  /** Optional excerpt used to highlight the landing line in the PDF TextLayer. */
  focusExcerpt?: string;
  /** Optional round-trip back button (standalone PDF reader mode). */
  returnTo?: PdfReturnTargetLike;
  onReturnTo?: (target: PdfReturnTargetLike) => void;
  /**
   * Compact side-by-side reference mode (used by the errata PDF reference drawer):
   * starts with outline collapsed, adds a `✕ 关闭对照` button, and scopes keyboard
   * shortcuts to focus inside the pane so it never interferes with text editing.
   */
  referenceMode?: boolean;
  referenceTitle?: string;
  onCloseReference?: () => void;
  /** Notifies the host when an async range read fails fatally. */
  onError?: (error: Error) => void;
}

export interface MountedPdfViewer {
  readonly artifactId: string;
  readonly numPages: number;
  currentPage(): number;
  jumpToPage(
    pageIndex: number,
    opts?: { candidatePages?: number[]; focusExcerpt?: string; backlinkNote?: string; pulse?: boolean },
  ): Promise<void>;
  dispose(): void;
}

interface PageSlotState {
  pageNumber: number;
  baseSize: { width: number; height: number };
  slotEl: HTMLDivElement;
  shellEl: HTMLDivElement;
  mountEpoch: number;
  pageProxy?: PDFPageProxy;
  renderTask?: ReturnType<PDFPageProxy["render"]>;
  textLayer?: MountedPdfTextLayer | null;
  textLayerPending?: Promise<MountedPdfTextLayer | null>;
  renderedScale?: number;
}

/**
 * Mounts Mathscape's virtualized continuous-scroll PDF viewer (with PDF.js 6.3
 * transparent TextLayer, cross-page search, zoom, outline, and backlink pulse)
 * into any host container. Shared by standalone PDF reading and the side-by-side
 * errata PDF reference drawer.
 */
export async function mountPdfViewer(
  host: HTMLElement,
  info: ContentInfo,
  isStale: () => boolean,
  options: PdfViewerMountOptions = {},
): Promise<MountedPdfViewer | null> {
  const requestedPageIndex = options.pdfPageIndex;
  const initial = await readRange(info.artifact_id, 0, Math.min(65536, info.byte_size));
  if (isStale()) return null;

  const failure = element("p");
  failure.className = "pdf-error-line";
  let firstFailure: Error | undefined;
  let rejectRead!: (error: Error) => void;
  const readFailure = new Promise<never>((_, reject) => { rejectRead = reject; });
  void readFailure.catch(() => {});

  let destroyed = false;
  let disposeRef: () => void = () => {};

  const transport = new ArtifactPdfTransport(
    info.byte_size,
    initial,
    (begin, length) => readRange(info.artifact_id, begin, length),
    (error) => {
      if (isStale() || firstFailure) return;
      firstFailure = error;
      failure.textContent = error.message;
      options.onError?.(error);
      console.error("PDF range read failed", { artifactId: info.artifact_id, error });
      rejectRead(error);
      disposeRef();
    },
  );

  const loading = getDocument({
    range: transport,
    disableAutoFetch: true,
    disableStream: true,
    cMapUrl: "/pdf-assets/cmaps/",
    cMapPacked: true,
    standardFontDataUrl: "/pdf-assets/standard_fonts/",
    wasmUrl: "/pdf-assets/wasm/",
  });

  const baseDispose = () => {
    transport.abort();
    if (destroyed) return;
    destroyed = true;
    void loading.destroy().catch((error) => console.warn("PDF cleanup failed", error));
  };
  disposeRef = baseDispose;

  host.append(failure);
  let pdf: PDFDocumentProxy;
  try {
    pdf = await Promise.race([loading.promise, readFailure]);
  } catch (error) {
    baseDispose();
    failure.remove();
    throw firstFailure ?? error;
  }
  if (isStale() || transport.stopped) {
    baseDispose();
    return null;
  }

  const controls = element("div");
  controls.className = "reader-controls";

  if (options.referenceMode) {
    const refHead = element("div");
    refHead.className = "errata-pdf-ref-header";
    const refTitle = element("strong", options.referenceTitle ?? "原书 PDF 对照");
    refTitle.className = "errata-pdf-ref-title";
    const closeBtn = element("button", "✕ 关闭对照");
    closeBtn.type = "button";
    closeBtn.className = "errata-pdf-ref-close";
    closeBtn.title = "关闭 PDF 对照面板（Esc）";
    closeBtn.onclick = () => options.onCloseReference?.();
    refHead.append(refTitle, closeBtn);
    host.append(refHead);
  }

  const previous = element("button", "上一页");
  previous.type = "button";
  const next = element("button", "下一页");
  next.type = "button";
  const pageInput = element("input");
  pageInput.type = "number";
  pageInput.min = "1";
  pageInput.max = String(pdf.numPages);
  pageInput.value = "1";
  pageInput.setAttribute("aria-label", "PDF 页码");

  const outlineToggle = element("button", "目录");
  outlineToggle.type = "button";
  outlineToggle.title = "显示或隐藏目录";

  const viewModeKey = "mathscape:pdf-view-mode";
  let viewMode: "continuous" | "single" = "continuous";
  if (!options.referenceMode) {
    try {
      if (localStorage.getItem(viewModeKey) === "single") viewMode = "single";
    } catch { /* Optional preference. */ }
  }
  const viewModeToggle = element("button", viewMode === "continuous" ? "连续滚动" : "单页视图");
  viewModeToggle.type = "button";
  viewModeToggle.title = "切换连续纵向滚动或单页翻页模式";
  viewModeToggle.setAttribute("aria-pressed", String(viewMode === "continuous"));

  const zoomOut = element("button", "−");
  zoomOut.type = "button";
  zoomOut.setAttribute("aria-label", "缩小");
  zoomOut.title = "缩小（Ctrl/⌘ −）";
  const zoomIn = element("button", "＋");
  zoomIn.type = "button";
  zoomIn.setAttribute("aria-label", "放大");
  zoomIn.title = "放大（Ctrl/⌘ +）";
  const zoom = element("select");
  zoom.setAttribute("aria-label", "缩放");
  zoom.title = "Ctrl/⌘ 0 恢复适合宽度；按住 Ctrl/⌘ 滚动或双指捏合可缩放";

  let show!: (number: number, opts?: { scrollToTop?: boolean }) => Promise<void>;
  let refreshAllHighlights!: () => HTMLElement | null;
  const findController = new PdfFindController(
    pdf,
    (match) => {
      if (!match) {
        refreshAllHighlights();
        return;
      }
      void show(match.pageNumber, { scrollToTop: viewMode === "single" }).then(() => {
        const hitEl = refreshAllHighlights();
        hitEl?.scrollIntoView({ behavior: "smooth", block: "center" });
      });
    },
    () => { refreshAllHighlights(); },
  );

  controls.append(
    previous,
    pageInput,
    element("span", "/ " + pdf.numPages + " 页"),
    next,
    outlineToggle,
    viewModeToggle,
    findController.toggleButton,
    zoomOut,
    zoom,
    zoomIn,
  );
  if (options.returnTo && options.onReturnTo) {
    const target = options.returnTo;
    const back = element("button", `← ${target.label ?? "返回 Markdown 原文"}`);
    back.type = "button";
    back.className = "reader-return-button";
    back.onclick = () => options.onReturnTo?.(target);
    controls.prepend(back);
  }

  let candidateGroup: HTMLElement | null = null;
  let candidateChipButtons: Array<{ pageNumber: number; button: HTMLButtonElement }> = [];
  const renderCandidateChips = (pages?: number[]): void => {
    candidateGroup?.remove();
    candidateGroup = null;
    candidateChipButtons = [];
    if (!pages || pages.length <= 1) return;
    const group = element("span");
    group.className = "reader-candidate-pages";
    group.append(element("span", "关联页："));
    for (const idx of pages) {
      const targetNum = idx + 1;
      if (targetNum < 1 || targetNum > pdf.numPages) continue;
      const chip = element("button", `第 ${targetNum} 页`);
      chip.type = "button";
      chip.className = "reader-page-chip";
      chip.onclick = () => void show(targetNum);
      candidateChipButtons.push({ pageNumber: targetNum, button: chip });
      group.append(chip);
    }
    if (candidateChipButtons.length > 1) {
      candidateGroup = group;
      controls.append(group);
    }
  };
  renderCandidateChips(options.candidatePages);

  const noteEl = element("p");
  noteEl.className = "reader-backlink-note";
  const setBacklinkNote = (text?: string): void => {
    noteEl.textContent = text ?? "";
    noteEl.hidden = !text;
  };
  setBacklinkNote(options.backlinkNote);

  const layout = element("div");
  layout.className = "pdf-layout";
  const outline = element("nav");
  outline.className = "pdf-outline";
  outline.setAttribute("aria-label", "PDF 目录");
  const viewport = element("div");
  viewport.className = "pdf-viewport";
  viewport.dataset.viewMode = viewMode;
  layout.append(outline, viewport);
  host.append(controls, findController.bar, noteEl, layout);

  const outlineKey = "mathscape:pdf-outline";
  const setOutline = (visibleOutline: boolean, persist = true): void => {
    outline.hidden = !visibleOutline;
    outlineToggle.setAttribute("aria-pressed", String(visibleOutline));
    if (persist && !options.referenceMode) {
      try { localStorage.setItem(outlineKey, visibleOutline ? "1" : "0"); } catch { /* Optional preference. */ }
    }
  };
  let outlineVisible = !options.referenceMode;
  if (!options.referenceMode) {
    try { outlineVisible = localStorage.getItem(outlineKey) !== "0"; } catch { /* Optional preference. */ }
  }
  setOutline(outlineVisible, false);
  outlineToggle.onclick = () => setOutline(outline.hidden === true, !options.referenceMode);

  let pageNumber = Number.isInteger(requestedPageIndex)
    ? Math.min(Math.max((requestedPageIndex as number) + 1, 1), pdf.numPages)
    : 1;
  let backlinkLandingPage: number | null = requestedPageIndex !== undefined ? pageNumber : null;
  let backlinkPhrase: string | null = requestedPageIndex !== undefined ? extractAnchorPhrase(options.focusExcerpt) : null;
  let backlinkTextScrolled = false;
  let backlinkPulseTimer: ReturnType<typeof setTimeout> | undefined;

  let renderVersion = 0;
  let mode: PdfZoomMode = options.referenceMode ? { kind: "fit-width" } : loadZoom(info.artifact_id);
  let scale = 1;
  let renderTimer: ReturnType<typeof setTimeout> | undefined;
  let scrollSyncRaf = 0;
  let programmaticScroll: { page: number; top: number } | null = null;

  const available = () => ({
    width: Math.max(1, viewport.clientWidth - 2),
    height: Math.max(200, viewport.clientHeight || window.innerHeight - viewport.getBoundingClientRect().top - 24),
  });

  const firstLoadedPage = await pdf.getPage(pageNumber);
  if (isStale() || transport.stopped) {
    baseDispose();
    return null;
  }
  const firstBaseView = firstLoadedPage.getViewport({ scale: 1 });
  let pageSize = { width: firstBaseView.width, height: firstBaseView.height };

  const slots: PageSlotState[] = [];
  for (let p = 1; p <= pdf.numPages; p++) {
    const slotEl = element("div");
    slotEl.className = "pdf-page-slot";
    slotEl.dataset.pageNumber = String(p);
    const shellEl = element("div");
    shellEl.className = "pdf-page-shell";
    slotEl.append(shellEl);
    viewport.append(slotEl);
    slots.push({
      pageNumber: p,
      baseSize: { ...pageSize },
      slotEl,
      shellEl,
      mountEpoch: 0,
      pageProxy: p === pageNumber ? firstLoadedPage : undefined,
    });
  }

  const unmountSlot = (slot: PageSlotState): void => {
    slot.mountEpoch += 1;
    slot.renderTask?.cancel();
    slot.renderTask = undefined;
    slot.textLayerPending = undefined;
    slot.textLayer?.dispose();
    slot.textLayer = null;
    slot.renderedScale = undefined;
    slot.shellEl.replaceChildren();
  };

  const refreshSlotHighlights = (slot: PageSlotState): HTMLElement | null => {
    if (!slot.textLayer) return null;
    const focusEl = slot.textLayer.applyHighlights({
      searchQuery: findController.currentQuery(),
      activeMatchIndexOnPage: findController.activeMatchForPage(slot.pageNumber),
      backlinkPhrase: backlinkLandingPage === slot.pageNumber ? backlinkPhrase : null,
    });
    if (
      backlinkLandingPage === slot.pageNumber &&
      backlinkPhrase &&
      focusEl &&
      !backlinkTextScrolled &&
      !findController.isOpen()
    ) {
      backlinkTextScrolled = true;
      focusEl.scrollIntoView({ behavior: "smooth", block: "center" });
    }
    return focusEl;
  };

  refreshAllHighlights = (): HTMLElement | null => {
    let activeFocusEl: HTMLElement | null = null;
    for (const slot of slots) {
      const hit = refreshSlotHighlights(slot);
      if (hit && findController.activeMatchForPage(slot.pageNumber) !== null) {
        activeFocusEl = hit;
      }
    }
    return activeFocusEl;
  };

  const updateActivePageUI = (num: number): void => {
    pageNumber = num;
    pageInput.value = String(num);
    previous.disabled = num === 1;
    next.disabled = num === pdf.numPages;
    for (const item of candidateChipButtons) {
      item.button.setAttribute("aria-pressed", String(item.pageNumber === num));
    }
    for (const slot of slots) {
      slot.slotEl.classList.toggle("is-active-page", slot.pageNumber === num);
    }
    pageSize = slots[num - 1]?.baseSize ?? pageSize;
  };

  const syncSlotDimensions = (): void => {
    viewport.dataset.viewMode = viewMode;
    for (const slot of slots) {
      const visibleInMode = viewMode === "continuous" || slot.pageNumber === pageNumber;
      slot.slotEl.hidden = !visibleInMode;
      if (!visibleInMode) {
        unmountSlot(slot);
        continue;
      }
      const w = slot.baseSize.width * scale;
      const h = slot.baseSize.height * scale;
      slot.shellEl.style.width = w + "px";
      slot.shellEl.style.height = h + "px";
      slot.shellEl.style.setProperty("--total-scale-factor", String(scale));
      const canvas = slot.shellEl.querySelector<HTMLCanvasElement>("canvas");
      if (canvas) {
        canvas.style.width = w + "px";
        canvas.style.height = h + "px";
      }
      if (slot.textLayer && slot.pageProxy) {
        slot.textLayer.updateViewport(slot.pageProxy.getViewport({ scale }));
      }
    }
  };

  const presetValues = PDF_ZOOM_STEPS.filter((step) => step >= 0.5 && step <= 3);
  const renderZoomControl = (): void => {
    const value = mode.kind === "scale" ? "s:" + mode.value : mode.kind;
    const optionsList: Array<[string, string]> = [
      ["fit-width", zoomLabel({ kind: "fit-width" }, mode.kind === "fit-width" ? scale : resolveZoom({ kind: "fit-width" }, pageSize, available()))],
      ["fit-page", zoomLabel({ kind: "fit-page" }, mode.kind === "fit-page" ? scale : resolveZoom({ kind: "fit-page" }, pageSize, available()))],
      ...presetValues.map((step) => ["s:" + step, Math.round(step * 100) + "%"] as [string, string]),
    ];
    if (mode.kind === "scale" && !presetValues.includes(mode.value)) {
      optionsList.push([value, Math.round(mode.value * 100) + "%"]);
    }
    zoom.replaceChildren(...optionsList.map(([key, label]) => new Option(label, key)));
    zoom.value = value;
    zoomOut.disabled = scale <= PDF_ZOOM_MIN + 1e-3;
    zoomIn.disabled = scale >= PDF_ZOOM_MAX - 1e-3;
    viewport.dataset.scale = String(scale);
    viewport.dataset.zoomMode = mode.kind;
  };

  const ensureSlotTextLayer = async (
    slot: PageSlotState,
    view: ReturnType<PDFPageProxy["getViewport"]>,
    version: number,
  ): Promise<void> => {
    if (slot.textLayer) {
      slot.textLayer.updateViewport(view);
      return;
    }
    if (!slot.pageProxy) return;
    if (!slot.textLayerPending) {
      const epoch = slot.mountEpoch;
      const p = mountPdfTextLayer(
        slot.pageProxy,
        view,
        slot.shellEl,
        () =>
          transport.stopped ||
          isStale() ||
          slot.mountEpoch !== epoch ||
          Boolean(slot.slotEl.hidden),
      ).finally(() => {
        if (slot.textLayerPending === p) slot.textLayerPending = undefined;
      });
      slot.textLayerPending = p;
    }
    const mounted = await slot.textLayerPending;
    if (!mounted) return;
    if (transport.stopped || isStale() || Boolean(slot.slotEl.hidden)) {
      mounted.dispose();
      return;
    }
    if (!slot.textLayer) {
      slot.textLayer = mounted;
    } else if (slot.textLayer !== mounted) {
      mounted.dispose();
    }
    if (version === renderVersion) {
      slot.textLayer.updateViewport(view);
    }
  };

  const renderSlotAtScale = async (slot: PageSlotState, version: number, targetScale: number): Promise<void> => {
    if (transport.stopped || version !== renderVersion || isStale()) return;
    if (slot.slotEl.hidden) return;
    if (
      slot.renderedScale !== undefined &&
      Math.abs(slot.renderedScale - targetScale) < 1e-4 &&
      slot.shellEl.querySelector("canvas") !== null
    ) {
      if (slot.pageProxy) {
        await ensureSlotTextLayer(slot, slot.pageProxy.getViewport({ scale: targetScale }), version);
      }
      refreshSlotHighlights(slot);
      return;
    }

    slot.renderTask?.cancel();
    try {
      if (!slot.pageProxy) {
        slot.pageProxy = await pdf.getPage(slot.pageNumber);
      }
      if (transport.stopped || version !== renderVersion || isStale() || slot.slotEl.hidden) return;
      const rawBase = slot.pageProxy.getViewport({ scale: 1 });
      if (Math.abs(rawBase.width - slot.baseSize.width) > 0.5 || Math.abs(rawBase.height - slot.baseSize.height) > 0.5) {
        slot.baseSize = { width: rawBase.width, height: rawBase.height };
        slot.shellEl.style.width = (rawBase.width * targetScale) + "px";
        slot.shellEl.style.height = (rawBase.height * targetScale) + "px";
      }

      const view = slot.pageProxy.getViewport({ scale: targetScale });
      const canvas = element("canvas");
      const ratio = canvasRatio(window.devicePixelRatio || 1, view.width, view.height);
      canvas.width = Math.ceil(view.width * ratio);
      canvas.height = Math.ceil(view.height * ratio);
      canvas.style.width = view.width + "px";
      canvas.style.height = view.height + "px";
      canvas.dataset.ratio = String(ratio);

      const existingCanvas = slot.shellEl.querySelector("canvas");
      if (!existingCanvas) {
        slot.shellEl.prepend(canvas);
      }
      const task = slot.pageProxy.render({ canvas, viewport: view, transform: [ratio, 0, 0, ratio, 0, 0] });
      slot.renderTask = task;
      await task.promise;
      if (transport.stopped || version !== renderVersion || isStale() || slot.slotEl.hidden) return;
      if (existingCanvas && existingCanvas !== canvas) {
        existingCanvas.replaceWith(canvas);
      }
      slot.renderedScale = targetScale;

      await ensureSlotTextLayer(slot, view, version);
      refreshSlotHighlights(slot);
    } catch (error) {
      if (
        !firstFailure &&
        !transport.stopped &&
        version === renderVersion &&
        !isStale() &&
        !(error instanceof Error && error.name === "RenderingCancelledException")
      ) {
        failure.textContent = String(error);
      }
    }
  };

  const computeVisibleRange = (): { first: number; last: number } => {
    if (viewMode === "single") return { first: pageNumber, last: pageNumber };
    const vTop = viewport.scrollTop;
    const vHeight = Math.max(240, viewport.clientHeight || 600);
    const vBottom = vTop + vHeight;
    let first = 0;
    let last = 0;
    for (const slot of slots) {
      if (slot.slotEl.hidden) continue;
      const top = slot.slotEl.offsetTop;
      const bottom = top + slot.slotEl.offsetHeight;
      if (bottom >= vTop && top <= vBottom) {
        if (first === 0) first = slot.pageNumber;
        last = slot.pageNumber;
      }
    }
    if (first === 0) return { first: pageNumber, last: pageNumber };
    return { first, last };
  };

  const syncVirtualWindow = async (version: number): Promise<void> => {
    if (transport.stopped || version !== renderVersion || isStale()) return;
    const targetScale = scale;
    if (viewMode === "single") {
      await renderSlotAtScale(slots[pageNumber - 1], version, targetScale);
      return;
    }
    const { first, last } = computeVisibleRange();
    const mountStart = Math.max(1, Math.min(first - 1, pageNumber));
    const mountEnd = Math.min(pdf.numPages, Math.max(last + 1, pageNumber));
    const keepStart = Math.max(1, first - 2);
    const keepEnd = Math.min(pdf.numPages, last + 2);

    for (const slot of slots) {
      if (slot.pageNumber !== pageNumber && (slot.pageNumber < keepStart || slot.pageNumber > keepEnd)) {
        unmountSlot(slot);
      }
    }

    const order: PageSlotState[] = [slots[pageNumber - 1]];
    for (let p = mountStart; p <= mountEnd; p++) {
      if (p !== pageNumber) order.push(slots[p - 1]);
    }
    await Promise.all(order.map((slot) => renderSlotAtScale(slot, version, targetScale)));
  };

  const applyZoom = (nextMode: PdfZoomMode, anchor?: { x: number; y: number }, persist = true): void => {
    mode = nextMode;
    if (persist && !options.referenceMode) saveZoom(info.artifact_id, mode);
    const nextScale = resolveZoom(mode, pageSize, available());
    if (Math.abs(nextScale - scale) < 1e-4) {
      renderZoomControl();
      return;
    }
    const x = anchor?.x ?? viewport.clientWidth / 2;
    const y = anchor?.y ?? viewport.clientHeight / 2;
    const targetScrollY = viewport.scrollTop + y;
    let anchorSlot = slots[pageNumber - 1];
    if (viewMode === "continuous") {
      for (const slot of slots) {
        const top = slot.slotEl.offsetTop;
        const bottom = top + slot.slotEl.offsetHeight;
        if (targetScrollY >= top && targetScrollY <= bottom) {
          anchorSlot = slot;
          break;
        }
      }
    }
    const slotTopBefore = anchorSlot.slotEl.offsetTop;
    const slotHeightBefore = Math.max(1, anchorSlot.baseSize.height * scale);
    const fracY = ((viewport.scrollTop + y) - slotTopBefore) / slotHeightBefore;
    const beforeWidth = Math.max(1, anchorSlot.baseSize.width * scale);
    const afterWidth = Math.max(1, anchorSlot.baseSize.width * nextScale);

    scale = nextScale;
    syncSlotDimensions();

    const slotTopAfter = anchorSlot.slotEl.offsetTop;
    const slotHeightAfter = Math.max(1, anchorSlot.baseSize.height * nextScale);
    const nextLeft = anchoredScroll(viewport.scrollLeft, x, beforeWidth, afterWidth);
    const nextTop = viewMode === "continuous"
      ? Math.max(0, slotTopAfter + fracY * slotHeightAfter - y)
      : anchoredScroll(viewport.scrollTop, y, slotHeightBefore, slotHeightAfter);
    programmaticScroll = { page: pageNumber, top: nextTop };
    viewport.scrollLeft = nextLeft;
    viewport.scrollTop = nextTop;

    renderZoomControl();
    const version = ++renderVersion;
    if (renderTimer) clearTimeout(renderTimer);
    renderTimer = setTimeout(() => {
      renderTimer = undefined;
      void syncVirtualWindow(version);
    }, 120);
  };

  show = async (number: number, opts?: { scrollToTop?: boolean }): Promise<void> => {
    if (transport.stopped) return;
    if (!Number.isInteger(number) || number < 1 || number > pdf.numPages) {
      pageInput.value = String(pageNumber);
      return;
    }
    if (renderTimer) {
      clearTimeout(renderTimer);
      renderTimer = undefined;
    }
    updateActivePageUI(number);
    scale = resolveZoom(mode, pageSize, available());
    syncSlotDimensions();
    renderZoomControl();

    if (viewMode === "continuous") {
      const targetTop = slots[number - 1].slotEl.offsetTop;
      viewport.scrollTop = targetTop;
      programmaticScroll = { page: number, top: viewport.scrollTop };
    } else if (opts?.scrollToTop !== false) {
      viewport.scrollTop = 0;
      programmaticScroll = { page: number, top: 0 };
    }
    await syncVirtualWindow(++renderVersion);
  };

  const pulseLandingPage = (targetNum: number): void => {
    if (backlinkPulseTimer) clearTimeout(backlinkPulseTimer);
    for (const slot of slots) slot.shellEl.classList.remove("is-backlink-target");
    const landingShell = slots[targetNum - 1]?.shellEl;
    if (landingShell) {
      landingShell.classList.add("is-backlink-target");
      backlinkPulseTimer = setTimeout(() => landingShell.classList.remove("is-backlink-target"), 2600);
    }
  };

  viewModeToggle.onclick = () => {
    viewMode = viewMode === "continuous" ? "single" : "continuous";
    viewModeToggle.textContent = viewMode === "continuous" ? "连续滚动" : "单页视图";
    viewModeToggle.setAttribute("aria-pressed", String(viewMode === "continuous"));
    if (!options.referenceMode) {
      try { localStorage.setItem(viewModeKey, viewMode); } catch { /* Optional preference. */ }
    }
    void show(pageNumber);
  };

  const onScroll = (): void => {
    if (viewMode !== "continuous" || transport.stopped) return;
    if (scrollSyncRaf) return;
    scrollSyncRaf = requestAnimationFrame(() => {
      scrollSyncRaf = 0;
      if (transport.stopped || isStale() || viewMode !== "continuous") return;
      if (programmaticScroll && Math.abs(viewport.scrollTop - programmaticScroll.top) <= 3) {
        void syncVirtualWindow(renderVersion);
        return;
      }
      programmaticScroll = null;
      const probeY = viewport.scrollTop + Math.max(80, viewport.clientHeight * 0.36);
      let bestPage = pageNumber;
      for (const slot of slots) {
        const top = slot.slotEl.offsetTop;
        const bottom = top + slot.slotEl.offsetHeight;
        if (probeY >= top && probeY <= bottom) {
          bestPage = slot.pageNumber;
          break;
        }
        if (top <= probeY) bestPage = slot.pageNumber;
      }
      if (bestPage !== pageNumber) {
        updateActivePageUI(bestPage);
      }
      void syncVirtualWindow(renderVersion);
    });
  };
  viewport.addEventListener("scroll", onScroll, { passive: true });

  const cleanups: Array<() => void> = [
    () => {
      viewport.removeEventListener("scroll", onScroll);
      if (scrollSyncRaf) cancelAnimationFrame(scrollSyncRaf);
      if (backlinkPulseTimer) clearTimeout(backlinkPulseTimer);
      findController.dispose();
      slots.forEach(unmountSlot);
    },
  ];
  const fullDispose = (): void => {
    renderVersion++;
    if (renderTimer) clearTimeout(renderTimer);
    cleanups.forEach((cleanup) => cleanup());
    baseDispose();
  };
  disposeRef = fullDispose;

  previous.onclick = () => void show(pageNumber - 1);
  next.onclick = () => void show(pageNumber + 1);
  pageInput.onchange = () => void show(pageInput.valueAsNumber);
  zoom.onchange = () => {
    const value = zoom.value;
    applyZoom(
      value === "fit-width" || value === "fit-page"
        ? { kind: value }
        : { kind: "scale", value: clampZoom(Number(value.slice(2))) },
    );
  };
  const stepBy = (direction: 1 | -1, anchor?: { x: number; y: number }) =>
    applyZoom({ kind: "scale", value: stepZoom(scale, direction) }, anchor);
  zoomIn.onclick = () => stepBy(1);
  zoomOut.onclick = () => stepBy(-1);

  const visible = () => viewport.isConnected && viewport.getClientRects().length > 0;
  const isEditableFocus = (el: Element | null) =>
    Boolean(el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT" || (el as HTMLElement).isContentEditable));
  const onKey = (event: KeyboardEvent): void => {
    if (event.defaultPrevented || !visible()) return;
    if (options.referenceMode) {
      // Only handle keyboard shortcuts when focus is inside the reference pane,
      // except Escape which dismisses the reference pane when not inside an editor textarea.
      const active = document.activeElement;
      if (event.key === "Escape" && !isEditableFocus(active)) {
        event.preventDefault();
        options.onCloseReference?.();
        return;
      }
      if (!active || !host.contains(active)) return;
    }
    if ((event.ctrlKey || event.metaKey) && !event.altKey) {
      if (event.key === "+" || event.key === "=") { stepBy(1); event.preventDefault(); return; }
      if (event.key === "-" || event.key === "_") { stepBy(-1); event.preventDefault(); return; }
      if (event.key === "0") { applyZoom(PDF_ZOOM_DEFAULT); event.preventDefault(); return; }
      if (event.key.toLowerCase() === "f") { findController.open(); event.preventDefault(); return; }
      return;
    }
    if (event.ctrlKey || event.metaKey || event.altKey || isEditableFocus(document.activeElement)) return;
    if (event.key === "PageDown" || event.key === "ArrowRight") {
      if (pageNumber < pdf.numPages) {
        event.preventDefault();
        void show(pageNumber + 1);
      }
    } else if (event.key === "PageUp" || event.key === "ArrowLeft") {
      if (pageNumber > 1) {
        event.preventDefault();
        void show(pageNumber - 1);
      }
    }
  };
  window.addEventListener("keydown", onKey);
  cleanups.push(() => window.removeEventListener("keydown", onKey));

  const pointAt = (clientX: number, clientY: number) => {
    const rect = viewport.getBoundingClientRect();
    return { x: clientX - rect.left, y: clientY - rect.top };
  };
  viewport.addEventListener("wheel", (event) => {
    if (!(event.ctrlKey || event.metaKey)) return;
    event.preventDefault();
    const delta = event.deltaY * (event.deltaMode === 1 ? 33 : event.deltaMode === 2 ? 400 : 1);
    applyZoom({ kind: "scale", value: clampZoom(scale * Math.exp(-delta * 0.0015)) }, pointAt(event.clientX, event.clientY));
  }, { passive: false });

  let gestureStart = 1;
  type Gesture = Event & { scale: number; clientX: number; clientY: number };
  viewport.addEventListener("gesturestart", (event) => { event.preventDefault(); gestureStart = scale; });
  viewport.addEventListener("gesturechange", (event) => {
    event.preventDefault();
    const gesture = event as Gesture;
    applyZoom({ kind: "scale", value: clampZoom(gestureStart * gesture.scale) }, pointAt(gesture.clientX, gesture.clientY));
  });
  viewport.addEventListener("gestureend", (event) => event.preventDefault());

  if (typeof ResizeObserver !== "undefined") {
    let lastWidth = viewport.clientWidth;
    let lastHeight = viewport.clientHeight;
    const observer = new ResizeObserver(() => {
      if (viewport.clientWidth === lastWidth && viewport.clientHeight === lastHeight) return;
      lastWidth = viewport.clientWidth;
      lastHeight = viewport.clientHeight;
      if (mode.kind !== "scale") applyZoom(mode, { x: 0, y: 0 }, false);
    });
    observer.observe(viewport);
    cleanups.push(() => observer.disconnect());
  }

  if (backlinkLandingPage !== null) {
    pulseLandingPage(backlinkLandingPage);
  }

  await show(pageNumber);
  if (transport.stopped || isStale()) {
    fullDispose();
    return null;
  }

  const items = await pdf.getOutline().catch((error) => {
    if (firstFailure || transport.stopped) return null;
    throw error;
  });
  if (isStale() || transport.stopped) {
    fullDispose();
    return null;
  }
  if (!items?.length) outline.append(element("p", "此 PDF 没有内置书签目录，可输入页码跳转。"));
  const add = (entries: NonNullable<typeof items>, parent: HTMLElement): void => {
    for (const item of entries) {
      const button = element("button", item.title);
      button.type = "button";
      button.disabled = !item.dest;
      button.onclick = () => void (async () => {
        const destination = typeof item.dest === "string" ? await pdf.getDestination(item.dest) : item.dest;
        if (!destination) return;
        const index = typeof destination[0] === "number" ? destination[0] : await pdf.getPageIndex(destination[0]);
        if (!isStale()) await show(index + 1);
      })().catch((error) => {
        if (!firstFailure && !transport.stopped && !isStale()) failure.textContent = String(error);
      });
      parent.append(button);
      if (item.items.length) {
        const child = element("div");
        parent.append(child);
        add(item.items, child);
      }
    }
  };
  if (items) add(items, outline);

  return {
    artifactId: info.artifact_id,
    numPages: pdf.numPages,
    currentPage: () => pageNumber,
    jumpToPage: async (targetPageIndex, jumpOpts = {}) => {
      const targetNum = Math.min(Math.max(targetPageIndex + 1, 1), pdf.numPages);
      backlinkLandingPage = targetNum;
      backlinkPhrase = extractAnchorPhrase(jumpOpts.focusExcerpt);
      backlinkTextScrolled = false;
      renderCandidateChips(jumpOpts.candidatePages);
      setBacklinkNote(jumpOpts.backlinkNote);
      if (jumpOpts.pulse !== false) pulseLandingPage(targetNum);
      await show(targetNum);
      refreshAllHighlights();
    },
    dispose: fullDispose,
  };
}
