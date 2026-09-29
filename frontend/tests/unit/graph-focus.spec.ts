import { describe, expect, it } from "vitest";
import { focusSubgraph, FOCUS_CONTRACTION_KIND } from "../../src/graph/focus";
import type {
  IncidenceEdge,
  IncidenceGraph,
  IncidenceNode,
} from "../../src/graph/projection";

describe("graph focus algorithm", () => {
  function makeEntity(id: string, domainKind: string, label: string): IncidenceNode {
    return {
      id,
      element_kind: "entity",
      domain_id: `domain:${id}`,
      domain_kind: domainKind,
      relation_kind: null,
      layer: "source",
      label,
      summary: null,
      lifecycle: "active",
      parent: null,
      boundary: false,
    };
  }

  function makeRelation(
    id: string,
    relationKind: "conversion" | "located_in" | "contains_span" | "has_part",
    label: string,
  ): IncidenceNode {
    return {
      id,
      element_kind: "relation",
      domain_id: `rel:${id}`,
      domain_kind: "SourceUnitRelation",
      relation_kind: relationKind,
      layer: "source",
      label,
      summary: null,
      lifecycle: "active",
      parent: null,
      boundary: false,
    };
  }

  function makeEdge(
    id: string,
    source: string,
    target: string,
    relationId: string,
    direction: "into_relation" | "out_of_relation" | "undirected",
  ): IncidenceEdge {
    return {
      id,
      source,
      target,
      relation_id: relationId,
      role: "member",
      ordinal: null,
      direction,
      directed: true,
    };
  }

  it("handles a single isolated node with zero depth traversal", () => {
    const root = makeEntity("root", "SourceArtifact", "Root PDF");
    const graph: IncidenceGraph = {
      schema_version: "mathscape.graph-projection.v1",
      lens: "source",
      nodes: [root],
      edges: [],
    };

    const result = focusSubgraph(graph, "root", {
      depth: 1,
      isEntityVisible: () => true,
    });

    expect(result.reachedCount).toBe(1);
    expect(result.contractedCount).toBe(0);
    expect(result.graph.nodes).toHaveLength(1);
    expect(result.graph.nodes[0].id).toBe("root");
  });

  it("respects depth constraints in derivation chain (has_part)", () => {
    // Structure:
    // root -> rel_1 (has_part) -> child_1 -> rel_2 (has_part) -> child_2
    const root = makeEntity("root", "SourceSemanticUnit", "Root Unit");
    const rel1 = makeRelation("rel_1", "has_part", "has part 1");
    const child1 = makeEntity("child_1", "SourceSemanticUnit", "Child Unit 1");
    const rel2 = makeRelation("rel_2", "has_part", "has part 2");
    const child2 = makeEntity("child_2", "SourceSemanticUnit", "Child Unit 2");

    const graph: IncidenceGraph = {
      schema_version: "mathscape.graph-projection.v1",
      lens: "source",
      nodes: [root, rel1, child1, rel2, child2],
      edges: [
        makeEdge("e1", "root", "rel_1", "rel_1", "into_relation"),
        makeEdge("e2", "rel_1", "child_1", "rel_1", "out_of_relation"),
        makeEdge("e3", "child_1", "rel_2", "rel_2", "into_relation"),
        makeEdge("e4", "rel_2", "child_2", "rel_2", "out_of_relation"),
      ],
    };

    // Depth 1: should reach child1, but NOT child2
    const depth1Result = focusSubgraph(graph, "root", {
      depth: 1,
      isEntityVisible: () => true,
    });
    expect(depth1Result.reachedCount).toBe(2); // root + child1
    expect(depth1Result.graph.nodes.map((n) => n.id)).toContain("child_1");
    expect(depth1Result.graph.nodes.map((n) => n.id)).not.toContain("child_2");

    // Depth 2: should reach child1 and child2
    const depth2Result = focusSubgraph(graph, "root", {
      depth: 2,
      isEntityVisible: () => true,
    });
    expect(depth2Result.reachedCount).toBe(3); // root + child1 + child2
    expect(depth2Result.graph.nodes.map((n) => n.id)).toContain("child_1");
    expect(depth2Result.graph.nodes.map((n) => n.id)).toContain("child_2");
  });

  it("contracts filtered intermediate nodes and counts contracted entities", () => {
    // Structure:
    // root -> rel_1 (has_part) -> hidden_unit -> rel_2 (has_part) -> visible_unit
    const root = makeEntity("root", "SourceSemanticUnit", "Root Unit");
    const rel1 = makeRelation("rel_1", "has_part", "has part 1");
    const hiddenUnit = makeEntity("hidden_unit", "SourceSemanticUnit", "Hidden Unit");
    const rel2 = makeRelation("rel_2", "has_part", "has part 2");
    const visibleUnit = makeEntity("visible_unit", "SourceSemanticUnit", "Visible Unit");

    const graph: IncidenceGraph = {
      schema_version: "mathscape.graph-projection.v1",
      lens: "source",
      nodes: [root, rel1, hiddenUnit, rel2, visibleUnit],
      edges: [
        makeEdge("e1", "root", "rel_1", "rel_1", "into_relation"),
        makeEdge("e2", "rel_1", "hidden_unit", "rel_1", "out_of_relation"),
        makeEdge("e3", "hidden_unit", "rel_2", "rel_2", "into_relation"),
        makeEdge("e4", "rel_2", "visible_unit", "rel_2", "out_of_relation"),
      ],
    };

    const result = focusSubgraph(graph, "root", {
      depth: "all",
      isEntityVisible: (node) => node.id !== "hidden_unit",
    });

    // Hidden node should be contracted
    expect(result.contractedCount).toBe(1);
    expect(result.reachedCount).toBe(3); // root, hidden_unit, visible_unit
    const nodeIds = result.graph.nodes.map((n) => n.id);
    expect(nodeIds).toContain("root");
    expect(nodeIds).toContain("visible_unit");
    expect(nodeIds).not.toContain("hidden_unit");

    // There should be a synthetic contraction relation bridging root and visible_unit
    const contractionRel = result.graph.nodes.find(
      (n) => n.domain_kind === FOCUS_CONTRACTION_KIND,
    );
    expect(contractionRel).toBeDefined();
  });
});
