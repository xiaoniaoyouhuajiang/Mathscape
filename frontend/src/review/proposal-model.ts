/**
 * Pure model of the 「提案」 review source (Phase2 staged change sets).
 *
 * Everything here is derived from what the backend returns; the frontend
 * never builds an investigation id, never assembles map nodes and never
 * decides acceptability — it only groups, selects and explains.
 */
import type { SourceReviewAnchor, SourceReviewProjection, SourceReviewUnit } from "./types";

export interface ChangeSetSelection {
  change_set_id: string;
  version: number;
  proposal_fingerprint: string;
}

export interface ChangeSetRef {
  changeSetId: string;
  version: number;
  referencedIds?: string[];
}

export interface ProposalDebt {
  kind?: string;
  code?: string;
  message?: string;
  local_ref?: string | null;
  value?: unknown;
}

export interface ProposalDiagnostic {
  level?: string;
  code: string;
  message: string;
  local_ref?: string | null;
}

export interface ProposedObject {
  local_ref: string;
  kind: string;
  op: string;
  payload?: Record<string, unknown> | null;
  target_id?: string | null;
  changes?: Record<string, unknown> | null;
}

export interface ProposedRelation {
  local_ref: string;
  kind: string;
  op: string;
  relation?: string | null;
  source?: string | null;
  target?: string | null;
  evidence_spans?: string[];
  target_id?: string | null;
}

export interface PendingChangeSet {
  changeSetId: string;
  version: number;
  state: "staged" | "blocked" | string;
  investigationId: string;
  roundId: string;
  proposedBy: string;
  createdAt: string;
  proposalFingerprint: string;
  baseCommit: string | null;
  proposal: {
    rationale?: string;
    proposed_objects?: ProposedObject[];
    proposed_relations?: ProposedRelation[];
  };
  result: {
    debts?: ProposalDebt[];
    diagnostics?: ProposalDiagnostic[];
    staged_ids?: Record<string, string> | null;
  };
  dependencies: ChangeSetRef[];
}

export interface ChangeSetQueue {
  investigationId: string;
  baseCommit: string | null;
  items: PendingChangeSet[];
}

export interface ImpactItem {
  object_id: string;
  kind: string;
  action: string;
  display_title?: string | null;
}

export interface ImpactPlan {
  plan_fingerprint: string;
  status: "ready" | "blocked" | string;
  base_commit: string | null;
  direct_changes: ImpactItem[];
  affected_objects: ImpactItem[];
  diagnostics: { code: string; message: string; object_id?: string | null }[];
}

export interface BatchPlan {
  investigationId: string;
  baseCommit: string | null;
  selections: ChangeSetSelection[];
  impact: ImpactPlan;
  perChangeSet: { changeSetId: string; version: number; directChanges: ImpactItem[] }[];
  batchPlanFingerprint: string;
}

export interface ImpactPreview {
  changeSetId: string;
  version: number;
  plannedWith: ChangeSetRef[];
  impact: ImpactPlan;
}

export const keyOf = (item: { changeSetId: string; version: number }): string => `${item.changeSetId}@${item.version}`;

export function selectionOf(item: PendingChangeSet): ChangeSetSelection {
  return { change_set_id: item.changeSetId, version: item.version, proposal_fingerprint: item.proposalFingerprint };
}

export function titleOf(item: PendingChangeSet): string {
  return item.proposal.rationale?.trim() || item.changeSetId.replace(/^mathscape:change-set\//, "").slice(0, 12);
}

/** Pending dependency keys of `item` (only those also pending in `items`). */
function pendingDependencies(items: PendingChangeSet[], item: PendingChangeSet): string[] {
  const known = new Set(items.map(keyOf));
  return item.dependencies.map(keyOf).filter(key => known.has(key));
}

/** key → keys of pending versions that directly depend on it. */
export function dependentsIndex(items: PendingChangeSet[]): Map<string, string[]> {
  const index = new Map<string, string[]>();
  for (const item of items) {
    for (const dependency of pendingDependencies(items, item)) {
      index.set(dependency, [...(index.get(dependency) ?? []), keyOf(item)]);
    }
  }
  return index;
}

function closure(start: string[], next: (key: string) => string[]): Set<string> {
  const seen = new Set<string>();
  const stack = [...start];
  while (stack.length) {
    const key = stack.pop()!;
    if (seen.has(key)) continue;
    seen.add(key);
    stack.push(...next(key));
  }
  return seen;
}

/** Transitive pending dependents of `keys` (excluding `keys`). */
export function transitiveDependents(items: PendingChangeSet[], keys: string[]): Set<string> {
  const index = dependentsIndex(items);
  const result = closure(keys.flatMap(key => index.get(key) ?? []), key => index.get(key) ?? []);
  for (const key of keys) result.delete(key);
  return result;
}

/** Transitive pending dependencies of `keys` (excluding `keys`). */
export function transitiveDependencies(items: PendingChangeSet[], keys: string[]): Set<string> {
  const byKey = new Map(items.map(item => [keyOf(item), item]));
  const deps = (key: string) => {
    const item = byKey.get(key);
    return item ? pendingDependencies(items, item) : [];
  };
  const result = closure(keys.flatMap(deps), deps);
  for (const key of keys) result.delete(key);
  return result;
}

/** The batch dialog starts with every staged version selected. */
export function defaultSelection(items: PendingChangeSet[]): Set<string> {
  return new Set(items.filter(item => item.state === "staged").map(keyOf));
}

/** Unchecking cascades to every transitive dependent. */
export function deselect(items: PendingChangeSet[], selected: Set<string>, key: string): Set<string> {
  const next = new Set(selected);
  next.delete(key);
  for (const dependent of transitiveDependents(items, [key])) next.delete(dependent);
  return next;
}

/**
 * Checking a version also checks its pending dependencies. Returns `null`
 * when a dependency is not staged (it cannot be accepted now).
 */
export function select(items: PendingChangeSet[], selected: Set<string>, key: string): Set<string> | null {
  const byKey = new Map(items.map(item => [keyOf(item), item]));
  const required = [key, ...transitiveDependencies(items, [key])];
  if (required.some(candidate => byKey.get(candidate)?.state !== "staged")) return null;
  return new Set([...selected, ...required]);
}

/** Connected components of the pending dependency graph, dependencies first. */
export function dependencyGroups(items: PendingChangeSet[]): PendingChangeSet[][] {
  const byKey = new Map(items.map(item => [keyOf(item), item]));
  const neighbours = new Map<string, Set<string>>();
  for (const item of items) {
    const key = keyOf(item);
    if (!neighbours.has(key)) neighbours.set(key, new Set());
    for (const dependency of pendingDependencies(items, item)) {
      neighbours.get(key)!.add(dependency);
      if (!neighbours.has(dependency)) neighbours.set(dependency, new Set());
      neighbours.get(dependency)!.add(key);
    }
  }
  const order = [...items].sort((a, b) => a.createdAt.localeCompare(b.createdAt) || keyOf(a).localeCompare(keyOf(b)));
  const assigned = new Set<string>();
  const groups: PendingChangeSet[][] = [];
  for (const item of order) {
    const key = keyOf(item);
    if (assigned.has(key)) continue;
    const members = closure([key], member => [...(neighbours.get(member) ?? [])]);
    members.forEach(member => assigned.add(member));
    groups.push(topological(order.filter(candidate => members.has(keyOf(candidate))), byKey, items));
  }
  return groups;
}

function topological(members: PendingChangeSet[], byKey: Map<string, PendingChangeSet>, items: PendingChangeSet[]): PendingChangeSet[] {
  const result: PendingChangeSet[] = [];
  const placed = new Set<string>();
  const visit = (item: PendingChangeSet, path: Set<string>) => {
    const key = keyOf(item);
    if (placed.has(key) || path.has(key)) return;
    path.add(key);
    for (const dependency of pendingDependencies(items, item)) {
      const target = byKey.get(dependency);
      if (target) visit(target, path);
    }
    placed.add(key);
    result.push(item);
  };
  members.forEach(item => visit(item, new Set()));
  return result;
}

/** What a discard of `roots` would do, mirroring the backend rule. */
export function discardImpact(items: PendingChangeSet[], roots: string[], cascade: boolean): { discarded: Set<string>; blocked: Set<string> } {
  const dependents = transitiveDependents(items, roots);
  return cascade
    ? { discarded: new Set([...roots, ...dependents]), blocked: new Set() }
    : { discarded: new Set(roots), blocked: dependents };
}

export function selectionRefs(items: PendingChangeSet[], selected: Set<string>): ChangeSetSelection[] {
  return items.filter(item => selected.has(keyOf(item))).map(selectionOf);
}

export function proposalDebts(item: PendingChangeSet): ProposalDebt[] {
  return item.result.debts ?? [];
}

export function debtLabel(debt: ProposalDebt): string {
  if (debt.kind === "agent_declared") return `Agent 声明：${typeof debt.value === "string" ? debt.value : JSON.stringify(debt.value)}`;
  if (debt.kind === "unresolved") return `未解决：${typeof debt.value === "string" ? debt.value : JSON.stringify(debt.value)}`;
  return [debt.code, debt.message].filter(Boolean).join("：");
}

const DIAGNOSTIC_LABELS: Record<string, string> = {
  quote_mismatch: "引文与字节范围不符",
  page_mapping_unavailable: "无法唯一映射到 PDF 页",
  block_locator_unavailable: "范围内没有 canonical 块",
  span_inactive: "同 id 证据已退役",
  dangling_staged_ref: "引用的对象不存在",
  dependency_blocked: "依赖的提案已阻塞",
  dependency_superseded: "依赖的提案已被替换",
  dependency_discarded: "依赖的提案已丢弃",
  dependency_not_accepted: "依赖的提案尚未接受",
  change_set_dependency_cycle: "提案依赖成环",
  span_already_exists: "证据已存在，将直接引用",
};

export function diagnosticLabel(code: string | undefined): string {
  return (code && DIAGNOSTIC_LABELS[code]) || code || "";
}

/** One snapshot's slice of a proposal in the SourceReviewProjection shape. */
export type ProposalReviewProjection = Pick<SourceReviewProjection, "units" | "structure_relations" | "relations" | "diagnostics"> & {
  version: "mathscape-proposal-review-projection-1";
  change_set_id: string;
  change_set_version: number;
  /** Markdown snapshot the anchors point into; `null` for proposals that only reference existing evidence. */
  source: { artifact_id: string | null };
};

function spanAnchor(object: ProposedObject): { artifact: string; anchor: SourceReviewAnchor } | null {
  const payload = object.payload ?? {};
  const range = payload.md_byte_range as { start?: number; end?: number } | undefined;
  const artifact = payload.artifact_id;
  if (typeof artifact !== "string" || typeof range?.start !== "number" || typeof range?.end !== "number") return null;
  return {
    artifact,
    anchor: {
      purpose: "unit",
      range: { start: range.start, end: range.end },
      block_ids: [],
      excerpt: typeof payload.quote === "string" ? payload.quote : "",
    },
  };
}

const newRef = (value: unknown): string | null => typeof value === "string" ? value.replace(/^\$new:/, "") : null;

/**
 * Projects a proposal into the review projection shape, one entry per
 * Markdown snapshot (grouped by the snapshot its new spans point into).
 */
export function proposalReviewProjection(item: PendingChangeSet): ProposalReviewProjection[] {
  const objects = item.proposal.proposed_objects ?? [];
  const relations = item.proposal.proposed_relations ?? [];
  const spans = new Map<string, { artifact: string; anchor: SourceReviewAnchor }>();
  for (const object of objects) {
    if (object.kind !== "source_span") continue;
    const anchor = spanAnchor(object);
    if (anchor) spans.set(object.local_ref, anchor);
  }
  const groups = new Map<string | null, ProposalReviewProjection>();
  const groupFor = (artifact: string | null) => {
    let group = groups.get(artifact);
    if (!group) {
      group = {
        version: "mathscape-proposal-review-projection-1",
        change_set_id: item.changeSetId,
        change_set_version: item.version,
        source: { artifact_id: artifact },
        units: [], structure_relations: [], relations: [], diagnostics: [],
      };
      groups.set(artifact, group);
    }
    return group;
  };
  const unitGroup = new Map<string, string | null>();
  for (const object of objects) {
    if (object.kind !== "source_unit") continue;
    const fields = (object.op === "edit" ? object.changes : object.payload) ?? {};
    const spanRefs = Array.isArray(fields.source_spans) ? fields.source_spans : [];
    const anchors = spanRefs.map(newRef).flatMap(ref => (ref && spans.get(ref) ? [spans.get(ref)!] : []));
    const artifact = anchors[0]?.artifact ?? null;
    const title = object.op === "edit"
      ? ((fields.display_title as { value?: string } | undefined)?.value ?? object.target_id ?? object.local_ref)
      : (typeof fields.display_title === "string" ? fields.display_title : object.local_ref);
    const parentField = object.op === "edit" ? (fields.parent as { value?: string } | undefined)?.value : fields.parent;
    const unit: SourceReviewUnit = {
      unit_ref: object.local_ref,
      unit_kind: typeof fields.kind === "string" ? fields.kind : "statement",
      display_title: title,
      source_title: null,
      source_labels: [],
      parent: newRef(parentField),
      completeness: typeof fields.completeness === "string" ? fields.completeness : "complete",
      fact_status: "processed",
      acceptance: object.op === "edit" ? "edit" : "create",
      anchors: anchors.map(entry => entry.anchor),
      source_excerpt: anchors.map(entry => entry.anchor.excerpt).join("\n"),
    };
    unitGroup.set(object.local_ref, artifact);
    const group = groupFor(artifact);
    group.units.push(unit);
    if (unit.parent) group.structure_relations.push({ relation_ref: `${object.local_ref}:parent`, parent: unit.parent, child: unit.unit_ref });
  }
  for (const relation of relations) {
    const source = newRef(relation.source) ?? relation.target_id ?? "";
    const target = newRef(relation.target) ?? "";
    const evidence = (relation.evidence_spans ?? []).map(newRef).flatMap(ref => (ref && spans.get(ref) ? [spans.get(ref)!] : []));
    const artifact = unitGroup.get(source) ?? evidence[0]?.artifact ?? null;
    groupFor(artifact).relations.push({
      relation_ref: relation.local_ref,
      source, target,
      relation: relation.relation ?? relation.op,
      evidence: evidence.map(entry => ({ ...entry.anchor, purpose: "relation_evidence" })),
      bases: [], origins: [],
    });
  }
  for (const diagnostic of item.result.diagnostics ?? []) {
    const artifact = diagnostic.local_ref ? (unitGroup.get(diagnostic.local_ref) ?? spans.get(diagnostic.local_ref)?.artifact ?? null) : null;
    groupFor(artifact).diagnostics.push({
      diagnostic_ref: `${diagnostic.code}:${diagnostic.local_ref ?? ""}`,
      severity: diagnostic.level ?? "error", stage: "proposal", code: diagnostic.code, path: diagnostic.local_ref ?? "",
      message: diagnostic.message, checked: true, related_units: diagnostic.local_ref ? [diagnostic.local_ref] : [],
    });
  }
  if (groups.size === 0) groupFor(null);
  return [...groups.values()];
}

/** One-line impact summary for the impact column. */
export function impactSummary(plan: ImpactPlan | null | undefined): string {
  if (!plan) return "—";
  if (plan.status !== "ready") return `阻塞：${plan.diagnostics.map(item => diagnosticLabel(item.code) || item.message).join("；") || "无法接受"}`;
  const created = plan.direct_changes.filter(item => item.action === "create").length;
  const edited = plan.direct_changes.length - created;
  const parts = [created ? `新建 ${created}` : "", edited ? `修改 ${edited}` : "", plan.affected_objects.length ? `波及 ${plan.affected_objects.length}` : ""].filter(Boolean);
  return parts.join(" · ") || "无变化";
}
