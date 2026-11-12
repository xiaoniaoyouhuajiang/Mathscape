import { describe, expect, it, vi } from "vitest";
import { alignmentPlainLabel, entityPlainLabel, reviewItemLabel } from "../../src/semantic/labels";
import { feedbackQuote, sendFeedbackToComposer } from "../../src/semantic/feedback";
import { semanticReviewCards, proposalLevelReviewItems, reviewItemCode } from "../../src/semantic/review-model";
import { materialSummary } from "../../src/semantic/learning-view";
import { recheckSummary } from "../../src/semantic/api";
import { nodeTypeKey, NODE_TYPES } from "../../src/graph/taxonomy";
import { semanticLayerClass } from "../../src/graph/cytoscape";
import { eventBus } from "../../src/core/events";
import type { IncidenceNode } from "../../src/graph/projection";
import type { PendingChangeSet } from "../../src/review/proposal-model";

const BANNED = ["平移", "错开", "滑动"];

describe("semantic plain labels", () => {
  it("follow the S9.1 templates and never use the banned words", () => {
    expect(alignmentPlainLabel({ relation: "specializes", parameter_map: [
      { source: "n", target: "n", role: "statement_parameter", map: "乙的第 n 条 ＝ 共享的第 n+1 条" }] }))
      .toBe("特例：乙的第 n 条 ＝ 共享的第 n+1 条");
    expect(alignmentPlainLabel({ relation: "expresses", parameter_map: [{ source: "n", target: "n", role: "statement_parameter", map: "n = n" }] }))
      .toBe("说的就是它");
    expect(alignmentPlainLabel({ relation: "overlaps_with", lost_information: [{ short: "缺 n = 1" }] })).toBe("部分重合：缺 n = 1");
    for (const relation of ["expresses", "specializes", "generalizes", "instance_of", "overlaps_with", "incompatible_with"]) {
      const label = alignmentPlainLabel({ relation });
      for (const word of BANNED) expect(label).not.toContain(word);
    }
    expect(entityPlainLabel({ preferred_labels: [{ lang: "en", label: "Fib" }, { lang: "zh", label: "斐波那契数" }] })).toBe("斐波那契数");
    expect(reviewItemLabel("mathlib_baseline_unchecked")).toContain("mathlib");
  });
});

describe("feedback to agent", () => {
  it("builds the S9.2 quote", () => {
    const text = feedbackQuote({
      noun: "对齐线", proposal: { title: "Cassini 恒等式", version: 2 }, name: "a.yi.thm",
      detail: "特例：乙的第 n 条 ＝ 共享的第 n+1 条", id: "mathscape:alignment/d1d5", code: "warn:relation_detail_missing",
    });
    expect(text).toBe("> [审核项] 提案「Cassini 恒等式」v2 · 对齐线 a.yi.thm（特例：乙的第 n 条 ＝ 共享的第 n+1 条）\n> id: mathscape:alignment/d1d5  · 码: warn:relation_detail_missing\n");
  });

  it("only inserts through the composer event and never sends", () => {
    expect(sendFeedbackToComposer({ noun: "一笔账", name: "d1" })).toBe(false);
    const handler = vi.fn(({ text, accept }: { text: string; accept: () => void }) => { expect(text).toContain("[一笔账]"); accept(); });
    const off = eventBus.on("insert-agent-feedback", handler);
    expect(sendFeedbackToComposer({ noun: "一笔账", name: "d1" })).toBe(true);
    off();
    expect(handler).toHaveBeenCalledTimes(1);
  });
});

const proposal = (): PendingChangeSet => ({
  changeSetId: "mathscape:change-set/x", version: 2, state: "staged", investigationId: "inv", roundId: "r", proposedBy: "agent:codex",
  createdAt: "", proposalFingerprint: "", baseCommit: null, dependencies: [],
  targetTitles: { "mathscape:source-unit/yi-thm": "乙 · 定理 3" },
  proposal: { proposed_objects: [
    { local_ref: "e.cassini", kind: "semantic_entity", op: "create", payload: { semantic_kind: "declaration", plain_label: "Cassini 恒等式",
      mathlib_baseline: { decl: "Nat.fib_add_two_mul_fib_sub_sq", status: "unchecked" } } },
    { local_ref: "a.yi.thm", kind: "alignment", op: "create", payload: { source: "mathscape:source-unit/yi-thm", target: "$new:e.cassini",
      relation: "specializes", status: "retrieved_candidate",
      parameter_map: [{ source: "n", target: "n", role: "statement_parameter", map: "乙的第 n 条 ＝ 共享的第 n+1 条" }] } },
    { local_ref: "d1", kind: "debt", op: "create", payload: { obligation: "证明对一切 n 成立", quantifier: "对一切 n ≥ 1", failure_mode: "theorem",
      status: "open", attached_to: "$new:a.yi.thm" } },
    { local_ref: "u", kind: "source_unit", op: "create", payload: {} },
  ] },
  result: {
    staged_ids: { "a.yi.thm": "mathscape:alignment/abc" },
    diagnostics: [
      { level: "warn", code: "certificate_debt_suggested", message: "", local_ref: "a.yi.thm" },
      { level: "info", code: "multi_target_change_set", message: "两个目标" },
    ],
  },
});

describe("semantic review cards", () => {
  it("turn semantic objects into plain-language cards with their review items", () => {
    const cards = semanticReviewCards(proposal());
    expect(cards.map(card => card.noun)).toEqual(["共享对象", "对齐线", "一笔账"]);
    expect(cards[0].lines).toContain("mathlib：Nat.fib_add_two_mul_fib_sub_sq（未核对）");
    expect(cards[1].title).toBe("乙 · 定理 3 —特例：乙的第 n 条 ＝ 共享的第 n+1 条→ Cassini 恒等式");
    expect(cards[1].stagedId).toBe("mathscape:alignment/abc");
    expect(reviewItemCode(cards[1].diagnostics[0])).toBe("warn:certificate_debt_suggested");
    expect(cards[2].lines).toContain("挂在：a.yi.thm");
    expect(proposalLevelReviewItems(proposal()).map(item => item.code)).toEqual(["multi_target_change_set"]);
    for (const card of cards) for (const line of [card.title, ...card.lines]) expect(line).not.toMatch(/[{}]/);
  });
});

describe("learning view and recheck summaries", () => {
  it("count fresh and seen units", () => {
    expect(materialSummary({ artifact_id: "a", units: [
      { unit_id: "u1", unit_title: "定义", alignment_id: "x", entity_id: "e", entity_label: "斐波那契数", relation: "expresses",
        label: "别的材料里见过", seen_in: ["b"], only_differs_by: [] },
      { unit_id: "u2", unit_title: "例题", alignment_id: "y", entity_id: "p", entity_label: "铺砖", relation: "instance_of",
        label: "新的", seen_in: [], only_differs_by: [] },
    ], new_conventions: [{ scope_id: "s", scope_label: "乙", text: "下标从 1 开始" }], open_debts: [] }))
      .toBe("新的 1 · 别的材料里见过 1 · 新约定 1 · 未还清的账 0");
  });

  it("explain recheck outcomes without claiming faithfulness", () => {
    expect(recheckSummary({ status: "unavailable", profile_id: null, mathlib_version: null, checked: 0, confirmed: [], not_found: [], commit_id: null, reason: "没有已登记的 Lean profile" }))
      .toContain("保持“未核对”");
    const committed = recheckSummary({ status: "committed", profile_id: "p", mathlib_version: "v4.33.1", checked: 2, confirmed: ["Nat.fib"], not_found: ["X"], commit_id: "c", reason: null });
    expect(committed).toContain("找到 Nat.fib");
    expect(committed).toContain("不说明与教材命题是一回事");
  });
});

describe("semantic taxonomy", () => {
  const node = (domain_kind: string, element_kind: "entity" | "relation" = "entity", summary: string | null = null): IncidenceNode => ({
    id: "n", element_kind, domain_id: "d", domain_kind, relation_kind: null, layer: "global_semantic", label: "x", summary,
    lifecycle: "active", parent: null, boundary: false,
  });
  it("maps semantic kinds to their own visuals", () => {
    expect(nodeTypeKey(node("SemanticEntity", "entity", "概念星 · mathlib：Nat.fib（未核对）"))).toBe("shared_entity");
    expect(nodeTypeKey(node("SemanticEntity", "entity", "星区"))).toBe("shared_theory");
    expect(nodeTypeKey(node("Debt"))).toBe("debt");
    expect(nodeTypeKey(node("DialectScope"))).toBe("dialect_scope");
    expect(nodeTypeKey(node("Alignment", "relation"))).toBe("alignment_relation");
    expect(NODE_TYPES.debt.borderStyle).toBe("dashed");
    expect(semanticLayerClass("Debt")).toBe(" debt-edge");
    expect(semanticLayerClass("Alignment")).toBe(" alignment-edge");
    expect(semanticLayerClass("SourceUnitRelation")).toBe("");
  });
});
