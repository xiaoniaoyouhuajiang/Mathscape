import { markdownPatchSource, markdownRenderer, removePatchMarkers } from "../reader/markdown";
import type { AnchoredMarkdown, MarkdownBlock } from "../reader/selection";
import type { DiscoveryAnchorView, SourceReviewAnchor } from "./types";

/** The single location contract of the workspace: byte range plus backend block ids. */
export interface ReviewAnchor {
  start: number;
  end: number;
  block_ids: string[];
  excerpt: string;
  purpose: string;
}

export function reviewAnchor(anchor: DiscoveryAnchorView | SourceReviewAnchor, purpose?: string): ReviewAnchor {
  const range = "range" in anchor ? anchor.range : anchor;
  return {
    start: range.start,
    end: range.end,
    block_ids: anchor.block_ids,
    excerpt: anchor.excerpt,
    purpose: purpose ?? anchor.purpose ?? "unit",
  };
}

export type AnchorCheck =
  | { ok: true; blocks: MarkdownBlock[]; before: string; selected: string; after: string }
  | { ok: false; reason: string };

/**
 * Validates an anchor against the exact snapshot. Nothing here searches for
 * the excerpt text: an anchor that does not fit is reported, never moved.
 */
export function checkAnchor(doc: AnchoredMarkdown, bytes: Uint8Array, anchor: ReviewAnchor): AnchorCheck {
  if (!Number.isSafeInteger(anchor.start) || !Number.isSafeInteger(anchor.end)
    || anchor.start < 0 || anchor.end <= anchor.start || anchor.end > bytes.length) {
    return { ok: false, reason: `字节范围 [${anchor.start}, ${anchor.end}) 超出当前 Markdown（${bytes.length} 字节）。` };
  }
  const blocks = doc.blocks.filter(block => block.range.start < anchor.end && block.range.end > anchor.start);
  if (blocks.length === 0) return { ok: false, reason: "字节范围不与任何原文块相交。" };
  const intersecting = new Set(blocks.map(block => block.id));
  const foreign = anchor.block_ids.filter(id => !intersecting.has(id));
  if (foreign.length > 0) return { ok: false, reason: `锚点声明的原文块 ${foreign.join("、")} 与字节范围不相交。` };
  const decoder = new TextDecoder("utf-8", { fatal: true });
  const contextStart = Math.min(blocks[0].range.start, anchor.start);
  const contextEnd = Math.max(blocks[blocks.length - 1].range.end, anchor.end);
  try {
    return {
      ok: true,
      blocks,
      before: decoder.decode(bytes.subarray(contextStart, anchor.start)),
      selected: decoder.decode(bytes.subarray(anchor.start, anchor.end)),
      after: decoder.decode(bytes.subarray(anchor.end, contextEnd)),
    };
  } catch {
    return { ok: false, reason: "字节范围没有落在 UTF-8 字符边界上。" };
  }
}

// Block syntax that a marker must not precede, or the line stops being a
// heading, list item, quote or display formula.
const LEADING_SYNTAX = /^(?:[ \t]*(?:#{1,6}[ \t]+|[-*+][ \t]+|\d{1,9}[.)][ \t]+|>[ \t]?|\$\$|\\\[))*/;

/** Wraps [from, to) line by line so a highlight never spans block boundaries. */
function decorate(text: string, from: number, to: number): string {
  let out = text.slice(0, from);
  let pos = from;
  while (pos < to) {
    const newline = text.indexOf("\n", pos);
    const lineEnd = newline < 0 || newline >= to ? to : newline;
    let segmentStart = pos;
    if (pos === 0 || text[pos - 1] === "\n") {
      segmentStart += LEADING_SYNTAX.exec(text.slice(pos, lineEnd))![0].length;
    }
    out += text.slice(pos, segmentStart);
    const segment = text.slice(segmentStart, lineEnd);
    out += segment.trim() ? markdownPatchSource("", segment, "", "anchor") : segment;
    if (lineEnd < to) { out += "\n"; pos = lineEnd + 1; } else pos = to;
  }
  return out + text.slice(to);
}

function stripLeakedMarkers(root: HTMLElement): boolean {
  let leaked = false;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const text = node.textContent ?? "";
    if (!text.includes("\uE000")) continue;
    node.textContent = removePatchMarkers(text);
    leaked = true;
  }
  return leaked;
}

/**
 * Renders every block the anchor touches with Markdown and KaTeX, marking
 * the anchored bytes. Where inline marking would change the block's syntax
 * (tables, code, markers swallowed by other syntax) the whole block is
 * outlined instead of guessing a narrower position.
 */
export function renderAnchorFragment(doc: AnchoredMarkdown, bytes: Uint8Array, anchor: ReviewAnchor, blocks: MarkdownBlock[]): HTMLElement {
  const md = markdownRenderer();
  const decoder = new TextDecoder("utf-8", { fatal: true });
  const fragment = document.createElement("div");
  fragment.className = "review-fragment markdown-body";
  for (const block of blocks) {
    const wrapper = document.createElement("div");
    wrapper.className = "review-fragment-block";
    wrapper.dataset.blockId = block.id;
    const index = doc.blocks.indexOf(block);
    wrapper.title = `第 ${index + 1} 块 · ${block.kind} · UTF-8 [${block.range.start}, ${block.range.end})`;
    const text = decoder.decode(bytes.subarray(block.range.start, block.range.end));
    const from = decoder.decode(bytes.subarray(block.range.start, Math.max(anchor.start, block.range.start))).length;
    const to = text.length - decoder.decode(bytes.subarray(Math.min(anchor.end, block.range.end), block.range.end)).length;
    const covered = text.slice(0, from).trim() === "" && text.slice(to).trim() === "";
    const inline = !covered && block.kind !== "table" && block.kind !== "code";
    wrapper.innerHTML = md.render(inline ? decorate(text, from, to) : text);
    if (!wrapper.innerHTML.trim()) wrapper.textContent = text;
    const leaked = stripLeakedMarkers(wrapper);
    if (!inline || leaked || wrapper.querySelector(".review-anchor-highlight") === null) {
      wrapper.classList.add("is-anchor-block");
    }
    wrapper.querySelectorAll("a").forEach(link => link.removeAttribute("href"));
    fragment.append(wrapper);
  }
  return fragment;
}

export function rawAnchorPreview(anchor: ReviewAnchor, check: Extract<AnchorCheck, { ok: true }>): HTMLElement {
  const pre = document.createElement("pre");
  pre.className = "review-raw-source";
  const mark = document.createElement("mark");
  mark.textContent = check.selected;
  pre.append(document.createTextNode(check.before), mark, document.createTextNode(check.after));
  const wrapper = document.createElement("div");
  wrapper.className = "review-raw";
  const caption = document.createElement("span");
  caption.className = "review-raw-caption";
  caption.textContent = `UTF-8 [${anchor.start}, ${anchor.end}) · 块 ${check.blocks.map(block => block.id).join("、")}`;
  wrapper.append(caption, pre);
  return wrapper;
}
