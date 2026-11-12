/** 语义层 Desktop 命令的类型化包装。全部只读，除了 mathlib 核对（服务端写入）。 */
import { safeInvoke } from "../core/ipc";

export interface SemanticObjectDetail {
  object_id: string;
  kind: "SemanticEntity" | "Alignment" | "Debt" | "DialectScope" | string;
  lifecycle: string;
  revision: string;
  payload: Record<string, unknown>;
  titles: Record<string, string>;
  debts: Array<Record<string, unknown> & { object_id: string }>;
}

export interface LearningUnit {
  unit_id: string;
  unit_title: string;
  alignment_id: string;
  entity_id: string;
  entity_label: string;
  relation: string;
  label: "新的" | "别的材料里见过" | string;
  seen_in: string[];
  only_differs_by: string[];
}

export interface LearningMaterial {
  artifact_id: string;
  units: LearningUnit[];
  new_conventions: Array<{ scope_id: string; scope_label: string; text: string }>;
  open_debts: Array<{ debt_id: string; obligation: string; status: string }>;
}

export interface LearningView {
  materials: LearningMaterial[];
}

export interface BaselineRecheckReport {
  status: "committed" | "unchanged" | "nothing_to_check" | "unavailable" | string;
  profile_id: string | null;
  mathlib_version: string | null;
  checked: number;
  confirmed: string[];
  not_found: string[];
  commit_id: string | null;
  reason: string | null;
}

export const getSemanticObject = (objectId: string) =>
  safeInvoke<SemanticObjectDetail>("get_semantic_object", { objectId });

export const getLearningView = () => safeInvoke<LearningView>("semantic_learning_view");

export const recheckMathlibBaselines = () => safeInvoke<BaselineRecheckReport>("recheck_mathlib_baselines");

/** 一句话说明核对结果。 */
export function recheckSummary(report: BaselineRecheckReport): string {
  switch (report.status) {
    case "unavailable": return `没有核对：${report.reason ?? "没有可用的 Lean profile"}。mathlib 名称保持“未核对”。`;
    case "nothing_to_check": return "地图上没有写了 mathlib 名称的共享对象。";
    default: {
      const where = `profile ${report.profile_id ?? "?"}（mathlib ${report.mathlib_version ?? "?"}）`;
      const found = report.confirmed.length ? `找到 ${report.confirmed.join("、")}` : "";
      const missing = report.not_found.length ? `没找到 ${report.not_found.join("、")}` : "";
      const result = [found, missing].filter(Boolean).join("；") || "没有名称";
      return `在 ${where} 中核对了 ${report.checked} 个名称：${result}。${report.status === "unchanged" ? "结果与上次相同，没有写入。" : "已写入地图。"}只说明名字存在，不说明与教材命题是一回事。`;
    }
  }
}
