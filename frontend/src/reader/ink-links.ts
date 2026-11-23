/**
 * N2 reader integration (§6.4, §6.5): block markers, the right-click menu,
 * "手写这段" and the patch-derived document hint. Block-level only.
 */
import { showContextMenu } from "../graph/context-menu";
import { t } from "../i18n";
import { eventBus } from "../core/events";
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
import { buildBlockErrataLookup, type MarkdownBlockErrata } from "./markdown-errata";
import { matchUnitsToBlocks } from "./markdown-outline";
import type { AnchoredMarkdown, MarkdownSelection } from "./selection";

const MENU_TARGET_CLASS = "is-ink-menu-target";

export class ReaderInkLinks {
  readonly hint = document.createElement("p");
  private links: InkMarkdownLinks | undefined;
  private unitMarkers: readonly MarkdownUnitMarker[] = [];
  private disposed = false;
  private version = 0;
  private scrollRaf = 0;
  private copyFeedbackActive = false;
  private lastPillRangeKey = "";
  private copyFeedbackTimer: number | undefined;
  private readonly unsubscribe: () => void;

  constructor(
    private data: AnchoredMarkdown,
    private readonly article: HTMLElement,
    private readonly selection: () => MarkdownSelection,
    private readonly isRaw: () => boolean,
    private readonly onOpenPdf?: (range: { first: number; last: number }) => void,
    initialUnitMarkers: readonly MarkdownUnitMarker[] = [],
    private readonly onEditErrata?: (range: { first: number; last: number }) => void,
    private readonly coveredBlockIdsFor?: (blockId: string) => string[],
    /** Present when this snapshot belongs to a 「规范解析正文」 view. */
    private readonly onContinueInCanonical?: (range: { first: number; last: number }) => void,
    private readonly getActiveErrata?: () => readonly MarkdownBlockErrata[],
  ) {
    this.unitMarkers = initialUnitMarkers;
    this.hint.className = "ink-patch-hint";
    this.hint.hidden = true;
    this.article.addEventListener("contextmenu", this.onContextMenu);
    document.addEventListener("scroll", this.onScroll, { capture: true, passive: true });
    const offCopy = bindReaderCopyShortcuts({
      article: this.article,
      isDisabled: () => this.disposed || this.isRaw(),
      getSelectedBlocksCopy: () => {
        const sel = this.selection();
        if (sel.isWholeSelection()) return null;
        const range = sel.targetRange();
        if (!range) return null;
        const blocks = this.visibleContentBlocks(range);
        if (!blocks.length) return null;
        return { text: this.rangeText(range), count: blocks.length };
      },
      onCopied: (count) => this.notifyCopied(count),
      onCopyFailed: () => this.selection().notifyFeedback(t("reader.copy.failed")),
    });
    const offLinks = eventBus.on("ink-links-changed", () => void this.reload());
    const offNotes = eventBus.on("ink-notes-changed", () => void this.reload());
    this.unsubscribe = () => { offCopy(); offLinks(); offNotes(); };
    void this.reload();
  }

  setDocument(data: AnchoredMarkdown): void {
    this.data = data;
    this.decorate();
  }

  setUnitMarkers(markers: readonly MarkdownUnitMarker[]): void {
    this.unitMarkers = markers;
    this.decorate();
  }

  private unitIdsByBlock(): Map<string, string[]> {
    const out = new Map<string, string[]>();
    if (!this.unitMarkers.length) return out;
    const matched = matchUnitsToBlocks(
      this.unitMarkers,
      this.data.blocks.map((b) => ({
        id: b.id,
        domKey: b.id,
        artifactId: this.data.snapshot.artifact_id,
        range: b.range,
        kind: b.kind,
      })),
    );
    for (const item of matched) {
      const list = out.get(item.blockId) ?? [];
      list.push(item.unit.unit_id);
      out.set(item.blockId, list);
    }
    return out;
  }

  dispose(): void {
    this.disposed = true;
    this.version++;
    if (this.scrollRaf) cancelAnimationFrame(this.scrollRaf);
    if (this.copyFeedbackTimer) window.clearTimeout(this.copyFeedbackTimer);
    this.article.removeEventListener("contextmenu", this.onContextMenu);
    document.removeEventListener("scroll", this.onScroll, { capture: true });
    this.unsubscribe();
    this.hint.remove();
  }

  async reload(): Promise<void> {
    const version = ++this.version;
    try {
      const links = await getInkMarkdownLinks(this.data.snapshot.artifact_id);
      if (this.disposed || version !== this.version) return;
      this.links = links;
    } catch {
      // Markers are advisory; the reader still works without them.
      if (this.disposed || version !== this.version) return;
      this.links = undefined;
    }
    this.decorate();
  }

  /** Re-applies markers after every render. */
  decorate(): void {
    this.article.querySelectorAll(".ink-block-marker").forEach(marker => marker.remove());
    this.article.querySelectorAll<HTMLElement>("[data-source-block].has-ink-marker").forEach(el => el.classList.remove("has-ink-marker"));
    const original = this.links?.original_note_count ?? 0;
    this.hint.hidden = original === 0;
    this.hint.textContent = original ? `原版本有 ${original} 份相关手稿` : "";
    if (!this.links || this.isRaw()) {
      this.syncActionPill();
      return;
    }
    const unitMap = this.unitIdsByBlock();
    for (const wrapper of this.article.querySelectorAll<HTMLElement>("[data-source-block]")) {
      const blockId = wrapper.dataset.sourceBlock ?? "";
      if (!blockId || wrapper.classList.contains("source-page-marker") || wrapper.classList.contains("is-errata-merged-hidden")) continue;
      const targetBlockIds = this.coveredBlockIdsFor?.(blockId) ?? [blockId];
      const related = overlappingNotesForBlocks(
        this.links.links,
        this.links.unit_links ?? [],
        targetBlockIds,
        unitMap,
        this.data.snapshot.content_hash,
      );
      if (!related.length) continue;
      const marker = document.createElement("button");
      marker.type = "button";
      marker.className = "ink-block-marker";
      marker.textContent = `✎ ${related.length}`;
      marker.setAttribute("aria-label", `${related.length} 份相关手稿`);
      marker.title = "查看相关手稿";
      marker.addEventListener("click", event => {
        event.stopPropagation();
        const bounds = marker.getBoundingClientRect();
        const latestNotes = overlappingNotesForBlocks(
          this.links?.links ?? [],
          this.links?.unit_links ?? [],
          this.coveredBlockIdsFor?.(blockId) ?? [blockId],
          this.unitIdsByBlock(),
          this.data.snapshot.content_hash,
        );
        openRelatedNotesPopover({ x: bounds.left, y: bounds.bottom + 4, title: "相关手稿", notes: latestNotes });
      });
      marker.addEventListener("keydown", event => event.stopPropagation());
      wrapper.classList.add("has-ink-marker");
      wrapper.append(marker);
    }
    this.syncActionPill();
  }

  /** Renders or updates the floating quick-action pill on the active block/range. */
  syncActionPill(): void {
    this.article.querySelectorAll(".block-action-pill").forEach(pill => pill.remove());
    this.article.querySelectorAll<HTMLElement>("[data-source-block].has-action-pill").forEach(el => el.classList.remove("has-action-pill"));
    if (this.disposed || this.isRaw()) return;
    const sel = this.selection();
    if (sel.isWholeSelection()) return;
    const range = sel.targetRange();
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
    const contentBlocks = this.data.blocks.slice(range.first, range.last + 1).filter(b => b.kind !== "page_marker");
    if (!contentBlocks.length) return;
    const wrappers = this.visibleContentBlocks(range)
      .map(b => this.article.querySelector<HTMLElement>(`[data-source-block="${b.id}"]`))
      .filter((w): w is HTMLElement => Boolean(w));
    const placement = pickVisibleActionAnchor(wrappers);
    if (!placement) return;
    const wrapper = placement.wrapper;

    const pill = document.createElement("div");
    pill.className = placement.pinBottom ? "block-action-pill is-pill-bottom" : "block-action-pill";
    pill.setAttribute("role", "toolbar");
    pill.setAttribute("aria-label", "所选段落快捷操作");
    const stop = (event: Event) => event.stopPropagation();
    pill.addEventListener("click", stop);
    pill.addEventListener("pointerdown", stop);
    pill.addEventListener("contextmenu", stop);
    pill.addEventListener("keydown", stop);

    const count = contentBlocks.length;
    const copyBtn = document.createElement("button");
    copyBtn.type = "button";
    copyBtn.className = "block-pill-btn block-pill-copy";
    copyBtn.textContent = this.copyFeedbackActive ? t("reader.copy.pillCopied") : t("reader.copy.pill");
    copyBtn.title = t("reader.copy.pillTitle");
    copyBtn.onclick = (event) => {
      event.stopPropagation();
      this.copyRange(range);
    };
    pill.append(copyBtn);

    const writeBtn = document.createElement("button");
    writeBtn.type = "button";
    writeBtn.className = "block-pill-btn block-pill-primary";
    writeBtn.textContent = count === 1 ? "✎ 手写此块" : `✎ 手写这 ${count} 块`;
    writeBtn.title = "新建关联手稿并打开左右并排书写视图";
    writeBtn.onclick = (event) => {
      event.stopPropagation();
      const bounds = writeBtn.getBoundingClientRect();
      this.create(range, bounds.left, bounds.bottom + 6);
    };
    pill.append(writeBtn);

    const linkBtn = document.createElement("button");
    linkBtn.type = "button";
    linkBtn.className = "block-pill-btn";
    linkBtn.textContent = "＋ 关联已有";
    linkBtn.title = "将所选段落关联到一份已有手稿";
    linkBtn.onclick = (event) => {
      event.stopPropagation();
      const bounds = linkBtn.getBoundingClientRect();
      const label = markdownPreviewLabel(this.rangeText(range));
      this.highlight(range);
      openLinkExistingPopover({
        x: bounds.left,
        y: bounds.bottom + 6,
        target: this.target(range),
        targetLabel: label,
        onClose: () => this.highlight(null),
      });
    };
    pill.append(linkBtn);

    if (this.onEditErrata) {
      const rangeBlocks = this.data.blocks.slice(range.first, range.last + 1);
      const crossesMarker = rangeBlocks.some(b => b.kind === "page_marker");
      const editBtn = document.createElement("button");
      editBtn.type = "button";
      editBtn.className = "block-pill-btn block-pill-errata";
      editBtn.textContent = count === 1 ? "✎ 校对" : `✎ 合并校对 ${count} 块`;
      editBtn.disabled = crossesMarker;
      editBtn.title = crossesMarker
        ? "所选区间跨越了 PDF 分页标记，请分段校对"
        : "就地修正 OCR 错字或公式（快捷键 E），保留全部手稿与地图锚点";
      editBtn.onclick = (event) => {
        event.stopPropagation();
        this.onEditErrata!(range);
      };
      pill.append(editBtn);
    }

    if (this.onOpenPdf) {
      const pdfBtn = document.createElement("button");
      pdfBtn.type = "button";
      pdfBtn.className = "block-pill-btn";
      pdfBtn.textContent = "↗ PDF 原页";
      pdfBtn.title = "跳转查看所选段落对应的原始 PDF 页";
      pdfBtn.onclick = (event) => {
        event.stopPropagation();
        this.onOpenPdf!(range);
      };
      pill.append(pdfBtn);
    }

    wrapper.classList.add("has-action-pill");
    wrapper.append(pill);
  }

  private blockIds(range: { first: number; last: number }): string[] {
    return this.data.blocks.slice(range.first, range.last + 1).map(block => block.id);
  }

  private highlight(range: { first: number; last: number } | null): void {
    const wanted = new Set(range ? this.blockIds(range) : []);
    this.article.querySelectorAll<HTMLElement>("[data-source-block]").forEach(block => {
      block.classList.toggle(MENU_TARGET_CLASS, wanted.has(block.dataset.sourceBlock ?? ""));
    });
  }

  private visibleContentBlocks(range: { first: number; last: number }) {
    const lookup = buildBlockErrataLookup(this.getActiveErrata?.() ?? []);
    const all = this.data.blocks.slice(range.first, range.last + 1).filter(block => block.kind !== "page_marker");
    const visible = all.filter(block => !lookup.hiddenSecondaryBlockIds.has(block.id));
    return visible.length > 0 ? visible : all;
  }

  private rangeText(range: { first: number; last: number }): string {
    const blocks = this.visibleContentBlocks(range);
    if (!blocks.length) return "";
    const lookup = buildBlockErrataLookup(this.getActiveErrata?.() ?? []);
    const bytes = new TextEncoder().encode(this.data.source);
    const decoder = new TextDecoder();
    const parts = blocks.map((block) => {
      const errata = lookup.primaryByBlockId.get(block.id);
      return errata ? errata.corrected_text : decoder.decode(bytes.subarray(block.range.start, block.range.end));
    });
    return joinBlocksMarkdown(parts);
  }

  private notifyCopied(count: number): void {
    this.copyFeedbackActive = true;
    if (this.copyFeedbackTimer) window.clearTimeout(this.copyFeedbackTimer);
    this.copyFeedbackTimer = window.setTimeout(() => {
      this.copyFeedbackActive = false;
      const liveBtn = this.article.querySelector<HTMLButtonElement>(".block-pill-copy");
      if (liveBtn) liveBtn.textContent = t("reader.copy.pill");
    }, 1400);
    const msg = count === 1 ? t("reader.copy.copiedSingle") : t("reader.copy.copiedMulti", { count });
    this.selection().notifyFeedback(msg);
    const btn = this.article.querySelector<HTMLButtonElement>(".block-pill-copy");
    if (btn) btn.textContent = t("reader.copy.pillCopied");
  }

  private copyRange(range: { first: number; last: number }): void {
    const count = this.visibleContentBlocks(range).length;
    const text = this.rangeText(range);
    void copyTextToClipboard(text).then((ok) => {
      if (ok) this.notifyCopied(count);
      else this.selection().notifyFeedback(t("reader.copy.failed"));
    });
  }

  private onScroll = (): void => {
    if (this.disposed || this.isRaw()) return;
    const sel = this.selection();
    if (sel.isWholeSelection()) return;
    const range = sel.targetRange();
    if (!range || range.last <= range.first) return;
    if (this.scrollRaf) return;
    this.scrollRaf = requestAnimationFrame(() => {
      this.scrollRaf = 0;
      if (this.disposed || this.isRaw()) return;
      const pill = this.article.querySelector<HTMLElement>(".block-action-pill");
      const currentRange = this.selection().targetRange();
      if (!pill || !currentRange) return;
      const wrappers = this.visibleContentBlocks(currentRange)
        .map(b => this.article.querySelector<HTMLElement>(`[data-source-block="${b.id}"]`))
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

  private target(range: { first: number; last: number }): InkLinkTargetRef {
    return {
      kind: "markdown",
      artifact_id: this.data.snapshot.artifact_id,
      content_hash: this.data.snapshot.content_hash,
      first_block: this.data.blocks[range.first].id,
      last_block: this.data.blocks[range.last].id,
    };
  }

  /** "新建关联手稿…" / "手写这段" for a block range. */
  create(range: { first: number; last: number }, x: number, y: number): void {
    const text = this.rangeText(range);
    const excerpt = text.replace(/\s+/g, " ").trim();
    this.highlight(range);
    startInkCreateFromTarget({
      x, y,
      target: this.target(range),
      targetLabel: markdownPreviewLabel(text),
      excerpt: [...excerpt].length > 120 ? [...excerpt].slice(0, 120).join("") + "…" : excerpt,
      onClose: () => this.highlight(null),
    });
  }

  private onContextMenu = (event: MouseEvent): void => {
    if (this.isRaw()) return;
    if ((event.target as Element).closest(".ink-block-marker, .block-action-pill, .errata-block-badge, .errata-inline-editor, .errata-diff-popover")) return;
    const wrapper = (event.target as Element).closest<HTMLElement>("[data-source-block]");
    if (!wrapper || !this.article.contains(wrapper)) return;
    const index = this.data.blocks.findIndex(block => block.id === wrapper.dataset.sourceBlock);
    if (index < 0 || this.data.blocks[index].kind === "page_marker") return;
    const range = this.selection().targetRange(index);
    if (!range) return;
    event.preventDefault();
    const related = overlappingNotesForBlocks(
      this.links?.links ?? [],
      this.links?.unit_links ?? [],
      this.blockIds(range),
      this.unitIdsByBlock(),
      this.data.snapshot.content_hash,
    );
    const { clientX: x, clientY: y } = event;
    this.highlight(range);
    const label = markdownPreviewLabel(this.rangeText(range));
    const rangeCrossesMarker = this.data.blocks.slice(range.first, range.last + 1).some(b => b.kind === "page_marker");
    const visibleCount = this.visibleContentBlocks(range).length;
    showContextMenu(x, y, [
      {
        label: visibleCount === 1
          ? t("reader.copy.menuSingle")
          : t("reader.copy.menuMulti", { count: visibleCount }),
        onSelect: () => { this.highlight(null); this.copyRange(range); },
      },
      ...(this.onEditErrata
        ? [{
            label: range.first === range.last ? "校对此块 OCR 正文…" : "合并校对所选正文块…",
            disabled: rangeCrossesMarker,
            onSelect: () => { this.highlight(null); this.onEditErrata!(range); },
          }]
        : []),
      ...(this.onOpenPdf
        ? [{ label: "定位对应 PDF 原页", onSelect: () => { this.highlight(null); this.onOpenPdf!(range); } }]
        : []),
      ...(this.onContinueInCanonical
        ? [{ label: t("reader.canonical.continueFromHere"), onSelect: () => { this.highlight(null); this.onContinueInCanonical!(range); } }]
        : []),
      { label: "新建关联手稿…", onSelect: () => this.create(range, x, y) },
      {
        label: "关联到已有手稿…",
        onSelect: () => {
          this.highlight(range);
          openLinkExistingPopover({ x, y, target: this.target(range), targetLabel: label, onClose: () => this.highlight(null) });
        },
      },
      {
        label: `查看相关手稿（${related.length}）`,
        disabled: related.length === 0,
        onSelect: () => {
          this.highlight(range);
          openRelatedNotesPopover({ x, y, title: "相关手稿", notes: related, onClose: () => this.highlight(null) });
        },
      },
    ], () => this.highlight(null));
  };
}
