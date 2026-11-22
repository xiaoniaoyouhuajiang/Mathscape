/**
 * 「规范解析正文」块级手稿透传与直建手稿控制器。
 *
 * 在不破坏不可变物理快照原则（不写新 Artifact、不改 InkMarkdownTargetRef 契约）的前提下：
 * - 将每个拼接分段映射回其底层物理快照的 `(artifact_id, content_hash, MarkdownBlock)`；
 * - 并行拉取并实时刷新各块的 `✎ N` 手稿徽章；
 * - 支持同分段内单击 / `Shift + 单击` 连选块、块旁浮动操作胶囊（`✎ 手写此块/这 K 块`、`＋ 关联已有`、`↗ PDF 原页`）与右键菜单直建手稿。
 */
import { eventBus } from "../core/events";
import { hideContextMenu, isContextMenuOpen, showContextMenu } from "../graph/context-menu";
import { t } from "../i18n";
import {
  getInkMarkdownLinks,
  markdownPreviewLabel,
  openLinkExistingPopover,
  openRelatedNotesPopover,
  overlappingNotesForBlocks,
  startInkCreateFromTarget,
  type InkLinkTargetRef,
  type InkMarkdownLinks,
} from "../ink/links";
import type { MarkdownUnitMarker } from "./content";
import {
  bindReaderCopyShortcuts,
  copyTextToClipboard,
  joinBlocksMarkdown,
  pickVisibleActionAnchor,
} from "./markdown-copy";
import { matchUnitsToBlocks } from "./markdown-outline";
import type { MarkdownBlock } from "./selection";

export interface CanonicalBlockItem {
  domKey: string;
  segmentIndex: number;
  segmentLabel: string;
  artifactId: string;
  conversionId: string;
  contentHash: string;
  block: MarkdownBlock;
  text: string;
  fallbackPdfPageIndex: number | null;
}

export interface CanonicalInkCallbacks {
  onOpenPdfRange: (item: CanonicalBlockItem, rangeItems: CanonicalBlockItem[]) => void;
  onOpenSnapshot: (artifactId: string, segmentLabel: string, rangeItems: CanonicalBlockItem[]) => void;
  onEditErrata?: (rangeItems: CanonicalBlockItem[]) => void;
  coveredBlockIdsFor?: (artifactId: string, blockId: string) => string[];
  hasPageMarkerBetween?: (firstItem: CanonicalBlockItem, lastItem: CanonicalBlockItem) => boolean;
  onStatusHint?: (message: string | null) => void;
}

const MENU_TARGET_CLASS = "is-ink-menu-target";

export class CanonicalInkController {
  private linksByArtifact = new Map<string, InkMarkdownLinks>();
  private unitMarkers: readonly MarkdownUnitMarker[] = [];
  private disposed = false;
  private reloadVersion = 0;
  private currentIdx = -1;
  private firstIdx = -1;
  private lastIdx = -1;
  private shiftAnchorIdx = -1;
  private selectedViaShift = false;
  private scrollRaf = 0;
  private copyFeedbackActive = false;
  private lastPillRangeKey = "";
  private copyFeedbackTimer: number | undefined;
  private copyHintTimer: number | undefined;
  private readonly indexByDomKey: Map<string, number>;
  private readonly unsubscribe: () => void;

  constructor(
    private readonly items: readonly CanonicalBlockItem[],
    private readonly article: HTMLElement,
    private readonly callbacks: CanonicalInkCallbacks,
  ) {
    this.indexByDomKey = new Map(items.map((item, idx) => [item.domKey, idx]));
    this.article.addEventListener("click", this.onClick);
    this.article.addEventListener("keydown", this.onKey);
    this.article.addEventListener("contextmenu", this.onContextMenu);
    document.addEventListener("scroll", this.onScroll, { capture: true, passive: true });
    const offCopy = bindReaderCopyShortcuts({
      article: this.article,
      isDisabled: () => this.disposed,
      getSelectedBlocksCopy: () => {
        const range = this.activeRange();
        if (!range) return null;
        const contentItems = this.contentItemsIn(range);
        if (!contentItems.length) return null;
        return { text: this.rangeText(contentItems), count: this.visibleContentItems(contentItems).length };
      },
      onCopied: (count) => this.notifyCopied(count),
      onCopyFailed: () => this.callbacks.onStatusHint?.(t("reader.copy.failed")),
    });
    const offLinks = eventBus.on("ink-links-changed", () => void this.reloadLinks());
    const offNotes = eventBus.on("ink-notes-changed", () => void this.reloadLinks());
    this.unsubscribe = () => {
      offCopy();
      offLinks();
      offNotes();
    };
    void this.reloadLinks();
  }

  dispose(): void {
    this.disposed = true;
    this.reloadVersion++;
    if (this.scrollRaf) cancelAnimationFrame(this.scrollRaf);
    if (this.copyFeedbackTimer) window.clearTimeout(this.copyFeedbackTimer);
    if (this.copyHintTimer) window.clearTimeout(this.copyHintTimer);
    this.article.removeEventListener("click", this.onClick);
    this.article.removeEventListener("keydown", this.onKey);
    this.article.removeEventListener("contextmenu", this.onContextMenu);
    document.removeEventListener("scroll", this.onScroll, { capture: true });
    this.unsubscribe();
  }

  async reloadLinks(): Promise<void> {
    const version = ++this.reloadVersion;
    const uniqueArtifacts = [...new Set(this.items.map((i) => i.artifactId))];
    const results = await Promise.all(
      uniqueArtifacts.map(async (aid) => {
        try {
          const res = await getInkMarkdownLinks(aid);
          return [aid, res] as const;
        } catch {
          return [aid, undefined] as const;
        }
      }),
    );
    if (this.disposed || version !== this.reloadVersion) return;
    this.linksByArtifact.clear();
    for (const [aid, res] of results) {
      if (res) this.linksByArtifact.set(aid, res);
    }
    this.decorate();
  }

  /**
   * The reader changes `items` in place inside `mutate` (a newly parsed
   * segment is inserted): the selection stays on the same blocks.
   */
  reindex(mutate: () => void): void {
    const held = [this.currentIdx, this.firstIdx, this.lastIdx, this.shiftAnchorIdx]
      .map(idx => this.items[idx]?.domKey);
    mutate();
    this.indexByDomKey.clear();
    this.items.forEach((item, idx) => this.indexByDomKey.set(item.domKey, idx));
    [this.currentIdx, this.firstIdx, this.lastIdx, this.shiftAnchorIdx] = held
      .map(key => (key === undefined ? -1 : this.indexByDomKey.get(key) ?? -1));
    this.wrappers = undefined;
    void this.reloadLinks();
  }

  setUnitMarkers(markers: readonly MarkdownUnitMarker[]): void {
    this.unitMarkers = markers;
    this.decorate();
  }

  private unitIdsBySegmentBlock(segmentIndex: number): Map<string, string[]> {
    const out = new Map<string, string[]>();
    if (!this.unitMarkers.length) return out;
    const segItems = this.items.filter((i) => i.segmentIndex === segmentIndex);
    const matched = matchUnitsToBlocks(
      this.unitMarkers,
      segItems.map((i) => ({
        id: i.block.id,
        domKey: i.domKey,
        artifactId: i.artifactId,
        range: i.block.range,
        kind: i.block.kind,
      })),
    );
    for (const m of matched) {
      const list = out.get(m.blockId) ?? [];
      list.push(m.unit.unit_id);
      out.set(m.blockId, list);
    }
    return out;
  }

  decorate(): void {
    this.article.querySelectorAll(".ink-block-marker").forEach((m) => m.remove());
    this.article.querySelectorAll<HTMLElement>("[data-canonical-key].has-ink-marker").forEach((el) => el.classList.remove("has-ink-marker"));

    const unitMapsBySeg = new Map<number, Map<string, string[]>>();
    const getUnitMap = (segIdx: number) => {
      let m = unitMapsBySeg.get(segIdx);
      if (!m) {
        m = this.unitIdsBySegmentBlock(segIdx);
        unitMapsBySeg.set(segIdx, m);
      }
      return m;
    };

    for (const item of this.items) {
      if (item.block.kind === "page_marker") continue;
      const wrapper = this.wrapperFor(item.domKey);
      if (!wrapper || wrapper.classList.contains("is-errata-merged-hidden")) continue;
      const data = this.linksByArtifact.get(item.artifactId);
      if (!data) continue;
      const targetBlockIds = this.callbacks.coveredBlockIdsFor?.(item.artifactId, item.block.id) ?? [item.block.id];
      const related = overlappingNotesForBlocks(
        data.links,
        data.unit_links ?? [],
        targetBlockIds,
        getUnitMap(item.segmentIndex),
        item.contentHash,
      );
      if (!related.length) continue;
      const marker = document.createElement("button");
      marker.type = "button";
      marker.className = "ink-block-marker";
      marker.textContent = `✎ ${related.length}`;
      marker.setAttribute("aria-label", `${related.length} 份相关手稿`);
      marker.title = "查看相关手稿";
      marker.addEventListener("click", (event) => {
        event.stopPropagation();
        const bounds = marker.getBoundingClientRect();
        const latestData = this.linksByArtifact.get(item.artifactId);
        const latestNotes = overlappingNotesForBlocks(
          latestData?.links ?? [],
          latestData?.unit_links ?? [],
          this.callbacks.coveredBlockIdsFor?.(item.artifactId, item.block.id) ?? [item.block.id],
          this.unitIdsBySegmentBlock(item.segmentIndex),
          item.contentHash,
        );
        openRelatedNotesPopover({
          x: bounds.left,
          y: bounds.bottom + 4,
          title: "相关手稿",
          notes: latestNotes,
        });
      });
      marker.addEventListener("keydown", (event) => event.stopPropagation());
      wrapper.classList.add("has-ink-marker");
      wrapper.append(marker);
    }
    this.refreshSelectionUI();
  }

  /** Block wrappers are created once and re-rendered in place, so one index
   * serves every decorate pass; a per-block attribute query is quadratic. */
  private wrappers: Map<string, HTMLElement> | undefined;

  private wrapperFor(domKey: string): HTMLElement | null {
    const cached = this.wrappers?.get(domKey);
    if (cached?.isConnected) return cached;
    this.wrappers = new Map();
    for (const el of this.article.querySelectorAll<HTMLElement>("[data-canonical-key]")) {
      this.wrappers.set(el.dataset.canonicalKey!, el);
    }
    return this.wrappers.get(domKey) ?? null;
  }

  private clearSelection(): void {
    this.currentIdx = this.firstIdx = this.lastIdx = this.shiftAnchorIdx = -1;
    this.selectedViaShift = false;
    this.callbacks.onStatusHint?.(null);
    this.refreshSelectionUI();
  }

  private activeRange(contextIdx?: number): { first: number; last: number } | null {
    const valid =
      this.firstIdx >= 0 &&
      this.lastIdx >= this.firstIdx &&
      this.lastIdx < this.items.length &&
      this.items[this.firstIdx].segmentIndex === this.items[this.lastIdx].segmentIndex;
    let range: { first: number; last: number } | null = null;
    if (contextIdx === undefined) {
      if (valid) range = { first: this.firstIdx, last: this.lastIdx };
      else if (this.currentIdx >= 0) range = { first: this.currentIdx, last: this.currentIdx };
    } else if (contextIdx >= 0 && contextIdx < this.items.length) {
      range = valid && contextIdx >= this.firstIdx && contextIdx <= this.lastIdx
        ? { first: this.firstIdx, last: this.lastIdx }
        : { first: contextIdx, last: contextIdx };
    }
    if (!range) return null;
    const slice = this.items.slice(range.first, range.last + 1);
    return slice.some((i) => i.block.kind !== "page_marker") ? range : null;
  }

  private contentItemsIn(range: { first: number; last: number }): CanonicalBlockItem[] {
    return this.items.slice(range.first, range.last + 1).filter((i) => i.block.kind !== "page_marker");
  }

  private targetFor(contentItems: readonly CanonicalBlockItem[]): InkLinkTargetRef {
    const first = contentItems[0];
    const last = contentItems[contentItems.length - 1];
    return {
      kind: "markdown",
      artifact_id: first.artifactId,
      content_hash: first.contentHash,
      first_block: first.block.id,
      last_block: last.block.id,
    };
  }

  private visibleContentItems(contentItems: readonly CanonicalBlockItem[]): CanonicalBlockItem[] {
    const visible = contentItems.filter((item) => {
      const w = this.wrapperFor(item.domKey);
      return !w || !w.classList.contains("is-errata-merged-hidden");
    });
    return visible.length > 0 ? visible : [...contentItems];
  }

  private rangeText(contentItems: readonly CanonicalBlockItem[]): string {
    return joinBlocksMarkdown(this.visibleContentItems(contentItems).map((i) => i.text));
  }

  private notifyCopied(count: number): void {
    this.copyFeedbackActive = true;
    const btn = this.article.querySelector<HTMLButtonElement>(".block-pill-copy");
    if (btn) btn.textContent = t("reader.copy.pillCopied");
    if (this.copyFeedbackTimer) window.clearTimeout(this.copyFeedbackTimer);
    this.copyFeedbackTimer = window.setTimeout(() => {
      this.copyFeedbackActive = false;
      const liveBtn = this.article.querySelector<HTMLButtonElement>(".block-pill-copy");
      if (liveBtn) liveBtn.textContent = t("reader.copy.pill");
    }, 1400);
    const msg = count === 1 ? t("reader.copy.copiedSingle") : t("reader.copy.copiedMulti", { count });
    this.callbacks.onStatusHint?.(msg);
    if (this.copyHintTimer) window.clearTimeout(this.copyHintTimer);
    this.copyHintTimer = window.setTimeout(() => {
      if (!this.disposed) this.callbacks.onStatusHint?.(null);
    }, 2400);
  }

  private copyContentItems(contentItems: readonly CanonicalBlockItem[]): void {
    const text = this.rangeText(contentItems);
    const count = this.visibleContentItems(contentItems).length;
    void copyTextToClipboard(text).then((ok) => {
      if (ok) this.notifyCopied(count);
      else this.callbacks.onStatusHint?.(t("reader.copy.failed"));
    });
  }

  private onScroll = (): void => {
    if (this.disposed || this.firstIdx < 0 || this.lastIdx <= this.firstIdx) return;
    if (this.scrollRaf) return;
    this.scrollRaf = requestAnimationFrame(() => {
      this.scrollRaf = 0;
      if (this.disposed) return;
      const pill = this.article.querySelector<HTMLElement>(".block-action-pill");
      const range = this.activeRange();
      if (!pill || !range) return;
      const wrappers = this.visibleContentItems(this.contentItemsIn(range))
        .map((i) => this.wrapperFor(i.domKey))
        .filter((w): w is HTMLElement => Boolean(w));
      const placement = pickVisibleActionAnchor(wrappers);
      if (!placement) return;
      const currentParent = pill.parentElement;
      const wasBottom = pill.classList.contains("is-pill-bottom");
      if (currentParent === placement.wrapper && wasBottom === placement.pinBottom) return;
      currentParent?.classList.remove("has-action-pill");
      pill.classList.toggle("is-pill-bottom", placement.pinBottom);
      placement.wrapper.classList.add("has-action-pill");
      placement.wrapper.append(pill);
    });
  };

  private highlightMenuTarget(range: { first: number; last: number } | null): void {
    const wanted = new Set<string>();
    if (range) {
      for (let i = range.first; i <= range.last; i++) wanted.add(this.items[i].domKey);
    }
    this.article.querySelectorAll<HTMLElement>("[data-canonical-key]").forEach((el) => {
      el.classList.toggle(MENU_TARGET_CLASS, wanted.has(el.dataset.canonicalKey ?? ""));
    });
  }

  private createFromRange(range: { first: number; last: number }, x: number, y: number): void {
    const contentItems = this.contentItemsIn(range);
    if (!contentItems.length) return;
    const text = this.rangeText(contentItems);
    const excerpt = text.replace(/\s+/g, " ").trim();
    this.highlightMenuTarget(range);
    startInkCreateFromTarget({
      x,
      y,
      target: this.targetFor(contentItems),
      targetLabel: markdownPreviewLabel(text),
      excerpt: [...excerpt].length > 120 ? [...excerpt].slice(0, 120).join("") + "…" : excerpt,
      onClose: () => this.highlightMenuTarget(null),
    });
  }

  private refreshSelectionUI(): void {
    const valid =
      this.firstIdx >= 0 &&
      this.lastIdx >= this.firstIdx &&
      this.items[this.firstIdx]?.segmentIndex === this.items[this.lastIdx]?.segmentIndex;
    this.article.querySelectorAll<HTMLElement>("[data-canonical-key]").forEach((el) => {
      const idx = this.indexByDomKey.get(el.dataset.canonicalKey ?? "") ?? -1;
      el.classList.toggle("is-current-block", idx === this.currentIdx);
      el.classList.toggle("is-selected-block", Boolean(valid && idx >= this.firstIdx && idx <= this.lastIdx));
    });

    this.article.querySelectorAll(".block-action-pill").forEach((p) => p.remove());
    this.article.querySelectorAll<HTMLElement>("[data-canonical-key].has-action-pill").forEach((el) => el.classList.remove("has-action-pill"));

    const range = this.activeRange();
    if (!range) {
      this.lastPillRangeKey = "";
      this.copyFeedbackActive = false;
      return;
    }
    const rangeKey = `${range.first}:${range.last}`;
    if (rangeKey !== this.lastPillRangeKey) {
      this.lastPillRangeKey = rangeKey;
      this.copyFeedbackActive = false;
    }
    const contentItems = this.contentItemsIn(range);
    if (!contentItems.length) return;
    const visibleItems = this.visibleContentItems(contentItems);
    const wrappers = visibleItems
      .map((i) => this.wrapperFor(i.domKey))
      .filter((w): w is HTMLElement => Boolean(w));
    const placement = pickVisibleActionAnchor(wrappers);
    if (!placement) return;
    const anchorWrapper = placement.wrapper;

    const pill = document.createElement("div");
    pill.className = placement.pinBottom ? "block-action-pill is-pill-bottom" : "block-action-pill";
    pill.setAttribute("role", "toolbar");
    pill.setAttribute("aria-label", "所选段落快捷操作");
    const stop = (e: Event) => e.stopPropagation();
    pill.addEventListener("click", stop);
    pill.addEventListener("pointerdown", stop);
    pill.addEventListener("contextmenu", stop);
    pill.addEventListener("keydown", stop);

    const count = contentItems.length;
    const copyBtn = document.createElement("button");
    copyBtn.type = "button";
    copyBtn.className = "block-pill-btn block-pill-copy";
    copyBtn.textContent = this.copyFeedbackActive ? t("reader.copy.pillCopied") : t("reader.copy.pill");
    copyBtn.title = t("reader.copy.pillTitle");
    copyBtn.onclick = (e) => {
      e.stopPropagation();
      this.copyContentItems(contentItems);
    };
    pill.append(copyBtn);

    const writeBtn = document.createElement("button");
    writeBtn.type = "button";
    writeBtn.className = "block-pill-btn block-pill-primary";
    writeBtn.textContent = count === 1 ? "✎ 手写此块" : `✎ 手写这 ${count} 块`;
    writeBtn.title = "新建关联手稿并打开左右并排书写视图";
    writeBtn.onclick = (e) => {
      e.stopPropagation();
      const bounds = writeBtn.getBoundingClientRect();
      this.createFromRange(range, bounds.left, bounds.bottom + 6);
    };
    pill.append(writeBtn);

    const linkBtn = document.createElement("button");
    linkBtn.type = "button";
    linkBtn.className = "block-pill-btn";
    linkBtn.textContent = "＋ 关联已有";
    linkBtn.title = "将所选段落关联到一份已有手稿";
    linkBtn.onclick = (e) => {
      e.stopPropagation();
      const bounds = linkBtn.getBoundingClientRect();
      const label = markdownPreviewLabel(this.rangeText(contentItems));
      this.highlightMenuTarget(range);
      openLinkExistingPopover({
        x: bounds.left,
        y: bounds.bottom + 6,
        target: this.targetFor(contentItems),
        targetLabel: label,
        onClose: () => this.highlightMenuTarget(null),
      });
    };
    pill.append(linkBtn);

    if (this.callbacks.onEditErrata) {
      const firstItem = contentItems[0];
      const lastItem = contentItems[contentItems.length - 1];
      const crossesMarker = Boolean(this.callbacks.hasPageMarkerBetween?.(firstItem, lastItem));
      const editBtn = document.createElement("button");
      editBtn.type = "button";
      editBtn.className = "block-pill-btn block-pill-errata";
      editBtn.textContent = count === 1 ? "✎ 校对" : `✎ 合并校对 ${count} 块`;
      editBtn.disabled = crossesMarker;
      editBtn.title = crossesMarker
        ? "所选区间跨越了 PDF 分页标记，请分段校对"
        : "就地修正 OCR 错字或公式（快捷键 E），保留全部手稿与地图锚点";
      editBtn.onclick = (e) => {
        e.stopPropagation();
        this.callbacks.onEditErrata!(contentItems);
      };
      pill.append(editBtn);
    }

    const pdfBtn = document.createElement("button");
    pdfBtn.type = "button";
    pdfBtn.className = "block-pill-btn";
    pdfBtn.textContent = "↗ PDF 原页";
    pdfBtn.title = "跳转查看所选段落对应的原始 PDF 页";
    pdfBtn.onclick = (e) => {
      e.stopPropagation();
      this.callbacks.onOpenPdfRange(contentItems[0], contentItems);
    };
    pill.append(pdfBtn);

    anchorWrapper.classList.add("has-action-pill");
    anchorWrapper.append(pill);
  }

  private onClick = (event: Event): void => {
    const target = event.target as Element | null;
    if (!target || target.closest(".block-action-pill, .ink-block-marker, .page-marker-jump, .unit-map-badge-row, .errata-block-badge, .errata-inline-editor, .errata-diff-popover")) return;
    const blockEl = target.closest<HTMLElement>("[data-canonical-key]");
    if (!blockEl || !this.article.contains(blockEl)) return;
    const idx = this.indexByDomKey.get(blockEl.dataset.canonicalKey ?? "") ?? -1;
    if (idx < 0 || this.items[idx].block.kind === "page_marker") return;

    const isShift = event instanceof MouseEvent && event.shiftKey;
    if (isShift) {
      window.getSelection()?.removeAllRanges();
      const rawAnchor = this.shiftAnchorIdx >= 0 ? this.shiftAnchorIdx : this.currentIdx >= 0 ? this.currentIdx : idx;
      const anchor = this.items[rawAnchor]?.block.kind === "page_marker" ? idx : rawAnchor;
      this.shiftAnchorIdx = anchor;
      const anchorSeg = this.items[anchor].segmentIndex;
      let endIdx = idx;
      if (this.items[idx].segmentIndex !== anchorSeg) {
        if (idx > anchor) {
          endIdx = anchor;
          while (endIdx + 1 < this.items.length && this.items[endIdx + 1].segmentIndex === anchorSeg) endIdx++;
        } else {
          endIdx = anchor;
          while (endIdx - 1 >= 0 && this.items[endIdx - 1].segmentIndex === anchorSeg) endIdx--;
        }
        this.callbacks.onStatusHint?.("跨解析快照分段已自动截断至当前分段边界；如需跨段请分段创建手稿。");
      } else {
        this.callbacks.onStatusHint?.(null);
      }
      this.currentIdx = endIdx;
      this.firstIdx = Math.min(anchor, endIdx);
      this.lastIdx = Math.max(anchor, endIdx);
      this.selectedViaShift = true;
      this.refreshSelectionUI();
      return;
    }

    this.currentIdx = idx;
    this.shiftAnchorIdx = idx;
    this.callbacks.onStatusHint?.(null);
    if (this.selectedViaShift) {
      this.firstIdx = -1;
      this.lastIdx = -1;
      this.selectedViaShift = false;
    }
    this.refreshSelectionUI();
  };

  private onKey = (event: KeyboardEvent): void => {
    const target = event.target as HTMLElement | null;
    if (target && (target.closest(".errata-inline-editor, .errata-diff-popover") || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName) || target.isContentEditable)) {
      return;
    }
    if (event.key === "Escape" && !event.defaultPrevented) {
      if (isContextMenuOpen()) {
        event.preventDefault();
        hideContextMenu();
        return;
      }
      if (this.currentIdx >= 0 || this.firstIdx >= 0) {
        event.preventDefault();
        this.clearSelection();
        return;
      }
    }
    if ((event.key === "e" || event.key === "E") && !event.ctrlKey && !event.metaKey && !event.altKey && this.callbacks.onEditErrata) {
      const range = this.activeRange();
      if (range) {
        const contentItems = this.contentItemsIn(range);
        if (contentItems.length) {
          event.preventDefault();
          this.callbacks.onEditErrata(contentItems);
          return;
        }
      }
    }
    if ((event.key === "Enter" || event.key === " ") && target?.hasAttribute("data-canonical-key")) {
      event.preventDefault();
      this.onClick(event);
    }
  };

  private onContextMenu = (event: MouseEvent): void => {
    const target = event.target as Element | null;
    if (!target || target.closest(".ink-block-marker, .block-action-pill, .unit-map-badge-row, .errata-block-badge, .errata-inline-editor, .errata-diff-popover")) return;
    const blockEl = target.closest<HTMLElement>("[data-canonical-key]");
    if (!blockEl || !this.article.contains(blockEl)) return;
    const idx = this.indexByDomKey.get(blockEl.dataset.canonicalKey ?? "") ?? -1;
    if (idx < 0 || this.items[idx].block.kind === "page_marker") return;
    const range = this.activeRange(idx);
    if (!range) return;
    const contentItems = this.contentItemsIn(range);
    if (!contentItems.length) return;

    event.preventDefault();
    const first = contentItems[0];
    const data = this.linksByArtifact.get(first.artifactId);
    const related = overlappingNotesForBlocks(
      data?.links ?? [],
      data?.unit_links ?? [],
      contentItems.map((i) => i.block.id),
      this.unitIdsBySegmentBlock(first.segmentIndex),
      first.contentHash,
    );
    const { clientX: x, clientY: y } = event;
    this.highlightMenuTarget(range);
    const label = markdownPreviewLabel(this.rangeText(contentItems));

    const crossesMarker = Boolean(this.callbacks.hasPageMarkerBetween?.(first, contentItems[contentItems.length - 1]));
    const visibleCount = this.visibleContentItems(contentItems).length;
    showContextMenu(
      x,
      y,
      [
        {
          label: visibleCount === 1
            ? t("reader.copy.menuSingle")
            : t("reader.copy.menuMulti", { count: visibleCount }),
          onSelect: () => {
            this.highlightMenuTarget(null);
            this.copyContentItems(contentItems);
          },
        },
        ...(this.callbacks.onEditErrata
          ? [{
              label: contentItems.length === 1 ? "校对此块 OCR 正文…" : `合并校对这 ${contentItems.length} 块…`,
              disabled: crossesMarker,
              onSelect: () => {
                this.highlightMenuTarget(null);
                this.callbacks.onEditErrata!(contentItems);
              },
            }]
          : []),
        {
          label: first.fallbackPdfPageIndex !== null
            ? `定位对应 PDF 原页（第 ${first.fallbackPdfPageIndex + 1} 页）`
            : "定位对应 PDF 原页",
          onSelect: () => {
            this.highlightMenuTarget(null);
            this.callbacks.onOpenPdfRange(first, contentItems);
          },
        },
        {
          label: "新建关联手稿…",
          onSelect: () => this.createFromRange(range, x, y),
        },
        {
          label: "关联到已有手稿…",
          onSelect: () => {
            this.highlightMenuTarget(range);
            openLinkExistingPopover({
              x,
              y,
              target: this.targetFor(contentItems),
              targetLabel: label,
              onClose: () => this.highlightMenuTarget(null),
            });
          },
        },
        {
          label: `查看相关手稿（${related.length}）`,
          disabled: related.length === 0,
          onSelect: () => {
            this.highlightMenuTarget(range);
            openRelatedNotesPopover({
              x,
              y,
              title: "相关手稿",
              notes: related,
              onClose: () => this.highlightMenuTarget(null),
            });
          },
        },
        {
          label: "打开所属物理快照",
          onSelect: () => {
            this.highlightMenuTarget(null);
            this.callbacks.onOpenSnapshot(first.artifactId, first.segmentLabel, contentItems);
          },
        },
      ],
      () => this.highlightMenuTarget(null),
    );
  };
}
