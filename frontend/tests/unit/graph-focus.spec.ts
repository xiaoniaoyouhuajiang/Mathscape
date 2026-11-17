import { describe, expect, it } from "vitest";
import { focusSubgraph, FOCUS_CONTRACTION_KIND } from "../../src/graph/focus";
import {
  type IncidenceEdge,
  type IncidenceGraph,
  type IncidenceNode,
  type RelationKind,
} from "../../src/graph/projection";

describe("graph focus algorithm", () => {
  function makeEntity(
    id: string,
    domainKind: string,
    label: string,
    summary: string | null = null,
  ): IncidenceNode {
    return {
      id,
      element_kind: "entity",
      domain_id: `domain:${id}`,
      domain_kind: domainKind,
      relation_kind: null,
      layer: "source",
      label,
      summary,
      lifecycle: "active",
      parent: null,
      boundary: false,
    };
  }

  function makeRelation(
    id: string,
    relationKind: RelationKind,
    label: string,
    domainKind = "SourceUnitRelation",
  ): IncidenceNode {
    return {
      id,
      element_kind: "relation",
      domain_id: `rel:${id}`,
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

  it("supports semantic roots and 2-hop cross-book bridges with Theory & foreign-book hub sinks", () => {
    const bookA = makeEntity("book_a", "SourceArtifact", "Book A.md", "text/markdown");
    const spanA = makeEntity("span_a", "SourceSpan", "Span A");
    const secA = makeEntity("sec_a", "SourceSemanticUnit", "§1");
    const thmA = makeEntity("thm_a", "SourceSemanticUnit", "Theorem A");
    const scopeA = makeEntity("scope_a", "DialectScope", "Scope A");

    const bookB = makeEntity("book_b", "SourceArtifact", "Book B.md", "text/markdown");
    const spanB = makeEntity("span_b", "SourceSpan", "Span B");
    const spanBOther = makeEntity("span_b_other", "SourceSpan", "Span B Other");
    const secB = makeEntity("sec_b", "SourceSemanticUnit", "§2");
    const thmB = makeEntity("thm_b", "SourceSemanticUnit", "Theorem B");
    const otherB = makeEntity("other_b", "SourceSemanticUnit", "Unrelated Problem B");

    const theory = makeEntity("theory", "SemanticEntity", "线性递推数列", "理论");
    const cassini = makeEntity("cassini", "SemanticEntity", "Cassini 恒等式", "精确说法");
    const qpow = makeEntity("qpow", "SemanticEntity", "矩阵幂引理", "精确说法");
    const unrelatedEntity = makeEntity("unrelated_ent", "SemanticEntity", "Binet 公式", "精确说法");
    const debt = makeEntity("debt_1", "Debt", "一笔账：代入核对");

    const relations = [
      makeRelation("loc_a", "located_in", "位于"),
      makeRelation("cs_a", "contains_span", "对应原文"),
      makeRelation("hp_a", "has_part", "包含"),
      makeRelation("sc_a", "scope_of", "约定适用于", "DialectScope"),
      makeRelation("loc_b", "located_in", "位于"),
      makeRelation("cs_b", "contains_span", "对应原文"),
      makeRelation("loc_b2", "located_in", "位于"),
      makeRelation("cs_b2", "contains_span", "对应原文"),
      makeRelation("hp_b", "has_part", "包含"),
      makeRelation("hp_b2", "has_part", "包含"),
      makeRelation("al_a", "aligns", "特例", "Alignment"),
      makeRelation("al_b", "aligns", "说的就是它", "Alignment"),
      makeRelation("th_c", "in_theory", "属于理论", "SemanticEntity"),
      makeRelation("th_q", "in_theory", "属于理论", "SemanticEntity"),
      makeRelation("th_u", "in_theory", "属于理论", "SemanticEntity"),
      makeRelation("uses_q", "uses_entity", "引用对象", "SemanticEntity"),
      makeRelation("debt_on_c", "debt_on", "挂账", "Debt"),
    ];

    const pair = (rel: string, from: string, to: string): IncidenceEdge[] => [
      makeEdge(`${rel}_in`, from, rel, `rel:${rel}`, "into_relation"),
      makeEdge(`${rel}_out`, rel, to, `rel:${rel}`, "out_of_relation"),
    ];

    const graph: IncidenceGraph = {
      schema_version: "mathscape.graph-projection.v1",
      lens: "landscape",
      nodes: [
        bookA, spanA, secA, thmA, scopeA,
        bookB, spanB, spanBOther, secB, thmB, otherB,
        theory, cassini, qpow, unrelatedEntity, debt,
        ...relations,
      ],
      edges: [
        ...pair("loc_a", "span_a", "book_a"),
        ...pair("cs_a", "thm_a", "span_a"),
        ...pair("hp_a", "sec_a", "thm_a"),
        ...pair("sc_a", "scope_a", "sec_a"),
        ...pair("loc_b", "span_b", "book_b"),
        ...pair("cs_b", "thm_b", "span_b"),
        ...pair("loc_b2", "span_b_other", "book_b"),
        ...pair("cs_b2", "other_b", "span_b_other"),
        ...pair("hp_b", "sec_b", "thm_b"),
        ...pair("hp_b2", "sec_b", "other_b"),
        ...pair("al_a", "thm_a", "cassini"),
        ...pair("al_b", "thm_b", "cassini"),
        ...pair("th_c", "cassini", "theory"),
        ...pair("th_q", "qpow", "theory"),
        ...pair("th_u", "unrelated_ent", "theory"),
        ...pair("uses_q", "cassini", "qpow"),
        ...pair("debt_on_c", "debt_1", "cassini"),
      ],
    };

    const visibleWithoutSpans = (n: IncidenceNode) => n.domain_kind !== "SourceSpan";

    // 1. Focusing `thm_a` at depth 2 bridges across `cassini` to `thm_b` (and contracts `span_b` to `book_b`),
    //    reaches `qpow`, `debt_1`, `scope_a`, and `theory`, while blocking `unrelated_ent` and `other_b`.
    const fromThmA = focusSubgraph(graph, "thm_a", {
      depth: 2,
      lens: "heuristic",
      isEntityVisible: visibleWithoutSpans,
    });
    const idsFromThmA = fromThmA.graph.nodes.map((n) => n.id);
    for (const expected of ["thm_a", "book_a", "scope_a", "cassini", "theory", "debt_1", "qpow", "thm_b", "book_b"]) {
      expect(idsFromThmA).toContain(expected);
    }
    expect(idsFromThmA).not.toContain("unrelated_ent");
    expect(idsFromThmA).not.toContain("other_b");
    expect(fromThmA.graph.nodes.some((n) => n.domain_kind === FOCUS_CONTRACTION_KIND)).toBe(true);

    // 2. Focusing directly on semantic node `cassini` at depth 1 reaches both books' theorems, `qpow`, `debt_1`, and `theory`.
    const fromCassini = focusSubgraph(graph, "cassini", {
      depth: 1,
      lens: "heuristic",
      isEntityVisible: visibleWithoutSpans,
    });
    const idsFromCassini = fromCassini.graph.nodes.map((n) => n.id);
    for (const expected of ["cassini", "thm_a", "thm_b", "qpow", "debt_1", "theory"]) {
      expect(idsFromCassini).toContain(expected);
    }
    expect(idsFromCassini).not.toContain("unrelated_ent");

    // 3. Focusing directly on `theory` lists all shelf members (`cassini`, `qpow`, `unrelated_ent`) at depth 1.
    const fromTheory = focusSubgraph(graph, "theory", {
      depth: 1,
      lens: "heuristic",
      isEntityVisible: visibleWithoutSpans,
    });
    const idsFromTheory = fromTheory.graph.nodes.map((n) => n.id);
    for (const member of ["theory", "cassini", "qpow", "unrelated_ent"]) {
      expect(idsFromTheory).toContain(member);
    }

    // 4. `semantic_only` lens from `cassini` excludes textbook artifacts and source units.
    const semOnly = focusSubgraph(graph, "cassini", {
      depth: 2,
      lens: "semantic_only",
      isEntityVisible: visibleWithoutSpans,
    });
    const semIds = semOnly.graph.nodes.map((n) => n.id);
    expect(semIds).toContain("cassini");
    expect(semIds).toContain("qpow");
    expect(semIds).toContain("debt_1");
    expect(semIds).not.toContain("thm_a");
    expect(semIds).not.toContain("book_a");
  });

  it("suppresses redundant 派生自 spiderweb lines from a non-seed SourceArtifact to 2-hop siblings and child units", () => {
    const book = makeEntity("book", "SourceArtifact", "母函数讲义.md", "text/markdown");
    const sSec = makeEntity("s_sec", "SourceSpan", "S0");
    const sThm = makeEntity("s_thm", "SourceSpan", "S1");
    const sProof = makeEntity("s_pf", "SourceSpan", "S2");
    const sDef = makeEntity("s_def", "SourceSpan", "S3");
    const sec = makeEntity("sec", "SourceSemanticUnit", "§2");
    const thm = makeEntity("thm", "SourceSemanticUnit", "定理 1");
    const proof = makeEntity("pf", "SourceSemanticUnit", "定理 1 的证明");
    const def5 = makeEntity("def5", "SourceSemanticUnit", "定义 5");

    const rLoc0 = makeRelation("r_l0", "located_in", "位于", "Evidence");
    const rLoc1 = makeRelation("r_l1", "located_in", "位于", "Evidence");
    const rLoc2 = makeRelation("r_l2", "located_in", "位于", "Evidence");
    const rLoc3 = makeRelation("r_l3", "located_in", "位于", "Evidence");
    const rSpan0 = makeRelation("r_s0", "contains_span", "范围", "Evidence");
    const rSpan1 = makeRelation("r_s1", "contains_span", "范围", "Evidence");
    const rSpan2 = makeRelation("r_s2", "contains_span", "范围", "Evidence");
    const rSpan3 = makeRelation("r_s3", "contains_span", "范围", "Evidence");
    const rPart = makeRelation("r_part", "has_part", "包含");
    const rProves = makeRelation("r_proves", "proves", "证明");
    const rMethod = makeRelation("r_method", "applies_method", "应用方法");

    const graph: IncidenceGraph = {
      schema_version: "mathscape.graph-projection.v1",
      lens: "landscape",
      nodes: [
        book, sSec, sThm, sProof, sDef, sec, thm, proof, def5,
        rLoc0, rLoc1, rLoc2, rLoc3, rSpan0, rSpan1, rSpan2, rSpan3, rPart, rProves, rMethod,
      ],
      edges: [
        makeEdge("el0a", "s_sec", "r_l0", "r_l0", "into_relation"),
        makeEdge("el0b", "r_l0", "book", "r_l0", "out_of_relation"),
        makeEdge("el1a", "s_thm", "r_l1", "r_l1", "into_relation"),
        makeEdge("el1b", "r_l1", "book", "r_l1", "out_of_relation"),
        makeEdge("el2a", "s_pf", "r_l2", "r_l2", "into_relation"),
        makeEdge("el2b", "r_l2", "book", "r_l2", "out_of_relation"),
        makeEdge("el3a", "s_def", "r_l3", "r_l3", "into_relation"),
        makeEdge("el3b", "r_l3", "book", "r_l3", "out_of_relation"),

        makeEdge("es0a", "sec", "r_s0", "r_s0", "into_relation"),
        makeEdge("es0b", "r_s0", "s_sec", "r_s0", "out_of_relation"),
        makeEdge("es1a", "thm", "r_s1", "r_s1", "into_relation"),
        makeEdge("es1b", "r_s1", "s_thm", "r_s1", "out_of_relation"),
        makeEdge("es2a", "pf", "r_s2", "r_s2", "into_relation"),
        makeEdge("es2b", "r_s2", "s_pf", "r_s2", "out_of_relation"),
        makeEdge("es3a", "def5", "r_s3", "r_s3", "into_relation"),
        makeEdge("es3b", "r_s3", "s_def", "r_s3", "out_of_relation"),

        makeEdge("ep1", "sec", "r_part", "r_part", "into_relation"),
        makeEdge("ep2", "r_part", "thm", "r_part", "out_of_relation"),
        makeEdge("epr1", "pf", "r_proves", "r_proves", "into_relation"),
        makeEdge("epr2", "r_proves", "thm", "r_proves", "out_of_relation"),
        makeEdge("em1", "def5", "r_method", "r_method", "into_relation"),
        makeEdge("em2", "r_method", "thm", "r_method", "out_of_relation"),
      ],
    };

    const res = focusSubgraph(graph, "thm", {
      depth: 2,
      lens: "heuristic",
      isEntityVisible: (n) => n.domain_kind !== "SourceSpan",
    });
    const contractedRels = res.graph.nodes.filter((n) => n.domain_kind === FOCUS_CONTRACTION_KIND);
    // Only `sec` (top-level section at depth 0) gets `派生自` from `book` — NOT `thm` (has visible parent `sec`), `pf`, or `def5`!
    expect(contractedRels).toHaveLength(1);
    expect(contractedRels[0].id).toContain("book~sec");
  });
});
