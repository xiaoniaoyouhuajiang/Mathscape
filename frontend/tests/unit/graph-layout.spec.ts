import { describe, expect, it } from "vitest";
import type { IncidenceEdge, IncidenceGraph, IncidenceNode, RelationKind } from "../../src/graph/projection";
import {
  computeLandscapeLayout,
  isAuxiliaryArtifactNode,
  landscapeLayoutKey,
  resolveIncrementalPositions,
} from "../../src/graph/layout";

function entity(
  id: string,
  domainKind: string,
  label: string,
  summary: string | null = null,
): IncidenceNode {
  return {
    id: `entity:${id}`,
    element_kind: "entity",
    domain_id: id,
    domain_kind: domainKind,
    relation_kind: null,
    layer: "source",
    label,
    summary,
    lifecycle: "active",
    parent: null,
    boundary: domainKind === "SourceSpan",
  };
}

function relation(id: string, domainKind: string, relationKind: RelationKind, label: string): IncidenceNode {
  return {
    id: `relation:${id}`,
    element_kind: "relation",
    domain_id: id,
    domain_kind: domainKind,
    relation_kind: relationKind,
    layer: "source",
    label,
    summary: null,
    lifecycle: "active",
    parent: null,
    boundary: false,
  };
}

function connect(
  edges: IncidenceEdge[],
  rel: IncidenceNode,
  source: IncidenceNode,
  target: IncidenceNode,
): void {
  const relationId = rel.domain_id ?? rel.id;
  edges.push(
    {
      id: `edge:${relationId}:in`,
      source: source.id,
      target: rel.id,
      relation_id: relationId,
      role: "source",
      ordinal: null,
      direction: "into_relation",
      directed: true,
    },
    {
      id: `edge:${relationId}:out`,
      source: rel.id,
      target: target.id,
      relation_id: relationId,
      role: "target",
      ordinal: null,
      direction: "out_of_relation",
      directed: true,
    },
  );
}

function fixture(): IncidenceGraph {
  const pdf = entity("pdf", "SourceArtifact", "组合数学.pdf", "application/pdf · 1024 bytes · available");
  const markdown = entity("markdown", "SourceArtifact", "组合数学.md", "text/markdown · 1024 bytes · available");
  const metadata = entity("metadata", "SourceArtifact", "解析元数据", "application/json · 64 bytes · available");
  const section = entity("section", "SourceSemanticUnit", "第一章 生成函数", "section");
  const theorem = entity("theorem", "SourceSemanticUnit", "定理 1：母函数恒等式", "statement");
  const span = entity("span", "SourceSpan", "来源范围", "Markdown 字节范围");
  const theory = entity("theory", "SemanticEntity", "理论：组合计数", "理论");
  const concept = entity("concept", "SemanticEntity", "概念：母函数", "概念");
  const nodes = [pdf, markdown, metadata, section, theorem, span, theory, concept];
  const edges: IncidenceEdge[] = [];
  const relations = [
    relation("conversion-markdown", "ConversionRun", "conversion", "解析"),
    relation("conversion-metadata", "ConversionRun", "conversion", "解析"),
    relation("part", "SourceUnitRelation", "has_part", "包含"),
    relation("contains", "SourceUnitRelation", "contains_span", "对应原文"),
    relation("located", "Evidence", "located_in", "来源于正文"),
    relation("alignment", "Alignment", "aligns", "说的就是它"),
    relation("theory", "SemanticEntity", "in_theory", "属于理论"),
  ];
  connect(edges, relations[0], pdf, markdown);
  connect(edges, relations[1], pdf, metadata);
  connect(edges, relations[2], section, theorem);
  connect(edges, relations[3], theorem, span);
  connect(edges, relations[4], span, markdown);
  connect(edges, relations[5], theorem, concept);
  connect(edges, relations[6], concept, theory);
  return {
    schema_version: "mathscape.graph-projection.v1",
    lens: "landscape",
    nodes: [...nodes, ...relations],
    edges,
  };
}

function sortedPositionObject(positions: ReadonlyMap<string, { x: number; y: number }>) {
  return Object.fromEntries(
    [...positions.entries()]
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([id, point]) => [id, point]),
  );
}

describe("deterministic organic landscape layout", () => {
  it("is invariant to projection insertion order and covers relation coordinates", () => {
    const graph = fixture();
    const reversed: IncidenceGraph = {
      ...graph,
      nodes: [...graph.nodes].reverse(),
      edges: [...graph.edges].reverse(),
    };
    const first = computeLandscapeLayout(graph);
    const second = computeLandscapeLayout(reversed);

    expect(sortedPositionObject(first)).toEqual(sortedPositionObject(second));
    for (const node of graph.nodes) {
      expect(first.has(node.id)).toBe(true);
    }
    expect(first.has("junc~conversion-markdown")).toBe(true);
    expect(first.has("relation:conversion-markdown")).toBe(true);
    expect(landscapeLayoutKey(graph)).toBe(landscapeLayoutKey(reversed));
  });

  it("keeps auxiliary artifacts and source spans near their semantic anchors", () => {
    const graph = fixture();
    const metadata = graph.nodes.find((node) => node.id === "entity:metadata")!;
    expect(isAuxiliaryArtifactNode(metadata)).toBe(true);

    const positions = computeLandscapeLayout(graph);
    const pdf = positions.get("entity:pdf")!;
    const metadataPosition = positions.get("entity:metadata")!;
    const theorem = positions.get("entity:theorem")!;
    const span = positions.get("entity:span")!;

    expect(Math.hypot(metadataPosition.x - pdf.x, metadataPosition.y - pdf.y)).toBeLessThan(220);
    expect(Math.hypot(span.x - theorem.x, span.y - theorem.y)).toBeLessThan(180);
  });

  it("preserves settled and pinned positions while adding new coordinates", () => {
    const graph = fixture();
    const full = computeLandscapeLayout(graph);
    const settled = new Map(full);
    settled.set("entity:section", { x: -321, y: 144 });
    const pinned = new Map([["entity:theorem", { x: 777, y: -333 }]]);

    const resolved = resolveIncrementalPositions(graph, settled, pinned, full);
    expect(resolved.get("entity:section")).toEqual({ x: -321, y: 144 });
    expect(resolved.get("entity:theorem")).toEqual({ x: 777, y: -333 });
    expect(resolved.get("relation:part")).toEqual({ x: 228, y: -94 });
  });
});
