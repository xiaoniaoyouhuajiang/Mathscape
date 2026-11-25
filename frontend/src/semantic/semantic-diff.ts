/**
 * 语义层变更对比与并排精读渲染器（契约 S9.2）：
 * - 解包 `FieldChange { action, value }`，生成人类可读的 `edit` 字段前后差异；
 * - 计算同一 change_set 历史版本修订（`v1 → v2` supersedes）的对象与字段差异；
 * - 为审核工作区右侧提供「来源原文 + 约定语境 ↔ 共享对象精确说法」并排对比与完整映射表。
 */
import { textElement } from "../core/dom";
import { texToPlain } from "../graph/tex-plain";
import { hasMessage, t } from "../i18n";
import type { markdownRenderer } from "../reader/markdown";
import type { PendingChangeSet, ProposalAnchor, ProposedObject } from "../review/proposal-model";

export type MarkdownRenderer = ReturnType<typeof markdownRenderer>;
import {
  alignmentPlainLabel,
  alignmentStatusLabel,
  baselineStatusLabel,
  debtStatusLabel,
  entityPlainLabel,
  failureModeLabel,
  lostDirectionLabel,
  relationLabel,
  representationBindingPlainLabel,
  semanticKindLabel,
  semanticObjectNoun,
} from "./labels";

type Json = Record<string, unknown>;
const str = (value: unknown): string | null => (typeof value === "string" ? value : null);
const arr = (value: unknown): Json[] =>
  Array.isArray(value) ? value.filter((v): v is Json => v !== null && typeof v === "object") : [];

export const DECLARATION_KIND_LABELS: Record<string, string> = {
  definition: "定义",
  axiom: "公理",
  theorem: "定理",
  lemma: "引理",
  proposition: "命题",
  conjecture: "猜想",
};

export const BINDER_ROLE_LABELS: Record<string, string> = {
  universal: "全称变量",
  index: "指标",
  parameter: "参数",
};

export function declarationKindLabel(kind: string | null | undefined): string {
  return (kind && DECLARATION_KIND_LABELS[kind]) || kind || "陈述";
}

export interface UnwrappedField<T = unknown> {
  isFieldChange: boolean;
  action: "set" | "clear" | "unchanged";
  value: T | undefined;
}

/** 解包 `FieldChange<T>`（`{ action: "set", value }` / `{ action: "clear" }`）或普通字段值。 */
export function unwrapFieldChange<T = unknown>(raw: unknown): UnwrappedField<T> {
  if (raw === undefined) {
    return { isFieldChange: false, action: "unchanged", value: undefined };
  }
  if (raw !== null && typeof raw === "object" && !Array.isArray(raw)) {
    const obj = raw as Json;
    if (obj.action === "set" && ("value" in obj || Object.keys(obj).length <= 2)) {
      return { isFieldChange: true, action: "set", value: obj.value as T };
    }
    if (obj.action === "clear" && Object.keys(obj).length === 1) {
      return { isFieldChange: true, action: "clear", value: undefined };
    }
  }
  return { isFieldChange: false, action: "set", value: raw as T };
}

/** 将 `edit` 的 `changes` 合并到基准对象 `basePayload`，得到提案生效后的完整语义视图。 */
export function effectiveProposalPayload(
  object: ProposedObject,
  basePayload?: Json | null,
): Json {
  if (object.op === "create") return (object.payload as Json | null | undefined) ?? {};
  if (object.op === "retire") return basePayload ? { ...basePayload } : {};
  const merged: Json = basePayload ? { ...basePayload } : {};
  const changes = (object.changes as Json | null | undefined) ?? {};
  for (const [key, raw] of Object.entries(changes)) {
    const unwrapped = unwrapFieldChange(raw);
    if (unwrapped.action === "clear") {
      delete merged[key];
    } else if (unwrapped.action === "set") {
      merged[key] = unwrapped.value;
    }
  }
  return merged;
}

export interface SemanticFieldDiff {
  field: string;
  fieldLabel: string;
  before: string;
  after: string;
  /** 纯文字版（不含花括号，用于左侧列表摘要）。 */
  plainSummary: string;
}

const FIELD_LABELS: Record<string, string> = {
  semantic_kind: "对象种类",
  home_theory: "所在理论",
  preferred_labels: "首选名称",
  aliases: "同义检索词",
  plain_label: "白话名称",
  conventions: "约定注记",
  mathlib_baseline: "mathlib 参照",
  declaration: "精确陈述",
  problem_schema: "题型结构",
  description: "说明",
  source: "来源单元",
  target: "共享对象",
  relation: "对齐关系",
  source_context: "来源约定语境",
  target_context: "目标理论语境",
  parameter_map: "参数对照",
  term_index_map: "数列项对照",
  required_assumptions: "所需前提",
  lost_information: "对不上的部分",
  evidence_spans: "原文证据",
  status: "状态",
  map_label: "地图标签",
  plain_note: "白话注记",
  debt_kind: "账目类型",
  attached_to: "挂接对象",
  obligation: "待补义务",
  quantifier: "适用量词",
  failure_mode: "不还清后果",
  failure_note: "风险补充",
  payoff_hint: "偿还思路",
  paid_by: "偿还依据",
  label: "约定卡名称",
  anchor_unit: "适用章节单元",
  applies_to: "作用范围",
  display_title: "显示标题",
  parent: "上级单元",
  target_branches: t("semantic.s2.targetBranches"),
  exclusions: t("semantic.s2.exclusionsHeading"),
  local_symbols: t("semantic.s3.localSymbolsHeading"),
  carrier: t("semantic.s3.carrier"),
  represented_object: t("semantic.s3.representedObject"),
  binders: "Binders",
  operations: t("semantic.s3.operationsHeading"),
  invariants_and_domain: t("semantic.s3.invariantsHeading"),
  dialect_scope: t("semantic.s3.dialectScope"),
  representation: t("semantic.s3.representationCore"),
  symbol_map: t("semantic.s3.symbolMapHeading"),
  index_and_domain_adaptations: t("semantic.s3.adaptationsHeading"),
  uses_entities: "Uses",
};

function formatBinders(binders: Json[]): string {
  if (!binders.length) return "—";
  return binders
    .map((b) => {
      const role = BINDER_ROLE_LABELS[str(b.role) ?? ""] ?? str(b.role) ?? "";
      return `${str(b.name) ?? "?"} : ${str(b.type) ?? "?"}${role ? `（${role}）` : ""}`;
    })
    .join("；");
}

function formatDeclarationSummary(decl: Json): string {
  const parts: string[] = [];
  if (str(decl.declaration_kind)) parts.push(`类型：${declarationKindLabel(str(decl.declaration_kind))}`);
  const binders = arr(decl.binders);
  if (binders.length) parts.push(`变量：${formatBinders(binders)}`);
  const assumptions = arr(decl.assumptions).map((a) => str(a.text)).filter(Boolean);
  if (assumptions.length) parts.push(`前提：${assumptions.join("；")}`);
  const stmt = (decl.statement ?? decl.definiens) as Json | undefined;
  if (stmt && str(stmt.text)) parts.push(`核心式：${str(stmt.text)}`);
  const branches = arr(decl.branches);
  if (branches.length) parts.push(t("semantic.s2.branchCount", { count: branches.length }));
  const exclusions = arr(decl.exclusions);
  if (exclusions.length) parts.push(t("semantic.s2.exclusionCount", { count: exclusions.length }));
  return parts.join(" | ") || "-";
}

function formatProblemSchemaSummary(ps: Json): string {
  const parts: string[] = [];
  const params = arr(ps.parameters);
  if (params.length) parts.push(`参数：${formatBinders(params)}`);
  const task = ps.task as Json | undefined;
  if (task && str(task.text)) parts.push(`任务：${str(task.text)}`);
  const branches = arr(ps.branches);
  if (branches.length) parts.push(t("semantic.s2.branchCount", { count: branches.length }));
  const exclusions = arr(ps.exclusions);
  if (exclusions.length) parts.push(t("semantic.s2.exclusionCount", { count: exclusions.length }));
  return parts.join(" | ") || "-";
}

export function formatTermIndexEntry(entry: Json): string {
  const src = str(entry.source) ?? "?";
  const dst = str(entry.target) ?? "?";
  const shift = typeof entry.index_shift === "number" ? entry.index_shift : 0;
  const valid = str(entry.valid_for) ? `（适用：${str(entry.valid_for)}）` : "";
  const note = str(entry.note) ? ` · ${str(entry.note)}` : "";
  if (shift === 0) {
    return `数列逐项对齐：${src} = ${dst}${valid}${note}`;
  }
  const signed = shift > 0 ? `+${shift}` : String(shift);
  return `${src} 对应 ${dst}（项号相差 ${signed}）${valid}${note}`;
}

function formatFieldValue(
  key: string,
  value: unknown,
  resolveTitle: (ref: string | null) => string,
): string {
  if (value === undefined || value === null) return "（空）";
  if (typeof value === "string") {
    if (key === "semantic_kind") return semanticKindLabel(value);
    if (key === "relation") return relationLabel(value);
    if (key === "failure_mode") return failureModeLabel(value);
    if (["home_theory", "source", "target", "target_context", "attached_to", "anchor_unit", "parent", "represented_object", "dialect_scope", "representation"].includes(key)) {
      return resolveTitle(value);
    }
    return value.trim() || "（空）";
  }
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  if (Array.isArray(value)) {
    if (value.length === 0) return "（无）";
    if (key === "preferred_labels") {
      return arr(value).map((l) => `${str(l.label) ?? ""}（${str(l.lang) ?? "?"}）`).join(" / ");
    }
    if (key === "aliases") {
      return value.map(str).filter(Boolean).join("、");
    }
    if (key === "conventions") {
      return arr(value).map((c) => str(c.text) ?? "").filter(Boolean).join("；") || `${value.length} 条约定`;
    }
    if (key === "parameter_map") {
      return arr(value).map((p) => str(p.map) ?? `${str(p.source) ?? "?"} ↔ ${str(p.target) ?? "?"}`).join("；");
    }
    if (key === "term_index_map") {
      return arr(value).map(formatTermIndexEntry).join("；");
    }
    if (key === "lost_information") {
      return arr(value).map((l) => `${str(l.short) ?? ""}${l.direction ? `（${lostDirectionLabel(str(l.direction))}）` : ""}`).join("；");
    }
    if (key === "required_assumptions") {
      return arr(value).map((a) => str(a.text) ?? "").filter(Boolean).join("；");
    }
    if (key === "evidence_spans") return `${value.length} 处原文范围`;
    if (key === "paid_by") return arr(value).map((r) => resolveTitle(str(r.ref))).join("、");
    if (key === "uses_entities") return value.map(str).filter(Boolean).map((id) => resolveTitle(id!)).join(" / ");
    if (key === "target_branches") return value.map(str).filter(Boolean).join(", ");
    if (key === "local_symbols") {
      return arr(value).map((s) => `${str(s.symbol) ?? "?"}: ${str(s.meaning) ?? ""}`).join(" | ");
    }
    if (key === "exclusions") {
      return arr(value).map((ex) => {
        const c = ex.case as Json | undefined;
        return `${str(c?.text) ?? ""} (${str(ex.reason) ?? ""})`;
      }).join(" | ");
    }
    if (key === "operations") {
      return arr(value).map((op) => {
        const sig = op.signature as Json | undefined;
        return `${str(op.name) ?? "?"}: ${str(sig?.text) ?? ""}`;
      }).join(" | ");
    }
    if (key === "invariants_and_domain" || key === "index_and_domain_adaptations") {
      return arr(value).map((item) => str(item.text) ?? "").filter(Boolean).join(" | ");
    }
    if (key === "symbol_map") {
      return arr(value).map((entry) => {
        const dst = entry.target_expr as Json | undefined;
        return `${str(entry.local_symbol) ?? "?"} -> ${str(dst?.text) ?? "?"}`;
      }).join(" | ");
    }
    return value.map((v) => formatFieldValue("", v, resolveTitle)).join("、");
  }
  if (typeof value === "object") {
    const obj = value as Json;
    if (key === "carrier") return str(obj.text) ?? "—";
    if (key === "mathlib_baseline") {
      if (!str(obj.decl)) return str(obj.note) ?? "暂无对应名称";
      return `${str(obj.decl)}（${baselineStatusLabel(str(obj.status))}）${str(obj.note) ? ` · ${str(obj.note)}` : ""}`;
    }
    if (key === "declaration") return formatDeclarationSummary(obj);
    if (key === "problem_schema") return formatProblemSchemaSummary(obj);
    if (key === "source_context") {
      if (str(obj.dialect_scope)) return `约定卡：${resolveTitle(str(obj.dialect_scope))}`;
      return `未设约定卡（${str(obj.reason) ?? "未说明原因"}）`;
    }
  }
  return texToPlain(String(value));
}

/** 计算 `edit` 提案相对基准 payload 的逐字段差异。 */
export function computeSemanticEditDiff(
  kind: string,
  basePayload: Json | null | undefined,
  changes: Json | null | undefined,
  resolveTitle: (ref: string | null) => string,
): SemanticFieldDiff[] {
  if (!changes) return [];
  const diffs: SemanticFieldDiff[] = [];
  for (const [field, raw] of Object.entries(changes)) {
    const unwrapped = unwrapFieldChange(raw);
    if (unwrapped.action === "unchanged") continue;
    const fieldLabel =
      field === "status"
        ? kind === "debt" || kind === "Debt"
          ? "账目状态"
          : "对齐状态"
        : FIELD_LABELS[field] ?? field;
    const formatStatus = (v: unknown): string => {
      if (field === "status" && typeof v === "string") {
        return kind === "debt" || kind === "Debt" ? debtStatusLabel(v) : alignmentStatusLabel(v);
      }
      return formatFieldValue(field, v, resolveTitle);
    };
    const before = basePayload ? formatStatus(basePayload[field]) : "（原值）";
    const after = unwrapped.action === "clear" ? "（清空）" : formatStatus(unwrapped.value);
    if (before === after) continue;
    const plainBefore = texToPlain(before).replace(/[{}]/g, "");
    const plainAfter = texToPlain(after).replace(/[{}]/g, "");
    diffs.push({
      field,
      fieldLabel,
      before,
      after,
      plainSummary: basePayload ? `${fieldLabel}：${plainBefore} → ${plainAfter}` : `将${fieldLabel}改为：${plainAfter}`,
    });
  }
  return diffs;
}

/** 计算两个完整 payload 之间的字段差异（用于 Inspector 中暂存对象对比已接受旧版本）。 */
export function computePayloadRevisionDiff(
  kind: string,
  acceptedPayload: Json,
  stagedPayload: Json,
  resolveTitle: (ref: string | null) => string,
): SemanticFieldDiff[] {
  const keys = new Set([...Object.keys(acceptedPayload), ...Object.keys(stagedPayload)]);
  keys.delete("trust");
  keys.delete("reviewed");
  const syntheticChanges: Json = {};
  for (const key of keys) {
    if (JSON.stringify(acceptedPayload[key]) !== JSON.stringify(stagedPayload[key])) {
      syntheticChanges[key] =
        stagedPayload[key] === undefined ? { action: "clear" } : { action: "set", value: stagedPayload[key] };
    }
  }
  return computeSemanticEditDiff(kind, acceptedPayload, syntheticChanges, resolveTitle);
}

export interface SupersedesDiffSummary {
  fromVersion: number;
  toVersion: number;
  rationaleChanged: boolean;
  added: string[];
  removed: string[];
  modified: Array<{ localRef: string; title: string; summaries: string[] }>;
}

function objectDisplayName(obj: ProposedObject, resolveTitle: (ref: string | null) => string): string {
  const body = effectiveProposalPayload(obj);
  if (obj.kind === "semantic_entity" || obj.kind === "representation_core") return entityPlainLabel(body);
  if (obj.kind === "alignment") return alignmentPlainLabel(body);
  if (obj.kind === "representation_binding") return representationBindingPlainLabel(body);
  return str(body.display_title) ?? str(body.label) ?? str(body.obligation) ?? resolveTitle(obj.target_id ?? null) ?? obj.local_ref;
}

/** 计算同一 `change_set` 旧版本提案与当前版本提案之间的修订差异（`v(k-1) → v(k)`）。 */
export function computeSupersedesDiff(
  oldVersion: number,
  oldProposal: PendingChangeSet["proposal"] | undefined,
  current: PendingChangeSet,
  resolveTitle: (ref: string | null) => string,
): SupersedesDiffSummary | null {
  if (!oldProposal) return null;
  const oldObjs = new Map((oldProposal.proposed_objects ?? []).map((o) => [o.local_ref, o]));
  const curObjs = new Map((current.proposal.proposed_objects ?? []).map((o) => [o.local_ref, o]));
  const added: string[] = [];
  const removed: string[] = [];
  const modified: Array<{ localRef: string; title: string; summaries: string[] }> = [];

  for (const [ref, obj] of curObjs) {
    if (obj.kind === "source_span") continue;
    const prev = oldObjs.get(ref);
    if (!prev) {
      added.push(`${semanticObjectNoun(obj.kind)}「${texToPlain(objectDisplayName(obj, resolveTitle)).replace(/[{}]/g, "")}」`);
      continue;
    }
    const prevBody = effectiveProposalPayload(prev);
    const curBody = effectiveProposalPayload(obj);
    if (prev.op !== obj.op) {
      modified.push({
        localRef: ref,
        title: texToPlain(objectDisplayName(obj, resolveTitle)).replace(/[{}]/g, ""),
        summaries: [`操作从 ${prev.op} 改为 ${obj.op}`],
      });
      continue;
    }
    const diffs = computePayloadRevisionDiff(obj.kind, prevBody, curBody, resolveTitle);
    if (diffs.length > 0) {
      modified.push({
        localRef: ref,
        title: texToPlain(objectDisplayName(obj, resolveTitle)).replace(/[{}]/g, ""),
        summaries: diffs.map((d) => d.plainSummary),
      });
    }
  }
  for (const [ref, prev] of oldObjs) {
    if (prev.kind === "source_span" || curObjs.has(ref)) continue;
    removed.push(`${semanticObjectNoun(prev.kind)}「${texToPlain(objectDisplayName(prev, resolveTitle)).replace(/[{}]/g, "")}」`);
  }
  const rationaleChanged = (oldProposal.rationale ?? "").trim() !== (current.proposal.rationale ?? "").trim();
  return {
    fromVersion: oldVersion,
    toVersion: current.version,
    rationaleChanged,
    added,
    removed,
    modified,
  };
}

// ------------------------------------------------ DOM Renderers (KaTeX)

export function renderMathNode(md: MarkdownRenderer, text: string, className = "semantic-math-block"): HTMLElement {
  const box = document.createElement("div");
  box.className = className;
  box.innerHTML = md.render(text);
  return box;
}

function kvRow(label: string, content: string | HTMLElement): HTMLElement {
  const row = textElement("div", "semantic-row", "");
  row.append(textElement("span", "semantic-term", label));
  if (typeof content === "string") row.append(textElement("span", "semantic-value", content));
  else row.append(content);
  return row;
}

/** 渲染字段级前后对比表（用于 `edit` 提案或 Inspector 暂存对比）。 */
export function renderFieldDiffTable(diffs: SemanticFieldDiff[], md: MarkdownRenderer, caption: string): HTMLElement {
  const section = textElement("section", "semantic-diff-section", "");
  section.append(textElement("h4", "semantic-diff-heading", caption));
  if (diffs.length === 0) {
    section.append(textElement("p", "semantic-muted", "无实质字段变化。"));
    return section;
  }
  const table = textElement("table", "semantic-table semantic-diff-table", "");
  const thead = document.createElement("thead");
  const headRow = document.createElement("tr");
  headRow.append(
    textElement("th", "", "修改字段"),
    textElement("th", "", "修改前（基准）"),
    textElement("th", "", "修改后（本提案）"),
  );
  thead.append(headRow);
  const tbody = document.createElement("tbody");
  for (const diff of diffs) {
    const tr = document.createElement("tr");
    const tdBefore = document.createElement("td");
    tdBefore.className = "semantic-diff-before";
    tdBefore.append(renderMathNode(md, diff.before, "semantic-diff-cell"));
    const tdAfter = document.createElement("td");
    tdAfter.className = "semantic-diff-after";
    tdAfter.append(renderMathNode(md, diff.after, "semantic-diff-cell"));
    tr.append(textElement("td", "semantic-diff-field", diff.fieldLabel), tdBefore, tdAfter);
    tbody.append(tr);
  }
  table.append(thead, tbody);
  section.append(table);
  return section;
}

function partitionStatusLabel(status: string | null): string {
  const key = `semantic.s2.partition.${status ?? "unspecified"}`;
  return hasMessage(key) ? t(key) : (status ?? "—");
}

function exclusionKindLabel(kind: string | null): string {
  const key = `semantic.s2.exclusion.${kind ?? ""}`;
  return hasMessage(key) ? t(key) : (kind ?? "—");
}

function symbolRoleLabel(role: string | null): string {
  const key = `semantic.s3.symbolRole.${role ?? ""}`;
  return hasMessage(key) ? t(key) : (role ?? "—");
}

function renderSegmentedBodySections(
  md: MarkdownRenderer,
  body: Json,
  conclusionKey: "conclusion" | "task",
  container: HTMLElement,
): void {
  const guards = arr(body.guards);
  const guardLabelById = new Map<string, string>();
  for (const g of guards) {
    const gid = str(g.guard_id);
    const pred = g.predicate as Json | undefined;
    if (gid) guardLabelById.set(gid, str(g.label) ?? str(pred?.text) ?? gid);
  }
  const partition = body.guard_partition as Json | undefined;
  if (partition && str(partition.status)) {
    const partText = `${partitionStatusLabel(str(partition.status))}${str(partition.note) ? ` · ${str(partition.note)}` : ""}`;
    container.append(kvRow(t("semantic.s2.guardPartition"), renderMathNode(md, partText, "semantic-value semantic-markdown")));
  }
  if (guards.length > 0) {
    const ul = textElement("ul", "semantic-list semantic-guards-list", "");
    for (const g of guards) {
      const li = document.createElement("li");
      const head = str(g.label) ?? str(g.guard_id) ?? "guard";
      li.append(textElement("strong", "", `${head}: `));
      const pred = g.predicate as Json | undefined;
      if (pred && str(pred.text)) {
        li.append(renderMathNode(md, str(pred.text)!, "semantic-markdown"));
      }
      ul.append(li);
    }
    container.append(kvRow(t("semantic.s2.guardsHeading"), ul));
  }
  const branches = arr(body.branches);
  if (branches.length > 0) {
    const branchList = textElement("div", "semantic-branches-list", "");
    for (const b of branches) {
      const box = textElement("div", `semantic-branch-item${b.is_degenerate ? " is-degenerate" : ""}`, "");
      const header = textElement("div", "semantic-branch-header", "");
      header.append(
        textElement("strong", "", str(b.label) ?? str(b.branch_id) ?? "branch"),
        textElement(
          "small",
          "semantic-tag",
          b.is_degenerate ? t("semantic.s2.degenerateBadge") : t("semantic.s2.regularBadge"),
        ),
      );
      const guardIds = Array.isArray(b.guards) ? b.guards.map(str).filter((v): v is string => !!v) : [];
      if (guardIds.length > 0) {
        const gNames = guardIds.map((id) => guardLabelById.get(id) ?? id).join(", ");
        header.append(textElement("small", "semantic-muted", t("semantic.s2.branchGuards", { guards: gNames })));
      }
      box.append(header);
      const concl = b[conclusionKey] as Json | undefined;
      if (concl && str(concl.text)) {
        box.append(renderMathNode(md, str(concl.text)!, "semantic-statement-box semantic-markdown"));
      }
      branchList.append(box);
    }
    container.append(kvRow(t("semantic.s2.branchesHeading"), branchList));
  }
  const exclusions = arr(body.exclusions);
  if (exclusions.length > 0) {
    const ul = textElement("ul", "semantic-list semantic-exclusions-list", "");
    for (const ex of exclusions) {
      const li = document.createElement("li");
      li.append(textElement("small", "semantic-tag", exclusionKindLabel(str(ex.kind))));
      const caseObj = ex.case as Json | undefined;
      if (caseObj && str(caseObj.text)) {
        li.append(renderMathNode(md, str(caseObj.text)!, "semantic-markdown"));
      }
      if (str(ex.reason)) {
        li.append(renderMathNode(md, str(ex.reason)!, "semantic-muted semantic-markdown"));
      }
      ul.append(li);
    }
    container.append(kvRow(t("semantic.s2.exclusionsHeading"), ul));
  }
}

export function renderLocalSymbolsCard(md: MarkdownRenderer, symbols: Json[]): HTMLElement {
  const card = textElement("section", "semantic-card semantic-local-symbols-card", "");
  card.append(textElement("h4", "", t("semantic.s3.localSymbolsHeading")));
  const table = textElement("table", "semantic-table", "");
  for (const sym of symbols) {
    const tr = document.createElement("tr");
    const symCell = renderMathNode(md, `$${str(sym.symbol) ?? "?"}$`, "semantic-td-cell");
    const roleCell = textElement("td", "", symbolRoleLabel(str(sym.role)));
    const meaningCell = document.createElement("td");
    meaningCell.append(renderMathNode(md, str(sym.meaning) ?? "—", "semantic-markdown"));
    const dom = sym.domain as Json | undefined;
    if (dom && str(dom.text)) {
      meaningCell.append(renderMathNode(md, str(dom.text)!, "semantic-muted semantic-markdown"));
    }
    tr.append(symCell, roleCell, meaningCell);
    table.append(tr);
  }
  card.append(table);
  return card;
}

export function renderRepresentationCoreCardBody(
  md: MarkdownRenderer,
  p: Json,
  resolveTitle: (ref: string | null) => string,
): HTMLElement {
  const wrap = textElement("div", "semantic-entity-body", "");
  wrap.append(kvRow(FIELD_LABELS.semantic_kind, t("semantic.kind.representation_core")));
  const labels = arr(p.preferred_labels).map((l) => `${str(l.label) ?? ""} (${str(l.lang) ?? "?"})`);
  if (labels.length) wrap.append(kvRow(FIELD_LABELS.preferred_labels, labels.join(" / ")));
  const aliases = Array.isArray(p.aliases) ? p.aliases.map(str).filter((v): v is string => !!v) : [];
  if (aliases.length) wrap.append(kvRow(FIELD_LABELS.aliases, aliases.join(", ")));
  if (str(p.home_theory)) wrap.append(kvRow(FIELD_LABELS.home_theory, resolveTitle(str(p.home_theory))));
  if (str(p.represented_object)) wrap.append(kvRow(t("semantic.s3.representedObject"), resolveTitle(str(p.represented_object))));
  if (str(p.description)) {
    wrap.append(kvRow(FIELD_LABELS.description, renderMathNode(md, str(p.description)!, "semantic-value semantic-markdown")));
  }
  const carrier = p.carrier as Json | undefined;
  if (carrier && str(carrier.text)) {
    const cNode = renderMathNode(md, str(carrier.text)!, "semantic-statement-box semantic-markdown");
    if (str(carrier.note)) cNode.append(textElement("small", "semantic-muted", str(carrier.note)!));
    wrap.append(kvRow(t("semantic.s3.carrier"), cNode));
  }
  const binders = arr(p.binders);
  if (binders.length) {
    wrap.append(kvRow(t("semantic.common.binders"), renderMathNode(md, formatBinders(binders), "semantic-value semantic-markdown")));
  }
  const ops = arr(p.operations);
  if (ops.length > 0) {
    const opsBox = textElement("section", "semantic-card", "");
    opsBox.append(textElement("h4", "", t("semantic.s3.operationsHeading")));
    const ul = textElement("ul", "semantic-list", "");
    for (const op of ops) {
      const li = document.createElement("li");
      li.append(textElement("strong", "", `${str(op.name) ?? "op"}: `));
      const sig = op.signature as Json | undefined;
      if (sig && str(sig.text)) li.append(renderMathNode(md, str(sig.text)!, "semantic-markdown"));
      if (str(op.note)) li.append(textElement("small", "semantic-muted", str(op.note)!));
      ul.append(li);
    }
    opsBox.append(ul);
    wrap.append(opsBox);
  }
  const invariants = arr(p.invariants_and_domain);
  if (invariants.length > 0) {
    const invBox = textElement("section", "semantic-card", "");
    invBox.append(textElement("h4", "", t("semantic.s3.invariantsHeading")));
    const ul = textElement("ul", "semantic-list", "");
    for (const inv of invariants) {
      if (!str(inv.text)) continue;
      const li = document.createElement("li");
      li.append(renderMathNode(md, str(inv.text)!, "semantic-markdown"));
      ul.append(li);
    }
    invBox.append(ul);
    wrap.append(invBox);
  }
  const baseline = p.mathlib_baseline as Json | null | undefined;
  if (baseline) {
    const bCard = textElement("section", "semantic-card semantic-baseline", "");
    bCard.append(textElement("h4", "", FIELD_LABELS.mathlib_baseline));
    if (!str(baseline.decl)) {
      bCard.append(textElement("p", "semantic-muted", str(baseline.note) ?? t("semantic.common.baselineNone")));
    } else {
      bCard.append(
        kvRow(t("semantic.common.baselineDecl"), str(baseline.decl)!),
        kvRow(t("semantic.common.baselineStatus"), baselineStatusLabel(str(baseline.status) ?? "unchecked")),
      );
      if (str(baseline.module)) bCard.append(kvRow(t("semantic.common.baselineModule"), str(baseline.module)!));
      if (str(baseline.note)) bCard.append(kvRow(t("semantic.common.baselineNote"), renderMathNode(md, str(baseline.note)!, "semantic-value semantic-markdown")));
    }
    wrap.append(bCard);
  }
  return wrap;
}

export function renderSymbolMapTable(md: MarkdownRenderer, entries: Json[]): HTMLElement {
  const table = textElement("table", "semantic-table", "");
  table.append(Object.assign(document.createElement("caption"), { textContent: t("semantic.s3.symbolMapHeading") }));
  const thead = document.createElement("thead");
  const hr = document.createElement("tr");
  hr.append(
    textElement("th", "", t("semantic.s3.localSymbolCol")),
    textElement("th", "", ""),
    textElement("th", "", t("semantic.s3.targetExprCol")),
    textElement("th", "", t("semantic.s3.noteCol")),
  );
  thead.append(hr);
  const tbody = document.createElement("tbody");
  for (const entry of entries) {
    const tr = document.createElement("tr");
    const dst = entry.target_expr as Json | undefined;
    tr.append(
      renderMathNode(md, `$${str(entry.local_symbol) ?? "?"}$`, "semantic-td-cell"),
      textElement("td", "", "↦"),
      renderMathNode(md, str(dst?.text) ?? "—", "semantic-td-cell"),
      renderMathNode(md, str(entry.note) ?? "—", "semantic-td-cell"),
    );
    tbody.append(tr);
  }
  table.append(thead, tbody);
  return table;
}
export function renderSemanticEntityCardBody(
  md: MarkdownRenderer,
  p: Json,
  resolveTitle: (ref: string | null) => string,
): HTMLElement {
  const wrap = textElement("div", "semantic-entity-body", "");
  const kind = str(p.semantic_kind);
  wrap.append(kvRow("对象种类", semanticKindLabel(kind)));
  const labels = arr(p.preferred_labels).map((l) => `${str(l.label) ?? ""}（${str(l.lang) ?? "?"}）`);
  if (labels.length) wrap.append(kvRow("首选名称", labels.join(" / ")));
  const aliases = Array.isArray(p.aliases) ? p.aliases.map(str).filter((v): v is string => !!v) : [];
  if (aliases.length) wrap.append(kvRow("同义检索词", aliases.join("、")));
  if (str(p.home_theory)) wrap.append(kvRow("所在理论", resolveTitle(str(p.home_theory))));
  if (str(p.description)) {
    wrap.append(kvRow("说明", renderMathNode(md, str(p.description)!, "semantic-value semantic-markdown")));
  }

  const decl = p.declaration as Json | null | undefined;
  if (decl) {
    const declBox = textElement("section", "semantic-card semantic-declaration-card", "");
    declBox.append(
      textElement("h4", "", `精确陈述 · ${declarationKindLabel(str(decl.declaration_kind))}`),
    );
    const binders = arr(decl.binders);
    if (binders.length) {
      declBox.append(kvRow("绑定变量", renderMathNode(md, formatBinders(binders), "semantic-value semantic-markdown")));
    }
    const assumptions = arr(decl.assumptions);
    if (assumptions.length) {
      const ul = textElement("ul", "semantic-list", "");
      for (const a of assumptions) {
        if (!str(a.text)) continue;
        const li = document.createElement("li");
        li.append(renderMathNode(md, str(a.text)!, "semantic-markdown"));
        if (str(a.note)) li.append(textElement("small", "semantic-muted", `注：${str(a.note)}`));
        ul.append(li);
      }
      declBox.append(kvRow("成立前提", ul));
    }
    const stmt = decl.statement as Json | undefined;
    if (stmt && str(stmt.text)) {
      declBox.append(kvRow("命题陈述", renderMathNode(md, str(stmt.text)!, "semantic-statement-box semantic-markdown")));
    }
    const defn = decl.definiens as Json | undefined;
    if (defn && str(defn.text)) {
      declBox.append(kvRow("定义式", renderMathNode(md, str(defn.text)!, "semantic-statement-box semantic-markdown")));
    }
    renderSegmentedBodySections(md, decl, "conclusion", declBox);
    if (str(decl.concept)) {
      declBox.append(kvRow("定义概念", resolveTitle(str(decl.concept))));
    }
    const uses = Array.isArray(decl.uses_entities) ? decl.uses_entities.map(str).filter((v): v is string => !!v) : [];
    if (uses.length) {
      declBox.append(kvRow("依赖对象", uses.map((id) => resolveTitle(id)).join("、")));
    }
    wrap.append(declBox);
  }

  const schema = p.problem_schema as Json | null | undefined;
  if (schema) {
    const schemaBox = textElement("section", "semantic-card semantic-declaration-card", "");
    schemaBox.append(textElement("h4", "", "题型结构（Problem Schema）"));
    const params = arr(schema.parameters);
    if (params.length) {
      schemaBox.append(kvRow("题型参数", renderMathNode(md, formatBinders(params), "semantic-value semantic-markdown")));
    }
    const assumptions = arr(schema.assumptions);
    if (assumptions.length) {
      const ul = textElement("ul", "semantic-list", "");
      for (const a of assumptions) {
        if (str(a.text)) {
          const li = document.createElement("li");
          li.append(renderMathNode(md, str(a.text)!, "semantic-markdown"));
          ul.append(li);
        }
      }
      schemaBox.append(kvRow("题设条件", ul));
    }
    const task = schema.task as Json | undefined;
    if (task && str(task.text)) {
      schemaBox.append(kvRow("求解/求证任务", renderMathNode(md, str(task.text)!, "semantic-statement-box semantic-markdown")));
    }
    renderSegmentedBodySections(md, schema, "task", schemaBox);
    const uses = Array.isArray(schema.uses_entities) ? schema.uses_entities.map(str).filter((v): v is string => !!v) : [];
    if (uses.length) {
      schemaBox.append(kvRow("涉及共享对象", uses.map((id) => resolveTitle(id)).join("、")));
    }
    wrap.append(schemaBox);
  }

  const conventions = arr(p.conventions);
  if (conventions.length) {
    const convBox = textElement("section", "semantic-card semantic-dialect-card", "");
    convBox.append(textElement("h4", "", "共享基线约定"));
    for (const conv of conventions) {
      const row = document.createElement("div");
      row.className = "semantic-convention";
      row.append(renderMathNode(md, str(conv.text) ?? "—", "semantic-markdown"));
      const tags: string[] = [];
      if (conv.explicitness === "explicit") tags.push("明说");
      else if (conv.explicitness === "inherited") tags.push("沿用上级");
      else if (conv.explicitness) tags.push("推断");
      if (conv.basis === "mathlib") tags.push("按 mathlib");
      if (tags.length) row.append(textElement("small", "semantic-tag", tags.join(" · ")));
      convBox.append(row);
    }
    wrap.append(convBox);
  }

  const baseline = p.mathlib_baseline as Json | null | undefined;
  if (baseline) {
    const bCard = textElement("section", "semantic-card semantic-baseline", "");
    bCard.append(textElement("h4", "", "mathlib 参照"));
    if (!str(baseline.decl)) {
      bCard.append(textElement("p", "semantic-muted", str(baseline.note) ?? "mathlib 中暂无对应名称。"));
    } else {
      bCard.append(
        kvRow("声明名称", str(baseline.decl)!),
        kvRow("核对状态", baselineStatusLabel(str(baseline.status) ?? "unchecked")),
      );
      if (str(baseline.module)) bCard.append(kvRow("模块路径", str(baseline.module)!));
      if (str(baseline.relation_hint)) bCard.append(kvRow("对应关系提示", relationLabel(str(baseline.relation_hint))));
      if (str(baseline.note)) bCard.append(kvRow("对照注记", renderMathNode(md, str(baseline.note)!, "semantic-value semantic-markdown")));
    }
    wrap.append(bCard);
  }
  return wrap;
}

function lookupObjectInProposalOrBase(
  item: PendingChangeSet,
  rawRef: string | null,
): { kind: string; payload: Json } | null {
  if (!rawRef) return null;
  const local = rawRef.replace(/^\$new:/, "");
  const proposed = (item.proposal.proposed_objects ?? []).find((o) => o.local_ref === local);
  if (proposed) {
    const base = proposed.target_id ? item.baseObjects?.[proposed.target_id]?.payload ?? null : null;
    return { kind: proposed.kind, payload: effectiveProposalPayload(proposed, base) };
  }
  const baseObj = item.baseObjects?.[rawRef];
  if (baseObj) return { kind: baseObj.kind, payload: baseObj.payload };
  return null;
}

function resolveAnyRefTitle(item: PendingChangeSet, rawRef: string | null): string {
  if (!rawRef) return "—";
  const local = rawRef.replace(/^\$new:/, "");
  const proposed = (item.proposal.proposed_objects ?? []).find((o) => o.local_ref === local);
  if (proposed) {
    const base = proposed.target_id ? item.baseObjects?.[proposed.target_id]?.payload ?? null : null;
    const p = effectiveProposalPayload(proposed, base);
    if (proposed.kind === "semantic_entity" || proposed.kind === "representation_core") return entityPlainLabel(p);
    if (proposed.kind === "alignment") return alignmentPlainLabel(p);
    if (proposed.kind === "representation_binding") return representationBindingPlainLabel(p);
    return str(p.display_title) ?? str(p.label) ?? str(p.obligation) ?? item.targetTitles?.[proposed.target_id ?? ""] ?? local;
  }
  return item.targetTitles?.[rawRef] ?? (rawRef.length > 32 ? `${rawRef.slice(0, 24)}…` : rawRef);
}

/**
 * 为审核工作区右侧 `.review-preview` 渲染一个语义卡片的完整精读对比面板：
 * - `Alignment`：左栏「来源原文摘录 + 约定语境」↔ 右栏「共享数学对象完整陈述」+ 下方四张对齐映射表；
 * - `SemanticEntity` / `Debt` / `DialectScope`：完整数学排版卡片；
 * - `edit` / `retire`：附带修改前后逐字段对比或退役级联失效账目清单。
 */
export function renderSemanticComparisonPanel(
  md: MarkdownRenderer,
  item: PendingChangeSet,
  localRef: string,
): HTMLElement | null {
  const object = (item.proposal.proposed_objects ?? []).find((o) => o.local_ref === localRef);
  if (!object) return null;
  const resolveTitle = (ref: string | null) => resolveAnyRefTitle(item, ref);
  const basePayload = object.target_id ? item.baseObjects?.[object.target_id]?.payload ?? null : null;
  const p = effectiveProposalPayload(object, basePayload);
  const panel = textElement("section", `semantic-comparison-panel kind-${object.kind} op-${object.op}`, "");

  // 1. 退役预览横幅与级联失效账目
  if (object.op === "retire") {
    const retireBox = textElement("section", "semantic-card semantic-retire-banner", "");
    retireBox.append(
      textElement("h4", "", `提议退役${semanticObjectNoun(object.kind)}：${resolveTitle(object.target_id ?? null)}`),
      textElement("p", "semantic-muted", "接受后该对象将转为退役状态（历史修订保留，不再参与活跃地图投影）。"),
    );
    const cascades = (object.target_id && item.retireCascades?.[object.target_id]) ?? [];
    if (cascades.length > 0) {
      retireBox.append(textElement("p", "semantic-warning-line", `以下 ${cascades.length} 笔未结清的账将随之自动标记为「已失效（obsolete）」：`));
      const ul = textElement("ul", "semantic-list", "");
      for (const debtId of cascades) {
        ul.append(textElement("li", "", resolveTitle(debtId)));
      }
      retireBox.append(ul);
    }
    panel.append(retireBox);
  }

  // 2. 修改（edit）字段级前后对比表
  if (object.op === "edit") {
    const diffs = computeSemanticEditDiff(object.kind, basePayload, object.changes ?? null, resolveTitle);
    panel.append(renderFieldDiffTable(diffs, md, `字段修改对比（${resolveTitle(object.target_id ?? null)}）`));
  }

  // 3. 按语义对象类型展示核心内容
  if (object.kind === "alignment") {
    const plain = alignmentPlainLabel(p);
    const banner = textElement("div", "semantic-alignment-banner", "");
    banner.append(
      textElement("span", "semantic-relation-pill", relationLabel(str(p.relation))),
      textElement("strong", "", plain),
      textElement("span", "semantic-status-pill", alignmentStatusLabel(str(p.status))),
    );
    if (str(p.plain_note)) {
      banner.append(renderMathNode(md, str(p.plain_note)!, "semantic-plain-note semantic-markdown"));
    }
    const targetBranches = Array.isArray(p.target_branches)
      ? p.target_branches.map(str).filter((v): v is string => !!v)
      : [];
    if (targetBranches.length > 0) {
      banner.append(
        textElement("small", "semantic-tag", `${t("semantic.s2.targetBranches")}: ${targetBranches.join(", ")}`),
      );
    }
    panel.append(banner);

    // 左右并排：来源原文与约定卡 ↔ 共享数学对象
    const grid = textElement("div", "semantic-side-by-side", "");
    const leftCol = textElement("section", "semantic-pane semantic-pane-source", "");
    const sourceRef = str(p.source);
    leftCol.append(
      textElement("div", "semantic-pane-eyebrow", "来源表述（教材 / 手稿）"),
      textElement("h4", "semantic-pane-title", resolveTitle(sourceRef)),
    );

    // 来源语境（DialectScope）
    const srcCtx = p.source_context as Json | undefined;
    if (srcCtx) {
      if (str(srcCtx.dialect_scope)) {
        const scopeRef = str(srcCtx.dialect_scope)!;
        const scopeObj = lookupObjectInProposalOrBase(item, scopeRef);
        const scopeBox = textElement("div", "semantic-context-box", "");
        scopeBox.append(textElement("strong", "", `本节约定卡：${resolveTitle(scopeRef)}`));
        if (scopeObj) {
          for (const conv of arr(scopeObj.payload.conventions)) {
            if (str(conv.text)) {
              scopeBox.append(renderMathNode(md, `• ${str(conv.text)}`, "semantic-convention-inline semantic-markdown"));
            }
          }
        }
        leftCol.append(scopeBox);
      } else {
        leftCol.append(
          textElement("p", "semantic-context-unscoped", `未设约定卡（原因：${str(srcCtx.reason) ?? "未注明"}）`),
        );
      }
    }

    // 来源原文引文（直接读取后端核对过的 anchors，支持多书切换零重载预览）
    const anchorMap = new Map<string, ProposalAnchor>();
    for (const a of item.anchors ?? []) {
      anchorMap.set(a.span_ref, a);
      if (a.span_id) anchorMap.set(a.span_id, a);
    }
    const spanRefs = Array.isArray(p.evidence_spans)
      ? p.evidence_spans.map((s) => (typeof s === "string" ? s.replace(/^\$new:/, "") : null)).filter((s): s is string => !!s)
      : [];
    const sourceObj = lookupObjectInProposalOrBase(item, sourceRef);
    if (spanRefs.length === 0 && sourceObj && Array.isArray(sourceObj.payload.source_spans)) {
      for (const s of sourceObj.payload.source_spans) {
        if (typeof s === "string") spanRefs.push(s.replace(/^\$new:/, ""));
      }
    }
    const matchedAnchors = spanRefs.map((ref) => anchorMap.get(ref)).filter((a): a is ProposalAnchor => !!a);
    if (matchedAnchors.length > 0) {
      for (const anc of matchedAnchors) {
        const quoteCard = textElement("blockquote", "semantic-source-quote", "");
        const metaParts = [
          anc.verified ? "已与快照字节核对" : "按提案引文显示",
          anc.page ? `PDF 第 ${anc.page.page_number} 页` : null,
        ].filter(Boolean);
        quoteCard.append(
          textElement("small", "semantic-quote-meta", metaParts.join(" · ")),
          renderMathNode(md, anc.excerpt, "semantic-markdown"),
        );
        leftCol.append(quoteCard);
      }
    } else if (sourceObj && str(sourceObj.payload.summary)) {
      leftCol.append(renderMathNode(md, str(sourceObj.payload.summary)!, "semantic-source-quote semantic-markdown"));
    } else {
      leftCol.append(textElement("p", "semantic-muted", "下方原文锚点区可查看完整上下文。"));
    }

    // 右栏：共享数学对象完整卡片
    const rightCol = textElement("section", "semantic-pane semantic-pane-target", "");
    const targetRef = str(p.target);
    rightCol.append(
      textElement("div", "semantic-pane-eyebrow", "共享数学对象（统一基线）"),
      textElement("h4", "semantic-pane-title", resolveTitle(targetRef)),
    );
    const targetObj = lookupObjectInProposalOrBase(item, targetRef);
    if (targetObj && targetObj.kind === "semantic_entity") {
      rightCol.append(renderSemanticEntityCardBody(md, targetObj.payload, resolveTitle));
    } else {
      rightCol.append(textElement("p", "semantic-muted", "共享对象详情将在接受或加载后显示。"));
    }

    grid.append(leftCol, rightCol);
    panel.append(grid);

    // 四张对齐映射明细表：parameter_map / term_index_map / lost_information / required_assumptions
    const mappings = textElement("div", "semantic-alignment-mappings", "");

    const params = arr(p.parameter_map);
    if (params.length > 0) {
      const table = textElement("table", "semantic-table", "");
      table.append(Object.assign(document.createElement("caption"), { textContent: "参数对照（哪一条命题对应哪一条命题）" }));
      const thead = document.createElement("thead");
      const hr = document.createElement("tr");
      hr.append(
        textElement("th", "", "来源参数"),
        textElement("th", "", ""),
        textElement("th", "", "共享参数"),
        textElement("th", "", "对应关系"),
      );
      thead.append(hr);
      const tbody = document.createElement("tbody");
      for (const entry of params) {
        const tr = document.createElement("tr");
        const srcText = `${str(entry.source) ?? ""}${str(entry.source_range) ? `（${str(entry.source_range)}）` : ""}`;
        const dstText = `${str(entry.target) ?? ""}${str(entry.target_range) ? `（${str(entry.target_range)}）` : ""}`;
        const mapTd = document.createElement("td");
        mapTd.append(renderMathNode(md, str(entry.map) ?? "—", "semantic-markdown"));
        if (str(entry.note)) mapTd.append(textElement("small", "semantic-muted", str(entry.note)!));
        tr.append(
          renderMathNode(md, srcText, "semantic-td-cell"),
          textElement("td", "", "↔"),
          renderMathNode(md, dstText, "semantic-td-cell"),
          mapTd,
        );
        tbody.append(tr);
      }
      table.append(thead, tbody);
      mappings.append(table);
    }

    const terms = arr(p.term_index_map);
    if (terms.length > 0) {
      const sec = textElement("section", "semantic-card", "");
      sec.append(textElement("h4", "", "数列项对照（项与项之间的对应）"));
      const ul = textElement("ul", "semantic-list", "");
      for (const entry of terms) {
        const li = document.createElement("li");
        li.append(renderMathNode(md, formatTermIndexEntry(entry), "semantic-markdown"));
        ul.append(li);
      }
      sec.append(ul);
      mappings.append(sec);
    }

    const lost = arr(p.lost_information);
    if (lost.length > 0) {
      const sec = textElement("section", "semantic-card semantic-lost-card", "");
      sec.append(textElement("h4", "", FIELD_LABELS.lost_information));
      const ul = textElement("ul", "semantic-list", "");
      for (const entry of lost) {
        const li = document.createElement("li");
        const head = `${str(entry.short) ?? ""}${entry.direction ? ` · ${lostDirectionLabel(str(entry.direction))}` : ""}`;
        li.append(textElement("strong", "", head));
        if (str(entry.claim)) li.append(renderMathNode(md, str(entry.claim)!, "semantic-markdown"));
        if (str(entry.see_also)) li.append(textElement("small", "semantic-muted", `另见：${resolveTitle(str(entry.see_also))}`));
        ul.append(li);
      }
      sec.append(ul);
      mappings.append(sec);
    }

    const reqs = arr(p.required_assumptions);
    if (reqs.length > 0) {
      const sec = textElement("section", "semantic-card", "");
      sec.append(textElement("h4", "", "需要补充的前提与挂接账目"));
      const ul = textElement("ul", "semantic-list", "");
      for (const entry of reqs) {
        const li = document.createElement("li");
        li.append(renderMathNode(md, str(entry.text) ?? "—", "semantic-markdown"));
        if (str(entry.debt)) {
          const debtObj = lookupObjectInProposalOrBase(item, str(entry.debt));
          const status = debtObj ? debtStatusLabel(str(debtObj.payload.status)) : "关联账目";
          li.append(textElement("small", "semantic-tag", `账目：${resolveTitle(str(entry.debt))}（${status}）`));
        }
        ul.append(li);
      }
      sec.append(ul);
      mappings.append(sec);
    }

    if (mappings.childElementCount > 0) panel.append(mappings);
    return panel;
  }

  if (object.kind === "semantic_entity") {
    const card = textElement("section", "semantic-card", "");
    card.append(
      textElement("h4", "", `${semanticObjectNoun(object.kind)} · ${entityPlainLabel(p)}`),
      renderSemanticEntityCardBody(md, p, resolveTitle),
    );
    panel.append(card);
    return panel;
  }

  if (object.kind === "debt") {
    const card = textElement("section", "semantic-card", "");
    card.append(textElement("h4", "", `一笔账 · ${debtStatusLabel(str(p.status) ?? "open")}`));
    if (str(p.obligation)) card.append(kvRow("待补义务", renderMathNode(md, str(p.obligation)!, "semantic-value semantic-markdown")));
    if (str(p.quantifier)) card.append(kvRow("适用量词", renderMathNode(md, str(p.quantifier)!, "semantic-value semantic-markdown")));
    if (str(p.failure_mode)) card.append(kvRow("不还清会怎样", failureModeLabel(str(p.failure_mode))));
    if (str(p.failure_note)) card.append(kvRow("风险说明", renderMathNode(md, str(p.failure_note)!, "semantic-value semantic-markdown")));
    if (str(p.payoff_hint)) card.append(kvRow("偿还思路", renderMathNode(md, str(p.payoff_hint)!, "semantic-value semantic-markdown")));
    if (str(p.attached_to)) card.append(kvRow("挂在", resolveTitle(str(p.attached_to))));
    const paidBy = arr(p.paid_by);
    if (paidBy.length > 0) {
      card.append(kvRow("偿还依据", paidBy.map((r) => `${resolveTitle(str(r.ref))}${str(r.note) ? `（${str(r.note)}）` : ""}`).join("；")));
    }
    panel.append(card);
    return panel;
  }

  if (object.kind === "dialect_scope") {
    const card = textElement("section", "semantic-card semantic-dialect-card", "");
    card.append(
      textElement("h4", "", `约定卡 · ${str(p.label) ?? localRef}`),
      kvRow("适用范围", `${resolveTitle(str(p.anchor_unit))} 及其下属单元`),
    );
    for (const conv of arr(p.conventions)) {
      const row = document.createElement("div");
      row.className = "semantic-convention";
      row.append(renderMathNode(md, str(conv.text) ?? "—", "semantic-markdown"));
      const tags: string[] = [];
      if (conv.explicitness === "explicit") tags.push("书里明说");
      else if (conv.explicitness === "inherited") tags.push("沿用上一级");
      else if (conv.explicitness) tags.push("推断");
      if (tags.length) row.append(textElement("small", "semantic-tag", tags.join(" · ")));
      card.append(row);
    }
    const symbols = arr(p.local_symbols);
    if (symbols.length > 0) {
      card.append(renderLocalSymbolsCard(md, symbols));
    }
    panel.append(card);
    return panel;
  }

  if (object.kind === "representation_core") {
    const card = textElement("section", "semantic-card", "");
    card.append(
      textElement("h4", "", `${semanticObjectNoun(object.kind)} · ${entityPlainLabel(p)}`),
      renderRepresentationCoreCardBody(md, p, resolveTitle),
    );
    panel.append(card);
    return panel;
  }

  if (object.kind === "representation_binding") {
    const plain = representationBindingPlainLabel(p);
    const banner = textElement("div", "semantic-alignment-banner", "");
    banner.append(
      textElement("span", "semantic-relation-pill", t("semantic.relation.binds_representation")),
      textElement("strong", "", plain),
      textElement("span", "semantic-status-pill", alignmentStatusLabel(str(p.status))),
    );
    if (str(p.plain_note)) {
      banner.append(renderMathNode(md, str(p.plain_note)!, "semantic-plain-note semantic-markdown"));
    }
    panel.append(banner);

    const grid = textElement("div", "semantic-side-by-side", "");
    const leftCol = textElement("section", "semantic-pane semantic-pane-source", "");
    const scopeRef = str(p.dialect_scope);
    leftCol.append(
      textElement("div", "semantic-pane-eyebrow", t("semantic.s3.dialectScope")),
      textElement("h4", "semantic-pane-title", resolveTitle(scopeRef)),
    );
    const scopeObj = lookupObjectInProposalOrBase(item, scopeRef);
    if (scopeObj) {
      for (const conv of arr(scopeObj.payload.conventions)) {
        if (str(conv.text)) {
          leftCol.append(renderMathNode(md, `• ${str(conv.text)}`, "semantic-convention-inline semantic-markdown"));
        }
      }
      const symbols = arr(scopeObj.payload.local_symbols);
      if (symbols.length > 0) {
        leftCol.append(renderLocalSymbolsCard(md, symbols));
      }
    }

    const rightCol = textElement("section", "semantic-pane semantic-pane-target", "");
    const repRef = str(p.representation);
    rightCol.append(
      textElement("div", "semantic-pane-eyebrow", t("semantic.s3.representationCore")),
      textElement("h4", "semantic-pane-title", resolveTitle(repRef)),
    );
    const repObj = lookupObjectInProposalOrBase(item, repRef);
    if (repObj) {
      rightCol.append(renderRepresentationCoreCardBody(md, repObj.payload, resolveTitle));
    }
    grid.append(leftCol, rightCol);
    panel.append(grid);

    const mappings = textElement("div", "semantic-alignment-mappings", "");
    const symbolMap = arr(p.symbol_map);
    if (symbolMap.length > 0) {
      mappings.append(renderSymbolMapTable(md, symbolMap));
    }
    const adaptations = arr(p.index_and_domain_adaptations);
    if (adaptations.length > 0) {
      const sec = textElement("section", "semantic-card", "");
      sec.append(textElement("h4", "", t("semantic.s3.adaptationsHeading")));
      const ul = textElement("ul", "semantic-list", "");
      for (const ad of adaptations) {
        if (!str(ad.text)) continue;
        const li = document.createElement("li");
        li.append(renderMathNode(md, str(ad.text)!, "semantic-markdown"));
        ul.append(li);
      }
      sec.append(ul);
      mappings.append(sec);
    }
    const lost = arr(p.lost_information);
    if (lost.length > 0) {
      const sec = textElement("section", "semantic-card semantic-lost-card", "");
      sec.append(textElement("h4", "", FIELD_LABELS.lost_information));
      const ul = textElement("ul", "semantic-list", "");
      for (const entry of lost) {
        const li = document.createElement("li");
        const head = `${str(entry.short) ?? ""}${entry.direction ? ` · ${lostDirectionLabel(str(entry.direction))}` : ""}`;
        li.append(textElement("strong", "", head));
        if (str(entry.claim)) li.append(renderMathNode(md, str(entry.claim)!, "semantic-markdown"));
        ul.append(li);
      }
      sec.append(ul);
      mappings.append(sec);
    }
    if (mappings.childElementCount > 0) panel.append(mappings);
    return panel;
  }

  return panel;
}
