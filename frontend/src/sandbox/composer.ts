/**
 * Chip composer: the dialogue input as a plain-text `contenteditable` whose
 * known ids render as atomic chips (composer-chips.ts decides which).
 *
 * The DOM holds only text nodes ("\n" for line breaks, `white-space:
 * pre-wrap`), chip spans (`contenteditable=false`, full id in
 * `data-cite-id`) and an optional trailing sentinel `<br>` that keeps a
 * final empty line visible. `value` reads that back with every chip expanded
 * to its full id, which is what `send` hands the agent.
 *
 * Offsets are measured in that expanded text, so re-chipping never moves the
 * caret relative to what the user typed. Typing keeps an id that ends at the
 * caret as text (it may still be growing); paste, drop, blur and a landscape
 * refresh chip everything. Backspace/Delete remove a chip whole, copy/cut
 * put the full ids on the clipboard, double-click turns a chip back into its
 * raw id for editing, and a click on a chip with a map target jumps there.
 */
import type { CiteResolver } from "./cite";
import { tokenizeComposer, type ComposerChip, type ComposerSegment } from "./composer-chips";

const CHIP_SELECTOR = "[data-cite-id]";
const TAIL_ATTR = "data-composer-tail";

function isChip(node: Node | null | undefined): node is HTMLElement {
  return node instanceof HTMLElement && node.matches(CHIP_SELECTOR);
}

function isTail(node: Node | null | undefined): boolean {
  return node instanceof HTMLElement && node.hasAttribute(TAIL_ATTR);
}

const BLOCK_TAGS = new Set(["DIV", "P", "LI"]);

/** Expanded text of a node tree: chips → full id, `<br>`/blocks → "\n". */
export function expandNodes(root: Node): string {
  let out = "";
  const walk = (node: Node, first: boolean) => {
    if (node.nodeType === Node.TEXT_NODE) {
      out += node.nodeValue ?? "";
      return;
    }
    if (!(node instanceof Element)) {
      node.childNodes.forEach((child, i) => walk(child, first && i === 0));
      return;
    }
    if (isChip(node)) {
      out += node.dataset.citeId ?? "";
      return;
    }
    if (isTail(node)) return;
    if (node.tagName === "BR") {
      out += "\n";
      return;
    }
    // Browsers may wrap lines in blocks (Enter in a foreign build, fill()).
    if (BLOCK_TAGS.has(node.tagName) && !first && !out.endsWith("\n")) out += "\n";
    node.childNodes.forEach((child, i) => walk(child, first && i === 0));
  };
  root.childNodes.forEach((child, i) => walk(child, i === 0));
  return out;
}

/** Expanded offset of a DOM position (selection boundary) under `root`. */
function expandedOffset(root: Node, container: Node, offset: number): number {
  let count = 0;
  let found: number | null = null;
  const walk = (node: Node): boolean => {
    if (node === container) {
      if (node.nodeType === Node.TEXT_NODE) {
        found = count + offset;
        return true;
      }
      for (let i = 0; i < node.childNodes.length; i += 1) {
        if (i === offset) {
          found = count;
          return true;
        }
        count += expandNodes(wrap(node.childNodes[i])).length;
      }
      found = count;
      return true;
    }
    if (node.nodeType === Node.TEXT_NODE) {
      count += node.nodeValue?.length ?? 0;
      return false;
    }
    if (isChip(node) || node.nodeName === "BR") {
      if (node.contains(container)) {
        found = count + (isChip(node) ? (node.dataset.citeId ?? "").length : 0);
        return true;
      }
      count += isChip(node) ? (node.dataset.citeId ?? "").length : isTail(node) ? 0 : 1;
      return false;
    }
    for (const child of Array.from(node.childNodes)) if (walk(child)) return true;
    return false;
  };
  if (container === root) {
    const before = Array.from(root.childNodes).slice(0, offset);
    return before.reduce((sum, child) => sum + expandNodes(wrap(child)).length, 0);
  }
  for (const child of Array.from(root.childNodes)) if (walk(child)) break;
  return found ?? count;
}

/** One node as a fragment-like root for `expandNodes`. */
function wrap(node: Node): Node {
  const fragment = document.createDocumentFragment();
  fragment.append(node.cloneNode(true));
  return fragment;
}

/** Comparable shape of the composer DOM (to skip no-op rebuilds). */
function domSignature(root: HTMLElement): string | null {
  const parts: string[] = [];
  let text = "";
  for (const node of Array.from(root.childNodes)) {
    if (node.nodeType === Node.TEXT_NODE) text += node.nodeValue ?? "";
    else if (isChip(node)) {
      if (text) parts.push(`t:${text}`);
      text = "";
      parts.push(`c:${node.dataset.citeId}|${node.dataset.chipClass}|${node.dataset.chipText}|${node.dataset.citeNode ?? ""}`);
    } else if (isTail(node) && node === root.lastChild) {
      if (text) parts.push(`t:${text}`);
      text = "";
      parts.push("tail");
    } else return null; // foreign structure (br/div): normalize
  }
  if (text) parts.push(`t:${text}`);
  return parts.join("\u0000");
}

function segmentSignature(segments: readonly ComposerSegment[]): string {
  return segments
    .map((segment) =>
      segment.kind === "text"
        ? `t:${segment.text}`
        : `c:${segment.chip.id}|${segment.chip.statusClass}|${segment.chip.text}|${segment.chip.jumpNode ?? ""}`,
    )
    .concat(needsTail(segments) ? ["tail"] : [])
    .join("\u0000");
}

/** A final chip or line break needs the sentinel `<br>` after it. */
function needsTail(segments: readonly ComposerSegment[]): boolean {
  const last = segments[segments.length - 1];
  return !!last && (last.kind === "chip" || last.text.endsWith("\n"));
}

export function chipElement(chip: ComposerChip): HTMLSpanElement {
  const span = document.createElement("span");
  span.className = `sandbox-cite sandbox-composer-chip ${chip.statusClass}`;
  span.contentEditable = "false";
  span.dataset.citeId = chip.id;
  span.dataset.chipClass = chip.statusClass;
  span.dataset.chipText = chip.text;
  if (chip.jumpNode) span.dataset.citeNode = chip.jumpNode;
  span.title = `${chip.title}\n${chip.id}`;
  span.textContent = chip.text;
  if (chip.status === "ambiguous") {
    const flag = document.createElement("span");
    flag.className = "sandbox-cite-flag";
    flag.textContent = "歧义";
    span.append(flag);
  }
  return span;
}

export interface ChipComposerOptions {
  placeholder: string;
  ariaLabel: string;
  /** Resolver for the current landscape + session tool context. */
  resolver: () => CiteResolver | undefined;
  /** Click on a chip that has a map target. */
  onJump?: (nodeId: string) => void;
  /** Enter without Shift (not while composing). */
  onSubmit: () => void;
}

export class ChipComposer {
  readonly element: HTMLDivElement;

  constructor(private readonly options: ChipComposerOptions) {
    const root = document.createElement("div");
    this.element = root;
    root.className = "sandbox-input sandbox-dock-input sandbox-composer";
    root.contentEditable = "true";
    root.spellcheck = false;
    root.setAttribute("role", "textbox");
    root.setAttribute("aria-multiline", "true");
    root.setAttribute("aria-label", options.ariaLabel);
    root.dataset.placeholder = options.placeholder;
    root.classList.add("is-empty");

    root.addEventListener("keydown", (event) => this.onKeyDown(event));
    root.addEventListener("input", (event) => this.onInput(event as InputEvent));
    root.addEventListener("compositionend", () => this.chipify(true));
    root.addEventListener("blur", () => this.chipify(false));
    root.addEventListener("paste", (event) => this.onPaste(event));
    root.addEventListener("copy", (event) => this.onCopy(event, false));
    root.addEventListener("cut", (event) => this.onCopy(event, true));
    root.addEventListener("drop", (event) => this.onDrop(event));
    root.addEventListener("click", (event) => this.onClick(event));
    root.addEventListener("dblclick", (event) => this.onDoubleClick(event));
  }

  /** Composer text with every chip expanded to its full id. */
  get value(): string {
    return expandNodes(this.element);
  }

  set value(text: string) {
    this.render(tokenizeComposer(text, this.options.resolver()), null);
  }

  clear(): void {
    this.element.replaceChildren();
    this.syncEmpty();
  }

  focus(): void {
    this.element.focus();
  }

  /** The landscape or tool context changed: re-resolve every id. */
  refresh(): void {
    this.chipify(false);
  }

  // -------------------------------------------------------------------------

  private caret(): number | null {
    const selection = this.element.ownerDocument.getSelection();
    if (!selection || selection.rangeCount === 0) return null;
    const range = selection.getRangeAt(0);
    if (!this.element.contains(range.startContainer)) return null;
    return expandedOffset(this.element, range.startContainer, range.startOffset);
  }

  private setCaret(offset: number): void {
    const root = this.element;
    const selection = root.ownerDocument.getSelection();
    if (!selection) return;
    const range = root.ownerDocument.createRange();
    let rest = offset;
    let placed = false;
    const children = Array.from(root.childNodes);
    for (let i = 0; i < children.length && !placed; i += 1) {
      const node = children[i];
      if (isTail(node)) break;
      if (node.nodeType === Node.TEXT_NODE) {
        const len = node.nodeValue?.length ?? 0;
        if (rest <= len) {
          range.setStart(node, rest);
          placed = true;
        } else rest -= len;
      } else if (isChip(node)) {
        const len = (node.dataset.citeId ?? "").length;
        if (rest === 0) {
          range.setStart(root, i);
          placed = true;
        } else if (rest <= len) {
          range.setStart(root, i + 1);
          placed = true;
        } else rest -= len;
      }
    }
    if (!placed) {
      const end = root.lastChild && isTail(root.lastChild) ? root.childNodes.length - 1 : root.childNodes.length;
      range.setStart(root, end);
    }
    range.collapse(true);
    selection.removeAllRanges();
    selection.addRange(range);
  }

  private render(segments: readonly ComposerSegment[], caret: number | null): void {
    const nodes: Node[] = segments.map((segment) =>
      segment.kind === "text" ? document.createTextNode(segment.text) : chipElement(segment.chip),
    );
    if (needsTail(segments)) {
      // Keeps a trailing empty line visible and gives the caret a place after a final chip.
      const tail = document.createElement("br");
      tail.setAttribute(TAIL_ATTR, "");
      nodes.push(tail);
    }
    this.element.replaceChildren(...nodes);
    if (caret !== null) this.setCaret(caret);
    this.syncEmpty();
  }

  /** Re-tokenize the expanded text; rebuild the DOM only if it changes. */
  private chipify(typing: boolean): void {
    const text = this.value;
    const caret = this.element.ownerDocument.activeElement === this.element ? this.caret() : null;
    const segments = tokenizeComposer(
      text,
      this.options.resolver(),
      typing && caret !== null ? { openAt: caret, chipStarts: this.chipStarts() } : {},
    );
    if (domSignature(this.element) !== segmentSignature(segments)) this.render(segments, caret);
    else this.syncEmpty();
  }

  /** Expanded start offsets of the chips currently in the DOM. */
  private chipStarts(): Set<number> {
    const starts = new Set<number>();
    for (const chip of Array.from(this.element.querySelectorAll<HTMLElement>(CHIP_SELECTOR))) {
      starts.add(expandedOffset(this.element, chip.parentNode!, Array.prototype.indexOf.call(chip.parentNode!.childNodes, chip)));
    }
    return starts;
  }

  private syncEmpty(): void {
    this.element.classList.toggle("is-empty", this.value === "");
  }

  private insertText(text: string): void {
    const selection = this.element.ownerDocument.getSelection();
    let range = selection && selection.rangeCount > 0 ? selection.getRangeAt(0) : null;
    if (!range || !this.element.contains(range.startContainer)) {
      range = this.element.ownerDocument.createRange();
      range.selectNodeContents(this.element);
      range.collapse(false);
    }
    range.deleteContents();
    const node = document.createTextNode(text);
    range.insertNode(node);
    range.setStartAfter(node);
    range.collapse(true);
    selection?.removeAllRanges();
    selection?.addRange(range);
  }

  private onKeyDown(event: KeyboardEvent): void {
    if (event.isComposing) return;
    if (event.key === "Enter") {
      event.preventDefault();
      if (event.shiftKey) {
        this.insertText("\n");
        this.chipify(true);
      } else {
        this.chipify(false);
        this.options.onSubmit();
      }
      return;
    }
    if (event.key === "Backspace" || event.key === "Delete") {
      const chip = this.adjacentChip(event.key === "Backspace" ? "before" : "after");
      if (chip) {
        event.preventDefault();
        const caret = this.caret() ?? 0;
        const at = event.key === "Backspace" ? caret - (chip.dataset.citeId ?? "").length : caret;
        chip.remove();
        this.chipify(false);
        this.setCaret(Math.max(0, at));
      }
    }
  }

  /** The chip right before/after a collapsed caret, if any. */
  private adjacentChip(side: "before" | "after"): HTMLElement | null {
    const selection = this.element.ownerDocument.getSelection();
    if (!selection || selection.rangeCount === 0 || !selection.isCollapsed) return null;
    const { startContainer: node, startOffset: offset } = selection.getRangeAt(0);
    if (!this.element.contains(node)) return null;
    let candidate: Node | null = null;
    if (node.nodeType === Node.TEXT_NODE) {
      const len = node.nodeValue?.length ?? 0;
      if (side === "before" && offset === 0) candidate = node.previousSibling;
      if (side === "after" && offset === len) candidate = node.nextSibling;
    } else {
      candidate = side === "before" ? node.childNodes[offset - 1] ?? null : node.childNodes[offset] ?? null;
    }
    // Skip empty text nodes left behind by edits.
    while (candidate && candidate.nodeType === Node.TEXT_NODE && !candidate.nodeValue) {
      candidate = side === "before" ? candidate.previousSibling : candidate.nextSibling;
    }
    if (candidate && !isChip(candidate) && candidate instanceof Element) {
      const inner = candidate.closest(CHIP_SELECTOR);
      return inner instanceof HTMLElement ? inner : null;
    }
    return isChip(candidate) ? candidate : null;
  }

  private onInput(event: InputEvent): void {
    if (event.isComposing) return;
    // A single typed character may still be extending an id at the caret.
    const typing = event.inputType === "insertText" && (event.data?.length ?? 0) <= 1;
    this.chipify(typing || event.inputType.startsWith("delete"));
  }

  private onPaste(event: ClipboardEvent): void {
    event.preventDefault();
    const text = event.clipboardData?.getData("text/plain") ?? "";
    if (!text) return;
    this.insertText(text.replace(/\r\n?/g, "\n"));
    this.chipify(false);
  }

  private onDrop(event: DragEvent): void {
    const text = event.dataTransfer?.getData("text/plain");
    event.preventDefault();
    if (!text) return;
    const doc = this.element.ownerDocument as Document & { caretRangeFromPoint?: (x: number, y: number) => Range | null };
    const range = doc.caretRangeFromPoint?.(event.clientX, event.clientY);
    if (range && this.element.contains(range.startContainer)) {
      const selection = doc.getSelection();
      selection?.removeAllRanges();
      selection?.addRange(range);
    }
    this.insertText(text);
    this.chipify(false);
  }

  private onCopy(event: ClipboardEvent, cut: boolean): void {
    const selection = this.element.ownerDocument.getSelection();
    if (!selection || selection.rangeCount === 0 || selection.isCollapsed || !event.clipboardData) return;
    const range = selection.getRangeAt(0);
    event.preventDefault();
    event.clipboardData.setData("text/plain", expandNodes(range.cloneContents()));
    if (cut) {
      range.deleteContents();
      this.chipify(false);
    }
  }

  private onClick(event: MouseEvent): void {
    const target = event.target instanceof Element ? event.target.closest<HTMLElement>("[data-cite-node]") : null;
    if (!target || !this.element.contains(target)) return;
    event.preventDefault();
    const nodeId = target.dataset.citeNode;
    if (nodeId) this.options.onJump?.(nodeId);
  }

  /** Double-click a chip to edit its raw id (it re-chips once you leave). */
  private onDoubleClick(event: MouseEvent): void {
    const chip = event.target instanceof Element ? event.target.closest<HTMLElement>(CHIP_SELECTOR) : null;
    if (!chip || !this.element.contains(chip)) return;
    event.preventDefault();
    const text = document.createTextNode(chip.dataset.citeId ?? "");
    chip.replaceWith(text);
    const range = this.element.ownerDocument.createRange();
    range.selectNodeContents(text);
    const selection = this.element.ownerDocument.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);
    this.syncEmpty();
  }
}
