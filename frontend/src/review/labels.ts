import type { WorkflowStatus } from "./types";

const lookup = (table: Record<string, string>) => (value: string): string => table[value] ?? value;

export const workflowStatusLabel: (value: WorkflowStatus | string) => string = lookup({
  running: "运行中", awaiting_patch_review: "等待审核修订", completed: "完成", partial: "部分完成",
  needs_attention: "需要处理", failed: "失败", cancelled: "已取消",
});

export const unitKindLabel = lookup({
  section: "章节", definition: "定义", statement: "命题", proof: "证明", problem: "习题", hint: "提示",
  solution: "解答", example: "例子", remark: "注记", method: "方法", notation: "记号", narrative: "叙述",
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

export const relationLabel = lookup({
  proves: "证明", solves: "解答", hints: "提示", explains: "解释", cites: "引用", applies_method: "应用方法",
});

export const issueKindLabel = lookup({
  left_truncated: "左侧截断", right_truncated: "右侧截断", internally_missing: "内部缺失",
  uncertain_boundary: "边界待确认", uncertain_parent: "父级待确认", ambiguous_reference: "引用有歧义",
  suspected_conversion_error: "疑似转换错误", missing_context: "缺少上下文", uncertain_classification: "分类待确认",
});

export const acceptanceLabel = lookup({
  not_selected: "未选择", selectable: "可接受", blocked: "阻塞", accepted: "已接受", registered: "已登记",
});

export const anchorPurposeLabel = lookup({
  unit: "单元原文", source_title: "原文标题", source_label: "原文标签", reference: "引用原文",
  relation_evidence: "关系证据", diagnostic_evidence: "诊断证据", selection: "原始选区",
});
