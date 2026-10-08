import cytoscape from "cytoscape";
import { describe, expect, it } from "vitest";
import { toCytoscapeElements } from "../../src/graph/cytoscape";
import { activityGraph } from "../../src/graph/lifecycle";
import type { IncidenceEdge, IncidenceGraph, IncidenceNode } from "../../src/graph/projection";

// Match Rust incidence.rs: vertex ids have entity:/relation: prefixes,
// while edge.relation_id is the unprefixed domain StableId.
function entity(name: string, lifecycle = "active", kind = "SourceSemanticUnit"): IncidenceNode {
  const domainId = `mathscape:entity/${name}`;
  return {
    id: `entity:${domainId}`, domain_id: domainId, element_kind: "entity",
    domain_kind: kind, relation_kind: null, layer: "source", label: name,
    summary: null, lifecycle, parent: null, boundary: false,
  };
}
function relation(name: string, lifecycle = "active"): IncidenceNode {
  const domainId = `mathscape:relation/${name}`;
  return {
    id: `relation:${domainId}`, domain_id: domainId, element_kind: "relation",
    domain_kind: "SourceUnitRelation", relation_kind: "explains", layer: "source",
    label: name, summary: null, lifecycle, parent: null, boundary: false,
  };
}
function spoke(
  node: IncidenceNode, owner: IncidenceNode, role: string,
  direction: "into_relation" | "out_of_relation", ordinal: number | null = null,
): IncidenceEdge {
  return {
    id: `incidence:${owner.domain_id ?? owner.id}:${role}:${ordinal ?? "single"}`,
    source: direction === "into_relation" ? node.id : owner.id,
    target: direction === "into_relation" ? owner.id : node.id,
    relation_id: owner.domain_id ?? owner.id, role, ordinal, direction, directed: true,
  };
}
function fixture(): IncidenceGraph {
  const a = entity("a"), b = entity("b"), r = relation("r");
  return {
    schema_version: "mathscape.graph-projection.v1", lens: "landscape",
    nodes: [a, b, r],
    edges: [spoke(a, r, "source", "into_relation"), spoke(b, r, "target", "out_of_relation")],
  };
}
function expectClosed(graph: IncidenceGraph): void {
  const ids = new Set(graph.nodes.map(node => node.id));
  for (const edge of graph.edges) {
    expect(ids.has(edge.source)).toBe(true);
    expect(ids.has(edge.target)).toBe(true);
  }
}

describe("current activity lens", () => {
  it("preserves live backend relations when render and domain ids differ", () => {
    const graph = fixture();
    expect(graph.nodes[2].id).not.toBe(graph.edges[0].relation_id);
    const actual = activityGraph(graph, false);
    expect(actual).toEqual(graph);
    expectClosed(actual);
  });

  it.each(["deprecated", "superseded", "rejected"])(
    "removes %s entities and resulting dangling relations, but retains candidates",
    lifecycle => {
      const graph = fixture();
      graph.nodes[1].lifecycle = lifecycle;
      const candidate = entity("candidate", "candidate");
      graph.nodes.push(candidate);
      const before = structuredClone(graph);
      const actual = activityGraph(graph, false);
      expect(actual.nodes.map(node => node.id)).toEqual([graph.nodes[0].id, candidate.id]);
      expect(actual.edges).toEqual([]);
      expect(graph).toEqual(before);
      expectClosed(actual);
    },
  );

  it("removes retired relations and all their spokes without retiring their entities", () => {
    const graph = fixture();
    graph.nodes[2].lifecycle = "deprecated";
    const actual = activityGraph(graph, false);
    expect(actual.nodes).toEqual(graph.nodes.slice(0, 2));
    expect(actual.edges).toEqual([]);
    expectClosed(actual);
  });

  it("preserves conversion hyperedges, role ordinals and multiple output artifacts", () => {
    const pdf = entity("pdf", "active", "SourceArtifact");
    const md = entity("md", "active", "SourceArtifact");
    const json = entity("json", "active", "SourceArtifact");
    const run = { ...relation("conversion"), domain_kind: "ConversionRun", relation_kind: "conversion" as const };
    const graph: IncidenceGraph = {
      ...fixture(), nodes: [pdf, md, json, run],
      edges: [spoke(pdf, run, "input", "into_relation", 0), spoke(md, run, "output", "out_of_relation", 0), spoke(json, run, "output", "out_of_relation", 1)],
    };
    expect(activityGraph(graph, false)).toEqual(graph);
  });

  it("preserves a still-supported multi-endpoint relation when one evidence node retires", () => {
    const graph = fixture();
    const span = entity("span", "deprecated", "SourceSpan");
    graph.nodes.push(span);
    graph.edges.push(spoke(span, graph.nodes[2], "evidence", "out_of_relation", 0));
    const actual = activityGraph(graph, false);
    expect(actual).toEqual(fixture());
    expectClosed(actual);
  });

  it("removes pre-existing invalid spokes without losing a valid relation", () => {
    const graph = fixture();
    graph.edges.push(spoke(entity("missing"), graph.nodes[2], "evidence", "out_of_relation", 0));
    const actual = activityGraph(graph, false);
    expect(actual).toEqual(fixture());
    expectClosed(actual);
  });

  it("supports synthetic focus relations without a domain id", () => {
    const graph = fixture();
    const owner = { ...graph.nodes[2], id: "relation:focus-contracted/a~b", domain_id: null, lifecycle: null };
    graph.nodes[2] = owner;
    graph.edges = [spoke(graph.nodes[0], owner, "derived_from", "into_relation"), spoke(graph.nodes[1], owner, "derives", "out_of_relation")];
    expect(activityGraph(graph, false)).toEqual(graph);
  });

  it("is idempotent when the controller applies the lens before both focus and filtering", () => {
    const graph = fixture();
    const once = activityGraph(graph, false);
    expect(activityGraph(once, false)).toEqual(once);
  });

  it("show retired is the unchanged current snapshot, not reconstructed history", () => {
    const graph = fixture();
    graph.nodes[1].lifecycle = "deprecated";
    expect(activityGraph(graph, true)).toBe(graph);
  });

  it.each(["semantic", "hyperedge"] as const)(
    "keeps one connected network through activity filtering and %s rendering",
    mode => {
      const graph = activityGraph(fixture(), false);
      const core = cytoscape({ headless: true, elements: toCytoscapeElements(graph, mode) });
      try {
        expect(core.edges()).toHaveLength(mode === "semantic" ? 1 : 2);
        expect(core.elements().components()).toHaveLength(1);
        expect(core.getElementById(graph.nodes[0].id).degree()).toBe(1);
        expect(core.getElementById(graph.nodes[1].id).degree()).toBe(1);
      } finally { core.destroy(); }
    },
  );
});
