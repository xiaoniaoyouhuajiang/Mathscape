import katex from "katex";
import "katex/dist/katex.min.css";

const escape = (text: string) =>
  text.replace(/[&<>"']/g, (ch) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[ch]!);

/** 文本里 `$…$` 的部分用 KaTeX 排版，其余转义。 */
export function inlineMathHtml(text: string): string {
  return text
    .split(/(\$[^$]+\$)/g)
    .map((part) =>
      part.length > 2 && part.startsWith("$") && part.endsWith("$")
        ? katex.renderToString(part.slice(1, -1), { throwOnError: false, output: "html" })
        : escape(part),
    )
    .join("");
}
