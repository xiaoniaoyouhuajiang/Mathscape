import { describe, expect, it } from "vitest";
import type { IncidenceEdge, IncidenceGraph, IncidenceNode, RelationKind } from "../../src/graph/projection";
import {
  buildStudyNodeBriefing,
  buildStudyOverviewPosture,
  buildStudySpinePosture,
  computeSpineDepths,
  relaxVerticalOrbit,
} from "../../src/graph/study-posture";
import type { LearningMaterial, LearningView } from "../../src/semantic/api";

function entity(id: string, domainId: string, domainKind: IncidenceNode["domain_kind"], label: string, summary: string | null = null): IncidenceNode {
  return {
    id,
    element_kind: "entity",
    domain_id: domainId,
    domain_kind: domainKind,
    relation_kind: null,
    layer: domainKind === "SemanticEntity" ? "global_semantic" : "source",
    label,
    summary,
    lifecycle: "active",
    parent: null,
    boundary: false,
  };
}

function relation(id: string, kind: RelationKind, label: string, domainKind: IncidenceNode["domain_kind"] = "SourceUnitRelation"): IncidenceNode {
  return {
    id,
    element_kind: "relation",
    domain_id: `dom:${id}`,
    domain_kind: domainKind,
    relation_kind: kind,
    layer: "source",
    label,
    summary: null,
    lifecycle: "active",
    parent: null,
    boundary: false,
  };
}

function pair(relId: string, from: string, to: string): IncidenceEdge[] {
  return [
    { id: `${relId}:in`, source: from, target: relId, relation_id: `dom:${relId}`, role: "source", ordinal: 0, direction: "into_relation", directed: true },
    { id: `${relId}:out`, source: relId, target: to, relation_id: `dom:${relId}`, role: "target", ordinal: 1, direction: "out_of_relation", directed: true },
  ];
}

describe("Study Spine-and-Orbit Posture & Briefing", () => {
  it("computes cycle-safe spine indentation depths capped at 4", () => {
    const depths = computeSpineDepths([
      { unit_id: "u0", unit_title: "章", unit_kind: "section", parent_unit: null, study_status: "unread", category: "unaligned", targets: [] },
      { unit_id: "u1", unit_title: "节", unit_kind: "section", parent_unit: "u0", study_status: "unread", category: "unaligned", targets: [] },
      { unit_id: "u2", unit_title: "定理", unit_kind: "statement", parent_unit: "u1", study_status: "unread", category: "fresh", targets: [] },
      // Cycle guard
      { unit_id: "cycA", unit_title: "A", unit_kind: "statement", parent_unit: "cycB", study_status: "unread", category: "unaligned", targets: [] },
      { unit_id: "cycB", unit_title: "B", unit_kind: "statement", parent_unit: "cycA", study_status: "unread", category: "unaligned", targets: [] },
    ]);
    expect(depths.get("u0")).toBe(0);
    expect(depths.get("u1")).toBe(1);
    expect(depths.get("u2")).toBe(2);
    expect(depths.get("cycA")).toBeGreaterThanOrEqual(0);
    expect(depths.get("cycB")).toBeGreaterThanOrEqual(0);
  });

  it("relaxes right-orbit vertical coordinates while preserving order and minimum gap", () => {
    const relaxed = relaxVerticalOrbit(
      [
        { id: "s1", idealY: 100 },
        { id: "s2", idealY: 110 },
        { id: "s3", idealY: 115 },
      ],
      90,
    );
    const y1 = relaxed.get("s1")!;
    const y2 = relaxed.get("s2")!;
    const y3 = relaxed.get("s3")!;
    expect(y2 - y1).toBeGreaterThanOrEqual(89);
    expect(y3 - y2).toBeGreaterThanOrEqual(89);
  });

  it("lays out left spine, staggered left Bezier reasoning arcs, right semantic orbit, spotlight signals, and briefing", () => {
    const graph: IncidenceGraph = {
      schema_version: "mathscape.graph-projection.v1",
      lens: "landscape",
      nodes: [
        entity("n-book", "book-1", "SourceArtifact", "母函数讲义 · 规范解析正文"),
        entity("n-other-book", "book-2", "SourceArtifact", "无关讲义.md"),
        entity("n-other-unit", "u-other", "SourceSemanticUnit", "无关定理"),
        entity("n-sec", "u-sec", "SourceSemanticUnit", "二、形式幂级数"),
        entity("n-def", "u-def", "SourceSemanticUnit", "定义 1：母函数"),
        entity("n-thm", "u-thm", "SourceSemanticUnit", "定理 1：展开式"),
        entity("n-pf", "u-pf", "SourceSemanticUnit", "定理 1 的证明"),
        entity("n-scope", "sc-1", "DialectScope", "第二章形式幂级数约定"),
        entity("n-sem-gf", "sem-gf", "SemanticEntity", "概念：母函数", "概念"),
        entity("n-sem-thm", "sem-thm", "SemanticEntity", "精确说法：定理 1", "精确说法"),
        entity("n-theory", "sem-th", "SemanticEntity", "理论：组合计数与母函数", "理论"),
        entity("n-debt", "debt-1", "Debt", "一笔账：核对自然数起点"),
        relation("r-hp1", "has_part", "包含"),
        relation("r-hp2", "has_part", "包含"),
        relation("r-proves", "proves", "证明"),
        relation("r-method", "applies_method", "应用方法"),
        relation("r-scope", "scope_of", "约定适用于", "DialectScope"),
        relation("r-al1", "aligns", "说的就是它", "Alignment"),
        relation("r-al2", "aligns", "部分重合：缺少收敛边界", "Alignment"),
        relation("r-th1", "in_theory", "属于理论", "SemanticEntity"),
        relation("r-th2", "in_theory", "属于理论", "SemanticEntity"),
        relation("r-debt", "debt_on", "挂账", "Debt"),
      ],
      edges: [
        ...pair("r-hp1", "n-sec", "n-def"),
        ...pair("r-hp2", "n-sec", "n-thm"),
        ...pair("r-proves", "n-pf", "n-thm"),
        ...pair("r-method", "n-pf", "n-def"),
        ...pair("r-scope", "n-scope", "n-sec"),
        ...pair("r-al1", "n-def", "n-sem-gf"),
        ...pair("r-al2", "n-thm", "n-sem-thm"),
        ...pair("r-th1", "n-sem-gf", "n-theory"),
        ...pair("r-th2", "n-sem-thm", "n-theory"),
        ...pair("r-debt", "n-debt", "n-sem-thm"),
      ],
    };

    const basePositions = new Map(
      graph.nodes.filter((n) => n.element_kind === "entity").map((n, i) => [n.id, { x: i * 50, y: i * -30 }]),
    );

    const material: LearningMaterial = {
      artifact_id: "book-1",
      units: [],
      new_conventions: [{ scope_id: "sc-1", scope_label: "约定", text: "不讨论收敛" }],
      open_debts: [{ debt_id: "debt-1", obligation: "核对自然数起点", status: "open" }],
      outline_units: [
        { unit_id: "u-sec", unit_title: "二、形式幂级数", unit_kind: "section", parent_unit: null, study_status: "mastered", category: "unaligned", targets: [] },
        {
          unit_id: "u-def",
          unit_title: "定义 1：母函数",
          unit_kind: "definition",
          parent_unit: "u-sec",
          study_status: "studying",
          category: "fresh",
          targets: [{ alignment_id: "dom:r-al1", entity_id: "sem-gf", entity_label: "概念：母函数", relation: "expresses", only_differs_by: [], seen_in: [] }],
        },
        {
          unit_id: "u-thm",
          unit_title: "定理 1：展开式",
          unit_kind: "statement",
          parent_unit: "u-sec",
          study_status: "unread",
          category: "fresh",
          targets: [{ alignment_id: "dom:r-al2", entity_id: "sem-thm", entity_label: "精确说法：定理 1", relation: "overlaps_with", only_differs_by: ["缺少收敛说明"], seen_in: [] }],
        },
        { unit_id: "u-pf", unit_title: "定理 1 的证明", unit_kind: "proof", parent_unit: "u-thm", study_status: "unread", category: "unaligned", targets: [] },
      ],
    };

    const view: LearningView = { materials: [material] };
    const overview = buildStudyOverviewPosture(graph, basePositions, view);
    expect(overview.inScopeNodeIds?.has("n-def")).toBe(true);
    expect(overview.inScopeNodeIds?.has("n-sem-gf")).toBe(true);
    expect(overview.inScopeNodeIds?.has("n-book")).toBe(false);

    const spine = buildStudySpinePosture(graph, basePositions, material, "novel_and_diff");
    // Unrelated material is excluded from scope
    expect(spine.inScopeNodeIds?.has("n-other-unit")).toBe(false);
    // Monotonic Y down the spine & indentation X by tree depth
    const pSec = spine.positions.get("n-sec")!;
    const pDef = spine.positions.get("n-def")!;
    const pThm = spine.positions.get("n-thm")!;
    const pPf = spine.positions.get("n-pf")!;
    expect(pSec.y).toBeLessThan(pDef.y);
    expect(pDef.y).toBeLessThan(pThm.y);
    expect(pThm.y).toBeLessThan(pPf.y);
    expect(pSec.x).toBeLessThan(pDef.x);
    expect(pThm.x).toBeLessThan(pPf.x);

    // DialectScope on left margin, SemanticEntity on right orbit, Theory & Debt on outer right orbit
    expect(spine.positions.get("n-scope")!.x).toBeLessThan(pSec.x);
    expect(spine.positions.get("n-sem-gf")!.x).toBeGreaterThan(pPf.x);
    expect(spine.positions.get("n-theory")!.x).toBeGreaterThan(spine.positions.get("n-sem-gf")!.x);
    expect(spine.positions.get("n-debt")!.x).toBeGreaterThan(spine.positions.get("n-sem-thm")!.x);

    // Staggered leftward Bezier arcs for `r-proves` and `r-method` so parallel arcs do not collide
    const arcProves = spine.spineArcOffsets.get("dom:r-proves")!;
    const arcMethod = spine.spineArcOffsets.get("dom:r-method")!;
    expect(arcProves.side).toBe("left");
    expect(arcMethod.side).toBe("left");
    expect(Math.abs(arcProves.magnitude - arcMethod.magnitude)).toBeGreaterThanOrEqual(30);

    // Non-destructive spotlight signal: all spine units remain in scope; hit vs muted classes distinguish them
    expect(spine.inScopeNodeIds?.has("n-sec")).toBe(true);
    expect(spine.nodeClasses.get("n-thm")).toContain("is-study-signal-hit");
    expect(spine.nodeClasses.get("n-sec")).toContain("is-study-signal-muted");
    expect(spine.nodeClasses.get("n-sec")).toContain("study-status-mastered");

    // Hover briefing extracts diffs, 1-hop intra-book neighbors, and attached open debts
    const briefing = buildStudyNodeBriefing("n-thm", graph, view, (id) => id);
    expect(briefing).not.toBeNull();
    expect(briefing!.alignmentTone).toBe("diff");
    expect(briefing!.targets[0].diffs).toEqual(["缺少收敛说明"]);
    expect(briefing!.intraBookNeighbors.some((nb) => nb.label === "定理 1 的证明")).toBe(true);
    expect(briefing!.openDebts).toHaveLength(1);
  });
});
