const BREAK = "\u200b";
const CJK = String.raw`\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}`;
const PUNCTUATION = "，。、；：！？（）《》〈〉「」『』【】〔〕“”‘’";
const PIECES = new RegExp(`[${CJK}${PUNCTUATION}]|\\s+|(?:_[${CJK}]+|[^\\s${CJK}${PUNCTUATION}])+`, "gu");
const CLOSING = /^(?:[，。、；：！？）》〉」』】〕”’,;:!?%)\]}]|\.(?!\d))+/u;
const OPENING = /[（《〈「『【〔“‘(\[{]+$/u;

/**
 * Cytoscape's "anywhere" wrapping splits Latin words and formulae, while its
 * whitespace wrapper cannot break CJK prose. Supply only safe break points:
 * CJK characters can wrap, Latin/math runs stay whole, opening punctuation
 * follows its next token and closing punctuation stays with the previous one.
 * Keep this in the renderer: graph data, search and copied titles stay clean.
 */
export function wrapGraphLabel(text: string): string {
  return text.split("\n").map(line => {
    const units: { text: string; before: string }[] = [];
    let space = "", prefix = "", prefixSpace = "";
    for (let part of line.match(PIECES) ?? []) {
      if (/^\s+$/u.test(part)) {
        if (!prefix) space = " ";
        continue;
      }
      const close = part.match(CLOSING)?.[0] ?? "";
      if (close) {
        if (units.length) units[units.length - 1].text += close;
        else prefix += close;
        part = part.slice(close.length);
        space = "";
      }
      const open = part.match(OPENING)?.[0] ?? "";
      const body = open ? part.slice(0, -open.length) : part;
      if (body) {
        units.push({ text: prefix + body, before: prefix ? prefixSpace : space });
        prefix = "";
        space = "";
      }
      if (open) {
        if (!prefix) prefixSpace = space;
        prefix += open;
        space = "";
      }
    }
    if (prefix && units.length) units[units.length - 1].text += prefix;
    return units.map((unit, index) => `${index ? unit.before || BREAK : ""}${unit.text}`).join("");
  }).join("\n");
}
