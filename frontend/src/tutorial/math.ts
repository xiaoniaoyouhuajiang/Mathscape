/**
 * 教程卡片的文字渲染：Markdown + TeX（`$…$`），与阅读器同一套
 * markdown-it + KaTeX 规则（不允许原始 HTML）。链接只允许 https，
 * 渲染成带 `data-external` 的 `<a>`，由播放器交给系统浏览器打开。
 */
import type MarkdownIt from "markdown-it";
import { markdownRenderer } from "../reader/markdown";

let renderer: MarkdownIt | null = null;

function tutorialRenderer(): MarkdownIt {
  if (renderer) return renderer;
  const md = markdownRenderer();
  md.validateLink = (url) => /^https:\/\//i.test(url.trim());
  md.renderer.rules.link_open = (tokens, idx) => {
    const href = md.utils.escapeHtml(tokens[idx].attrGet("href") ?? "");
    return `<a class="tutorial-link" href="${href}" title="${href}" data-external="true" target="_blank" rel="noopener noreferrer">`;
  };
  renderer = md;
  return md;
}

/** 一行文字（不包段落）。 */
export function inlineMathHtml(text: string): string {
  return tutorialRenderer().renderInline(text);
}

/** 一段原文（可含段落）。 */
export function markdownHtml(text: string): string {
  return tutorialRenderer().render(text);
}
