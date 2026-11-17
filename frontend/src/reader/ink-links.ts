/**
 * N2 reader integration (§6.4, §6.5): block markers, the right-click menu,
 * "手写这段" and the patch-derived document hint. Block-level only.
 */
import { showContextMenu } from "../graph/context-menu";
import { eventBus } from "../core/events";
import {
  getInkMarkdownLinks,
  markdownPreviewLabel,
  openLinkExistingPopover,
  openRelatedNotesPopover,
  overlappingNotes,
  startInkCreateFromTarget,
  type InkLinkTargetRef,
  type InkMarkdownLinks,
} from "../ink/links";
import type { AnchoredMarkdown, MarkdownSelection } from "./selection";

const MENU_TARGET_CLASS = "is-ink-menu-target";

export class ReaderInkLinks {
  readonly hint = document.createElement("p");
  private links: InkMarkdownLinks | undefined;
  private disposed = false;
  private version = 0;
  private readonly unsubscribe: () => void;

  constructor(
    private data: AnchoredMarkdown,
    private readonly article: HTMLElement,
    private readonly selection: () => MarkdownSelection,
    private readonly isRaw: () => boolean,
    private readonly onOpenPdf?: (range: { first: number; last: number }) => void,
  ) {
    this.hint.className = "ink-patch-hint";
    this.hint.hidden = true;
    this.article.addEventListener("contextmenu", this.onContextMenu);
    const offLinks = eventBus.on("ink-links-changed", () => void this.reload());
    const offNotes = eventBus.on("ink-notes-changed", () => void this.reload());
    this.unsubscribe = () => { offLinks(); offNotes(); };
    void this.reload();
  }

  setDocument(data: AnchoredMarkdown): void {
    this.data = data;
    this.decorate();
  }

  dispose(): void {
    this.disposed = true;
    this.version++;
    this.article.removeEventListener("contextmenu", this.onContextMenu);
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
    const original = this.links?.original_note_count ?? 0;
    this.hint.hidden = original === 0;
    this.hint.textContent = original ? `原版本有 ${original} 份相关手稿` : "";
    if (!this.links || this.isRaw()) return;
    const notesByBlock = new Map<string, Set<string>>();
    for (const { link } of this.links.links) {
      if (link.target.kind !== "markdown" || link.target.content_hash !== this.data.snapshot.content_hash) continue;
      for (const block of link.target.block_ids) {
        const notes = notesByBlock.get(block) ?? new Set<string>();
        notes.add(link.document_id);
        notesByBlock.set(block, notes);
      }
    }
    for (const wrapper of this.article.querySelectorAll<HTMLElement>("[data-source-block]")) {
      const blockId = wrapper.dataset.sourceBlock ?? "";
      const notes = notesByBlock.get(blockId);
      if (!notes?.size || wrapper.classList.contains("source-page-marker")) continue;
      const marker = document.createElement("button");
      marker.type = "button";
      marker.className = "ink-block-marker";
      marker.textContent = `✎ ${notes.size}`;
      marker.setAttribute("aria-label", `${notes.size} 份相关手稿`);
      marker.title = "查看相关手稿";
      marker.addEventListener("click", event => {
        event.stopPropagation();
        const bounds = marker.getBoundingClientRect();
        openRelatedNotesPopover({ x: bounds.left, y: bounds.bottom + 4, title: "相关手稿", notes: overlappingNotes(this.links?.links ?? [], [blockId]) });
      });
      marker.addEventListener("keydown", event => event.stopPropagation());
      wrapper.append(marker);
    }
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

  private rangeText(range: { first: number; last: number }): string {
    const blocks = this.data.blocks.slice(range.first, range.last + 1).filter(block => block.kind !== "page_marker");
    if (!blocks.length) return "";
    const bytes = new TextEncoder().encode(this.data.source);
    return new TextDecoder().decode(bytes.subarray(blocks[0].range.start, blocks[blocks.length - 1].range.end));
  }

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
    if ((event.target as Element).closest(".ink-block-marker")) return;
    const wrapper = (event.target as Element).closest<HTMLElement>("[data-source-block]");
    if (!wrapper || !this.article.contains(wrapper)) return;
    const index = this.data.blocks.findIndex(block => block.id === wrapper.dataset.sourceBlock);
    if (index < 0 || this.data.blocks[index].kind === "page_marker") return;
    const range = this.selection().targetRange(index);
    if (!range) return;
    event.preventDefault();
    const related = overlappingNotes(this.links?.links ?? [], this.blockIds(range));
    const { clientX: x, clientY: y } = event;
    this.highlight(range);
    const label = markdownPreviewLabel(this.rangeText(range));
    showContextMenu(x, y, [
      ...(this.onOpenPdf
        ? [{ label: "定位对应 PDF 原页", onSelect: () => { this.highlight(null); this.onOpenPdf!(range); } }]
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
