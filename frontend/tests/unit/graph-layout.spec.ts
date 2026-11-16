import cytoscape from "cytoscape";
import { describe, expect, it } from "vitest";
import { syncCytoscapeGraph, toCytoscapeElements } from "../../src/graph/cytoscape";
import {
  collapseRedundantConversions,
  computeLandscapeLayout,
  isAuxiliaryArtifactNode,
  resolveIncrementalPositions,
} from "../../src/graph/layout";
import type { IncidenceEdge, IncidenceGraph, IncidenceNode, RelationKind } from "../../src/graph/projection";

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

function relation(
  id: string,
  domainKind: string,
  relationKind: RelationKind,
  label: string,
): IncidenceNode {
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

function binary(
  edges: IncidenceEdge[],
  rel: IncidenceNode,
  from: IncidenceNode,
  to: IncidenceNode,
  fromRole = "source",
  toRole = "target",
): void {
  const rid = rel.domain_id ?? rel.id;
  edges.push(
    {
      id: `e:${rid}:in:${from.id}`,
      source: from.id,
      target: rel.id,
      relation_id: rid,
      role: fromRole,
      ordinal: null,
      direction: "into_relation",
      directed: true,
    },
    {
      id: `e:${rid}:out:${to.id}`,
      source: rel.id,
      target: to.id,
      relation_id: rid,
      role: toRole,
      ordinal: null,
      direction: "out_of_relation",
      directed: true,
    },
  );
}

function buildOrganicEmirFixture(): IncidenceGraph {
  const nodes: IncidenceNode[] = [];
  const edges: IncidenceEdge[] = [];

  const pdfA = entity("art:pdfA", "SourceArtifact", "母函数-史济怀.pdf", "application/pdf · 1024 bytes · available");
  const mdA = entity("art:mdA", "SourceArtifact", "母函数 · 规范解析正文", "text/markdown · canonical · complete");
  const metaA = entity("art:metaA", "SourceArtifact", "母函数 · PDF 1-8 页 · 解析元数据", "application/json · 200 bytes · available");
  const imgA = entity("art:imgA", "SourceArtifact", "母函数 · 解析图片", "image/jpeg · 200 bytes · available");
  const secA = entity("u:secA", "SourceSemanticUnit", "二、形式幂级数及其运算", "section");
  const defA = entity("u:defA", "SourceSemanticUnit", "定义 1：无穷数列的母函数", "definition");
  const thmA = entity("u:thmA", "SourceSemanticUnit", "定理 1：(1-x)^(-n) 的展开", "statement");
  const pfA = entity("u:pfA", "SourceSemanticUnit", "定理 1 的证明", "proof");
  const spA1 = entity("sp:a1", "SourceSpan", "来源范围", "Markdown 字节范围");
  const spA2 = entity("sp:a2", "SourceSpan", "来源范围", "Markdown 字节范围");
  const scopeA = entity("ds:a", "DialectScope", "形式幂级数约定", "约定卡");

  const pdfB = entity("art:pdfB", "SourceArtifact", "数学分析典型问题.pdf", "application/pdf · 1024 bytes · available");
  const mdB = entity("art:mdB", "SourceArtifact", "数学分析 · 规范解析正文", "text/markdown · canonical · complete");
  const secB = entity("u:secB", "SourceSemanticUnit", "第五章 级数", "section");
  const exB = entity("u:exB", "SourceSemanticUnit", "例 5.2.3 幂级数求和", "example");
  const spB1 = entity("sp:b1", "SourceSpan", "来源范围", "Markdown 字节范围");

  const pdfC = entity("art:pdfC", "SourceArtifact", "高等代数.pdf", "application/pdf · 1024 bytes · available");
  const mdC = entity("art:mdC", "SourceArtifact", "高等代数 · 规范解析正文", "text/markdown · canonical · complete");
  const secC = entity("u:secC", "SourceSemanticUnit", "§3 Euclid 空间", "section");
  const propC = entity("u:propC", "SourceSemanticUnit", "命题 3.1.1 线性流形", "statement");
  const spC1 = entity("sp:c1", "SourceSpan", "来源范围", "Markdown 字节范围");

  const theory = entity("sem:theory", "SemanticEntity", "理论：组合计数与母函数", "理论");
  const concept = entity("sem:gf", "SemanticEntity", "概念：母函数", "概念");
  const decl = entity("sem:thm1", "SemanticEntity", "精确说法：定理 1", "精确说法");
  const debt = entity("debt:1", "Debt", "一笔账：确认自然数是否含 0", "还欠着");

  nodes.push(
    pdfA, mdA, metaA, imgA, secA, defA, thmA, pfA, spA1, spA2, scopeA,
    pdfB, mdB, secB, exB, spB1,
    pdfC, mdC, secC, propC, spC1,
    theory, concept, decl, debt,
  );

  const addRel = (id: string, dk: string, rk: RelationKind, lbl: string, a: IncidenceNode, b: IncidenceNode) => {
    const r = relation(id, dk, rk, lbl);
    nodes.push(r);
    binary(edges, r, a, b);
  };

  addRel("conv:a1", "ConversionRun", "conversion", "解析 1-8", pdfA, mdA);
  addRel("conv:a1_dup", "ConversionRun", "conversion", "解析 9-16", pdfA, mdA);
  addRel("conv:a2", "ConversionRun", "conversion", "解析元数据", pdfA, metaA);
  addRel("conv:a3", "ConversionRun", "conversion", "解析图片", pdfA, imgA);
  addRel("conv:b1", "ConversionRun", "conversion", "解析", pdfB, mdB);
  addRel("conv:c1", "ConversionRun", "conversion", "解析", pdfC, mdC);

  addRel("loc:a1", "Evidence", "located_in", "来源于 Markdown", spA1, mdA);
  addRel("loc:a2", "Evidence", "located_in", "来源于 Markdown", spA2, mdA);
  addRel("cs:secA", "SourceUnitRelation", "contains_span", "对应原文", secA, spA1);
  addRel("cs:defA", "SourceUnitRelation", "contains_span", "对应原文", defA, spA1);
  addRel("cs:thmA", "SourceUnitRelation", "contains_span", "对应原文", thmA, spA2);
  addRel("cs:pfA", "SourceUnitRelation", "contains_span", "对应原文", pfA, spA2);
  addRel("hp:defA", "SourceUnitRelation", "has_part", "包含", secA, defA);
  addRel("hp:thmA", "SourceUnitRelation", "has_part", "包含", secA, thmA);
  addRel("prv:pfA", "SourceUnitRelation", "proves", "证明", pfA, thmA);
  addRel("sc:a", "DialectScope", "scope_of", "约定适用于", scopeA, secA);

  addRel("loc:b1", "Evidence", "located_in", "来源于 Markdown", spB1, mdB);
  addRel("cs:secB", "SourceUnitRelation", "contains_span", "对应原文", secB, spB1);
  addRel("cs:exB", "SourceUnitRelation", "contains_span", "对应原文", exB, spB1);
  addRel("hp:exB", "SourceUnitRelation", "has_part", "包含", secB, exB);

  addRel("loc:c1", "Evidence", "located_in", "来源于 Markdown", spC1, mdC);
  addRel("cs:secC", "SourceUnitRelation", "contains_span", "对应原文", secC, spC1);
  addRel("cs:propC", "SourceUnitRelation", "contains_span", "对应原文", propC, spC1);
  addRel("hp:propC", "SourceUnitRelation", "has_part", "包含", secC, propC);

  addRel("it:gf", "SemanticEntity", "in_theory", "属于理论", concept, theory);
  addRel("it:thm1", "SemanticEntity", "in_theory", "属于理论", decl, theory);
  addRel("al:defA", "Alignment", "aligns", "说的就是它", defA, concept);
  addRel("al:exB", "Alignment", "aligns", "部分重合", exB, concept);
  addRel("al:thmA", "Alignment", "aligns", "说的就是它", thmA, decl);
  addRel("dt:1", "Debt", "debt_on", "挂账", debt, decl);

  return {
    schema_version: "mathscape.graph-projection.v1",
    lens: "landscape",
    nodes,
    edges,
  };
}

function filterSubset(full: IncidenceGraph, allowedEntityDomainKinds: Set<string>): IncidenceGraph {
  let nodes = full.nodes.filter((n) => {
    if (n.element_kind === "entity") {
      if (isAuxiliaryArtifactNode(n)) return false;
      if (n.domain_kind === "DialectScope" || n.domain_kind === "Debt") {
        return allowedEntityDomainKinds.has("SemanticEntity");
      }
      return n.domain_kind !== null && allowedEntityDomainKinds.has(n.domain_kind);
    }
    if (n.element_kind === "relation") {
      if (n.domain_kind === "ConversionRun") return allowedEntityDomainKinds.has("SourceArtifact");
      if (n.domain_kind === "SourceUnitRelation") {
        return allowedEntityDomainKinds.has("SourceSemanticUnit") || allowedEntityDomainKinds.has("SourceSpan");
      }
      if (n.domain_kind === "Evidence") {
        return allowedEntityDomainKinds.has("SourceArtifact") && allowedEntityDomainKinds.has("SourceSpan");
      }
      return allowedEntityDomainKinds.has("SemanticEntity");
    }
    return true;
  });
  const ids = new Set(nodes.map((n) => n.id));
  let edges = full.edges.filter((e) => ids.has(e.source) && ids.has(e.target));
  if (allowedEntityDomainKinds.has("SourceArtifact")) {
    const collapsed = collapseRedundantConversions(nodes, edges);
    nodes = collapsed.nodes;
    edges = collapsed.edges;
  }
  return { ...full, nodes, edges };
}

describe("deterministic physics-backed organic layout", () => {
  it("is 100% deterministic across repeated runs and node/edge input permutations", () => {
    const g1 = buildOrganicEmirFixture();
    const g2: IncidenceGraph = {
      ...g1,
      nodes: [...g1.nodes].reverse(),
      edges: [...g1.edges].reverse(),
    };
    const pos1 = computeLandscapeLayout(g1, { width: 1200, height: 800 });
    const pos1Repeat = computeLandscapeLayout(g1, { width: 1200, height: 800 });
    const pos2 = computeLandscapeLayout(g2, { width: 1200, height: 800 });
    expect(pos1).toEqual(pos1Repeat);
    expect(pos1).toEqual(pos2);
  });

  it("keeps shared semantic stars gravitationally nestled between their aligned units and keeps high canvas density", () => {
    const full = buildOrganicEmirFixture();
    const pos = computeLandscapeLayout(full, { width: 1200, height: 800 });

    // Shared concept `sem:gf` sits organically close to both `u:defA` and `u:exB`
    const gf = pos.get("entity:sem:gf")!;
    const defA = pos.get("entity:u:defA")!;
    const exB = pos.get("entity:u:exB")!;
    expect(Math.hypot(gf.x - defA.x, gf.y - defA.y)).toBeLessThan(260);
    expect(Math.hypot(gf.x - exB.x, gf.y - exB.y)).toBeLessThan(260);

    // Debt sits right next to its holder declaration
    const debt = pos.get("entity:debt:1")!;
    const decl = pos.get("entity:sem:thm1")!;
    expect(Math.hypot(debt.x - decl.x, debt.y - decl.y)).toBeLessThan(160);

    // Overall primary bounding box stays compact (not stretched into multi-thousand-px empty space)
    const primaryPts = full.nodes
      .filter((n) => n.element_kind === "entity" && n.domain_kind !== "SourceSpan" && !isAuxiliaryArtifactNode(n))
      .map((n) => pos.get(n.id)!);
    const xs = primaryPts.map((p) => p.x);
    const ys = primaryPts.map((p) => p.y);
    expect(Math.max(...xs) - Math.min(...xs)).toBeLessThan(1200);
    expect(Math.max(...ys) - Math.min(...ys)).toBeLessThan(900);
  });

  it("is strictly order-invariant when hiding all kinds and re-enabling them in any order, with no synthetic spaghetti edges", () => {
    const full = buildOrganicEmirFixture();
    const basePositions = computeLandscapeLayout(full, { width: 1200, height: 800 });

    const runSequence = (order: string[]): { pos: Record<string, { x: number; y: number }>; edgeCount: number } => {
      const core = cytoscape({
        headless: true,
        elements: toCytoscapeElements(filterSubset(full, new Set()), "semantic"),
      });
      try {
        const active = new Set<string>();
        for (const kind of order) {
          active.add(kind);
          syncCytoscapeGraph(core, filterSubset(full, active), "semantic", { positions: basePositions });
        }
        return {
          pos: Object.fromEntries(
            core.nodes().map((n) => [n.id(), { x: Math.round(n.position("x")), y: Math.round(n.position("y")) }]),
          ),
          edgeCount: core.edges().length,
        };
      } finally {
        core.destroy();
      }
    };

    const s1 = runSequence(["SourceArtifact", "SourceSemanticUnit", "SemanticEntity"]);
    const s2 = runSequence(["SemanticEntity", "SourceSemanticUnit", "SourceArtifact"]);
    const s3 = runSequence(["SourceSemanticUnit", "SourceArtifact", "SemanticEntity"]);

    expect(s1.pos).toEqual(s2.pos);
    expect(s2.pos).toEqual(s3.pos);

    // Duplicate conversion run (`conv:a1_dup`) was collapsed into a single PDF -> Markdown edge,
    // and no synthetic book-bridge edges were fabricated!
    const core = cytoscape({
      headless: true,
      elements: toCytoscapeElements(
        filterSubset(full, new Set(["SourceArtifact", "SourceSemanticUnit", "SemanticEntity"])),
        "semantic",
      ),
    });
    try {
      const convEdges = core.edges('[domainKind = "ConversionRun"]');
      expect(convEdges.length).toBe(3); // 1 each for Books A, B, C (conv:a1_dup collapsed, meta/img hidden)
      expect(core.edges(".contracted-edge").length).toBe(0);
    } finally {
      core.destroy();
    }
  });

  it("places newly staged nodes near settled neighbors while keeping settled & user-pinned nodes motionless", () => {
    const base = buildOrganicEmirFixture();
    const settled = computeLandscapeLayout(base);
    const pinned = new Map([["entity:sem:gf", { x: 42, y: -77 }]]);

    const next: IncidenceGraph = structuredClone(base);
    const stagedDebt = entity("debt:staged", "Debt", "一笔账：新提议检查", "还欠着");
    const rel = relation("dt:staged", "Debt", "debt_on", "挂账");
    next.nodes.push(stagedDebt, rel);
    binary(next.edges, rel, stagedDebt, base.nodes.find((n) => n.id === "entity:sem:thm1")!);

    const updated = resolveIncrementalPositions(next, settled, pinned);
    expect(updated.get("entity:sem:gf")).toEqual({ x: 42, y: -77 });
    expect(updated.get("entity:u:thmA")).toEqual(settled.get("entity:u:thmA"));
    const holderPos = updated.get("entity:sem:thm1")!;
    const stagedPos = updated.get("entity:debt:staged")!;
    expect(Math.hypot(stagedPos.x - holderPos.x, stagedPos.y - holderPos.y)).toBeLessThan(200);
  });
});
