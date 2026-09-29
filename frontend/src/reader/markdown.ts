import MarkdownIt from "markdown-it";
import katex from "katex";
import "katex/dist/katex.min.css";

export type MarkdownPatchHighlight = "old" | "new";
/** Source backlinks use their own variant: a located anchor is not a patch deletion or insertion. */
export type MarkdownHighlight = MarkdownPatchHighlight | "anchor";

type PatchMarkerKind = "start" | "end";

const VARIANTS = ["old", "new", "anchor"] as const;

export const HIGHLIGHT_MARKERS: Record<MarkdownHighlight, Record<PatchMarkerKind, string>> = {
  old: {
    start: "\uE000mathscape-patch-old-start\uE001",
    end: "\uE000mathscape-patch-old-end\uE001",
  },
  new: {
    start: "\uE000mathscape-patch-new-start\uE001",
    end: "\uE000mathscape-patch-new-end\uE001",
  },
  anchor: {
    start: "\uE000mathscape-anchor-start\uE001",
    end: "\uE000mathscape-anchor-end\uE001",
  },
};
const PATCH_MARKERS = HIGHLIGHT_MARKERS;

const HIGHLIGHT_CLASS: Record<MarkdownHighlight, string> = {
  old: "markdown-patch-highlight-old",
  new: "markdown-patch-highlight-new",
  anchor: "review-anchor-highlight",
};

function markerAt(source: string, position: number): { variant: MarkdownHighlight; kind: PatchMarkerKind; length: number } | null {
  for (const variant of VARIANTS) {
    for (const kind of ["start", "end"] as const) {
      const value = PATCH_MARKERS[variant][kind];
      if (source.startsWith(value, position)) return { variant, kind, length: value.length };
    }
  }
  return null;
}

function patchVariant(source: string): MarkdownHighlight | null {
  for (const variant of VARIANTS) {
    if (source.includes(PATCH_MARKERS[variant].start) || source.includes(PATCH_MARKERS[variant].end)) return variant;
  }
  return null;
}

export function removePatchMarkers(source: string): string {
  let result = source;
  for (const variant of VARIANTS) {
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
  variant: MarkdownHighlight,
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

const TEXT_TERMINATORS = new Set([
  0x0a, 0x21, 0x23, 0x24, 0x25, 0x26, 0x2a, 0x2b, 0x2d, 0x3a, 0x3c, 0x3d, 0x40,
  0x5b, 0x5c, 0x5d, 0x5e, 0x5f, 0x60, 0x7b, 0x7d, 0x7e, 0xe000,
]);

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
  // MarkdownIt's text rule consumes every non-terminator character, which
  // would swallow a marker that directly follows plain text. Same rule, with
  // the private-use marker prefix added to the terminators.
  md.inline.ruler.at("text", (state, silent) => {
    let pos = state.pos;
    while (pos < state.posMax && !TEXT_TERMINATORS.has(state.src.charCodeAt(pos))) pos++;
    if (pos === state.pos) return false;
    if (!silent) state.pending += state.src.slice(state.pos, pos);
    state.pos = pos;
    return true;
  });
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
    const marker = tokens[index].meta as { variant: MarkdownHighlight; kind: PatchMarkerKind };
    return marker.kind === "start" ? `<mark class="${HIGHLIGHT_CLASS[marker.variant]}">` : "</mark>";
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
    const displayClass = token.meta.display ? " markdown-patch-highlight-formula-display" : "";
    return `<span class="${HIGHLIGHT_CLASS[variant]} markdown-patch-highlight-formula${displayClass}">${html}</span>`;
  };
  // Never load original image URLs. The reader resolves them from the manifest.
  md.renderer.rules.image = (tokens, index) => {
    const token = tokens[index];
    return '<span class="reader-image" data-image-ref="' + md.utils.escapeHtml(token.attrGet("src") ?? "") +
      '">' + md.utils.escapeHtml(token.content || "图片") + "</span>";
  };
  return md;
}
