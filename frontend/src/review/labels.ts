import { t } from "../i18n";
import type { WorkflowStatus } from "./types";

const lookup = (table: Record<string, string>) => (value: string): string => table[value] ?? value;

export const workflowStatusLabel: (value: WorkflowStatus | string) => string = lookup({
  running: "运行中", awaiting_patch_review: "等待审核修订", completed: "完成", partial: "部分完成",
  needs_attention: "需要处理", failed: "失败", cancelled: "已取消",
});

export const unitKindLabel = lookup({
  section: "章节", definition: "定义", statement: "命题", proof: "证明", problem: "习题", hint: "提示",
  solution: "解答", example: "例子", remark: "注记", method: "方法", notation: "记号", narrative: "叙述",
  concept: "共享概念", declaration: "精确说法", theory: "理论", problem_schema: "题型",
  alignment: "对齐线", debt: "一笔账", dialect_scope: "约定卡",
});

export const completenessLabel = lookup({
  complete: "内容完整", left_truncated: "左侧截断", right_truncated: "右侧截断",
  internally_missing: "内部缺失", uncertain: "边界待确认",
});

export const factStatusLabel = lookup({
  processed: "来源事实已验证", pending: "来源事实待发现", requires_unit_revision: "需要修订单元",
});

export const referenceStatusLabel = lookup({
  unresolved: "未解析", candidate: "存在候选", resolved_local: "已解析到本地单元", ambiguous: "存在歧义",
});

const knownRelationLabel = lookup({
  proves: "证明", solves: "解答", hints: "提示", explains: "解释", cites: "引用", applies_method: "应用方法",
  expresses: "说的就是它", specializes: "是特例", generalizes: "是推广", instance_of: "是实例",
  representation_of: "换一种表示", logically_equivalent: "逻辑等价", definitionally_equivalent: "定义等价",
  exact_realization: "精确实现", notation_for: "是记号", overlaps_with: "部分重合", incompatible_with: "对不上",
  in_theory: "属于理论", debt_on: "挂账", scope_of: "约定适用于", uses_entity: "引用对象",
});

/** `unknown_relation`: a proposal edits or retires a relation the review cannot read. */
export const relationLabel = (value: string): string =>
  value === "unknown_relation" ? t("review.relation.unknown") : knownRelationLabel(value);

export const issueKindLabel = lookup({
  left_truncated: "左侧截断", right_truncated: "右侧截断", internally_missing: "内部缺失",
  uncertain_boundary: "边界待确认", uncertain_parent: "父级待确认", ambiguous_reference: "引用有歧义",
  suspected_conversion_error: "疑似转换错误", missing_context: "缺少上下文", uncertain_classification: "分类待确认",
});

export const acceptanceLabel = lookup({
  not_selected: "未选择", selectable: "可接受", blocked: "阻塞", accepted: "已接受", registered: "已登记",
  discarded: "已放弃", withdrawn: "登记已撤回", restored: "已恢复审核", reviewable: "待审核",
});

export const anchorPurposeLabel = lookup({
  unit: "单元原文", source_title: "原文标题", source_label: "原文标签", reference: "引用原文",
  relation_evidence: "关系证据", diagnostic_evidence: "诊断证据", selection: "原始选区",
  semantic_evidence: "语义证据",
});
