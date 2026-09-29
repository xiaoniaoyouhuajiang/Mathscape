import type { ReviewStatus, WorkflowStatus } from "./types";

const lookup = (table: Record<string, string>) => (value: string): string => table[value] ?? value;

export const discoveryRunStatusLabel: (status: ReviewStatus | string) => string = lookup({
  processing: "处理中",
  reviewable: "待审核",
  accepted: "已接受，尚未登记",
  registered: "已登记",
  discarded: "已放弃",
  diagnostic: "需查看诊断",
  terminal: "已结束",
});

export const discoveryWorkflowStatusLabel: (status: WorkflowStatus | string) => string = lookup({
  running: "运行中",
  awaiting_patch_review: "等待 Markdown 修订审核",
  completed: "完成",
  partial: "部分完成",
  needs_attention: "需要处理",
  failed: "失败",
  cancelled: "已取消",
});

export const discoveryStageLabel: (stage: string) => string = lookup({
  "unit-discovery": "单元识别",
  "source-facts": "来源事实发现",
  "discovery-repair": "有界修订",
  "markdown-patch": "Markdown 修订建议",
  unit_discovery: "单元识别",
  source_facts: "来源事实发现",
  discovery_repair: "有界修订",
  markdown_patch: "Markdown 修订建议",
});

export function shortRunId(runId: string): string {
  return runId.length > 34 ? `${runId.slice(0, 17)}…${runId.slice(-12)}` : runId;
}
