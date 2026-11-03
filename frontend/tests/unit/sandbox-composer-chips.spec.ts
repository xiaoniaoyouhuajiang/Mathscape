import { describe, expect, it } from "vitest";
import type { IncidenceNode } from "../../src/graph/projection";
import { buildCiteIndex, buildRoundCiteIndex, citeChipHtml, citeChipParts, citeResolver, resolveCite } from "../../src/sandbox/cite";
import { composerChip, expandComposer, tokenizeComposer } from "../../src/sandbox/composer-chips";

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
