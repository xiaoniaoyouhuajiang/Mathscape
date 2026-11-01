import { describe, expect, it } from "vitest";
import type { IncidenceNode } from "../../src/graph/projection";
import { parseMathscapeId, shortStableId } from "../../src/core/stable-id";
import { shortRunId } from "../../src/discovery/labels";
import {
  buildCiteIndex,
  buildRoundCiteIndex,
  citeResolver,
  citeTextHtml,
  collectCiteEntries,
  mapNodeIds,
  resolveCite,
  scanIds,
  toolCallIds,
  toolResultIds,
} from "../../src/sandbox/cite";
import { renderBubbleHtml } from "../../src/sandbox/bubble";
import type { AgentEventLike } from "../../src/sandbox/format";

const H1 = "a".repeat(64);
const H2 = "b".repeat(64);
const DOC = "0123456789abcdef0123456789abcdef";
const UNIT = `mathscape:source-unit/${H1}`;
const ART = `mathscape:artifact/${H2}`;

function node(id: string, domainId: string, kind: string, label: string, extra: Partial<IncidenceNode> = {}): IncidenceNode {
  return {
    id, element_kind: "entity", domain_id: domainId, domain_kind: kind, relation_kind: null, layer: "source",
    label, summary: null, lifecycle: "active", parent: null, boundary: false, ...extra,
  } as IncidenceNode;
}

const nodes = [
  node("r-unit", UNIT, "SourceSemanticUnit", "定理 3.2"),
  node("r-art", ART, "SourceArtifact", "notes.md"),
  node("r-art-copy", ART, "SourceArtifact", "notes.md", { boundary: true }),
];

describe("stable id helpers", () => {
  it("shortens head…tail and keeps short ids", () => {
    expect(shortStableId("short")).toBe("short");
    const long = "mathscape:discovery-run/" + "c".repeat(40);
    expect(shortStableId(long)).toBe(`${long.slice(0, 17)}…${long.slice(-12)}`);
    expect(shortRunId(long)).toBe(shortStableId(long));
    expect(shortStableId("x".repeat(31), { max: 30, head: 14, tail: 10 })).toBe(`${"x".repeat(14)}…${"x".repeat(10)}`);
  });

  it("parses canonical ids", () => {
    expect(parseMathscapeId(UNIT)).toEqual({ kind: "source-unit", hash: H1 });
    expect(parseMathscapeId(H1)).toBeNull();
  });
});

describe("scanIds", () => {
  it("finds canonical and bare hex ids, trims sentence punctuation", () => {
    const text = `见 ${UNIT}. 手稿 ${DOC}，以及 ${H2}`;
    expect(scanIds(text).map((hit) => hit.raw)).toEqual([UNIT, DOC, H2]);
  });

  it("ignores hex of other lengths and hex inside words", () => {
    expect(scanIds("abc " + "d".repeat(40) + " x" + "e".repeat(32))).toEqual([]);
  });
});

describe("resolveCite", () => {
  const index = buildCiteIndex(nodes);

  it("resolves exact ids to the entity node", () => {
    const hit = resolveCite(ART, undefined, index);
    expect(hit.status).toBe("map");
    expect(hit.target?.nodeId).toBe("r-art");
  });

  it("resolves bare hashes and prefix variants", () => {
    expect(resolveCite(H1, undefined, index).target?.nodeId).toBe("r-unit");
    expect(resolveCite(`mathscape:unit/${H1}`, undefined, index).target?.nodeId).toBe("r-unit");
  });

  it("marks shared hashes ambiguous and lets the round disambiguate", () => {
    const twin = buildCiteIndex([...nodes, node("r-span", `mathscape:source-span/${H1}`, "SourceSpan", "原文")]);
    expect(resolveCite(H1, undefined, twin).status).toBe("ambiguous");
    const round = buildRoundCiteIndex([
      { type: "tool_result", round_id: "r1", payload: { raw: { result: { node: { id: `mathscape:source-span/${H1}`, label: "原文" } } } } },
    ]);
    const hit = resolveCite(H1, round, twin);
    expect(hit.status).toBe("map");
    expect(hit.target?.nodeId).toBe("r-span");
  });

  it("uses round labels for ids not on the map and leaves unknown ids unjumpable", () => {
    const round = buildRoundCiteIndex([
      { type: "tool_result", payload: { raw: { result: { groups: [{ hits: [{ kind: "note", id: DOC, title: "我的手稿" }] }] } } } },
    ]);
    const note = resolveCite(DOC, round, index);
    expect(note).toMatchObject({ status: "label", label: "我的手稿" });
    expect(resolveCite(`mathscape:source-unit/${"f".repeat(64)}`, round, index).status).toBe("unknown");
  });

  it("aliases a snapshot id to its snapshot artifact on the map", () => {
    const snap = `mathscape:ink-snapshot/${"9".repeat(32)}`;
    const round = buildRoundCiteIndex([
      { type: "tool_result", payload: { raw: { result: { content: [{ type: "text", text: JSON.stringify({ node: { id: ART, snapshot_id: snap, label: "快照" } }) }] } } } },
    ]);
    expect(resolveCite(snap, round, index).target?.nodeId).toBe("r-art");
  });
});

describe("tool echo ids", () => {
  const index = buildCiteIndex(nodes);
  const call: AgentEventLike = { type: "tool_call", payload: { tool: "graph.neighborhood", raw: { arguments: { seed_ids: [UNIT, ART], depth: 1 } } } };
  const result: AgentEventLike = {
    type: "tool_result",
    payload: { raw: { result: { structuredContent: { nodes: [{ id: ART, label: "notes.md", content_ref: { artifact_id: ART } }, { id: `mathscape:source-unit/${"7".repeat(64)}` }] } } } },
  };

  it("collects seeds from arguments and hits from results, mapped and unique", () => {
    expect(mapNodeIds(toolCallIds(call), index)).toEqual(["r-unit", "r-art"]);
    expect(mapNodeIds(toolResultIds(result), index)).toEqual(["r-art"]);
  });

  it("falls back to args_summary when raw arguments are absent", () => {
    expect(toolCallIds({ type: "tool_call", payload: { args_summary: `node_id=${UNIT}` } })).toEqual([UNIT]);
  });

  it("caps the echo", () => {
    const many = Array.from({ length: 60 }, (_, i) => node(`n${i}`, `mathscape:source-unit/${i.toString(16).padStart(64, "0")}`, "SourceSemanticUnit", `u${i}`));
    const big = buildCiteIndex(many);
    expect(mapNodeIds(many.map((item) => item.domain_id!), big)).toHaveLength(40);
  });

  it("labels search hits through their target", () => {
    const entries = collectCiteEntries({ hits: [{ id: "x", title: "标题", target: { object_id: UNIT } }] });
    expect(entries).toContainEqual(expect.objectContaining({ id: UNIT, label: "标题" }));
  });
});

describe("chip-safe rendering", () => {
  const resolve = citeResolver(buildCiteIndex(nodes));

  it("escapes text around chips and puts the full id in data attributes", () => {
    const html = citeTextHtml(`<b>x</b> ${UNIT} ${"c".repeat(64)}`, resolve);
    expect(html).toContain("&lt;b&gt;x&lt;/b&gt;");
    expect(html).toContain(`data-cite-node="r-unit"`);
    expect(html).toContain(">定理 3.2</button>");
    expect(html).toContain(`class="sandbox-cite is-unknown" data-cite-id="${"c".repeat(64)}"`);
    expect(html).not.toContain("<b>");
  });

  it("escapes hostile labels", () => {
    const evil = citeResolver(buildCiteIndex([node("r-x", UNIT, "SourceSemanticUnit", `<img src=x onerror=alert(1)>"`)]));
    const html = citeTextHtml(UNIT, evil);
    expect(html).not.toContain("<img");
    expect(html).toContain("&lt;img");
  });

  it("renders markdown + KaTeX bubbles with chips in text and code, not in math", () => {
    const html = renderBubbleHtml(`**结论**：$a_n \\to 0$，见 ${UNIT} 与 \`${ART}\`。<script>alert(1)</script> [链接](https://example.com)`, resolve);
    expect(html).toContain("<strong>结论</strong>");
    expect(html).toContain("katex");
    expect(html).toContain(`data-cite-node="r-unit"`);
    expect(html).toMatch(/<code><button[^>]*data-cite-node="r-art"/);
    expect(html).not.toContain("<script>");
    expect(html).not.toContain("href=");
    expect(html).toContain('class="sandbox-link" title="https://example.com"');
  });
});
