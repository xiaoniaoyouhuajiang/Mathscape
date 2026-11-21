import { contentInfo, getMarkdownUnitMarkers, readSmallFile, resolveProductPath, type ContentInfo } from "./content";
import { getSourceBacklink } from "./backlink";
import { markdownRenderer } from "./markdown";
import { canonicalReaderProjection, gapLabel, pagesLabel, projectionSummary, segmentLabel, splitByPageMarkers, type CanonicalReaderSegment } from "./canonical";
import { isCanonicalMarkdownId } from "./openable";
import { focusMatchesBlock, pageIndexOfBlock, planCanonicalLanding, snapshotPageSpan, type CanonicalLanding } from "./canonical-landing";
import { blockLabel, isInViewport, mountReturnPoint, pinToViewportTop, scrollParentOf, viewportTopOf } from "./reading-position";
import { markdownDocument, MarkdownSelection, type AnchoredMarkdown, type MarkdownBlock, type PatchReviewMode } from "./selection";
import { ReaderInkLinks } from "./ink-links";
import { CanonicalInkController, type CanonicalBlockItem } from "./canonical-ink";
import { mountMarkdownOutline, type OutlineBlockDescriptor } from "./markdown-outline";
import {
  buildBlockErrataLookup,
  closeErrataDiffPopover,
  computeErrataCoverRange,
  InlineErrataEditorController,
  listMarkdownErrata,
  openErrataDiffPopover,
  revertMarkdownErrata,
  type MarkdownBlockErrata,
} from "./markdown-errata";
import { mountPdfViewer, type MountedPdfViewer } from "./pdf-viewer";
import { showContextMenu } from "../graph/context-menu";
import { eventBus } from "../core/events";
import { openInkSnapshot } from "../ink/persistence";
import { mountInkSnapshotReader, type InkReaderFocus } from "../ink/snapshot-reader";
import "./reader.css";

function element<K extends keyof HTMLElementTagNameMap>(tag: K, text = ""): HTMLElementTagNameMap[K] {
  const result = document.createElement(tag);
  result.textContent = text;
  return result;
}

export interface ReaderFocus {
  start: number;
  end: number;
  blockIds?: string[];
  /** Physical snapshot the coordinates belong to. The 「规范解析正文」 view only
   * matches blocks of that snapshot when it is set. */
  artifactId?: string;
  /** Zero-based PDF page of the position: where the 「规范解析正文」 view lands
   * when it shows that page from another snapshot. */
  pdfPageIndex?: number;
  /** Put the block this many px below the top of the reader instead of
   * centering it, so a switch of view does not move what is being read. */
  viewportTop?: number;
  /** Names the block on the 「回到出发处」 chip. */
  label?: string;
}

/** The 「规范解析正文」 view a physical snapshot is stitched into. */
export interface CanonicalReadingTarget { id: string; title: string }

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
  /** Optional plain/Markdown excerpt used to highlight the landing line inside PDF TextLayer. */
  focusExcerpt?: string;
  /** Optional round-trip target to jump right back to the Markdown block/view. */
  returnTo?: ReaderReturnTarget;
  /** Opens the Markdown patch review of a run instead of the latest pending one. */
  patch?: { runId: string; mode: PatchReviewMode };
  conversionId?: string | null;
  inkFocus?: InkReaderFocus;
  /** Reading, not discovery or review: a Markdown snapshot that belongs to a
   * 「规范解析正文」 opens there, at the same block. Ignored with `patch`. */
  preferCanonical?: boolean;
  /** Shown above a physical snapshot the reader fell back to. */
  notice?: string;
}

interface ErrataPdfReferenceTarget {
  pdfArtifactId: string;
  pdfPageIndex: number;
  candidatePages?: number[];
  backlinkNote?: string;
  focusExcerpt?: string;
  title?: string;
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
  private mainPane: HTMLElement | null = null;
  private pdfRefGeneration = 0;
  private pdfRefPane: HTMLElement | null = null;
  private pdfRefViewer: MountedPdfViewer | null = null;
  private pdfRefCloseListeners = new Set<() => void>();
  private canonicalResolver: ((artifactId: string) => CanonicalReadingTarget | undefined) | undefined;

  constructor(private host: HTMLElement) {}

  /** Physical snapshot id → the 「规范解析正文」 view it belongs to, if any. */
  setCanonicalResolver(resolver: ((artifactId: string) => CanonicalReadingTarget | undefined) | undefined): void {
    this.canonicalResolver = resolver;
  }

  setOverlayCloseHandler(handler: (() => void) | undefined): void {
    this.overlayCloseHandler = handler;
  }

  private scrollContainer(): HTMLElement {
    return this.host.classList.contains("has-errata-pdf-pane") && this.mainPane
      ? this.mainPane
      : this.host;
  }

  private isPdfReferenceOpen(): boolean {
    return this.pdfRefPane !== null;
  }

  private currentPdfReferencePageIndex(): number | null {
    return this.pdfRefViewer ? Math.max(0, this.pdfRefViewer.currentPage() - 1) : null;
  }

  private closeErrataPdfReference(): void {
    this.pdfRefGeneration++;
    this.pdfRefViewer?.dispose();
    this.pdfRefViewer = null;
    this.pdfRefPane?.remove();
    this.pdfRefPane = null;
    this.host.classList.remove("has-errata-pdf-pane");
    const listeners = [...this.pdfRefCloseListeners];
    this.pdfRefCloseListeners.clear();
    listeners.forEach((fn) => fn());
  }

  private async openErrataPdfReference(
    target: ErrataPdfReferenceTarget,
    parentGeneration: number,
    onClosed?: () => void,
  ): Promise<boolean> {
    if (parentGeneration !== this.generation) return false;
    if (onClosed) this.pdfRefCloseListeners.add(onClosed);

    // If the same PDF is already mounted in the reference pane, just jump to the requested page.
    if (this.pdfRefPane && this.pdfRefViewer && this.pdfRefViewer.artifactId === target.pdfArtifactId) {
      await this.pdfRefViewer.jumpToPage(target.pdfPageIndex, {
        candidatePages: target.candidatePages,
        focusExcerpt: target.focusExcerpt,
        backlinkNote: target.backlinkNote,
        pulse: true,
      });
      return true;
    }

    const refGen = ++this.pdfRefGeneration;
    this.pdfRefViewer?.dispose();
    this.pdfRefViewer = null;
    this.pdfRefPane?.remove();

    const pane = element("aside");
    pane.className = "errata-pdf-reference-pane";
    pane.setAttribute("aria-label", "原书 PDF 对照面板");
    const loadingLine = element("p", "正在加载原书 PDF 对照页…");
    loadingLine.className = "errata-pdf-ref-loading";
    pane.append(loadingLine);

    this.pdfRefPane = pane;
    this.host.classList.add("has-errata-pdf-pane");
    this.host.append(pane);

    try {
      const pdfInfo = await contentInfo(target.pdfArtifactId);
      if (parentGeneration !== this.generation || refGen !== this.pdfRefGeneration) return false;
      if (pdfInfo.media_type !== "application/pdf") {
        throw new Error(`关联来源不是 PDF（${pdfInfo.media_type}）`);
      }
      loadingLine.remove();
      const mounted = await mountPdfViewer(
        pane,
        pdfInfo,
        () => parentGeneration !== this.generation || refGen !== this.pdfRefGeneration,
        {
          pdfPageIndex: target.pdfPageIndex,
          candidatePages: target.candidatePages,
          backlinkNote: target.backlinkNote,
          focusExcerpt: target.focusExcerpt,
          referenceMode: true,
          referenceTitle: target.title ?? `对照 PDF 第 ${target.pdfPageIndex + 1} 页`,
          onCloseReference: () => this.closeErrataPdfReference(),
        },
      );
      if (!mounted || parentGeneration !== this.generation || refGen !== this.pdfRefGeneration) {
        mounted?.dispose();
        return false;
      }
      this.pdfRefViewer = mounted;
      return true;
    } catch (error) {
      if (parentGeneration === this.generation && refGen === this.pdfRefGeneration && this.pdfRefPane === pane) {
        pane.replaceChildren();
        const header = element("div");
        header.className = "errata-pdf-ref-header";
        const heading = element("strong", "原书 PDF 对照");
        heading.className = "errata-pdf-ref-title";
        const closeBtn = element("button", "✕ 关闭对照");
        closeBtn.type = "button";
        closeBtn.className = "errata-pdf-ref-close";
        closeBtn.onclick = () => this.closeErrataPdfReference();
        header.append(heading, closeBtn);
        const msg = element("p", `无法加载 PDF 原页：${error instanceof Error ? error.message : String(error)}`);
        msg.className = "errata-pdf-ref-error";
        pane.append(header, msg);
      }
      return false;
    }
  }

  private revokeImages(): void {
    this.urls.forEach(url => URL.revokeObjectURL(url));
    this.urls = [];
  }

  close(): void {
    this.generation++;
    this.closeErrataPdfReference();
    this.disposeDocument?.();
    this.disposeDocument = undefined;
    this.revokeImages();
    this.host.replaceChildren();
    this.mainPane = null;
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
    if (options.preferCanonical && !options.patch && !isCanonicalMarkdownId(id)) {
      const canonical = this.canonicalResolver?.(id);
      if (canonical) {
        const focus = options.focus ?? { start: 0, end: 0 };
        return this.openCanonicalAt(canonical, { ...focus, artifactId: focus.artifactId ?? id }, {
          id,
          title,
          options: { ...options, preferCanonical: false },
        });
      }
    }
    this.close();
    this.state = "loading";
    eventBus.emit("reader-opened");
    const { focus, patch, conversionId } = options;
    const generation = this.generation;
    const mainPane = element("div");
    mainPane.className = "reader-main-pane";
    this.mainPane = mainPane;
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
      mainPane.append(headerRow, status);
    } else {
      mainPane.append(heading, status);
    }
    this.host.append(mainPane);
    try {
      if (isCanonicalMarkdownId(id)) {
        // Logical canonical id: a stitched read-only view, never content_info.
        const landing = await this.canonical(id, title, generation, focus, mainPane);
        if (generation !== this.generation) return false;
        if (landing.kind === "snapshot" && focus?.artifactId) {
          // The position is not readable in the stitched view: keep the snapshot.
          const { artifactId, pdfPageIndex: _page, viewportTop: _top, ...snapshotFocus } = focus;
          const fallback = this.snapshotFallback;
          return this.open(artifactId, fallback?.id === artifactId ? fallback.title : title, {
            ...(fallback?.id === artifactId ? fallback.options : {}),
            focus: snapshotFocus,
            preferCanonical: false,
            notice: "这一处在规范解析正文中没有对应的可读位置，已保留在解析快照中。",
          });
        }
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
        this.disposeDocument = mountInkSnapshotReader(mainPane, snapshot, options.inkFocus);
        this.inkOpen = true;
      } else if (info.media_type === "application/pdf") {
        await this.pdf(info, generation, options, mainPane);
      } else if (info.media_type === "text/markdown") {
        await this.markdown(info, title, generation, mainPane, focus, patch, conversionId, options.notice);
      } else if (info.media_type === "application/json" || info.media_type.startsWith("text/")) {
        const bytes = await readSmallFile(info);
        if (generation !== this.generation) return false;
        const text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
        const pre = element("pre", info.media_type === "application/json" ? JSON.stringify(JSON.parse(text), null, 2) : text);
        pre.className = "reader-text";
        mainPane.append(pre);
      } else if (["image/png", "image/jpeg", "image/webp", "image/gif"].includes(info.media_type)) {
        const bytes = await readSmallFile(info);
        if (generation !== this.generation) return false;
        const img = element("img");
        img.src = this.blobUrl(bytes, info.media_type);
        img.alt = title;
        mainPane.append(img);
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

  /** Snapshot to go back to when the stitched view cannot show the position. */
  private snapshotFallback: { id: string; title: string; options: ReaderOpenOptions } | undefined;

  private async openCanonicalAt(
    canonical: CanonicalReadingTarget,
    focus: ReaderFocus,
    fallback: { id: string; title: string; options: ReaderOpenOptions },
  ): Promise<boolean> {
    this.snapshotFallback = fallback;
    try {
      return await this.open(canonical.id, canonical.title, { focus });
    } finally {
      if (this.snapshotFallback === fallback) this.snapshotFallback = undefined;
    }
  }

  /** The element that scrolls the open document. */
  private scrollerOf(element: HTMLElement): HTMLElement {
    return scrollParentOf(element, this.scrollContainer());
  }

  private blobUrl(bytes: Uint8Array, media: string): string {
    const url = URL.createObjectURL(new Blob([bytes.slice().buffer], { type: media }));
    this.urls.push(url);
    return url;
  }

  private async pdf(
    info: ContentInfo,
    generation: number,
    pdfOptions: ReaderOpenOptions = {},
    container: HTMLElement = this.host,
  ): Promise<void> {
    const mounted = await mountPdfViewer(
      container,
      info,
      () => generation !== this.generation,
      {
        pdfPageIndex: pdfOptions.pdfPageIndex,
        candidatePages: pdfOptions.candidatePages,
        backlinkNote: pdfOptions.backlinkNote,
        focusExcerpt: pdfOptions.focusExcerpt,
        returnTo: pdfOptions.returnTo,
        onReturnTo: (target) => {
          void this.open(target.id, target.title, target.options as ReaderOpenOptions | undefined);
        },
        onError: (error) => {
          if (generation !== this.generation) return;
          this.state = "error";
          this.error = error.message;
        },
      },
    );
    if (!mounted) return;
    if (generation !== this.generation) {
      mounted.dispose();
      return;
    }
    this.disposeDocument = () => mounted.dispose();
  }

  /**
   * 「规范解析正文」 unified view: every coverage segment in page order with
   * gaps marked. Each segment's blocks retain their immutable physical snapshot
   * coordinates so `✎ N` markers, `Shift+单击` selection, and direct ink-note
   * creation work seamlessly without writing a synthetic stitched Artifact.
   */
  private async canonical(
    id: string,
    title: string,
    generation: number,
    focus?: ReaderFocus,
    container: HTMLElement = this.host,
  ): Promise<CanonicalLanding> {
    const projection = await canonicalReaderProjection(id);
    if (generation !== this.generation) return { kind: "none" };
    let landing: CanonicalLanding = focus ? planCanonicalLanding(projection, focus) : { kind: "none" };
    if (focus?.artifactId && landing.kind === "needs_page") {
      // The block's page is shown from another snapshot; find which page it is.
      let pageIndex: number | undefined;
      try {
        const backlink = await getSourceBacklink({
          artifact_id: focus.artifactId,
          conversion_id: null,
          start: focus.start,
          end: focus.end,
          block_ids: focus.blockIds ?? [],
        });
        pageIndex = backlink.marker_page_ids?.[0] ?? backlink.pdf_page_indices?.[0];
      } catch { /* No page mapping: the snapshot stays the place to read it. */ }
      if (generation !== this.generation) return { kind: "none" };
      landing = pageIndex === undefined ? { kind: "snapshot" } : planCanonicalLanding(projection, focus, pageIndex);
    }
    if (landing.kind === "snapshot") return landing;
    const controls = element("div"); controls.className = "reader-controls";
    controls.append(element("span", projectionSummary(projection)));
    const openRootPdf = element("button", "打开原始 PDF");
    openRootPdf.type = "button";
    openRootPdf.onclick = () => void this.open(projection.source_pdf_artifact_id, `${title} · 原始 PDF`, {
      returnTo: { id, title, label: "返回规范解析正文" },
    });
    const article = element("article"); article.className = "markdown-body canonical-reader";
    article.dataset.canonicalId = projection.canonical_id;
    const outlineHandle = mountMarkdownOutline(article, (domKey) =>
      article.querySelector<HTMLElement>(`[data-canonical-key="${CSS.escape(domKey)}"]`),
    );
    controls.append(outlineHandle.toggleButton, openRootPdf);

    const statusHint = element("p");
    statusHint.className = "canonical-status-hint";
    statusHint.setAttribute("role", "status");
    statusHint.hidden = true;
    const setStatusHint = (msg: string | null) => {
      statusHint.textContent = msg ?? "";
      statusHint.hidden = !msg;
    };

    const layout = element("div");
    layout.className = "md-reader-layout";
    const paperStage = element("div");
    paperStage.className = "md-paper-stage";
    paperStage.append(article);
    layout.append(outlineHandle.nav, paperStage);
    container.append(controls, statusHint, layout);

    const openCanonicalPdfPage = (pageIndex: number, candidatePages?: number[], returnFocus?: ReaderFocus) => {
      void this.open(projection.source_pdf_artifact_id, `${title} · PDF 原页`, {
        pdfPageIndex: pageIndex,
        candidatePages,
        returnTo: {
          id,
          title,
          options: returnFocus ? { focus: returnFocus } : undefined,
          label: "返回规范解析正文",
        },
      });
    };

    const md = markdownRenderer();
    const decoder = new TextDecoder("utf-8", { fatal: true });
    interface CanonicalSegBlockEntry {
      domKey: string;
      block: MarkdownBlock;
      rawText: string;
      fallbackPdfPageIndex: number | null;
      wrapper: HTMLElement;
    }
    interface CanonicalSegState {
      segmentIndex: number;
      segment: CanonicalReaderSegment;
      body: HTMLElement;
      env: Record<string, unknown>;
      allBlocks: CanonicalSegBlockEntry[];
    }
    const segments: CanonicalSegState[] = [];
    const canonicalItems: CanonicalBlockItem[] = [];
    const canonicalItemByDomKey = new Map<string, CanonicalBlockItem>();
    const outlineBlocks: OutlineBlockDescriptor[] = [];
    const outlineBlockByDomKey = new Map<string, OutlineBlockDescriptor>();
    const errataByArtifact = new Map<string, MarkdownBlockErrata[]>();
    const errataEditor = new InlineErrataEditorController();
    let loadedUnitMarkers: Awaited<ReturnType<typeof getMarkdownUnitMarkers>> = [];
    let segIdx = 0;

    /** Resolves to whether the segment had images to place (they shift the layout). */
    const resolveSegmentImages = async (segment: CanonicalReaderSegment, body: HTMLElement): Promise<boolean> => {
      const placeholders = body.querySelectorAll<HTMLElement>("[data-image-ref]");
      if (!placeholders.length) return false;
      let info: ContentInfo | undefined;
      try { info = await contentInfo(segment.artifact_id); } catch { /* shown per image */ }
      if (generation !== this.generation) return false;
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
          if (generation !== this.generation) return false;
          const img = element("img"); img.alt = placeholder.textContent ?? "";
          img.src = this.blobUrl(data, imageInfo.media_type);
          placeholder.replaceWith(img);
        } catch (error) {
          placeholder.textContent = "[" + String(error) + "]";
        }
      }
      return true;
    };

    const applyErrataUpdate = (artifactId: string, saved?: MarkdownBlockErrata, removeIds: readonly string[] = []) => {
      const drop = new Set(removeIds);
      const next = (errataByArtifact.get(artifactId) ?? []).filter(
        (item) => item.status === "active" && !drop.has(item.errata_id),
      );
      if (saved && saved.status === "active") {
        next.push(saved);
        next.sort((a, b) => a.base_byte_start - b.base_byte_start);
      }
      errataByArtifact.set(artifactId, next);
    };

    let openCanonicalErrataForRange!: (seg: CanonicalSegState, firstIdx: number, lastIdx: number) => void;
    let canonicalInk: CanonicalInkController | undefined;

    const renderCanonicalSegmentBlocks = (seg: CanonicalSegState, refreshChrome: boolean): void => {
      const activeList = errataByArtifact.get(seg.segment.artifact_id) ?? [];
      const lookup = buildBlockErrataLookup(activeList);

      for (let i = 0; i < seg.allBlocks.length; i++) {
        const entry = seg.allBlocks[i];
        if (entry.block.kind === "page_marker") continue;
        const wrapper = entry.wrapper;
        wrapper.classList.remove("has-errata", "is-errata-merged-hidden");
        delete wrapper.dataset.errataId;
        wrapper.querySelectorAll(".errata-block-badge").forEach((el) => el.remove());

        if (lookup.hiddenSecondaryBlockIds.has(entry.block.id)) {
          wrapper.classList.add("is-errata-merged-hidden");
          wrapper.replaceChildren();
          const ob = outlineBlockByDomKey.get(entry.domKey);
          if (ob) ob.text = "";
          continue;
        }

        const activeErrata = lookup.primaryByBlockId.get(entry.block.id);
        const effectiveText = activeErrata ? activeErrata.corrected_text : entry.rawText;
        const ci = canonicalItemByDomKey.get(entry.domKey);
        if (ci) ci.text = effectiveText;
        const ob = outlineBlockByDomKey.get(entry.domKey);
        if (ob) ob.text = effectiveText;

        wrapper.innerHTML = md.render(effectiveText, seg.env);
        if (!wrapper.innerHTML.trim()) wrapper.append(element("pre", effectiveText));
        wrapper.querySelectorAll("a").forEach((link) => {
          link.removeAttribute("href");
          link.title = "链接导航尚未启用";
        });

        if (activeErrata) {
          wrapper.classList.add("has-errata");
          wrapper.dataset.errataId = activeErrata.errata_id;
          const badge = element("button", "已校对");
          badge.type = "button";
          badge.className = "errata-block-badge";
          badge.title = "查看原 OCR 对比、继续编辑或一键恢复原版";
          badge.setAttribute("aria-label", "已校对，点击查看原 OCR 对比");
          const blockIdx = i;
          badge.addEventListener("click", (event) => {
            event.stopPropagation();
            openErrataDiffPopover({
              anchor: badge,
              errata: activeErrata,
              onEdit: () => openCanonicalErrataForRange(seg, blockIdx, blockIdx),
              onRevert: async () => {
                const prevScroll = this.host.scrollTop;
                const res = await revertMarkdownErrata(activeErrata.errata_id);
                applyErrataUpdate(seg.segment.artifact_id, undefined, [res.errata.errata_id]);
                renderCanonicalSegmentBlocks(seg, true);
                this.host.scrollTop = prevScroll;
                setStatusHint("已恢复该块的原始 OCR 文本。");
              },
            });
          });
          badge.addEventListener("keydown", (event) => event.stopPropagation());
          wrapper.append(badge);
        }
      }

      if (refreshChrome) {
        outlineHandle.update(outlineBlocks, loadedUnitMarkers);
        canonicalInk?.decorate();
        void resolveSegmentImages(seg.segment, seg.body);
      }
    };

    for (const piece of projection.pieces) {
      if (piece.kind === "gap") {
        const gap = element("div", gapLabel(piece));
        gap.className = "canonical-gap";
        gap.setAttribute("role", "note");
        gap.dataset.pages = `${piece.pages.from}-${piece.pages.to}`;
        article.append(gap);
        continue;
      }
      const currentSegIdx = segIdx++;
      if (!errataByArtifact.has(piece.artifact_id)) {
        errataByArtifact.set(piece.artifact_id, (piece.errata ?? []).filter((e) => e.status === "active"));
      }
      const segLbl = segmentLabel(piece);
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
      open.title = "单独打开此物理快照，可用于发起 LLM 内容发现";
      open.onclick = () => void this.open(piece.artifact_id, title + " · " + segLbl);
      headerActions.append(open);
      header.append(element("span", segLbl), headerActions);
      const body = element("div");
      let activePageIndex: number | null = piece.pages ? piece.pages.from - 1 : null;
      const segEnv: Record<string, unknown> = {};
      const segAllBlocks: CanonicalSegBlockEntry[] = [];

      if (piece.blocks && piece.blocks.length > 0 && piece.content_hash) {
        md.parse(piece.markdown, segEnv);
        const segBytes = new TextEncoder().encode(piece.markdown);
        piece.blocks.forEach((b, blockOrdinal) => {
          const relStart = Math.max(0, b.range.start - piece.byte_start);
          const relEnd = Math.min(segBytes.length, Math.max(relStart, b.range.end - piece.byte_start));
          const text = decoder.decode(segBytes.subarray(relStart, relEnd));
          const domKey = `${currentSegIdx}:${b.id}`;
          if (b.kind === "page_marker") {
            const pageMatch = /^\s*\{(\d+)\}-{16,}\s*$/.exec(text)?.[1];
            if (pageMatch !== undefined) activePageIndex = Number(pageMatch);
            const marker = element("div");
            marker.className = "source-block source-page-marker";
            marker.dataset.canonicalKey = domKey;
            marker.dataset.sourceBlock = b.id;
            const pageNum = pageMatch !== undefined ? Number(pageMatch) : activePageIndex;
            if (pageNum !== null) marker.dataset.pdfPageIndex = String(pageNum);
            marker.append(element("span", pageNum !== null ? `PDF 第 ${pageNum + 1} 页` : "PDF 页边界"));
            if (pageNum !== null) {
              const jump = element("button", "查看原页 ↗");
              jump.type = "button";
              jump.className = "page-marker-jump";
              const targetIdx = pageNum;
              jump.onclick = (event) => { event.stopPropagation(); openCanonicalPdfPage(targetIdx); };
              marker.append(jump);
            }
            body.append(marker);
            segAllBlocks.push({ domKey, block: b, rawText: text, fallbackPdfPageIndex: activePageIndex, wrapper: marker });
            return;
          }

          const wrapper = element("div");
          wrapper.className = "source-block";
          wrapper.dataset.canonicalKey = domKey;
          wrapper.dataset.sourceBlock = b.id;
          wrapper.dataset.segmentArtifactId = piece.artifact_id;
          wrapper.dataset.segmentLabel = segLbl;
          if (activePageIndex !== null) wrapper.dataset.pdfPageIndex = String(activePageIndex);
          const isFocused = Boolean(focus && landing.kind === "block" && focusMatchesBlock(focus, piece.artifact_id, b));
          if (isFocused) wrapper.classList.add("source-block-focus");
          wrapper.tabIndex = 0;
          wrapper.setAttribute("aria-label", `分段 ${currentSegIdx + 1} 原文块 ${blockOrdinal + 1}`);
          wrapper.title = `分段 ${currentSegIdx + 1} · 第 ${blockOrdinal + 1} 块 · 单击选择，Shift+单击连选多块，按 E 就地校对，右键手写或反查 PDF`;
          body.append(wrapper);

          const item: CanonicalBlockItem = {
            domKey,
            segmentIndex: currentSegIdx,
            segmentLabel: segLbl,
            artifactId: piece.artifact_id,
            conversionId: piece.conversion_id,
            contentHash: piece.content_hash!,
            block: b,
            text,
            fallbackPdfPageIndex: activePageIndex,
          };
          canonicalItems.push(item);
          canonicalItemByDomKey.set(domKey, item);
          const ob: OutlineBlockDescriptor = {
            id: b.id,
            domKey,
            artifactId: piece.artifact_id,
            range: b.range,
            text,
            kind: b.kind,
          };
          outlineBlocks.push(ob);
          outlineBlockByDomKey.set(domKey, ob);
          segAllBlocks.push({ domKey, block: b, rawText: text, fallbackPdfPageIndex: activePageIndex, wrapper });
        });
      } else {
        let fallbackOrdinal = 0;
        for (const chunk of splitByPageMarkers(piece.markdown)) {
          if (chunk.kind === "page") {
            activePageIndex = chunk.pageId;
            const marker = element("div"); marker.className = "source-block source-page-marker";
            marker.dataset.pdfPageIndex = String(chunk.pageId);
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
          const domKey = `${currentSegIdx}:chunk-${fallbackOrdinal++}`;
          const block = element("div"); block.className = "source-block";
          block.dataset.canonicalKey = domKey;
          if (activePageIndex !== null) block.dataset.pdfPageIndex = String(activePageIndex);
          block.dataset.segmentArtifactId = piece.artifact_id;
          block.dataset.segmentLabel = segLbl;
          block.innerHTML = md.render(chunk.text, {});
          if (!block.innerHTML.trim()) block.append(element("pre", chunk.text));
          body.append(block);
          outlineBlocks.push({
            id: domKey,
            domKey,
            artifactId: piece.artifact_id,
            range: { start: 0, end: 0 },
            text: chunk.text,
          });
        }
      }
      if (piece.pages) section.dataset.pages = `${piece.pages.from}-${piece.pages.to}`;
      section.append(header, body);
      article.append(section);
      const segState: CanonicalSegState = {
        segmentIndex: currentSegIdx,
        segment: piece,
        body,
        env: segEnv,
        allBlocks: segAllBlocks,
      };
      segments.push(segState);
      if (segAllBlocks.length > 0) {
        renderCanonicalSegmentBlocks(segState, false);
      }
    }

    const openCanonicalPdfForItems = (firstItem: CanonicalBlockItem, rangeItems: CanonicalBlockItem[]) => {
      void (async () => {
        const start = rangeItems[0].block.range.start;
        const end = rangeItems[rangeItems.length - 1].block.range.end;
        const blockIds = rangeItems.map((i) => i.block.id);
        const returnFocus: ReaderFocus = { start, end, blockIds };
        try {
          const backlink = await getSourceBacklink({
            artifact_id: firstItem.artifactId,
            conversion_id: firstItem.conversionId,
            start,
            end,
            block_ids: blockIds,
          });
          if (generation !== this.generation) return;
          if (backlink.pdf_artifact_id && backlink.pdf_page_indices?.length) {
            await this.open(backlink.pdf_artifact_id, `${title} · PDF 原页`, {
              pdfPageIndex: backlink.pdf_page_indices[0],
              candidatePages: backlink.pdf_page_indices,
              backlinkNote: backlink.kind === "candidate_pdf_pages" ? backlink.reason : undefined,
              focusExcerpt: firstItem.text.slice(0, 240),
              returnTo: {
                id,
                title,
                options: { focus: returnFocus },
                label: "返回规范解析正文",
              },
            });
            return;
          }
        } catch {
          /* Fall back to page marker index if available. */
        }
        if (firstItem.fallbackPdfPageIndex !== null) {
          openCanonicalPdfPage(firstItem.fallbackPdfPageIndex, undefined, returnFocus);
        } else {
          setStatusHint("当前段落没有对应的 PDF 页映射。");
        }
      })();
    };

    openCanonicalErrataForRange = (seg: CanonicalSegState, firstIdx: number, lastIdx: number) => {
      const contentHash = seg.segment.content_hash;
      if (!contentHash) return;
      const activeList = errataByArtifact.get(seg.segment.artifact_id) ?? [];
      const cover = computeErrataCoverRange(
        seg.allBlocks.map((entry) => ({ block: entry.block, rawText: entry.rawText })),
        firstIdx,
        lastIdx,
        activeList,
      );
      if (!cover.ok) {
        setStatusHint(cover.reason);
        return;
      }
      setStatusHint(null);
      let anchorEl = seg.allBlocks[cover.startIdx].wrapper;
      for (let i = cover.endIdx; i >= cover.startIdx; i--) {
        if (!seg.allBlocks[i].wrapper.classList.contains("is-errata-merged-hidden")) {
          anchorEl = seg.allBlocks[i].wrapper;
          break;
        }
      }
      const rangeItems = seg.allBlocks
        .slice(cover.startIdx, cover.endIdx + 1)
        .map((e) => canonicalItemByDomKey.get(e.domKey))
        .filter((x): x is CanonicalBlockItem => Boolean(x));
      const fallbackPage = seg.allBlocks[cover.startIdx].fallbackPdfPageIndex;
      const toggleSidePdf = () => {
        if (this.isPdfReferenceOpen()) {
          this.closeErrataPdfReference();
          errataEditor.setPdfReferenceOpen(false);
          return;
        }
        void (async () => {
          let pdfArtifactId: string | null = projection.source_pdf_artifact_id || null;
          let targetPage: number | null = fallbackPage;
          let candidatePages: number[] | undefined;
          let backlinkNote: string | undefined;
          if (rangeItems.length > 0) {
            const firstItem = rangeItems[0];
            const start = rangeItems[0].block.range.start;
            const end = rangeItems[rangeItems.length - 1].block.range.end;
            const blockIds = rangeItems.map((i) => i.block.id);
            try {
              const backlink = await getSourceBacklink({
                artifact_id: firstItem.artifactId,
                conversion_id: firstItem.conversionId,
                start,
                end,
                block_ids: blockIds,
              });
              if (generation !== this.generation) return;
              if (backlink.pdf_artifact_id && backlink.pdf_page_indices?.length) {
                pdfArtifactId = backlink.pdf_artifact_id;
                targetPage = backlink.pdf_page_indices[0];
                candidatePages = backlink.pdf_page_indices;
                backlinkNote = backlink.kind === "candidate_pdf_pages" ? backlink.reason : undefined;
              }
            } catch {
              /* Fall back to canonical source PDF + marker page index. */
            }
          }
          if (!pdfArtifactId || targetPage === null) {
            setStatusHint("当前段落没有对应的 PDF 页映射。");
            return;
          }
          errataEditor.setPdfReferenceOpen(true);
          await this.openErrataPdfReference(
            {
              pdfArtifactId,
              pdfPageIndex: targetPage,
              candidatePages,
              backlinkNote,
              focusExcerpt: cover.initialText.slice(0, 240),
              title: `对照 PDF 第 ${targetPage + 1} 页`,
            },
            generation,
            () => errataEditor.setPdfReferenceOpen(false),
          );
        })();
      };
      errataEditor.open({
        anchorElement: anchorEl,
        artifactId: seg.segment.artifact_id,
        baseContentHash: contentHash,
        target: cover,
        verifiedPdfPage: fallbackPage,
        getVerifiedPdfPage: () => this.currentPdfReferencePageIndex(),
        isPdfReferenceOpen: () => this.isPdfReferenceOpen(),
        onOpenPdf: (rangeItems.length > 0 || (projection.source_pdf_artifact_id && fallbackPage !== null))
          ? toggleSidePdf
          : undefined,
        onClosePdf: () => this.closeErrataPdfReference(),
        onClose: () => this.closeErrataPdfReference(),
        onSaved: (res) => {
          const scroller = this.scrollContainer();
          const prevScroll = scroller.scrollTop;
          applyErrataUpdate(seg.segment.artifact_id, res.errata, res.superseded_ids);
          renderCanonicalSegmentBlocks(seg, true);
          scroller.scrollTop = prevScroll;
          const unitHint = res.affected_unit_ids.length > 0
            ? `（关联 ${res.affected_unit_ids.length} 个已入图节点，随时可在详情面板复核）`
            : "";
          setStatusHint(`已保存块级校对，当前正文与关联锚点已同步生效${unitHint}。`);
        },
        onReverted: (res) => {
          const scroller = this.scrollContainer();
          const prevScroll = scroller.scrollTop;
          applyErrataUpdate(seg.segment.artifact_id, undefined, [res.errata.errata_id]);
          renderCanonicalSegmentBlocks(seg, true);
          scroller.scrollTop = prevScroll;
          setStatusHint("已恢复该块的原始 OCR 文本。");
        },
      });
    };

    let fallbackMenuListener: ((event: MouseEvent) => void) | undefined;
    if (canonicalItems.length > 0) {
      canonicalInk = new CanonicalInkController(canonicalItems, article, {
        onStatusHint: setStatusHint,
        coveredBlockIdsFor: (artifactId, blockId) => {
          const active = errataByArtifact.get(artifactId) ?? [];
          const match = active.find((e) => e.status === "active" && e.first_block_id === blockId);
          return match?.covered_block_ids.length ? match.covered_block_ids : [blockId];
        },
        hasPageMarkerBetween: (firstItem, lastItem) => {
          const seg = segments[firstItem.segmentIndex];
          if (!seg) return false;
          const a = seg.allBlocks.findIndex((e) => e.block.id === firstItem.block.id);
          const b = seg.allBlocks.findIndex((e) => e.block.id === lastItem.block.id);
          if (a < 0 || b < a) return false;
          return seg.allBlocks.slice(a, b + 1).some((e) => e.block.kind === "page_marker");
        },
        onEditErrata: (rangeItems) => {
          const seg = segments[rangeItems[0].segmentIndex];
          if (!seg) return;
          const firstIdx = seg.allBlocks.findIndex((e) => e.block.id === rangeItems[0].block.id);
          const lastIdx = seg.allBlocks.findIndex((e) => e.block.id === rangeItems[rangeItems.length - 1].block.id);
          if (firstIdx >= 0 && lastIdx >= firstIdx) {
            openCanonicalErrataForRange(seg, firstIdx, lastIdx);
          }
        },
        onOpenSnapshot: (artifactId, segLbl, rangeItems) => {
          const start = rangeItems[0].block.range.start;
          const end = rangeItems[rangeItems.length - 1].block.range.end;
          const blockIds = rangeItems.map((i) => i.block.id);
          void this.open(artifactId, `${title} · ${segLbl}`, {
            focus: { start, end, blockIds },
          });
        },
        onOpenPdfRange: openCanonicalPdfForItems,
      });
    } else {
      fallbackMenuListener = (event: MouseEvent): void => {
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
      article.addEventListener("contextmenu", fallbackMenuListener);
    }

    // Where the reader arrived: the block itself, or the start of its PDF page
    // when that page is shown from another snapshot.
    const landingTarget = (): HTMLElement | null => {
      if (landing.kind === "block") return article.querySelector<HTMLElement>(".source-block-focus");
      if (landing.kind === "segment") {
        return article.querySelector<HTMLElement>(`.canonical-segment[data-artifact-id="${CSS.escape(landing.artifactId)}"]`);
      }
      if (landing.kind !== "page") return null;
      const marker = article.querySelector<HTMLElement>(`.source-page-marker[data-pdf-page-index="${landing.pageIndex}"]`);
      if (marker) return marker;
      return [...article.querySelectorAll<HTMLElement>(".canonical-segment[data-pages]")].find((section) => {
        const [from, to] = (section.dataset.pages ?? "").split("-").map(Number);
        return from <= landing.pageIndex + 1 && landing.pageIndex + 1 <= to;
      }) ?? null;
    };
    let pin: ReturnType<typeof pinToViewportTop> | undefined;
    let disposeReturnPoint: (() => void) | undefined;

    this.disposeDocument = () => {
      errataEditor.close();
      closeErrataDiffPopover();
      canonicalInk?.dispose();
      outlineHandle.dispose();
      pin?.dispose();
      disposeReturnPoint?.();
      if (fallbackMenuListener) article.removeEventListener("contextmenu", fallbackMenuListener);
    };

    if (!projection.pieces.length) article.append(element("p", "规范解析正文暂无可读快照。"));
    const arrivedAt = landingTarget();
    if (focus && arrivedAt) {
      if (landing.kind === "page" && focus.artifactId) {
        const snapshotId = focus.artifactId;
        const { pdfPageIndex: _page, viewportTop: _top, artifactId: _artifact, ...snapshotFocus } = focus;
        setStatusHint(`这一块所在的 PDF 第 ${landing.pageIndex + 1} 页在规范解析正文中由另一个解析快照显示，已定位到该页开头。`);
        const openSnapshot = element("button", "在原快照中查看这一块");
        openSnapshot.type = "button";
        openSnapshot.className = "canonical-status-action";
        openSnapshot.onclick = () => void this.open(snapshotId, `${title} · 解析快照`, { focus: snapshotFocus });
        statusHint.append(" ", openSnapshot);
      }
      if (focus.viewportTop !== undefined) {
        pin = pinToViewportTop(landingTarget, focus.viewportTop, (target) => this.scrollerOf(target), this.host);
        pin.apply();
      } else {
        arrivedAt.scrollIntoView(landing.kind === "block" ? { behavior: "smooth", block: "center" } : { block: "start" });
      }
      if (landing.kind !== "segment") {
        disposeReturnPoint = mountReturnPoint({
          container,
          listenOn: this.host,
          label: focus.label ?? (landing.kind === "page" ? `PDF 第 ${landing.pageIndex + 1} 页` : blockLabel(arrivedAt)),
          find: landingTarget,
          scrollerOf: (target) => this.scrollerOf(target),
        });
      }
    }
    article.querySelectorAll("a").forEach(link => { link.removeAttribute("href"); link.title = "链接导航尚未启用"; });

    const uniqueSegArtifacts = [...new Set(segments.map((s) => s.segment.artifact_id))];
    loadedUnitMarkers = await getMarkdownUnitMarkers(uniqueSegArtifacts);
    if (generation !== this.generation) return landing;
    outlineHandle.update(outlineBlocks, loadedUnitMarkers);
    canonicalInk?.setUnitMarkers(loadedUnitMarkers);
    pin?.apply();
    for (const seg of segments) {
      const placed = await resolveSegmentImages(seg.segment, seg.body);
      if (generation !== this.generation) return landing;
      if (placed) pin?.apply();
    }
    return landing;
  }

  private async markdown(
    info: ContentInfo,
    title: string,
    generation: number,
    container: HTMLElement = this.host,
    focus?: ReaderFocus,
    patch?: ReaderOpenOptions["patch"],
    conversionId?: string | null,
    notice?: string,
  ): Promise<void> {
    const [initialDoc, unitMarkers, initialErrata] = await Promise.all([
      markdownDocument(info.artifact_id, conversionId ?? info.contexts[0]?.conversion_id ?? null),
      getMarkdownUnitMarkers([info.artifact_id]),
      listMarkdownErrata(info.artifact_id),
    ]);
    let document = initialDoc;
    let activeErrata: MarkdownBlockErrata[] = initialErrata.filter((e) => e.status === "active");
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
    const article = element("article"); article.className = "markdown-body";
    const outlineHandle = mountMarkdownOutline(article, (domKey) =>
      article.querySelector<HTMLElement>(`[data-source-block="${CSS.escape(domKey)}"]`),
    );
    controls.append(outlineHandle.toggleButton, raw);
    if (info.contexts.length > 1) controls.append(contextSelect);
    const layout = element("div");
    layout.className = "md-reader-layout";
    const paperStage = element("div");
    paperStage.className = "md-paper-stage";
    paperStage.append(article);
    layout.append(outlineHandle.nav, paperStage);
    container.append(controls, layout);
    if (notice) {
      const note = element("p", notice);
      note.className = "canonical-status-hint";
      note.setAttribute("role", "status");
      controls.after(note);
    }
    let rawMode = false, version = 0;
    let initialFocusScrolled = false;
    let inkLinks: ReaderInkLinks | undefined;
    let selection!: MarkdownSelection;
    const errataEditor = new InlineErrataEditorController();
    const currentConversionId = (): string | null =>
      info.contexts[Number(contextSelect.value) || 0]?.conversion_id ?? conversionId ?? null;

    // Reading on past this snapshot: it is one parse run's pages, while the
    // 「规范解析正文」 view stitches every run. Review of a patch stays here.
    const canonicalTarget = patch ? undefined : this.canonicalResolver?.(info.artifact_id);
    let encoded: { of: AnchoredMarkdown; bytes: Uint8Array } | undefined;
    const blockText = (index: number): string => {
      if (encoded?.of !== document) encoded = { of: document, bytes: new TextEncoder().encode(document.source) };
      const { range } = document.blocks[index];
      return new TextDecoder().decode(encoded.bytes.subarray(range.start, range.end));
    };
    const blockElement = (index: number): HTMLElement | null =>
      article.querySelector<HTMLElement>(`[data-source-block="${CSS.escape(document.blocks[index].id)}"]`);
    const enterCanonical = (index: number | null): void => {
      if (!canonicalTarget) return;
      const block = index === null ? undefined : document.blocks[index];
      const wrapper = index === null ? null : blockElement(index);
      const snapshotFocus = block ? { start: block.range.start, end: block.range.end, blockIds: [block.id] } : undefined;
      void this.openCanonicalAt(
        canonicalTarget,
        snapshotFocus
          ? {
              ...snapshotFocus,
              artifactId: info.artifact_id,
              pdfPageIndex: pageIndexOfBlock(document.blocks, blockText, index!),
              viewportTop: wrapper ? viewportTopOf(wrapper, this.scrollerOf(wrapper)) : undefined,
            }
          : { start: 0, end: 0, artifactId: info.artifact_id },
        { id: info.artifact_id, title, options: { conversionId: currentConversionId(), focus: snapshotFocus } },
      );
    };
    /** The block being read: the selection or located block when it is on
     * screen, otherwise the first block showing in the reader. */
    const currentReadingBlock = (): number | null => {
      if (rawMode) return null;
      const onScreen = (index: number): boolean => {
        const wrapper = document.blocks[index].kind === "page_marker" ? null : blockElement(index);
        return Boolean(wrapper && isInViewport(wrapper, this.scrollerOf(wrapper)));
      };
      const range = selection.targetRange();
      if (range) {
        for (let index = range.first; index <= range.last; index++) if (onScreen(index)) return index;
      }
      const located = article.querySelector<HTMLElement>(".source-block-focus")?.dataset.sourceBlock;
      const locatedIndex = located ? document.blocks.findIndex(block => block.id === located) : -1;
      if (locatedIndex >= 0 && onScreen(locatedIndex)) return locatedIndex;
      const first = document.blocks.findIndex((_, index) => onScreen(index));
      return first >= 0 ? first : null;
    };
    const boundaryCard = (edge: "start" | "end"): { card: HTMLElement; text: HTMLElement; button: HTMLButtonElement } => {
      const card = element("div");
      card.className = `reader-boundary-card is-${edge}`;
      card.setAttribute("role", "note");
      card.hidden = true;
      const text = element("span");
      const button = element("button", edge === "start" ? "继续向上阅读" : "继续向下阅读");
      button.type = "button";
      button.title = "进入按页序拼接的规范解析正文，停在这一块";
      card.append(text, button);
      return { card, text, button };
    };
    const startCard = canonicalTarget ? boundaryCard("start") : undefined;
    const endCard = canonicalTarget ? boundaryCard("end") : undefined;
    if (canonicalTarget && startCard && endCard) {
      const continueReading = element("button", "在规范解析正文中继续阅读");
      continueReading.type = "button";
      continueReading.className = "reader-canonical-entry";
      continueReading.title = "切换到按页序拼接的完整正文并停在当前这一块，可以继续向前或向后阅读";
      continueReading.onclick = () => enterCanonical(currentReadingBlock());
      controls.append(continueReading);
      paperStage.prepend(startCard.card);
      paperStage.append(endCard.card);
    }
    const refreshBoundaryCards = (): void => {
      if (!startCard || !endCard) return;
      const content = document.blocks.map((block, index) => (block.kind === "page_marker" ? -1 : index)).filter(index => index >= 0);
      const span = snapshotPageSpan(document.blocks, blockText);
      const usable = !rawMode && content.length > 0;
      // A snapshot that starts on the first PDF page has nothing before it.
      startCard.card.hidden = !usable || span?.first === 0;
      endCard.card.hidden = !usable;
      startCard.text.textContent = span
        ? `本快照从 PDF 第 ${span.first + 1} 页开始。要接着往前读，请进入规范解析正文。`
        : "本快照只包含一次解析的页。要接着往前读，请进入规范解析正文。";
      endCard.text.textContent = span
        ? `本快照到 PDF 第 ${span.last + 1} 页为止。要接着往后读，请进入规范解析正文。`
        : "本快照只包含一次解析的页。要接着往后读，请进入规范解析正文。";
      startCard.button.onclick = () => enterCanonical(content[0]);
      endCard.button.onclick = () => enterCanonical(content[content.length - 1]);
    };
    let disposeReturnPoint: (() => void) | undefined;
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
    const applyLocalErrataState = (saved?: MarkdownBlockErrata, removeIds: readonly string[] = []) => {
      const drop = new Set(removeIds);
      const next = activeErrata.filter((item) => item.status === "active" && !drop.has(item.errata_id));
      if (saved && saved.status === "active") {
        next.push(saved);
        next.sort((a, b) => a.base_byte_start - b.base_byte_start);
      }
      activeErrata = next;
    };
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
        const lookup = buildBlockErrataLookup(activeErrata);
        const firstErrata = lookup.byAnyBlockId.get(contentBlocks[0].id);
        let focusExcerpt = "";
        if (firstErrata) {
          focusExcerpt = firstErrata.corrected_text.slice(0, 240);
        } else {
          const excerptBytes = new TextEncoder().encode(document.source).subarray(start, Math.min(end, start + 360));
          try { focusExcerpt = new TextDecoder("utf-8", { fatal: false }).decode(excerptBytes); } catch { /* optional */ }
        }
        await this.open(backlink.pdf_artifact_id, `${title} · PDF 原页`, {
          pdfPageIndex: targetPage,
          candidatePages: pages,
          backlinkNote: backlink.kind === "candidate_pdf_pages" ? backlink.reason : undefined,
          focusExcerpt,
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
    let render!: () => Promise<void>;
    const openErrataForRange = (range: { first: number; last: number }): void => {
      if (rawMode) return;
      const sourceBytes = new TextEncoder().encode(document.source);
      const decoder = new TextDecoder("utf-8", { fatal: true });
      const items = document.blocks.map((block) => ({
        block,
        rawText: decoder.decode(sourceBytes.subarray(block.range.start, block.range.end)),
      }));
      const cover = computeErrataCoverRange(items, range.first, range.last, activeErrata);
      if (!cover.ok) {
        selection.notifyFeedback(cover.reason);
        return;
      }
      let anchorEl = article.querySelector<HTMLElement>(`[data-source-block="${CSS.escape(cover.firstBlockId)}"]`);
      for (let idx = cover.endIdx; idx >= cover.startIdx; idx--) {
        const candidate = article.querySelector<HTMLElement>(`[data-source-block="${CSS.escape(document.blocks[idx].id)}"]`);
        if (candidate && !candidate.classList.contains("is-errata-merged-hidden")) {
          anchorEl = candidate;
          break;
        }
      }
      if (!anchorEl) return;
      let inferredPage: number | null = null;
      for (let idx = cover.startIdx - 1; idx >= 0; idx--) {
        if (document.blocks[idx].kind === "page_marker") {
          const m = /^\s*\{(\d+)\}-{16,}\s*$/.exec(items[idx].rawText);
          if (m) inferredPage = Number(m[1]);
          break;
        }
      }
      const toggleSidePdf = () => {
        if (this.isPdfReferenceOpen()) {
          this.closeErrataPdfReference();
          errataEditor.setPdfReferenceOpen(false);
          return;
        }
        void (async () => {
          const contentBlocks = document.blocks.slice(cover.startIdx, cover.endIdx + 1).filter((b) => b.kind !== "page_marker");
          if (!contentBlocks.length) return;
          const start = contentBlocks[0].range.start;
          const end = contentBlocks[contentBlocks.length - 1].range.end;
          const blockIds = contentBlocks.map((b) => b.id);
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
            const targetPage = inferredPage !== null && pages.includes(inferredPage) ? inferredPage : pages[0];
            errataEditor.setPdfReferenceOpen(true);
            await this.openErrataPdfReference(
              {
                pdfArtifactId: backlink.pdf_artifact_id,
                pdfPageIndex: targetPage,
                candidatePages: pages,
                backlinkNote: backlink.kind === "candidate_pdf_pages" ? backlink.reason : undefined,
                focusExcerpt: cover.initialText.slice(0, 240),
                title: `对照 PDF 第 ${targetPage + 1} 页`,
              },
              generation,
              () => errataEditor.setPdfReferenceOpen(false),
            );
          } catch (error) {
            if (generation === this.generation) {
              selection.notifyFeedback("PDF 回链查询失败：" + (error instanceof Error ? error.message : String(error)));
            }
          }
        })();
      };
      errataEditor.open({
        anchorElement: anchorEl,
        artifactId: document.snapshot.artifact_id,
        baseContentHash: document.snapshot.content_hash,
        target: cover,
        verifiedPdfPage: inferredPage,
        getVerifiedPdfPage: () => this.currentPdfReferencePageIndex(),
        isPdfReferenceOpen: () => this.isPdfReferenceOpen(),
        onOpenPdf: toggleSidePdf,
        onClosePdf: () => this.closeErrataPdfReference(),
        onClose: () => this.closeErrataPdfReference(),
        onSaved: async (res) => {
          const scroller = this.scrollContainer();
          const prevScroll = scroller.scrollTop;
          applyLocalErrataState(res.errata, res.superseded_ids);
          await render();
          scroller.scrollTop = prevScroll;
          const unitHint = res.affected_unit_ids.length > 0
            ? `（关联 ${res.affected_unit_ids.length} 个已入图节点，随时可在详情面板复核）`
            : "";
          selection.notifyFeedback(`已保存块级校对，当前正文与关联锚点已同步生效${unitHint}。`);
        },
        onReverted: async (res) => {
          const scroller = this.scrollContainer();
          const prevScroll = scroller.scrollTop;
          applyLocalErrataState(undefined, [res.errata.errata_id]);
          await render();
          scroller.scrollTop = prevScroll;
          selection.notifyFeedback("已恢复该块的原始 OCR 文本。");
        },
      });
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
      unitMarkers,
      (range) => openErrataForRange(range),
      (blockId) => {
        const match = activeErrata.find((e) => e.status === "active" && e.first_block_id === blockId);
        return match?.covered_block_ids.length ? match.covered_block_ids : [blockId];
      },
      canonicalTarget
        ? (range) => {
            const index = document.blocks.findIndex((block, i) => i >= range.first && i <= range.last && block.kind !== "page_marker");
            enterCanonical(index >= 0 ? index : null);
          }
        : undefined,
    );
    selection.setOnSelectionChange(() => inkLinks?.syncActionPill());
    selection.setOnEditErrata((range) => openErrataForRange(range));
    controls.after(inkLinks.hint);
    this.disposeDocument = () => {
      errataEditor.close();
      closeErrataDiffPopover();
      selection.dispose();
      inkLinks?.dispose();
      outlineHandle.dispose();
      disposeReturnPoint?.();
    };
    render = async (): Promise<void> => {
      const current = ++version;
      errataEditor.close();
      closeErrataDiffPopover();
      this.revokeImages();
      refreshBoundaryCards();
      if (rawMode) {
        outlineHandle.setHiddenForRaw(true);
        article.replaceChildren(element("pre", document.source));
        inkLinks?.decorate();
        return;
      }
      outlineHandle.setHiddenForRaw(false);
      const md = markdownRenderer();
      const environment = {};
      // Parse the full source once to retain reference-style link/image definitions.
      md.parse(document.source, environment);
      const sourceBytes = new TextEncoder().encode(document.source);
      const decoder = new TextDecoder("utf-8", { fatal: true });
      const errataLookup = buildBlockErrataLookup(activeErrata);
      const outlineBlocks: OutlineBlockDescriptor[] = [];
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
        wrapper.title = `第 ${i + 1} 块 · 单击选择，按住 Shift 单击连选多块，按 E 就地校对，右键手写或反查 PDF`;
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
        if (errataLookup.hiddenSecondaryBlockIds.has(block.id)) {
          wrapper.classList.add("is-errata-merged-hidden");
          article.append(wrapper);
          return;
        }
        const blockErrata = errataLookup.primaryByBlockId.get(block.id);
        const effectiveText = blockErrata ? blockErrata.corrected_text : text;
        wrapper.innerHTML = md.render(effectiveText, environment);
        if (!wrapper.innerHTML.trim()) wrapper.append(element("pre", effectiveText));
        if (blockErrata) {
          wrapper.classList.add("has-errata");
          wrapper.dataset.errataId = blockErrata.errata_id;
          const badge = element("button", "已校对");
          badge.type = "button";
          badge.className = "errata-block-badge";
          badge.title = "查看原 OCR 对比、继续编辑或一键恢复原版";
          badge.setAttribute("aria-label", "已校对，点击查看原 OCR 对比");
          const blockIdx = i;
          badge.addEventListener("click", (event) => {
            event.stopPropagation();
            openErrataDiffPopover({
              anchor: badge,
              errata: blockErrata,
              onEdit: () => openErrataForRange({ first: blockIdx, last: blockIdx }),
              onRevert: async () => {
                const prevScroll = this.host.scrollTop;
                const res = await revertMarkdownErrata(blockErrata.errata_id);
                applyLocalErrataState(undefined, [res.errata.errata_id]);
                await render();
                this.host.scrollTop = prevScroll;
                selection.notifyFeedback("已恢复该块的原始 OCR 文本。");
              },
            });
          });
          badge.addEventListener("keydown", (event) => event.stopPropagation());
          wrapper.append(badge);
        }
        article.append(wrapper);
        outlineBlocks.push({
          id: block.id,
          domKey: block.id,
          artifactId: info.artifact_id,
          range: block.range,
          text: effectiveText,
          kind: block.kind,
        });
      });
      outlineHandle.update(outlineBlocks, unitMarkers);
      if (focus && !initialFocusScrolled) {
        initialFocusScrolled = true;
        const located = article.querySelector<HTMLElement>(".source-block-focus");
        located?.scrollIntoView({ behavior: "smooth", block: "center" });
        if (located) {
          disposeReturnPoint = mountReturnPoint({
            container,
            listenOn: this.host,
            label: focus.label ?? blockLabel(located),
            find: () => article.querySelector<HTMLElement>(".source-block-focus"),
            scrollerOf: (target) => this.scrollerOf(target),
          });
        }
      }
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
      errataEditor.close();
      closeErrataDiffPopover();
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
        selection.setOnEditErrata((range) => openErrataForRange(range));
        inkLinks?.setDocument(document);
        raw.disabled = false;
        return render();
      }).catch(error => {
        if (generation === this.generation && current === version) article.textContent = String(error);
      });
    };
    // Discovery patch approvals land as errata on this snapshot; reload the
    // effective text in place (manual saves already update local state).
    const stopErrataSync = eventBus.on("markdown-errata-changed", (detail) => {
      if (generation !== this.generation) { stopErrataSync(); return; }
      if (detail.action !== "discovery_patch" || detail.artifactId !== info.artifact_id) return;
      void listMarkdownErrata(info.artifact_id).then((items) => {
        if (generation !== this.generation) return;
        activeErrata = items.filter((item) => item.status === "active");
        return render();
      });
    });
    await render();
  }
}
