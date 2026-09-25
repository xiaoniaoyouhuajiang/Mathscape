import MarkdownIt from "markdown-it";
import katex from "katex";
import "katex/dist/katex.min.css";

export type MarkdownPatchHighlight = "old" | "new";

type PatchMarkerKind = "start" | "end";

const PATCH_MARKERS: Record<MarkdownPatchHighlight, Record<PatchMarkerKind, string>> = {
  old: {
    start: "\uE000mathscape-patch-old-start\uE001",
    end: "\uE000mathscape-patch-old-end\uE001",
  },
  new: {
    start: "\uE000mathscape-patch-new-start\uE001",
    end: "\uE000mathscape-patch-new-end\uE001",
  },
};

function markerAt(source: string, position: number): { variant: MarkdownPatchHighlight; kind: PatchMarkerKind; length: number } | null {
  for (const variant of ["old", "new"] as const) {
    for (const kind of ["start", "end"] as const) {
      const value = PATCH_MARKERS[variant][kind];
      if (source.startsWith(value, position)) return { variant, kind, length: value.length };
    }
  }
  return null;
}

function patchVariant(source: string): MarkdownPatchHighlight | null {
  for (const variant of ["old", "new"] as const) {
    if (source.includes(PATCH_MARKERS[variant].start) || source.includes(PATCH_MARKERS[variant].end)) return variant;
  }
  return null;
}

function removePatchMarkers(source: string): string {
  let result = source;
  for (const variant of ["old", "new"] as const) {
    result = result
      .split(PATCH_MARKERS[variant].start).join("")
      .split(PATCH_MARKERS[variant].end).join("");
  }
  return result;
}

/**
 * Put renderer-only markers inside a changed fragment. For a complete math
 * expression the markers go inside the delimiters, so Markdown and KaTeX
 * still parse the expression normally while the renderer can highlight it.
 */
export function markdownPatchSource(
  before: string,
  changed: string,
  after: string,
  variant: MarkdownPatchHighlight,
): string {
  const markers = PATCH_MARKERS[variant];
  const delimiters = [
    ["$$", "$$"],
    ["\\[", "\\]"],
    ["\\(", "\\)"],
    ["$", "$"],
  ] as const;
  let decorated: string | null = null;
  for (const [open, close] of delimiters) {
    const openAt = changed.indexOf(open);
    const closeAt = changed.lastIndexOf(close);
    if (openAt < 0 || closeAt <= openAt + open.length || changed.slice(0, openAt).trim() !== "" || changed.slice(closeAt + close.length).trim() !== "") continue;
    decorated = changed.slice(0, openAt + open.length)
      + markers.start
      + changed.slice(openAt + open.length, closeAt)
      + markers.end
      + changed.slice(closeAt);
    break;
  }
  return before + (decorated ?? markers.start + changed + markers.end) + after;
}

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
  // This rule must precede MarkdownIt's text rule; otherwise a text token
  // would consume the private-use marker before we can turn it into a span.
  md.inline.ruler.before("text", "mathscape_patch_marker", (state, silent) => {
    const marker = markerAt(state.src, state.pos);
    if (!marker) return false;
    if (!silent) {
      const token = state.push("mathscape_patch_marker", "", 0);
      token.meta = { variant: marker.variant, kind: marker.kind };
    }
    state.pos += marker.length;
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
  md.renderer.rules.mathscape_patch_marker = (tokens, index) => {
    const marker = tokens[index].meta as { variant: MarkdownPatchHighlight; kind: PatchMarkerKind };
    const className = marker.variant === "old" ? "markdown-patch-highlight-old" : "markdown-patch-highlight-new";
    return marker.kind === "start" ? `<mark class="${className}">` : "</mark>";
  };
  // Remove renderer markers before KaTeX sees the source. This keeps the
  // source correction out of the formula while retaining a visible wrapper.
  md.renderer.rules.mathscape_math = (tokens, index, options, env, self) => {
    const token = tokens[index];
    const variant = patchVariant(token.content);
    const content = removePatchMarkers(token.content);
    const html = katex.renderToString(content, {
      displayMode: token.meta.display, throwOnError: false, trust: false,
      maxExpand: 1000, maxSize: 20,
    });
    if (!variant) return html;
    const className = variant === "old" ? "markdown-patch-highlight-old" : "markdown-patch-highlight-new";
    const displayClass = token.meta.display ? " markdown-patch-highlight-formula-display" : "";
    return `<span class="${className} markdown-patch-highlight-formula${displayClass}">${html}</span>`;
  };
  // Never load original image URLs. The reader resolves them from the manifest.
  md.renderer.rules.image = (tokens, index) => {
    const token = tokens[index];
    return '<span class="reader-image" data-image-ref="' + md.utils.escapeHtml(token.attrGet("src") ?? "") +
      '">' + md.utils.escapeHtml(token.content || "图片") + "</span>";
  };
  return md;
}
