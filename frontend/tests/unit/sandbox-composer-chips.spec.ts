import { describe, expect, it } from "vitest";
import type { IncidenceNode } from "../../src/graph/projection";
import { buildCiteIndex, buildRoundCiteIndex, citeChipHtml, citeChipParts, citeResolver, resolveCite } from "../../src/sandbox/cite";
import { chipSpans, composerChip, expandComposer, mentionSuggestions, mentionTargets, mentionTrigger, spaceAfterChip, tokenizeComposer } from "../../src/sandbox/composer-chips";

const H1 = "a".repeat(64);
const H2 = "b".repeat(64);
const DOC = "0123456789abcdef0123456789abcdef";
const UNIT = `mathscape:source-unit/${H1}`;
const ART = `mathscape:artifact/${H2}`;
const STRANGER = `mathscape:source-unit/${"f".repeat(64)}`;

function node(id: string, domainId: string, kind: string, label: string): IncidenceNode {
  return {
    id, element_kind: "entity", domain_id: domainId, domain_kind: kind, relation_kind: null, layer: "source",
    label, summary: null, lifecycle: "active", parent: null, boundary: false,
  } as IncidenceNode;
}

const index = buildCiteIndex([node("r-unit", UNIT, "SourceSemanticUnit", "定理 3.2"), node("r-art", ART, "SourceArtifact", "notes.md")]);
const round = buildRoundCiteIndex([
  { type: "tool_result", payload: { raw: { result: { groups: [{ hits: [{ kind: "note", id: DOC, title: "我的手稿" }] }] } } } },
]);
const resolve = citeResolver(index, round);

describe("composer tokenization", () => {
  it("chips landscape-known ids with the short label and keeps the full id", () => {
    const segments = tokenizeComposer(`看看 ${UNIT} 和 ${ART}。`, resolve);
    expect(segments.map((s) => s.kind)).toEqual(["text", "chip", "text", "chip", "text"]);
    const [, unit, , art] = segments;
    expect(unit).toMatchObject({ kind: "chip", chip: { id: UNIT, status: "map", text: "定理 3.2", jumpNode: "r-unit", statusClass: "is-map" } });
    expect(art).toMatchObject({ kind: "chip", chip: { id: ART, text: "notes.md", jumpNode: "r-art" } });
  });

  it("chips ids only the session's tool context knows, without a map jump", () => {
    const [, chip] = tokenizeComposer(`手稿 ${DOC}`, resolve);
    expect(chip).toMatchObject({ kind: "chip", chip: { id: DOC, status: "label", text: "我的手稿", statusClass: "is-label" } });
    expect(chip.kind === "chip" && chip.chip.jumpNode).toBeUndefined();
  });

  it("leaves index misses as plain text (no grey chip in the composer)", () => {
    const text = `未知 ${STRANGER} 以及 ${"c".repeat(64)}`;
    expect(tokenizeComposer(text, resolve)).toEqual([{ kind: "text", text }]);
    expect(composerChip(STRANGER, resolve)).toBeNull();
  });

  it("keeps everything as text when there is no landscape yet", () => {
    expect(tokenizeComposer(UNIT, undefined)).toEqual([{ kind: "text", text: UNIT }]);
  });

  it("does not chip an id that ends at the caret while typing (it may still grow)", () => {
    const text = `看看 ${UNIT}`;
    expect(tokenizeComposer(text, resolve, { openAt: text.length })).toEqual([{ kind: "text", text }]);
    expect(tokenizeComposer(`${text} `, resolve, { openAt: text.length + 1 }).map((s) => s.kind)).toEqual(["text", "chip", "text"]);
    // A 32-hex prefix of a known 64-hex hash is not chipped mid-typing.
    const half = H1.slice(0, 32);
    expect(tokenizeComposer(half, resolve, { openAt: 32 })).toEqual([{ kind: "text", text: half }]);
  });

  it("never dissolves an existing chip that ends at the caret", () => {
    const text = `看看 ${UNIT}`;
    const segments = tokenizeComposer(text, resolve, { openAt: text.length, chipStarts: new Set([3]) });
    expect(segments.map((s) => s.kind)).toEqual(["text", "chip"]);
  });

  it("trims sentence punctuation off a chip, like reply chips", () => {
    const segments = tokenizeComposer(`见 ${UNIT}.`, resolve);
    expect(segments[1]).toMatchObject({ kind: "chip", chip: { id: UNIT } });
    expect(segments[2]).toEqual({ kind: "text", text: "." });
  });
});

describe("expand on send", () => {
  it("round-trips: chips expand back to the exact full ids", () => {
    const text = `比较 ${UNIT}\n与 ${ART}，未知 ${STRANGER}，手稿 ${DOC}`;
    const segments = tokenizeComposer(text, resolve);
    expect(segments.filter((s) => s.kind === "chip")).toHaveLength(3);
    expect(expandComposer(segments)).toBe(text);
  });

  it("keeps the id as written (case, prefix variant)", () => {
    const variant = `mathscape:unit/${H1.toUpperCase()}`;
    const segments = tokenizeComposer(`x ${variant}`, resolve);
    expect(segments[1]).toMatchObject({ kind: "chip", chip: { id: variant, jumpNode: "r-unit" } });
    expect(expandComposer(segments)).toBe(`x ${variant}`);
  });

  it("reuses the reply chip presentation (same label/title as citeChipHtml)", () => {
    const parts = citeChipParts(resolveCite(UNIT, undefined, index));
    expect(parts).toMatchObject({ statusClass: "is-map", text: "定理 3.2", jumpNode: "r-unit" });
    expect(citeChipHtml(resolveCite(UNIT, undefined, index))).toContain(`title="${parts.title}"`);
  });
});

describe("chip token model", () => {
  it("measures chip spans in expanded offsets", () => {
    const text = `看看 ${UNIT} 和 ${ART}`;
    const spans = chipSpans(tokenizeComposer(text, resolve));
    expect(spans).toEqual([[3, 3 + UNIT.length], [3 + UNIT.length + 3, text.length]]);
    expect(spans.map(([a, b]) => text.slice(a, b))).toEqual([UNIT, ART]);
  });

  it("adds one trailing space after a chip that ends at the caret and moves the caret past it", () => {
    const text = `看看 ${UNIT}`;
    const spans = chipSpans(tokenizeComposer(text, resolve));
    expect(spaceAfterChip(text, text.length, spans)).toEqual({ text: `${text} `, caret: text.length + 1 });
  });

  it("reuses an existing space and puts one before following text or a line break", () => {
    const spaced = `${UNIT} 好`;
    expect(spaceAfterChip(spaced, UNIT.length, chipSpans(tokenizeComposer(spaced, resolve)))).toEqual({ text: spaced, caret: UNIT.length + 1 });
    const broken = `${UNIT}\n第二行`;
    expect(spaceAfterChip(broken, UNIT.length, chipSpans(tokenizeComposer(broken, resolve)))).toEqual({ text: `${UNIT} \n第二行`, caret: UNIT.length + 1 });
  });

  it("leaves text alone when no chip ends at the caret", () => {
    const text = `看看 ${STRANGER}`;
    expect(spaceAfterChip(text, text.length, chipSpans(tokenizeComposer(text, resolve)))).toEqual({ text, caret: text.length });
    const mid = `${UNIT} 好`;
    expect(spaceAfterChip(mid, mid.length, chipSpans(tokenizeComposer(mid, resolve)))).toEqual({ text: mid, caret: mid.length });
  });
});

describe("composer @ mentions", () => {
  const H3 = "c".repeat(64);
  const LEMMA = `mathscape:source-unit/${H3}`;
  const mapIndex = buildCiteIndex([
    node("r-unit", UNIT, "SourceSemanticUnit", "定理 3.2"),
    node("r-art", ART, "SourceArtifact", "notes.md"),
    node("r-lemma", LEMMA, "SourceSemanticUnit", "引理 1.4"),
    node("r-plain", "unit-plain", "SourceSemanticUnit", "定理 9（非稳定 id）"),
  ]);

  it("offers one entry per map node whose id would chip", () => {
    const ids = mentionTargets(mapIndex).map((t) => t.domainId).sort();
    expect(ids).toEqual([ART, UNIT, LEMMA].sort());
    expect(mentionTargets(null)).toEqual([]);
  });

  it("filters by label, short id and type, every keyword must hit", () => {
    const targets = mentionTargets(mapIndex);
    const ids = (query: string) => mentionSuggestions(targets, query).map((s) => s.target.domainId);
    expect(ids("")).toHaveLength(3);
    expect(ids("引理")).toEqual([LEMMA]);
    expect(ids("NOTES")).toEqual([ART]);
    expect(ids("cccc")).toEqual([LEMMA]);
    expect(ids("artifact")).toEqual([ART]);
    expect(ids("内容单元").sort()).toEqual([UNIT, LEMMA].sort());
    expect(ids("内容单元 3.2")).toEqual([UNIT]);
    expect(ids("zzz")).toEqual([]);
    const [first] = mentionSuggestions(targets, "定理");
    expect(first).toMatchObject({ label: "定理 3.2", kind: "内容单元", shortId: "source-unit/aaaaaaaa…aaaaaa" });
  });

  it("ranks label prefix hits before substring and id hits", () => {
    const targets = mentionTargets(buildCiteIndex([
      node("r1", `mathscape:source-unit/${"1".repeat(64)}`, "SourceSemanticUnit", "推论 紧集"),
      node("r2", `mathscape:source-unit/${"2".repeat(64)}`, "SourceSemanticUnit", "紧集定义"),
    ]));
    expect(mentionSuggestions(targets, "紧集").map((s) => s.label)).toEqual(["紧集定义", "推论 紧集"]);
  });

  it("finds the @ under the caret, not inside emails, chips or across lines", () => {
    expect(mentionTrigger("@", 1)).toEqual({ start: 0, query: "" });
    expect(mentionTrigger("看看 @引理 1", 8)).toEqual({ start: 3, query: "引理 1" });
    expect(mentionTrigger("看看@引理", 5)).toEqual({ start: 2, query: "引理" });
    expect(mentionTrigger("mail a@b.com", 12)).toBeNull();
    expect(mentionTrigger("@a\nb", 4)).toBeNull();
    expect(mentionTrigger("@a  b", 5)).toBeNull();
    expect(mentionTrigger("no at", 5)).toBeNull();
    // A chip between the @ and the caret closes the mention.
    expect(mentionTrigger(`@x ${UNIT} y`, 6 + UNIT.length, [[3, 3 + UNIT.length]])).toBeNull();
    // Caret before the @: nothing.
    expect(mentionTrigger("ab @c", 2)).toBeNull();
  });
});
