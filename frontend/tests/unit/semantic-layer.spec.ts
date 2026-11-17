import { describe, expect, it, vi } from "vitest";
import { alignmentPlainLabel, entityPlainLabel, reviewItemLabel } from "../../src/semantic/labels";
import { feedbackQuote, sendFeedbackToComposer } from "../../src/semantic/feedback";
import { semanticReviewCards, proposalLevelReviewItems, reviewItemCode, refTitle } from "../../src/semantic/review-model";
import { computeProposalMapScope, findProposalTargetForDomainObject } from "../../src/review/map-review-sync";
import { computeSupersedesDiff, unwrapFieldChange } from "../../src/semantic/semantic-diff";
import { titleOf } from "../../src/review/proposal-model";
import { filterOutlineUnits, formatMergedUnitDetail, materialSummary, mergeLearningUnits, studyProgressSummary } from "../../src/semantic/learning-view";
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
      { source: "n", target: "n", role: "statement_parameter", map: "灯塔的第 n 条 ＝ 共享的第 n+1 条" }] }))
      .toBe("特例：灯塔的第 n 条 ＝ 共享的第 n+1 条");
    expect(alignmentPlainLabel({ relation: "expresses", parameter_map: [{ source: "n", target: "n", role: "statement_parameter", map: "n = n" }] }))
      .toBe("说的就是它");
    expect(alignmentPlainLabel({ relation: "expresses", parameter_map: [{ source: "n", target: "n", role: "statement_parameter", map: "n ↦ n" }] }))
      .toBe("说的就是它");
    expect(alignmentPlainLabel({ relation: "overlaps_with", lost_information: [{ short: "缺 n = 1" }] })).toBe("部分重合：缺 n = 1");
    for (const relation of ["expresses", "specializes", "generalizes", "instance_of", "overlaps_with", "incompatible_with"]) {
      const label = alignmentPlainLabel({ relation });
      for (const word of BANNED) expect(label).not.toContain(word);
    }
    expect(entityPlainLabel({ preferred_labels: [{ lang: "en", label: "Fib" }, { lang: "zh", label: "斐波那契数" }] })).toBe("斐波那契数");
    expect(reviewItemLabel("mathlib_baseline_unchecked")).toContain("mathlib");
    expect(reviewItemLabel("theory_name_too_specific")).toContain("专题名");
    expect(reviewItemLabel("theory_aliases_missing")).toContain("同义词");
  });
});

describe("feedback to agent", () => {
  it("builds the S9.2 quote", () => {
    const text = feedbackQuote({
      noun: "对齐线", proposal: { title: "Cassini 恒等式", version: 2 }, name: "a.yi.thm",
      detail: "特例：灯塔的第 n 条 ＝ 共享的第 n+1 条", id: "mathscape:alignment/d1d5", code: "warn:relation_detail_missing",
    });
    expect(text).toBe("> [审核项] 提案「Cassini 恒等式」v2 · 对齐线 a.yi.thm（特例：灯塔的第 n 条 ＝ 共享的第 n+1 条）\n> id: mathscape:alignment/d1d5  · 码: warn:relation_detail_missing\n");
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
  targetTitles: { "mathscape:source-unit/yi-thm": "灯塔 · 定理 3" },
  proposal: { proposed_objects: [
    { local_ref: "e.cassini", kind: "semantic_entity", op: "create", payload: { semantic_kind: "declaration", plain_label: "Cassini 恒等式",
      mathlib_baseline: { decl: "Nat.fib_add_two_mul_fib_sub_sq", status: "unchecked" } } },
    { local_ref: "a.yi.thm", kind: "alignment", op: "create", payload: { source: "mathscape:source-unit/yi-thm", target: "$new:e.cassini",
      relation: "specializes", status: "retrieved_candidate",
      parameter_map: [{ source: "n", target: "n", role: "statement_parameter", map: "灯塔的第 n 条 ＝ 共享的第 n+1 条" }] } },
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
    expect(cards[1].title).toBe("灯塔 · 定理 3 —特例：灯塔的第 n 条 ＝ 共享的第 n+1 条→ Cassini 恒等式");
    expect(cards[1].stagedId).toBe("mathscape:alignment/abc");
    expect(reviewItemCode(cards[1].diagnostics[0])).toBe("warn:certificate_debt_suggested");
    expect(cards[2].lines).toContain("挂在：a.yi.thm");
    expect(proposalLevelReviewItems(proposal()).map(item => item.code)).toEqual(["multi_target_change_set"]);
    for (const card of cards) for (const line of [card.title, ...card.lines]) expect(line).not.toMatch(/[{}]/);
  });

  it("unwraps FieldChange on edit, renders non-zero term_index_map shifts without banned words, and lists retire cascades", () => {
    expect(unwrapFieldChange({ action: "set", value: "新标题" })).toEqual({ isFieldChange: true, action: "set", value: "新标题" });
    expect(unwrapFieldChange({ action: "clear" })).toEqual({ isFieldChange: true, action: "clear", value: undefined });

    const editAndRetire: PendingChangeSet = {
      changeSetId: "mathscape:change-set/edit-1",
      version: 2,
      state: "staged",
      investigationId: "inv",
      roundId: "r2",
      proposedBy: "agent:codex",
      createdAt: "",
      proposalFingerprint: "fp",
      baseCommit: null,
      dependencies: [],
      targetObjectTitle: "Cassini 恒等式",
      targetTitles: {
        "mathscape:semantic-entity/cassini": "Cassini 恒等式",
        "mathscape:semantic-entity/t-old": "旧理论",
        "mathscape:semantic-entity/t-new": "线性递推数列",
        "mathscape:alignment/a-bad": "旧对齐线",
        "mathscape:debt/d-open": "待补归纳证明",
        "mathscape:source-unit/yi-thm": "定理 2.4.5",
      },
      baseObjects: {
        "mathscape:semantic-entity/cassini": {
          kind: "semantic_entity",
          payload: {
            semantic_kind: "declaration",
            home_theory: "mathscape:semantic-entity/t-old",
            plain_label: "旧标签",
            mathlib_baseline: { decl: null, status: "unchecked" },
          },
        },
      },
      retireCascades: {
        "mathscape:alignment/a-bad": ["mathscape:debt/d-open"],
      },
      history: [
        {
          version: 1,
          state: "superseded",
          createdAt: "t1",
          diagnostics: [],
          proposal: {
            proposed_objects: [
              {
                local_ref: "a.shift",
                kind: "alignment",
                op: "create",
                payload: {
                  source: "mathscape:source-unit/yi-thm",
                  target: "mathscape:semantic-entity/cassini",
                  relation: "expresses",
                  status: "retrieved_candidate",
                },
              },
            ],
          },
        },
      ],
      proposal: {
        rationale: "一段很长的提案理由说明文字，不应取代简短的目标对象名称",
        proposed_objects: [
          {
            local_ref: "edit.cassini",
            kind: "semantic_entity",
            op: "edit",
            target_id: "mathscape:semantic-entity/cassini",
            changes: {
              home_theory: "mathscape:semantic-entity/t-new",
              plain_label: { action: "set", value: "Cassini 恒等式（修正版）" },
              mathlib_baseline: {
                action: "set",
                value: { decl: "Nat.fib_cassini", status: "unchecked" },
              },
            },
          },
          {
            local_ref: "a.shift",
            kind: "alignment",
            op: "create",
            payload: {
              source: "mathscape:source-unit/yi-thm",
              target: "mathscape:semantic-entity/cassini",
              relation: "specializes",
              status: "context_checked",
              term_index_map: [{ source: "a_k", target: "F_k", index_shift: 1, valid_for: "k ≥ 1" }],
            },
          },
          {
            local_ref: "retire.bad",
            kind: "alignment",
            op: "retire",
            target_id: "mathscape:alignment/a-bad",
          },
        ],
      },
      result: { staged_ids: { "a.shift": "mathscape:alignment/a-shift" } },
    };

    expect(titleOf(editAndRetire)).toBe("Cassini 恒等式");
    const cards = semanticReviewCards(editAndRetire);
    expect(cards).toHaveLength(3);
    // 1. Edit card unwraps FieldChange and renders before -> after diffs without "暂无对应名称" bug
    expect(cards[0].lines.some((l) => l.includes("旧理论 → 线性递推数列"))).toBe(true);
    expect(cards[0].lines).toContain("mathlib 参照：暂无对应名称 → Nat.fib_cassini（未核对）");
    expect(cards[0].lines).not.toContain("mathlib：暂无对应名称");
    // 2. Non-zero index_shift is surfaced and uses no banned words
    expect(cards[1].lines.some((l) => l.includes("+1"))).toBe(true);
    for (const word of BANNED) {
      for (const line of cards[1].lines) expect(line).not.toContain(word);
    }
    // 3. Retire card surfaces cascading obsolete debts
    expect(cards[2].lines.some((l) => l.includes("待补归纳证明"))).toBe(true);
    for (const card of cards) for (const line of [card.title, ...card.lines]) expect(line).not.toMatch(/[{}]/);

    // 4. Supersedes diff v1 -> v2
    const revDiff = computeSupersedesDiff(
      1,
      editAndRetire.history![0].proposal,
      editAndRetire,
      (r) => refTitle(editAndRetire, r),
    );
    expect(revDiff?.added.length).toBe(2);
    expect(revDiff?.modified[0]?.summaries.some((s) => s.includes("说的就是 → 是特例"))).toBe(true);

    // 5. Map-linked scope & reverse lookup
    const located = findProposalTargetForDomainObject([editAndRetire], "mathscape:alignment/a-shift");
    expect(located).toEqual({ changeSetKey: "mathscape:change-set/edit-1@2", focusTarget: "semantic:a.shift" });
    const scope = computeProposalMapScope(
      {
        schema_version: "mathscape.graph-projection.v1",
        lens: "landscape",
        nodes: [
          { id: "n1", element_kind: "entity", domain_id: "mathscape:source-unit/yi-thm", domain_kind: "SourceSemanticUnit", relation_kind: null, layer: "source", label: "定理", summary: null, lifecycle: "active", parent: null, boundary: false },
          { id: "n2", element_kind: "entity", domain_id: "mathscape:semantic-entity/cassini", domain_kind: "SemanticEntity", relation_kind: null, layer: "global_semantic", label: "Cassini", summary: null, lifecycle: "active", parent: null, boundary: false },
          { id: "n3", element_kind: "entity", domain_id: "mathscape:source-unit/other", domain_kind: "SourceSemanticUnit", relation_kind: null, layer: "source", label: "无关", summary: null, lifecycle: "active", parent: null, boundary: false },
          { id: "rel:a1", element_kind: "relation", domain_id: "mathscape:alignment/a-shift", domain_kind: "Alignment", relation_kind: "aligns", layer: "alignment", label: "特例", summary: null, lifecycle: "active", parent: null, boundary: false },
        ],
        edges: [
          { id: "e1", source: "n1", target: "rel:a1", relation_id: "mathscape:alignment/a-shift", role: "source", ordinal: 0, direction: "into_relation", directed: true },
          { id: "e2", source: "rel:a1", target: "n2", relation_id: "mathscape:alignment/a-shift", role: "target", ordinal: 1, direction: "out_of_relation", directed: true },
        ],
      },
      [editAndRetire],
      editAndRetire,
      true,
    );
    expect(scope.nodeIds).toContain("n1");
    expect(scope.nodeIds).toContain("n2");
    expect(scope.nodeIds).not.toContain("n3");
    expect(scope.relationIds).toEqual(["rel:a1", "mathscape:alignment/a-shift"]);
  });
});

describe("learning view and recheck summaries", () => {
  it("count fresh and seen units", () => {
    expect(materialSummary({ artifact_id: "a", units: [
      { unit_id: "u1", unit_title: "定义", alignment_id: "x", entity_id: "e", entity_label: "斐波那契数", relation: "expresses",
        label: "别的材料里见过", seen_in: ["b"], only_differs_by: [] },
      { unit_id: "u2", unit_title: "例题", alignment_id: "y", entity_id: "p", entity_label: "铺砖", relation: "instance_of",
        label: "新的", seen_in: [], only_differs_by: [] },
    ], new_conventions: [{ scope_id: "s", scope_label: "灯塔", text: "下标从 1 开始" }], open_debts: [] }))
      .toBe("新的 1 · 别的材料里见过 1 · 新约定 1 · 未还清的账 0");
  });

  it("merges multiple alignments of the same unit into one row", () => {
    const merged = mergeLearningUnits([
      { unit_id: "u-def", unit_title: "定义 2.4.1", alignment_id: "a1", entity_id: "e1", entity_label: "概念：Fibonacci 数列", relation: "expresses",
        label: "别的材料里见过", seen_in: ["jia"], only_differs_by: [] },
      { unit_id: "u-def", unit_title: "定义 2.4.1", alignment_id: "a2", entity_id: "e2", entity_label: "精确说法：Fibonacci 数列（F₀ = 0 起编，同 mathlib）", relation: "overlaps_with",
        label: "别的材料里见过", seen_in: ["jia"], only_differs_by: ["灯塔没有第 0 项"] },
      { unit_id: "u-ex", unit_title: "习题 2.4.7", alignment_id: "a3", entity_id: "e3", entity_label: "题型：剪拼悖论", relation: "expresses",
        label: "新的", seen_in: [], only_differs_by: [] },
    ]);
    expect(merged).toHaveLength(2);
    expect(merged[0].unit_title).toBe("定义 2.4.1");
    expect(merged[0].targets).toHaveLength(2);
    expect(formatMergedUnitDetail(merged[0])).toBe("见过：Fibonacci 数列；Fibonacci 数列（F₀ = 0 起编，同 mathlib）（只差：灯塔没有第 0 项）");
    expect(materialSummary({ artifact_id: "yi", units: [
      { unit_id: "u-def", unit_title: "定义 2.4.1", alignment_id: "a1", entity_id: "e1", entity_label: "Fibonacci 数列", relation: "expresses",
        label: "别的材料里见过", seen_in: [], only_differs_by: [] },
      { unit_id: "u-def", unit_title: "定义 2.4.1", alignment_id: "a2", entity_id: "e2", entity_label: "精确说法", relation: "overlaps_with",
        label: "别的材料里见过", seen_in: [], only_differs_by: ["灯塔没有第 0 项"] },
      { unit_id: "u-ex", unit_title: "习题 2.4.7", alignment_id: "a3", entity_id: "e3", entity_label: "剪拼悖论", relation: "expresses",
        label: "新的", seen_in: [], only_differs_by: [] },
    ], new_conventions: [{ scope_id: "s", scope_label: "灯塔", text: "下标从 1 开始" }, { scope_id: "s", scope_label: "灯塔", text: "叫作兔子数列" }],
      open_debts: [{ debt_id: "d1", obligation: "x", status: "open" }, { debt_id: "d2", obligation: "y", status: "open" }, { debt_id: "d3", obligation: "z", status: "open" }, { debt_id: "d4", obligation: "w", status: "open" }] }))
      .toBe("新的 1 · 别的材料里见过 1 · 新约定 2 · 未还清的账 4");
  });

  it("explain recheck outcomes without claiming faithfulness", () => {
    expect(recheckSummary({ status: "unavailable", profile_id: null, mathlib_version: null, checked: 0, confirmed: [], not_found: [], commit_id: null, reason: "没有已登记的 Lean profile" }))
      .toContain("保持“未核对”");
    const committed = recheckSummary({ status: "committed", profile_id: "p", mathlib_version: "v4.33.1", checked: 2, confirmed: ["Nat.fib"], not_found: ["X"], commit_id: "c", reason: null });
    expect(committed).toContain("找到 Nat.fib");
    expect(committed).toContain("不说明与教材命题是一回事");
  });

  it("summarizes full outline progress, convention diffs, and filters unaligned or novel units", () => {
    const outline = [
      { unit_id: "sec", unit_title: "§1", unit_kind: "section", study_status: "mastered", category: "unaligned", targets: [] },
      { unit_id: "u1", unit_title: "定义 1", unit_kind: "definition", parent_unit: "sec", study_status: "mastered", category: "mastered_elsewhere",
        targets: [{ alignment_id: "a1", entity_id: "e1", entity_label: "Fib", relation: "expresses", only_differs_by: [], seen_in: ["a"], mastered_in: ["a"] }] },
      { unit_id: "u2", unit_title: "定义 2", unit_kind: "definition", parent_unit: "sec", study_status: "studying", category: "mastered_elsewhere",
        targets: [{ alignment_id: "a2", entity_id: "e2", entity_label: "FibDef", relation: "overlaps_with", only_differs_by: ["缺第 0 项"], seen_in: ["a"], mastered_in: ["a"] }] },
      { unit_id: "u3", unit_title: "习题 3", unit_kind: "problem", parent_unit: "sec", study_status: "unread", category: "fresh",
        targets: [{ alignment_id: "a3", entity_id: "e3", entity_label: "剪拼", relation: "expresses", only_differs_by: [], seen_in: [] }] },
    ];
    const summary = studyProgressSummary({
      artifact_id: "b",
      units: [],
      outline_units: outline,
      new_conventions: [{ scope_id: "s", scope_label: "约定", text: "从 0 起" }, { scope_id: "s", scope_label: "约定", text: "叫兔子数列" }],
      conventions_detail: [
        { scope_id: "s", scope_label: "约定", key: "sequence_index_origin", text: "从 0 起", is_baseline_match: true },
        { scope_id: "s", scope_label: "约定", key: "terminology", text: "叫兔子数列", is_baseline_match: false },
      ],
      open_debts: [{ debt_id: "d1", obligation: "补证", status: "open" }],
    });
    expect(summary).toBe("研读进度：已掌握 2/4（在读 1） · 新内容 1 · 他处已掌握 2 · 待对齐 1 · 特有约定 1 · 未还清的账 1");
    expect(filterOutlineUnits(outline, "unaligned").map((u) => u.unit_id)).toEqual(["sec"]);
    // `u1` is mastered elsewhere with zero differences, so `novel_and_diff` filters it out while keeping `u2` (has diff) and `u3` (fresh)!
    expect(filterOutlineUnits(outline, "novel_and_diff").map((u) => u.unit_id)).toEqual(["u2", "u3"]);
  });
});

describe("semantic taxonomy", () => {
  const node = (domain_kind: string, element_kind: "entity" | "relation" = "entity", summary: string | null = null): IncidenceNode => ({
    id: "n", element_kind, domain_id: "d", domain_kind, relation_kind: null, layer: "global_semantic", label: "x", summary,
    lifecycle: "active", parent: null, boundary: false,
  });
  it("maps semantic kinds to their own visuals", () => {
    expect(nodeTypeKey(node("SemanticEntity", "entity", "概念 · mathlib：Nat.fib（未核对）"))).toBe("shared_entity");
    expect(nodeTypeKey(node("SemanticEntity", "entity", "理论"))).toBe("shared_theory");
    expect(nodeTypeKey(node("Debt"))).toBe("debt");
    expect(nodeTypeKey(node("DialectScope"))).toBe("dialect_scope");
    expect(nodeTypeKey(node("Alignment", "relation"))).toBe("alignment_relation");
    expect(NODE_TYPES.debt.borderStyle).toBe("dashed");
    expect(semanticLayerClass("Debt")).toBe(" debt-edge");
    expect(semanticLayerClass("Alignment")).toBe(" alignment-edge");
    expect(semanticLayerClass("SourceUnitRelation")).toBe("");
  });
});
