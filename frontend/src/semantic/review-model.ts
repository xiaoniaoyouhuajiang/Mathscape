/**
 * 审核页的语义层模型（契约 S9.2）：把提案里的共享对象、对齐线、一笔账、约定卡
 * 变成白话卡片，并挑出不阻塞的审核项。纯函数，便于单测；不显示 JSON。
 */
import { texToPlain, truncatePlain } from "../graph/tex-plain";
import type { PendingChangeSet, ProposalDiagnostic, ProposedObject } from "../review/proposal-model";
import {
  alignmentPlainLabel,
  alignmentStatusLabel,
  baselineStatusLabel,
  debtStatusLabel,
  entityPlainLabel,
  failureModeLabel,
  lostDirectionLabel,
  reviewItemLabel,
  SEMANTIC_PROPOSAL_KINDS,
  SEMANTIC_REVIEW_CODES,
  semanticKindLabel,
  semanticObjectNoun,
} from "./labels";
import {
  computeSemanticEditDiff,
  declarationKindLabel,
  effectiveProposalPayload,
  formatTermIndexEntry,
} from "./semantic-diff";

type Json = Record<string, unknown>;
const str = (value: unknown): string | null => (typeof value === "string" ? value : null);
const arr = (value: unknown): Json[] =>
  Array.isArray(value) ? value.filter((v): v is Json => v !== null && typeof v === "object") : [];

const cleanPlain = (value: string, max = 80): string =>
  truncatePlain(texToPlain(value).replace(/[{}]/g, ""), max);

export interface SemanticReviewCard {
  localRef: string;
  kind: string;
  op: string;
  noun: string;
  title: string;
  /** 白话说明行（纯文字 / Unicode 数学符号，不含花括号）。 */
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

function basePayloadOf(item: PendingChangeSet, object: ProposedObject): Json | null {
  if (!object.target_id) return null;
  return item.baseObjects?.[object.target_id]?.payload ?? null;
}

function bodyOf(item: PendingChangeSet, object: ProposedObject): Json {
  return effectiveProposalPayload(object, basePayloadOf(item, object));
}

/** 提案里的一个引用 → 显示名：`$new:x` 用本提案里的对象，已有 id 用后端给的标题。 */
export function refTitle(item: PendingChangeSet, ref: string | null): string {
  if (!ref) return "—";
  const local = ref.replace(/^\$new:/, "");
  const object = (item.proposal.proposed_objects ?? []).find((candidate) => candidate.local_ref === local);
  if (object) {
    const body = bodyOf(item, object);
    if (object.kind === "semantic_entity") return cleanPlain(entityPlainLabel(body), 60);
    const named = str(body.display_title) ?? str(body.label) ?? str(body.obligation);
    return named ? cleanPlain(named, 60) : local;
  }
  const resolved = item.targetTitles?.[ref];
  if (resolved) return cleanPlain(resolved, 60);
  return ref.length > 32 ? `${ref.slice(0, 24)}…` : ref;
}

function lineFor(item: PendingChangeSet, object: ProposedObject, body: Json): { title: string; lines: string[] } {
  const lines: string[] = [];
  if (object.op === "edit") {
    const diffs = computeSemanticEditDiff(object.kind, basePayloadOf(item, object), object.changes ?? null, (r) =>
      refTitle(item, r),
    );
    for (const diff of diffs) lines.push(diff.plainSummary);
  }
  switch (object.kind) {
    case "semantic_entity": {
      if (object.op === "create") {
        if (body.semantic_kind) lines.push(semanticKindLabel(str(body.semantic_kind)));
        if (body.home_theory) lines.push(`所在理论：${refTitle(item, str(body.home_theory))}`);
        const decl = body.declaration as Json | undefined;
        if (decl) {
          const stmt = (decl.statement ?? decl.definiens) as Json | undefined;
          if (stmt && str(stmt.text)) {
            lines.push(`${declarationKindLabel(str(decl.declaration_kind))}：${cleanPlain(str(stmt.text)!, 72)}`);
          }
        }
        const schema = body.problem_schema as Json | undefined;
        if (schema) {
          const task = schema.task as Json | undefined;
          if (task && str(task.text)) lines.push(`任务：${cleanPlain(str(task.text)!, 72)}`);
        }
        if (!decl && !schema && str(body.description)) {
          lines.push(`说明：${cleanPlain(str(body.description)!, 72)}`);
        }
        const aliases = Array.isArray(body.aliases) ? body.aliases.map(str).filter((v): v is string => !!v) : [];
        if (aliases.length) lines.push(`检索词：${aliases.map((a) => cleanPlain(a, 24)).join("、")}`);
        for (const convention of arr(body.conventions)) {
          if (str(convention.text)) lines.push(`约定：${cleanPlain(str(convention.text)!, 72)}`);
        }
        const baseline = body.mathlib_baseline as Json | null | undefined;
        if (baseline && str(baseline.decl)) {
          lines.push(`mathlib：${str(baseline.decl)}（${baselineStatusLabel(str(baseline.status))}）`);
        } else if (baseline) {
          lines.push("mathlib：暂无对应名称");
        }
      }
      const title =
        object.op === "create"
          ? cleanPlain(entityPlainLabel(body), 64)
          : refTitle(item, object.target_id ?? null);
      return { title, lines };
    }
    case "alignment": {
      const plain = cleanPlain(alignmentPlainLabel(body), 64);
      if (object.op === "create") {
        if (body.status) {
          lines.push(`状态：${alignmentStatusLabel(str(body.status))}（只是候选，你在审核页确认后才算审阅）`);
        }
        for (const entry of arr(body.parameter_map)) {
          if (str(entry.map)) lines.push(`参数对照：${cleanPlain(str(entry.map)!, 72)}`);
        }
        for (const entry of arr(body.term_index_map)) {
          lines.push(cleanPlain(formatTermIndexEntry(entry), 80));
        }
        for (const lost of arr(body.lost_information)) {
          if (str(lost.short)) {
            const dir = lost.direction ? `（${lostDirectionLabel(str(lost.direction))}）` : "";
            lines.push(`对不上：${cleanPlain(str(lost.short)!, 60)}${dir}`);
          }
        }
        for (const req of arr(body.required_assumptions)) {
          if (str(req.text)) lines.push(`需补前提：${cleanPlain(str(req.text)!, 64)}`);
        }
      }
      const title =
        object.op === "create"
          ? `${refTitle(item, str(body.source))} —${plain}→ ${refTitle(item, str(body.target))}`
          : refTitle(item, object.target_id ?? null);
      return { title, lines };
    }
    case "debt": {
      if (object.op === "create") {
        if (str(body.quantifier)) lines.push(`量词：${cleanPlain(str(body.quantifier)!, 48)}`);
        if (body.failure_mode) lines.push(`不还清会怎样：${failureModeLabel(str(body.failure_mode))}`);
        if (body.status) lines.push(debtStatusLabel(str(body.status)));
        if (body.attached_to) lines.push(`挂在：${refTitle(item, str(body.attached_to))}`);
      }
      const rawTitle = str(body.obligation) ?? refTitle(item, object.target_id ?? null);
      return { title: cleanPlain(rawTitle, 64), lines };
    }
    default: {
      if (object.op === "create") {
        for (const convention of arr(body.conventions)) {
          if (str(convention.text)) lines.push(cleanPlain(str(convention.text)!, 72));
        }
        if (body.anchor_unit) lines.push(`适用于：${refTitle(item, str(body.anchor_unit))} 及其下的内容`);
      }
      const rawTitle = str(body.label) ?? refTitle(item, object.target_id ?? null);
      return { title: cleanPlain(rawTitle, 64), lines };
    }
  }
}

export function semanticReviewCards(item: PendingChangeSet): SemanticReviewCard[] {
  const diagnostics = item.result.diagnostics ?? [];
  return (item.proposal.proposed_objects ?? [])
    .filter((object) => SEMANTIC_PROPOSAL_KINDS.has(object.kind))
    .map((object) => {
      const body = bodyOf(item, object);
      let title: string;
      let lines: string[];
      if (object.op === "retire") {
        title = refTitle(item, object.target_id ?? null);
        const cascades: string[] = object.target_id ? (item.retireCascades?.[object.target_id] ?? []) : [];
        lines = ["提议退役（仍需你接受）；挂在上面的账会自动失效"];
        if (cascades.length > 0) {
          lines.push(`将级联失效 ${cascades.length} 笔未清账目：${cascades.map((id: string) => refTitle(item, id)).join("、")}`);
        }
      } else {
        ({ title, lines } = lineFor(item, object, body));
      }
      const evidenceSource =
        object.op === "create"
          ? object.payload
          : { ...(basePayloadOf(item, object) ?? {}), ...(object.changes ?? {}) };
      return {
        localRef: object.local_ref,
        kind: object.kind,
        op: object.op,
        noun: semanticObjectNoun(object.kind),
        title,
        lines,
        evidenceRefs: collectSemanticSpanRefs(evidenceSource),
        diagnostics: diagnostics.filter((diagnostic) => diagnostic.local_ref === object.local_ref),
        stagedId: item.result.staged_ids?.[object.local_ref] ?? object.target_id ?? null,
      };
    });
}

/** 不针对某个对象的语义审核项（例如一个提案涉及多个目标）。 */
export function proposalLevelReviewItems(item: PendingChangeSet): ProposalDiagnostic[] {
  return (item.result.diagnostics ?? []).filter(
    (diagnostic) => !diagnostic.local_ref && SEMANTIC_REVIEW_CODES.has(diagnostic.code),
  );
}

export function reviewItemText(diagnostic: ProposalDiagnostic): string {
  return reviewItemLabel(diagnostic.code) || diagnostic.message;
}

export function reviewItemCode(diagnostic: ProposalDiagnostic): string {
  return `${diagnostic.level ?? "error"}:${diagnostic.code}`;
}
