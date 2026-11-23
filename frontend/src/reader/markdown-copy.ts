/**
 * Markdown reader copy helpers:
 * - Cleanly joins selected physical/canonical blocks (preserving errata & LaTeX);
 * - Restores `$...$` / `$$...$$` TeX source from KaTeX DOM on partial text drag-selections;
 * - Keeps the floating `.block-action-pill` anchored inside the scroll viewport on multi-block selections.
 */
import { scrollParentOf } from "./reading-position";

const CHROME_SELECTOR = [
  ".block-action-pill",
  ".ink-block-marker",
  ".unit-map-badge-row",
  ".errata-block-badge",
  ".page-marker-jump",
  ".source-page-marker",
  "button",
].join(", ");

const BLOCK_TAGS = new Set([
  "P", "DIV", "SECTION", "ARTICLE", "BLOCKQUOTE", "PRE",
  "UL", "OL", "TABLE", "H1", "H2", "H3", "H4", "H5", "H6",
]);

let internalFallbackCopying = false;
let lastCopiedText = "";

/** Last Markdown string copied through reader helpers (also useful in headless test harnesses). */
export function getLastReaderCopiedText(): string {
  return lastCopiedText;
}

export function joinBlocksMarkdown(texts: readonly string[]): string {
  return texts
    .map((text) => text.replace(/\r?\n+$/, ""))
    .filter((text) => text.trim().length > 0)
    .join("\n\n");
}

function isEditableElement(node: EventTarget | Element | null): boolean {
  if (!(node instanceof Element)) return false;
  if (node.closest(".errata-inline-editor, .errata-diff-popover, .markdown-patch-review")) return true;
  const html = node as HTMLElement;
  if (html.isContentEditable) return true;
  return ["INPUT", "TEXTAREA", "SELECT"].includes(html.tagName);
}

/** Converts a cloned selection DOM fragment into clean text with `$...$` / `$$...$$` LaTeX restored. */
export function fragmentToTexMarkdown(fragment: DocumentFragment): string {
  fragment.querySelectorAll(CHROME_SELECTOR).forEach((el) => el.remove());

  fragment.querySelectorAll(".katex-display").forEach((displayEl) => {
    const ann = displayEl.querySelector('annotation[encoding="application/x-tex"]');
    if (!ann) return;
    const tex = (ann.textContent ?? "").trim();
    const formatted = tex.includes("\n") ? `\n\n$$\n${tex}\n$$\n\n` : `\n\n$$${tex}$$\n\n`;
    displayEl.replaceWith(document.createTextNode(formatted));
  });

  fragment.querySelectorAll(".katex").forEach((inlineEl) => {
    const ann = inlineEl.querySelector('annotation[encoding="application/x-tex"]');
    if (!ann) return;
    const tex = (ann.textContent ?? "").trim();
    inlineEl.replaceWith(document.createTextNode(`$${tex}$`));
  });

  // If a partial drag cut halfway into a formula so <annotation> was outside the range,
  // drop the hidden MathML twin so characters are not duplicated.
  fragment.querySelectorAll(".katex-mathml").forEach((el) => el.remove());

  const parts: string[] = [];
  const walk = (node: Node): void => {
    if (node.nodeType === Node.TEXT_NODE) {
      parts.push(node.nodeValue ?? "");
      return;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return;
    const el = node as HTMLElement;
    const tag = el.tagName;
    if (tag === "BR") {
      parts.push("\n");
      return;
    }
    const isBlock = BLOCK_TAGS.has(tag) || el.classList.contains("source-block");
    if (isBlock && parts.length > 0) parts.push("\n\n");
    else if ((tag === "LI" || tag === "TR") && parts.length > 0) parts.push("\n");

    for (let child = el.firstChild; child; child = child.nextSibling) {
      walk(child);
      if ((tag === "TR") && child.nextSibling && child.nodeType === Node.ELEMENT_NODE) {
        parts.push("\t");
      }
    }

    if (isBlock) parts.push("\n\n");
  };

  for (let child = fragment.firstChild; child; child = child.nextSibling) {
    walk(child);
  }

  return parts
    .join("")
    .replace(/[ \t]+\n/g, "\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

export function selectionRangeToMarkdown(selection: Selection | null, article: HTMLElement): string | null {
  if (!selection || selection.isCollapsed || selection.rangeCount === 0) return null;
  const range = selection.getRangeAt(0);
  const ancestor = range.commonAncestorContainer;
  const ancestorEl = ancestor.nodeType === Node.ELEMENT_NODE ? (ancestor as Element) : ancestor.parentElement;
  if (!ancestorEl || !article.contains(ancestorEl)) return null;
  if (isEditableElement(ancestorEl)) return null;
  const text = fragmentToTexMarkdown(range.cloneContents());
  return text.length > 0 ? text : null;
}

export async function copyTextToClipboard(text: string): Promise<boolean> {
  if (!text) return false;
  lastCopiedText = text;
  try {
    if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // Fall back to execCommand below when WebView clipboard permissions block async API.
  }
  if (typeof document === "undefined") return false;
  const prevActive = document.activeElement as HTMLElement | null;
  const textarea = document.createElement("textarea");
  textarea.value = text;
  textarea.setAttribute("readonly", "");
  textarea.style.cssText = "position:fixed;left:-9999px;top:0;opacity:0;pointer-events:none;";
  document.body.append(textarea);
  internalFallbackCopying = true;
  try {
    textarea.focus({ preventScroll: true });
    textarea.select();
    return document.execCommand("copy");
  } catch {
    return false;
  } finally {
    internalFallbackCopying = false;
    textarea.remove();
    prevActive?.focus?.({ preventScroll: true });
  }
}

export interface ActionAnchorPlacement {
  wrapper: HTMLElement;
  pinBottom: boolean;
}

/**
 * Picks which selected block wrapper should host `.block-action-pill` so the
 * toolbar remains visible even when the first selected block scrolls off-screen.
 */
export function pickVisibleActionAnchor(wrappers: readonly HTMLElement[]): ActionAnchorPlacement | null {
  if (!wrappers.length) return null;
  if (wrappers.length === 1) return { wrapper: wrappers[0], pinBottom: false };
  const scroller = scrollParentOf(wrappers[0], document.documentElement);
  const view = scroller.getBoundingClientRect();
  if (view.height <= 0) return { wrapper: wrappers[0], pinBottom: false };

  for (const wrapper of wrappers) {
    const rect = wrapper.getBoundingClientRect();
    if (rect.height <= 0) continue;
    if (rect.top >= view.top + 12 && rect.top <= view.bottom - 20) {
      return { wrapper, pinBottom: false };
    }
  }
  for (let i = wrappers.length - 1; i >= 0; i--) {
    const rect = wrappers[i].getBoundingClientRect();
    if (rect.height > 0 && rect.bottom >= view.top + 28 && rect.top < view.bottom - 12) {
      return { wrapper: wrappers[i], pinBottom: true };
    }
  }
  return { wrapper: wrappers[0], pinBottom: false };
}

export interface BindReaderCopyOptions {
  article: HTMLElement;
  isDisabled?: () => boolean;
  getSelectedBlocksCopy: () => { text: string; count: number } | null;
  onCopied: (count: number) => void;
  onCopyFailed?: () => void;
}

/**
 * Binds `⌘/Ctrl+C` and native `copy` events for a Markdown reader article:
 * - Non-empty text drag inside `article` copies text with KaTeX `$...$` restored;
 * - Collapsed text selection with selected reader blocks copies clean block Markdown.
 */
export function bindReaderCopyShortcuts(options: BindReaderCopyOptions): () => void {
  const { article } = options;
  const activeAndEligible = (target: EventTarget | null): boolean => {
    if (internalFallbackCopying) return false;
    if (options.isDisabled?.()) return false;
    if (!article.isConnected || article.getClientRects().length === 0) return false;
    if (isEditableElement(target) || isEditableElement(document.activeElement)) return false;
    if (document.activeElement?.closest?.("#sandbox-dock, dialog[open]")) return false;
    return true;
  };

  const onCopy = (event: ClipboardEvent): void => {
    if (!activeAndEligible(event.target)) return;
    const sel = window.getSelection();
    if (sel && !sel.isCollapsed && sel.rangeCount > 0) {
      const texMd = selectionRangeToMarkdown(sel, article);
      if (texMd && event.clipboardData) {
        lastCopiedText = texMd;
        event.clipboardData.setData("text/plain", texMd);
        event.preventDefault();
      }
      return;
    }
    const blockCopy = options.getSelectedBlocksCopy();
    if (!blockCopy || !blockCopy.text) return;
    if (event.clipboardData) {
      lastCopiedText = blockCopy.text;
      event.clipboardData.setData("text/plain", blockCopy.text);
      event.preventDefault();
      options.onCopied(blockCopy.count);
    }
  };

  const onKeyDown = (event: KeyboardEvent): void => {
    if (event.defaultPrevented) return;
    if (!(event.metaKey || event.ctrlKey) || event.shiftKey || event.altKey) return;
    if (event.key !== "c" && event.key !== "C") return;
    if (!activeAndEligible(event.target)) return;
    const sel = window.getSelection();
    if (sel && !sel.isCollapsed && sel.rangeCount > 0) {
      // Let native `copy` fire so `onCopy` handles partial text drag synchronously.
      return;
    }
    const blockCopy = options.getSelectedBlocksCopy();
    if (!blockCopy || !blockCopy.text) return;
    event.preventDefault();
    void copyTextToClipboard(blockCopy.text).then((ok) => {
      if (ok) options.onCopied(blockCopy.count);
      else options.onCopyFailed?.();
    });
  };

  document.addEventListener("copy", onCopy);
  document.addEventListener("keydown", onKeyDown);
  return () => {
    document.removeEventListener("copy", onCopy);
    document.removeEventListener("keydown", onKeyDown);
  };
}
