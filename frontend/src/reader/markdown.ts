import MarkdownIt from "markdown-it";
import katex from "katex";
import "katex/dist/katex.min.css";

// Formula tokens are recognized before Markdown consumes backslash escapes.
export function markdownRenderer(): MarkdownIt {
  const md = new MarkdownIt({ html: false, linkify: false });
  md.block.ruler.before("fence", "mathscape_math_block", (state, startLine, endLine, silent) => {
    if (state.sCount[startLine] - state.blkIndent >= 4) return false;
    const first = state.bMarks[startLine] + state.tShift[startLine];
    const line = state.src.slice(first, state.eMarks[startLine]);
    const open = line.startsWith("$$") ? "$$" : line.startsWith("\\[") ? "\\[" : null;
    if (!open) return false;
    const close = open === "$$" ? "$$" : "\\]";
    let lastLine = startLine;
    const end = state.src.indexOf(close, first + open.length);
    if (end < 0) return false;
    while (lastLine < endLine && state.eMarks[lastLine] < end + close.length) lastLine++;
    if (lastLine >= endLine || state.src.slice(end + close.length, state.eMarks[lastLine]).trim()) return false;
    if (silent) return true;
    const token = state.push("mathscape_math", "", 0);
    token.block = true;
    token.content = state.src.slice(first + open.length, end);
    token.meta = { display: true };
    token.map = [startLine, lastLine + 1];
    state.line = lastLine + 1;
    return true;
  });
  md.inline.ruler.before("escape", "mathscape_math", (state, silent) => {
    const start = state.pos;
    const pair = [["$$", "$$"], ["\\[", "\\]"], ["\\(", "\\)"], ["$", "$"]]
      .find(([open]) => state.src.startsWith(open, start));
    if (!pair) return false;
    const [open, close] = pair;
    if (open === "$" && /\s/.test(state.src[start + 1] ?? " ")) return false;
    let end = start + open.length;
    for (;;) {
      end = state.src.indexOf(close, end);
      if (end < 0) return false;
      let slashes = 0;
      for (let p = end - 1; p >= 0 && state.src[p] === "\\"; p--) slashes++;
      if (slashes % 2 === 0) break;
      end += close.length;
    }
    if (open === "$" && (/\s/.test(state.src[end - 1]) || /\d/.test(state.src[end + 1] ?? ""))) return false;
    if (!silent) {
      const token = state.push("mathscape_math", "", 0);
      token.content = state.src.slice(start + open.length, end);
      token.meta = { display: open === "$$" || open === "\\[" };
    }
    state.pos = end + close.length;
    return true;
  });
  md.renderer.rules.mathscape_math = (tokens, index) => katex.renderToString(tokens[index].content, {
    displayMode: tokens[index].meta.display, throwOnError: false, trust: false,
    maxExpand: 1000, maxSize: 20,
  });
  // Never load original image URLs. The reader resolves them from the manifest.
  md.renderer.rules.image = (tokens, index) => {
    const token = tokens[index];
    return '<span class="reader-image" data-image-ref="' + md.utils.escapeHtml(token.attrGet("src") ?? "") +
      '">' + md.utils.escapeHtml(token.content || "图片") + "</span>";
  };
  return md;
}
