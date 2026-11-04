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
 * Caret anchors: WebKit (the Tauri WebView on macOS) cannot paint a caret at
 * an element-level position next to a `contenteditable=false` chip (e.g.
 * root offset right after a trailing chip): the selection is right but the
 * caret is drawn at the start of the line. So the caret is always parked in
 * a text node, and a chip with no text after it gets a one-character anchor
 * text node (U+FEFF, `ANCHOR`). Anchors are invisible to `value`, offsets
 * and signatures; U+FEFF is stripped from inserted text so the DOM only
 * ever holds it as an anchor.
 *
 * Offsets are measured in that expanded text, so re-chipping never moves the
 * caret relative to what the user typed. Typing keeps an id that ends at the
 * caret as text (it may still be growing); paste, drop, blur and a landscape
 * refresh chip everything. Backspace/Delete remove a chip whole, copy/cut
 * put the full ids on the clipboard, double-click turns a chip back into its
 * raw id for editing, and a click on a chip with a map target jumps there.
 *
 * `@` mentions: typing `@` (at the start, after whitespace or after CJK
 * text, not inside an email-ish word) opens a menu of current landscape
 * nodes filtered by the keywords after it (label / short id / type, see
 * `mentionSuggestions`). ↑/↓ move, Enter or Tab pick, Esc or a click
 * elsewhere dismisses (Esc keeps that `@` closed until it is retyped),
 * Shift+Enter still breaks the line. Picking replaces `@query` with the
 * node's full domain_id plus one space (none if a space already follows)
 * and re-chips, so the chip is the same one a paste makes and the caret
 * lands in text after it. The menu is a fixed overlay on `document.body`.
 */
import type { CiteResolver, CiteTarget } from "./cite";
import {
  mentionSuggestions,
  mentionTrigger,
  tokenizeComposer,
  type ComposerChip,
  type ComposerSegment,
  type MentionSuggestion,
  type MentionTrigger,
} from "./composer-chips";

const CHIP_SELECTOR = "[data-cite-id]";
const TAIL_ATTR = "data-composer-tail";
/** Zero-width caret anchor after a chip (see header). Never part of `value`. */
const ANCHOR = "\uFEFF";
const ANCHORS = /\uFEFF/g;

/** A text node's content as `value` sees it (anchors removed). */
function logicalText(raw: string | null | undefined): string {
  return (raw ?? "").replace(ANCHORS, "");
}

/** Only anchors (or nothing) in `raw`. */
function blank(raw: string | null | undefined): boolean {
  return logicalText(raw) === "";
}

/** DOM offset in `raw` of logical offset `logical` (after any anchors there). */
function domOffset(raw: string, logical: number): number {
  let seen = 0;
  for (let i = 0; i < raw.length; i += 1) {
    if (raw[i] === ANCHOR) continue;
    if (seen === logical) return i;
    seen += 1;
  }
  return raw.length;
}

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
      out += logicalText(node.nodeValue);
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
        found = count + logicalText(node.nodeValue?.slice(0, offset)).length;
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
      count += logicalText(node.nodeValue).length;
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
    if (node.nodeType === Node.TEXT_NODE) text += logicalText(node.nodeValue);
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
  /** Landscape nodes the `@` menu offers (omit to disable mentions). */
  mentions?: () => readonly CiteTarget[];
}

interface MentionState {
  trigger: MentionTrigger;
  items: MentionSuggestion[];
  active: number;
}

let menuSeq = 0;

export class ChipComposer {
  readonly element: HTMLDivElement;
  /** `@` suggestion list (fixed overlay, attached to body while open). */
  readonly menu: HTMLDivElement;
  private mention: MentionState | null = null;
  /** `@` offset the user dismissed with Esc (stays closed until retyped). */
  private dismissedAt: number | null = null;
  private composing = false;

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

    const menu = document.createElement("div");
    this.menu = menu;
    menuSeq += 1;
    menu.id = `sandbox-composer-menu-${menuSeq}`;
    menu.className = "sandbox-composer-menu";
    menu.setAttribute("role", "listbox");
    menu.setAttribute("aria-label", "引用地图节点");
    menu.hidden = true;
    // Keep focus (and the caret) in the composer while picking with the mouse.
    menu.addEventListener("mousedown", (event) => {
      event.preventDefault();
      const option = event.target instanceof Element ? event.target.closest<HTMLElement>("[data-mention-index]") : null;
      if (option) this.pickMention(Number(option.dataset.mentionIndex));
    });
    menu.addEventListener("mousemove", (event) => {
      const option = event.target instanceof Element ? event.target.closest<HTMLElement>("[data-mention-index]") : null;
      if (option && this.mention) this.setActive(Number(option.dataset.mentionIndex), false);
    });
    if (options.mentions) {
      root.setAttribute("aria-autocomplete", "list");
      root.setAttribute("aria-controls", menu.id);
      root.setAttribute("aria-expanded", "false");
    }

    root.addEventListener("keydown", (event) => this.onKeyDown(event));
    root.addEventListener("input", (event) => this.onInput(event as InputEvent));
    root.addEventListener("compositionstart", () => {
      this.composing = true;
    });
    root.addEventListener("compositionend", () => {
      this.composing = false;
      this.chipify(true);
      this.updateMention();
    });
    root.addEventListener("blur", () => {
      this.closeMention();
      this.chipify(false);
    });
    root.addEventListener("paste", (event) => this.onPaste(event));
    root.addEventListener("copy", (event) => this.onCopy(event, false));
    root.addEventListener("cut", (event) => this.onCopy(event, true));
    root.addEventListener("drop", (event) => this.onDrop(event));
    root.addEventListener("click", (event) => this.onClick(event));
    root.addEventListener("dblclick", (event) => this.onDoubleClick(event));
    if (options.mentions) {
      const doc = root.ownerDocument;
      // Caret moves (arrows, clicks) re-evaluate the `@` under the caret.
      doc.addEventListener("selectionchange", () => {
        if (doc.activeElement === root) this.updateMention();
      });
      doc.addEventListener("mousedown", (event) => {
        const target = event.target instanceof Node ? event.target : null;
        if (this.mention && target && !menu.contains(target) && !root.contains(target)) this.closeMention();
      }, true);
    }
  }

  /** The `@` menu is open. */
  get mentionOpen(): boolean {
    return this.mention !== null;
  }

  /** Composer text with every chip expanded to its full id. */
  get value(): string {
    return expandNodes(this.element);
  }

  set value(text: string) {
    this.render(tokenizeComposer(text, this.options.resolver()), null);
  }

  clear(): void {
    this.closeMention();
    this.dismissedAt = null;
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

  /**
   * Put a collapsed caret at expanded offset `offset`, always inside a text
   * node: an element-level position beside a chip is where WebKit paints the
   * caret at the start of the line (see header), so one gets an anchor.
   */
  private setCaret(offset: number): void {
    const root = this.element;
    const selection = root.ownerDocument.getSelection();
    if (!selection) return;
    let rest = offset;
    let at: { node: Node; offset: number } | null = null;
    let index: number | null = null; // root child index when no text node holds the offset
    const children = Array.from(root.childNodes);
    for (let i = 0; i < children.length && !at && index === null; i += 1) {
      const node = children[i];
      if (isTail(node)) break;
      if (node.nodeType === Node.TEXT_NODE) {
        const raw = node.nodeValue ?? "";
        const len = logicalText(raw).length;
        if (!raw) continue;
        if (rest <= len) at = { node, offset: rest === len ? raw.length : domOffset(raw, rest) };
        else rest -= len;
      } else if (isChip(node)) {
        const len = (node.dataset.citeId ?? "").length;
        if (rest === 0) index = i;
        else if (rest <= len) index = i + 1;
        else rest -= len;
      }
    }
    if (!at) {
      if (index === null) index = root.lastChild && isTail(root.lastChild) ? children.length - 1 : children.length;
      at = this.caretAnchor(children, index);
    }
    const range = root.ownerDocument.createRange();
    range.setStart(at.node, at.offset);
    range.collapse(true);
    selection.removeAllRanges();
    selection.addRange(range);
  }

  /** A text position equivalent to root child index `index` (creating an anchor if needed). */
  private caretAnchor(children: readonly ChildNode[], index: number): { node: Node; offset: number } {
    const next = children[index];
    if (next?.nodeType === Node.TEXT_NODE && next.nodeValue) return { node: next, offset: domOffset(next.nodeValue, 0) };
    const prev = children[index - 1];
    if (prev?.nodeType === Node.TEXT_NODE && prev.nodeValue) return { node: prev, offset: prev.nodeValue.length };
    const anchor = document.createTextNode(ANCHOR);
    this.element.insertBefore(anchor, next ?? null);
    return { node: anchor, offset: 1 };
  }

  private render(segments: readonly ComposerSegment[], caret: number | null): void {
    const nodes: Node[] = [];
    segments.forEach((segment, i) => {
      if (segment.kind === "text") {
        nodes.push(document.createTextNode(segment.text));
        return;
      }
      nodes.push(chipElement(segment.chip));
      // A caret right after this chip needs a text node to live in (WebKit).
      if (segments[i + 1]?.kind !== "text") nodes.push(document.createTextNode(ANCHOR));
    });
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

  private insertText(raw: string): void {
    // U+FEFF in the DOM always means a caret anchor; never let input add one.
    const text = raw.replace(ANCHORS, "");
    if (!text) return;
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
    // Inside the new text node, not after it: an element-level caret beside
    // a chip is painted at line start in WebKit.
    range.setStart(node, text.length);
    range.collapse(true);
    selection?.removeAllRanges();
    selection?.addRange(range);
  }

  private onKeyDown(event: KeyboardEvent): void {
    if (event.isComposing) return;
    if (this.mention && this.onMentionKey(event)) return;
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
      // Anchors are invisible: a caret beside only anchors is beside the node edge.
      const raw = node.nodeValue ?? "";
      if (side === "before" && blank(raw.slice(0, offset))) candidate = node.previousSibling;
      if (side === "after" && blank(raw.slice(offset))) candidate = node.nextSibling;
    } else {
      candidate = side === "before" ? node.childNodes[offset - 1] ?? null : node.childNodes[offset] ?? null;
    }
    // Skip empty (or anchor-only) text nodes left behind by edits.
    while (candidate && candidate.nodeType === Node.TEXT_NODE && blank(candidate.nodeValue)) {
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
    this.updateMention();
  }

  // -------------------------------------------------------------------------
  // @ mentions

  /** Expanded [start, end) of every chip (a mention query never spans one). */
  private chipRanges(): [number, number][] {
    return Array.from(this.element.querySelectorAll<HTMLElement>(CHIP_SELECTOR)).map((chip) => {
      const start = expandedOffset(this.element, chip.parentNode!, Array.prototype.indexOf.call(chip.parentNode!.childNodes, chip));
      return [start, start + (chip.dataset.citeId ?? "").length];
    });
  }

  /** Open, refilter or close the menu for the `@` before the caret. */
  private updateMention(): void {
    const source = this.options.mentions;
    const selection = this.element.ownerDocument.getSelection();
    if (!source || this.composing || this.element.ownerDocument.activeElement !== this.element || !selection?.isCollapsed) {
      this.closeMention();
      return;
    }
    const caret = this.caret();
    const trigger = caret === null ? null : mentionTrigger(this.value, caret, this.chipRanges());
    if (!trigger) {
      this.dismissedAt = null;
      this.closeMention();
      return;
    }
    if (this.dismissedAt !== null && this.dismissedAt !== trigger.start) this.dismissedAt = null;
    if (this.dismissedAt === trigger.start) {
      this.closeMention();
      return;
    }
    const previous = this.mention;
    if (previous && previous.trigger.start === trigger.start && previous.trigger.query === trigger.query) {
      this.positionMenu();
      return;
    }
    const items = mentionSuggestions(source(), trigger.query);
    if (!items.length) {
      this.closeMention();
      return;
    }
    this.mention = { trigger, items, active: 0 };
    this.renderMenu();
  }

  private renderMenu(): void {
    const state = this.mention;
    if (!state) return;
    const menu = this.menu;
    menu.replaceChildren(
      ...state.items.map((item, i) => {
        const option = document.createElement("div");
        option.className = "sandbox-composer-menu-item";
        option.id = `${menu.id}-${i}`;
        option.setAttribute("role", "option");
        option.dataset.mentionIndex = String(i);
        option.dataset.domainId = item.target.domainId;
        option.title = item.target.domainId;
        const label = document.createElement("span");
        label.className = "sandbox-composer-menu-label";
        label.textContent = item.label;
        const meta = document.createElement("span");
        meta.className = "sandbox-composer-menu-meta";
        meta.textContent = [item.kind, item.shortId].filter(Boolean).join(" · ");
        option.append(label, meta);
        return option;
      }),
    );
    if (!menu.isConnected) this.element.ownerDocument.body.append(menu);
    menu.hidden = false;
    this.element.setAttribute("aria-expanded", "true");
    this.setActive(state.active, true);
    this.positionMenu();
  }

  private setActive(index: number, scroll: boolean): void {
    const state = this.mention;
    if (!state) return;
    state.active = index;
    Array.from(this.menu.children).forEach((option, i) => {
      option.classList.toggle("is-active", i === index);
      option.setAttribute("aria-selected", i === index ? "true" : "false");
      if (i === index && scroll) (option as HTMLElement).scrollIntoView?.({ block: "nearest" });
    });
    this.element.setAttribute("aria-activedescendant", `${this.menu.id}-${index}`);
  }

  /** Above the caret (the composer sits at the bottom of the dock), below if no room. */
  private positionMenu(): void {
    if (this.menu.hidden) return;
    const selection = this.element.ownerDocument.getSelection();
    const range = selection && selection.rangeCount > 0 ? selection.getRangeAt(0) : null;
    let rect: DOMRect | null = range?.getClientRects()[0] ?? null;
    if (!rect || (rect.width === 0 && rect.height === 0 && rect.left === 0 && rect.top === 0)) rect = this.element.getBoundingClientRect();
    const view = this.element.ownerDocument.defaultView ?? window;
    const menu = this.menu;
    const width = menu.offsetWidth;
    const height = menu.offsetHeight;
    const left = Math.max(8, Math.min(rect.left, view.innerWidth - width - 8));
    const top = rect.top - height - 6 >= 8 ? rect.top - height - 6 : Math.min(rect.bottom + 6, view.innerHeight - height - 8);
    menu.style.left = `${Math.round(left)}px`;
    menu.style.top = `${Math.round(Math.max(8, top))}px`;
  }

  private closeMention(): void {
    if (!this.mention && this.menu.hidden) return;
    this.mention = null;
    this.menu.hidden = true;
    this.menu.replaceChildren();
    this.menu.remove();
    this.element.removeAttribute("aria-activedescendant");
    if (this.options.mentions) this.element.setAttribute("aria-expanded", "false");
  }

  /** Menu keys; true when the key was consumed. */
  private onMentionKey(event: KeyboardEvent): boolean {
    const state = this.mention!;
    const count = state.items.length;
    switch (event.key) {
      case "ArrowDown":
      case "ArrowUp":
        event.preventDefault();
        this.setActive((state.active + (event.key === "ArrowDown" ? 1 : count - 1)) % count, true);
        return true;
      case "Enter":
      case "Tab":
        if (event.shiftKey) return false;
        event.preventDefault();
        this.pickMention(state.active);
        return true;
      case "Escape":
        event.preventDefault();
        event.stopPropagation();
        this.dismissedAt = state.trigger.start;
        this.closeMention();
        return true;
      default:
        return false;
    }
  }

  /** Replace `@query` with the node's full id (+ a space) and chip it. */
  private pickMention(index: number): void {
    const state = this.mention;
    const item = state?.items[index];
    if (!state || !item) return;
    const { start, query } = state.trigger;
    const text = this.value;
    const id = item.target.domainId;
    const rest = text.slice(start + 1 + query.length);
    const space = /^\s/.test(rest) ? "" : " ";
    const caret = start + id.length + (rest.startsWith(" ") || space ? 1 : 0);
    this.closeMention();
    this.dismissedAt = null;
    this.render(tokenizeComposer(text.slice(0, start) + id + space + rest, this.options.resolver()), null);
    this.element.focus();
    this.setCaret(caret);
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
