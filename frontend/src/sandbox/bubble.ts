/**
 * Markdown + KaTeX bubbles for the sandbox dialogue (A-PR3).
 *
 * Reuses the reader's `markdownRenderer()` (html:false, `$…$` / `$$…$$` /
 * `\(…\)` / `\[…\]` math through KaTeX, throwOnError:false) and hooks cite
 * chips into text and code tokens. Links never navigate the webview: they
 * render as plain text with the URL on hover. Raw HTML in a message stays
 * escaped text.
 */
import type MarkdownIt from "markdown-it";
import { markdownRenderer } from "../reader/markdown";
import { escapeHtml, withCiteChips, type CiteResolver } from "./cite";

let renderer: MarkdownIt | null = null;

function bubbleRenderer(): MarkdownIt {
  if (renderer) return renderer;
  const md = withCiteChips(markdownRenderer());
  md.renderer.rules.link_open = (tokens, idx) =>
    `<span class="sandbox-link" title="${escapeHtml(tokens[idx].attrGet("href") ?? "")}">`;
  md.renderer.rules.link_close = () => "</span>";
  renderer = md;
  return md;
}

/** Message text → safe HTML for `.sandbox-line-text.markdown-body`. */
export function renderBubbleHtml(text: string, resolve?: CiteResolver): string {
  return bubbleRenderer().render(text, { resolve });
}
