/**
 * 语义层的白话名字（docs/semantic-write-contract.md S9）。与
 * `crates/mathscape-query/src/semantic_graph.rs` 保持同一套说法。
 * 禁用词：平移、错开、滑动。
 */

import { hasMessage, t } from "../i18n";

const lookup = (table: Record<string, string>, fallback: string) => (value: string | null | undefined): string =>
  (value && table[value]) || fallback;

export const semanticKindLabel = lookup({
  concept: "概念", declaration: "精确说法", theory: "理论", problem_schema: "题型",
}, "共享对象");

export const relationLabel = lookup({
  expresses: "说的就是", specializes: "是特例", generalizes: "是推广", instance_of: "是实例",
  representation_of: "换一种表示", logically_equivalent: "逻辑等价", definitionally_equivalent: "定义等价",
  exact_realization: "精确实现", notation_for: "是记号", overlaps_with: "部分重合", incompatible_with: "对不上",
}, "对齐");

export const alignmentStatusLabel = lookup({
  unmatched: "未匹配", retrieved_candidate: "候选", context_checked: "已核对上下文", partially_aligned: "部分对上",
  ambiguous: "有歧义", disputed: "有争议", incompatible: "对不上", formally_related: "形式化相关",
  semantically_reviewed: "你已审阅", accepted: "你已确认", rejected: "你已否决", superseded: "已被替代",
}, "未知状态");

export const debtStatusLabel = lookup({
  open: "还欠着", claimed_paid: "Agent 说已还清，待你确认", paid: "你已确认还清", waived: "你已免除", obsolete: "已失效",
}, "未知状态");

export const failureModeLabel = lookup({
  theorem: "定理本身可能不成立", statement: "命题的说法可能不对", route: "证明路线可能走不通",
}, "—");

export const baselineStatusLabel = lookup({
  unchecked: "未核对", confirmed: "已在本机 mathlib 中找到", not_found: "本机 mathlib 中没找到",
}, "未知");

export const lostDirectionLabel = lookup({
  source_has_target_lacks: "来源有、共享对象没有", target_has_source_lacks: "共享对象有、来源没有", both: "两边各有对方没有的",
}, "");

/** 审核项（不阻塞的 warn / info 诊断）的白话说明。 */
const legacyReviewItemLabel = lookup({
  relation_detail_missing: "特例 / 推广没写清参数对照或对不上的部分",
  overlap_scope_missing: "部分重合没写清哪里对不上",
  certificate_debt_suggested: "这种关系需要证明，建议补一笔账",
  dialect_scope_mismatch: "约定卡不覆盖这条来源",
  uncontextualized: "这条来源没有约定卡",
  duplicate_alignment: "同一条来源对同一个共享对象有两条对齐线",
  inferred_convention: "约定是推断出来的，书里没明说",
  mathlib_baseline_unchecked: "mathlib 名称还没在本机核对",
  multi_target_change_set: "一个提案涉及多个目标对象",
  theory_aliases_missing: "理论没有写检索同义词，后续按别称可能不容易搜到",
  theory_name_too_specific: "理论名称里带有具体算式或公式符号，建议改成专题名",
}, "");

export function reviewItemLabel(code: string | null | undefined): string {
  const key = `review.${code ?? ""}`;
  return hasMessage(key) ? t(key) : legacyReviewItemLabel(code);
}

export const SEMANTIC_REVIEW_CODES = new Set([
  "relation_detail_missing", "overlap_scope_missing", "certificate_debt_suggested", "dialect_scope_mismatch",
  "uncontextualized", "duplicate_alignment", "inferred_convention", "mathlib_baseline_unchecked",
  "multi_target_change_set", "theory_aliases_missing", "theory_name_too_specific",
  "label_lang_missing", "possible_duplicate_name", "duplicate_mathlib_decl",
]);

export const SEMANTIC_PROPOSAL_KINDS = new Set(["semantic_entity", "alignment", "debt", "dialect_scope"]);
export const SEMANTIC_DOMAIN_KINDS = new Set(["SemanticEntity", "Alignment", "Debt", "DialectScope"]);

export const semanticObjectNoun = lookup({
  semantic_entity: "共享对象", alignment: "对齐线", debt: "一笔账", dialect_scope: "约定卡",
  SemanticEntity: "共享对象", Alignment: "对齐线", Debt: "一笔账", DialectScope: "约定卡",
}, "语义对象");

type Json = Record<string, unknown>;
const str = (value: unknown): string | null => typeof value === "string" ? value : null;
const arr = (value: unknown): Json[] => Array.isArray(value) ? value.filter((v): v is Json => v !== null && typeof v === "object") : [];

/** 对齐线白话标签（S9.1 模板）；`map_label` 只作备选。 */
export function alignmentPlainLabel(payload: Json): string {
  const relation = str(payload.relation) ?? "";
  const parameterNote = (role: string | null): string | null => {
    for (const entry of arr(payload.parameter_map)) {
      if (role !== null && entry.role !== role) continue;
      const map = (str(entry.map) ?? "").trim();
      const source = str(entry.source) ?? "", target = str(entry.target) ?? "";
      const compact = map.replace(/\s+/g, "");
      const identity = source === target && (compact === "" || ["=", "↦", "→"].some(arrow => compact === `${source}${arrow}${target}`));
      if (!identity && map) return map;
    }
    return null;
  };
  const withNote = (base: string, note: string | null) => note ? `${base}：${note}` : base;
  const lost = str(arr(payload.lost_information)[0]?.short);
  switch (relation) {
    case "expresses": return withNote("说的就是它", parameterNote(null));
    case "specializes": return withNote("特例", parameterNote("statement_parameter"));
    case "generalizes": return withNote("推广", parameterNote("statement_parameter"));
    case "overlaps_with": return withNote("部分重合", lost);
    default: return str(payload.map_label) ?? relationLabel(relation);
  }
}

/** 共享对象的白话名。 */
export function entityPlainLabel(payload: Json): string {
  const plain = str(payload.plain_label);
  if (plain) return plain;
  const labels = arr(payload.preferred_labels);
  return str((labels.find(label => label.lang === "zh") ?? labels[0])?.label) ?? "共享对象";
}
