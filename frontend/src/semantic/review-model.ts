/**
 * 审核页的语义层模型（契约 S9.2）：把提案里的共享对象、对齐线、一笔账、约定卡
 * 变成白话卡片，并挑出不阻塞的审核项。纯函数，便于单测；不显示 JSON。
 */
import type { PendingChangeSet, ProposalDiagnostic, ProposedObject } from "../review/proposal-model";
import {
  alignmentPlainLabel, alignmentStatusLabel, baselineStatusLabel, debtStatusLabel, entityPlainLabel,
  failureModeLabel, reviewItemLabel, SEMANTIC_PROPOSAL_KINDS, SEMANTIC_REVIEW_CODES, semanticKindLabel,
  semanticObjectNoun,
} from "./labels";

type Json = Record<string, unknown>;
const str = (value: unknown): string | null => typeof value === "string" ? value : null;
const arr = (value: unknown): Json[] => Array.isArray(value) ? value.filter((v): v is Json => v !== null && typeof v === "object") : [];

export interface SemanticReviewCard {
  localRef: string;
  kind: string;
  op: string;
  noun: string;
  title: string;
  /** 白话说明行。 */
  lines: string[];
  /** 这张卡引用的全部原文片段（去掉 `$new:` 后的 `span_ref` 或稳定 id）。 */
  evidenceRefs: string[];
  /** 这张卡上的审核项（warn / info）与阻塞诊断。 */
  diagnostics: ProposalDiagnostic[];
  /** 已暂存的稳定 id（有的话）。 */
  stagedId: string | null;
}

export function collectSemanticSpanRefs(value: unknown, out: string[] = []): string[] {
  if (!value || typeof value !== "object") return out;
  if (Array.isArray(value)) {
    for (const item of value) collectSemanticSpanRefs(item, out);
    return out;
  }
  const map = value as Json;
  for (const [key, child] of Object.entries(map)) {
    if (key === "evidence_spans" && Array.isArray(child)) {
      for (const raw of child) {
        if (typeof raw !== "string") continue;
        const ref = raw.replace(/^\$new:/, "");
        if (ref && !out.includes(ref)) out.push(ref);
      }
    } else {
      collectSemanticSpanRefs(child, out);
    }
  }
  return out;
}

function bodyOf(object: ProposedObject): Json {
  return (object.op === "create" ? object.payload : object.changes) ?? {};
}

/** 提案里的一个引用 → 显示名：`$new:x` 用本提案里的对象，已有 id 用后端给的标题。 */
export function refTitle(item: PendingChangeSet, ref: string | null): string {
  if (!ref) return "—";
  const local = ref.replace(/^\$new:/, "");
  const object = (item.proposal.proposed_objects ?? []).find(candidate => candidate.local_ref === local);
  if (object) {
    const body = bodyOf(object);
    if (object.kind === "semantic_entity") return entityPlainLabel(body);
    return str(body.display_title) ?? str(body.label) ?? str(body.obligation) ?? local;
  }
  return item.targetTitles?.[ref] ?? (ref.length > 32 ? `${ref.slice(0, 24)}…` : ref);
}

function lineFor(item: PendingChangeSet, object: ProposedObject, body: Json): { title: string; lines: string[] } {
  const lines: string[] = [];
  switch (object.kind) {
    case "semantic_entity": {
      if (body.semantic_kind) lines.push(semanticKindLabel(str(body.semantic_kind)));
      if (body.home_theory) lines.push(`所在理论：${refTitle(item, str(body.home_theory))}`);
      const aliases = Array.isArray(body.aliases) ? body.aliases.map(str).filter((v): v is string => !!v) : [];
      if (aliases.length) lines.push(`检索词：${aliases.join("、")}`);
      for (const convention of arr(body.conventions)) if (str(convention.text)) lines.push(`约定：${str(convention.text)}`);
      const baseline = body.mathlib_baseline as Json | null | undefined;
      if (baseline && str(baseline.decl)) lines.push(`mathlib：${str(baseline.decl)}（${baselineStatusLabel(str(baseline.status))}）`);
      else if (baseline) lines.push("mathlib：暂无对应名称");
      return { title: object.op === "create" ? entityPlainLabel(body) : refTitle(item, object.target_id ?? null), lines };
    }
    case "alignment": {
      const plain = alignmentPlainLabel(body);
      if (body.status) lines.push(`状态：${alignmentStatusLabel(str(body.status))}（只是候选，你在审核页确认后才算审阅）`);
      for (const entry of arr(body.parameter_map)) if (str(entry.map)) lines.push(`参数对照：${str(entry.map)}`);
      for (const entry of arr(body.term_index_map)) {
        if (entry.index_shift === 0) lines.push(`数列逐项对齐：${str(entry.source)} = ${str(entry.target)}`);
      }
      for (const lost of arr(body.lost_information)) if (str(lost.short)) lines.push(`对不上：${str(lost.short)}`);
      const title = object.op === "create"
        ? `${refTitle(item, str(body.source))} —${plain}→ ${refTitle(item, str(body.target))}`
        : refTitle(item, object.target_id ?? null);
      return { title, lines };
    }
    case "debt": {
      if (str(body.quantifier)) lines.push(`量词：${str(body.quantifier)}`);
      if (body.failure_mode) lines.push(`不还清会怎样：${failureModeLabel(str(body.failure_mode))}`);
      if (body.status) lines.push(debtStatusLabel(str(body.status)));
      if (body.attached_to) lines.push(`挂在：${refTitle(item, str(body.attached_to))}`);
      return { title: str(body.obligation) ?? refTitle(item, object.target_id ?? null), lines };
    }
    default: {
      for (const convention of arr(body.conventions)) if (str(convention.text)) lines.push(str(convention.text)!);
      if (body.anchor_unit) lines.push(`适用于：${refTitle(item, str(body.anchor_unit))} 及其下的内容`);
      return { title: str(body.label) ?? refTitle(item, object.target_id ?? null), lines };
    }
  }
}

export function semanticReviewCards(item: PendingChangeSet): SemanticReviewCard[] {
  const diagnostics = item.result.diagnostics ?? [];
  return (item.proposal.proposed_objects ?? [])
    .filter(object => SEMANTIC_PROPOSAL_KINDS.has(object.kind))
    .map(object => {
      const body = bodyOf(object);
      const { title, lines } = object.op === "retire"
        ? { title: refTitle(item, object.target_id ?? null), lines: ["提议退役（仍需你接受）；挂在上面的账会自动失效"] }
        : lineFor(item, object, body);
      return {
        localRef: object.local_ref,
        kind: object.kind,
        op: object.op,
        noun: semanticObjectNoun(object.kind),
        title,
        lines,
        evidenceRefs: collectSemanticSpanRefs(body),
        diagnostics: diagnostics.filter(diagnostic => diagnostic.local_ref === object.local_ref),
        stagedId: item.result.staged_ids?.[object.local_ref] ?? null,
      };
    });
}

/** 不针对某个对象的语义审核项（例如一个提案涉及多个目标）。 */
export function proposalLevelReviewItems(item: PendingChangeSet): ProposalDiagnostic[] {
  return (item.result.diagnostics ?? []).filter(diagnostic => !diagnostic.local_ref && SEMANTIC_REVIEW_CODES.has(diagnostic.code));
}

export function reviewItemText(diagnostic: ProposalDiagnostic): string {
  return reviewItemLabel(diagnostic.code) || diagnostic.message;
}

export function reviewItemCode(diagnostic: ProposalDiagnostic): string {
  return `${diagnostic.level ?? "error"}:${diagnostic.code}`;
}
