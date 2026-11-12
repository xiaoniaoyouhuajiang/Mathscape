import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { toCytoscapeElements } from "../../src/graph/cytoscape";
import { alignmentPlainLabel } from "../../src/semantic/labels";
import { TUTORIAL_PROJECTION_SCHEMA, tutorialGraph, type TutorialProjection } from "../../src/tutorial/projection";
import { buildTutorialShots, fib, shotText } from "../../src/tutorial/script";

const projection = JSON.parse(
  readFileSync(resolve(__dirname, "../../../fixtures/emir/fibonacci-two-books/tutorial-projection.json"), "utf8"),
) as TutorialProjection;
const BANNED = ["平移", "错开", "滑动"];

describe("tutorial projection (fixtures/emir/fibonacci-two-books)", () => {
  it("is the read-only projection of the six accepted change sets", () => {
    expect(projection.schema).toBe(TUTORIAL_PROJECTION_SCHEMA);
    expect(projection.label).toContain("虚构教材");
    const kinds = Object.values(projection.objects).map((o) => o.kind);
    expect(kinds.filter((k) => k === "alignment")).toHaveLength(9);
    expect(kinds.filter((k) => k === "debt")).toHaveLength(4);
    expect(kinds.filter((k) => k === "dialect_scope")).toHaveLength(2);
    expect(kinds.filter((k) => k === "semantic_entity")).toHaveLength(6);
  });

  it("keeps agent statuses and never raises trust on accept", () => {
    for (const object of Object.values(projection.objects)) {
      const p = object.payload as { trust?: { proof?: string; alignment?: string }; status?: string; mathlib_baseline?: { status?: string } | null };
      if (object.kind === "alignment") {
        expect(p.trust?.proof).toBe("none");
        expect(["retrieved_candidate", "context_checked", "partially_aligned"]).toContain(p.status);
      }
      if (object.kind === "debt") expect(p.status).toBe("open");
      if (object.kind === "semantic_entity" && p.mathlib_baseline) expect(p.mathlib_baseline.status).toBe("unchecked");
    }
  });

  it("labels every alignment exactly like the frontend template (Rust ↔ TS parity)", () => {
    const relations = projection.graph.nodes.filter((n) => n.element_kind === "relation" && n.domain_kind === "Alignment");
    expect(relations).toHaveLength(9);
    for (const node of relations) {
      const object = Object.values(projection.objects).find((o) => o.id === node.domain_id)!;
      expect(node.label).toBe(alignmentPlainLabel(object.payload));
    }
    const all = projection.graph.nodes.map((n) => `${n.label} ${n.summary ?? ""}`).join("\n");
    for (const word of BANNED) expect(all).not.toContain(word);
    expect(all).toContain("特例：乙的第 n 条 ＝ 共享的第 n+1 条");
    expect(all).toContain("部分重合：乙没有第 0 项");
  });

  it("renders on the main-map stylesheet without source spans", () => {
    const graph = tutorialGraph(projection);
    expect(graph.nodes.some((n) => n.domain_kind === "SourceSpan")).toBe(false);
    const elements = toCytoscapeElements(graph, "semantic");
    const types = new Set(elements.map((e) => e.data.nodeType).filter(Boolean));
    for (const type of ["shared_entity", "shared_theory", "debt", "dialect_scope", "semantic_unit"]) expect(types).toContain(type);
    expect(elements.some((e) => String(e.classes ?? "").includes("alignment-edge"))).toBe(true);
  });
});

describe("tutorial script", () => {
  const shots = buildTutorialShots(projection);

  it("has the eleven storyboard shots, every reveal resolvable", () => {
    expect(shots.map((s) => s.id)).toEqual([
      "teaser", "spans", "units", "same-star", "index-zero", "cassini", "matrix", "no-merge", "plain-labels", "debts", "accept-and-learn",
    ]);
    expect(shots.reduce((sum, s) => sum + s.seconds, 0)).toBe(112);
    for (const shot of shots) {
      for (const local of [...shot.reveal, ...shot.focus]) {
        const resolved = local.startsWith("artifact:") ? projection.artifacts[local.slice(9)] : projection.refs[local];
        expect(resolved, `${shot.id}: ${local}`).toBeTruthy();
      }
    }
  });

  it("never uses the banned words or leaks raw data", () => {
    for (const shot of shots) {
      for (const line of shotText(shot)) {
        for (const word of BANNED) expect(line, shot.id).not.toContain(word);
        expect(line, shot.id).not.toMatch(/mathscape:|[{}]{2}|undefined|\[object/);
      }
    }
  });

  it("derives the numbers of shots 5 and 6 from the definition", () => {
    expect([0, 1, 2, 3, 4, 5, 6].map(fib)).toEqual([0, 1, 1, 2, 3, 5, 8]);
    const table = shots[4].extra!;
    expect(table.kind).toBe("sequence");
    if (table.kind === "sequence") expect(table.rows[1].cells[0]).toBeNull();
    const numbering = shots[5].extra!;
    expect(numbering.kind).toBe("numbering");
    if (numbering.kind === "numbering") {
      expect(numbering.shift).toBe(1);
      expect(numbering.sequenceNote).toBe("数列没有动：f_k = F_k");
      expect(numbering.shared.map((s) => s.text)).toEqual(["共享第 1 条 = -1", "共享第 2 条 = 1", "共享第 3 条 = -1", "共享第 4 条 = 1"]);
      expect(numbering.yi.map((s) => s.text)).toEqual(["乙第 1 条 = 1", "乙第 2 条 = -1", "乙第 3 条 = 1"]);
    }
  });

  it("ends with accept ≠ proof and the learning labels of 乙", () => {
    const last = shots[10].extra!;
    expect(last.kind).toBe("learning");
    if (last.kind === "learning") {
      expect(last.banner).toBe("接受只表示写入地图，不代表数学上已验证");
      expect(last.badge).toBe("可信度：候选");
      expect(last.units.some((u) => u.label === "别的材料里见过")).toBe(true);
      expect(last.units.some((u) => u.label === "新的")).toBe(true);
      expect(last.summary).toMatch(/^新的 \d+ · 别的材料里见过 \d+ · 新约定 2 · 未还清的账 4$/);
    }
  });
});
