/**
 * Pure model of the 「提案」 review source (Phase2 staged change sets).
 *
 * Everything here is derived from what the backend returns; the frontend
 * never builds an investigation id, never assembles map nodes and never
 * decides acceptability — it only groups, selects and explains.
 */
import type { AnchoredMarkdown } from "../reader/selection";
import type { DiscoveryOutcome, SourceReviewAnchor, SourceReviewProjection, SourceReviewUnit } from "./types";

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
  /** Existing (head or staged) object the diagnostic is about. */
  target_id?: string | null;
}

/** One Markdown anchor of a proposal, resolved by the backend. */
export interface ProposalAnchor {
  /** Local ref (without `$new:`) of a span this version creates, or the stable id it references. */
  span_ref: string;
  span_id: string | null;
  /** `new`: created by this version · `head`: already in the map · `staged`: created by another staged version. */
  origin: "new" | "head" | "staged" | string;
  artifact: string;
  conversion: string | null;
  range: { start: number; end: number };
  block_ids: string[];
  page: { pdf_page_index: number; page_number: number } | null;
  granularity: string | null;
  excerpt: string;
  /** `excerpt` was read from the snapshot at `range` on the backend. */
  verified: boolean;
  /** Block ids and page come from the compiled operations. */
  compiled: boolean;
}

export interface HistoryVersion {
  version: number;
  state: string;
  createdAt: string;
  diagnostics: ProposalDiagnostic[];
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
  /** Server-resolved Markdown anchors (never rebuilt from the raw payload). */
  anchors?: ProposalAnchor[];
  /** Older versions of the same change set, newest first. */
  history?: HistoryVersion[];
  /** Titles of existing (head or staged) objects the proposal names: diagnostic targets, relation endpoints, parents, edit targets. */
  targetTitles?: Record<string, string>;
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
  /** Pre-existing problems of untouched head objects; informational, never blocking. */
  head_issues?: { code: string; message: string; object_id?: string | null }[];
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

/** Same tie-break as the backend batch order: `(change_set_id, version)`, never `createdAt`. */
export function compareKeys(a: { changeSetId: string; version: number }, b: { changeSetId: string; version: number }): number {
  return a.changeSetId < b.changeSetId ? -1 : a.changeSetId > b.changeSetId ? 1 : a.version - b.version;
}

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
  const order = [...items].sort(compareKeys);
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

/** Who a discard of `roots` touches, for the confirmation preview. */
export function discardPreview(items: PendingChangeSet[], roots: string[]): { roots: PendingChangeSet[]; dependents: PendingChangeSet[] } {
  const dependents = transitiveDependents(items, roots);
  const rootSet = new Set(roots);
  const sorted = [...items].sort(compareKeys);
  return {
    roots: sorted.filter(item => rootSet.has(keyOf(item))),
    dependents: sorted.filter(item => dependents.has(keyOf(item))),
  };
}

export function selectionRefs(items: PendingChangeSet[], selected: Set<string>): ChangeSetSelection[] {
  return items.filter(item => selected.has(keyOf(item))).map(selectionOf);
}

export function proposalDebts(item: PendingChangeSet): ProposalDebt[] {
  return item.result.debts ?? [];
}

/** Structured display of one Debt entry: body text, a kind tag and an optional id. */
export interface DebtView {
  origin: string;
  tag: string | null;
  body: string;
  id: string | null;
}

const DEBT_TEXT_KEYS = ["summary", "message", "description", "text", "reason", "note"];

function scalarText(value: unknown): string | null {
  if (typeof value === "string") return value.trim() || null;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return null;
}

/** Plain text of an arbitrary Agent value, never raw JSON. */
function plainText(value: unknown): string {
  const scalar = scalarText(value);
  if (scalar !== null) return scalar;
  if (Array.isArray(value)) return value.map(plainText).filter(Boolean).join("；");
  if (value && typeof value === "object") {
    return Object.entries(value as Record<string, unknown>)
      .map(([key, entry]) => { const text = plainText(entry); return text ? `${key}：${text}` : ""; })
      .filter(Boolean).join("；");
  }
  return "";
}

export function debtView(debt: ProposalDebt & { debt_id?: string }): DebtView {
  const origin = debt.kind === "agent_declared" ? "Agent 声明" : debt.kind === "unresolved" ? "未解决" : "校验";
  if (debt.kind === "agent_declared" || debt.kind === "unresolved") {
    const value = debt.value;
    if (value && typeof value === "object" && !Array.isArray(value)) {
      const record = value as Record<string, unknown>;
      const bodyKey = DEBT_TEXT_KEYS.find(key => scalarText(record[key]) !== null);
      const rest = Object.fromEntries(Object.entries(record).filter(([key]) => key !== bodyKey && key !== "kind" && key !== "id"));
      const body = [bodyKey ? scalarText(record[bodyKey])! : "", plainText(rest)].filter(Boolean).join("；");
      return { origin, tag: scalarText(record.kind), body: body || "（无说明）", id: scalarText(record.id) };
    }
    return { origin, tag: null, body: plainText(value) || "（无说明）", id: null };
  }
  return { origin, tag: debt.code ? diagnosticLabel(debt.code) : null, body: debt.message ?? "", id: debt.debt_id ?? null };
}

export function debtLabel(debt: ProposalDebt): string {
  const view = debtView(debt);
  return [view.origin, view.tag, view.body].filter(Boolean).join(" · ");
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
  span_too_long: "证据范围超过 4KB",
  head_integrity: "工作区已有对象存在问题（不阻塞）",
  invalid_locator: "原文证据缺少物理定位",
  page_markers_absent: "快照没有分页标记",
};

export function diagnosticLabel(code: string | undefined): string {
  return (code && DIAGNOSTIC_LABELS[code]) || code || "";
}

/** One snapshot's slice of a proposal in the SourceReviewProjection shape. */
export type ProposalReviewProjection = Pick<SourceReviewProjection, "units" | "structure_relations" | "relations" | "diagnostics"> & {
  version: "mathscape-proposal-review-projection-1";
  change_set_id: string;
  change_set_version: number;
  /** Markdown snapshot the anchors point into (and the conversion the backend reads it through); `null` when nothing anchors into Markdown. */
  source: { artifact_id: string | null; conversion_id: string | null };
  /** Server anchors of this group, by `span_ref`. */
  anchors: ProposalAnchor[];
};

const newRef = (value: unknown): string | null => typeof value === "string" ? value.replace(/^\$new:/, "") : null;

/** Span refs of a unit: `source_spans` of a create payload, or the set value of an edit. */
function spanRefs(fields: Record<string, unknown>): string[] {
  const raw = fields.source_spans;
  const list = Array.isArray(raw) ? raw : (raw && typeof raw === "object" && Array.isArray((raw as { value?: unknown }).value) ? (raw as { value: unknown[] }).value : []);
  return list.map(newRef).filter((ref): ref is string => ref !== null);
}

function reviewAnchorOf(anchor: ProposalAnchor, purpose: string): SourceReviewAnchor {
  return { purpose, range: { start: anchor.range.start, end: anchor.range.end }, block_ids: anchor.block_ids, excerpt: anchor.excerpt };
}

/**
 * Projects a proposal into the review projection shape, one entry per
 * Markdown snapshot. Anchors (snapshot, conversion, range, block ids, page,
 * excerpt) come from the backend (`PendingChangeSet.anchors`); the payload
 * only contributes titles, kinds and structure.
 */
export function proposalReviewProjection(item: PendingChangeSet): ProposalReviewProjection[] {
  const objects = item.proposal.proposed_objects ?? [];
  const relations = item.proposal.proposed_relations ?? [];
  const anchors = new Map<string, ProposalAnchor>();
  for (const anchor of item.anchors ?? []) {
    anchors.set(anchor.span_ref, anchor);
    if (anchor.span_id && !anchors.has(anchor.span_id)) anchors.set(anchor.span_id, anchor);
  }
  const groups = new Map<string | null, ProposalReviewProjection>();
  const groupFor = (anchor: ProposalAnchor | null) => {
    const artifact = anchor?.artifact ?? null;
    let group = groups.get(artifact);
    if (!group) {
      group = {
        version: "mathscape-proposal-review-projection-1",
        change_set_id: item.changeSetId,
        change_set_version: item.version,
        source: { artifact_id: artifact, conversion_id: anchor?.conversion ?? null },
        units: [], structure_relations: [], relations: [], diagnostics: [], anchors: [],
      };
      groups.set(artifact, group);
    }
    if (anchor && !group.anchors.includes(anchor)) group.anchors.push(anchor);
    return group;
  };
  const unitGroup = new Map<string, string | null>();
  for (const object of objects) {
    if (object.kind !== "source_unit") continue;
    const fields = (object.op === "edit" ? object.changes : object.payload) ?? {};
    const unitAnchors = spanRefs(fields).flatMap(ref => anchors.get(ref) ?? []);
    const first = unitAnchors[0] ?? null;
    const title = object.op === "edit"
      ? ((fields.display_title as { value?: string } | undefined)?.value ?? object.target_id ?? object.local_ref)
      : (typeof fields.display_title === "string" ? fields.display_title : object.local_ref);
    const parentField = object.op === "edit" ? (fields.parent as { value?: string } | undefined)?.value : fields.parent;
    // Anchors in other snapshots cannot be shown against this group's document.
    const local = unitAnchors.filter(anchor => anchor.artifact === first?.artifact);
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
      anchors: local.map(anchor => reviewAnchorOf(anchor, "unit")),
      source_excerpt: local.map(anchor => anchor.excerpt).join("\n"),
    };
    unitGroup.set(object.local_ref, first?.artifact ?? null);
    const group = groupFor(first);
    local.forEach(anchor => groupFor(anchor));
    group.units.push(unit);
    if (unit.parent) group.structure_relations.push({ relation_ref: `${object.local_ref}:parent`, parent: unit.parent, child: unit.unit_ref });
  }
  const sampleFor = (artifact: string | null) => artifact === null ? null : (item.anchors ?? []).find(anchor => anchor.artifact === artifact) ?? null;
  for (const relation of relations) {
    const source = newRef(relation.source) ?? relation.target_id ?? "";
    const target = newRef(relation.target) ?? "";
    const evidence = (relation.evidence_spans ?? []).map(newRef).flatMap(ref => (ref && anchors.get(ref)) || []);
    // Home: the source unit's snapshot, else the target unit's, else the evidence's.
    const artifact = unitGroup.has(source) ? unitGroup.get(source)! : unitGroup.has(target) ? unitGroup.get(target)! : evidence[0]?.artifact ?? null;
    const homes = [groupFor(sampleFor(artifact))];
    // A cross-snapshot relation is also listed in the target unit's snapshot.
    if (unitGroup.has(source) && unitGroup.has(target) && unitGroup.get(target) !== artifact) homes.push(groupFor(sampleFor(unitGroup.get(target)!)));
    for (const group of homes) {
      group.relations.push({
        relation_ref: relation.local_ref,
        source, target,
        relation: relation.relation ?? relation.op,
        evidence: evidence.filter(anchor => anchor.artifact === group.source.artifact_id).map(anchor => reviewAnchorOf(anchor, "relation_evidence")),
        bases: [], origins: [],
      });
    }
  }
  // Anchors no unit or relation claimed still belong to their snapshot.
  for (const anchor of item.anchors ?? []) groupFor(anchor);
  for (const diagnostic of item.result.diagnostics ?? []) {
    if (!diagnostic.local_ref) continue;
    const artifact = unitGroup.get(diagnostic.local_ref) ?? anchors.get(diagnostic.local_ref)?.artifact ?? null;
    const group = groups.get(artifact) ?? groupFor(null);
    group.diagnostics.push({
      diagnostic_ref: `${diagnostic.code}:${diagnostic.local_ref}`,
      severity: diagnostic.level ?? "error", stage: "proposal", code: diagnostic.code, path: diagnostic.local_ref,
      message: diagnostic.message, checked: true, related_units: [diagnostic.local_ref],
    });
  }
  if (groups.size === 0) groupFor(null);
  return [...groups.values()];
}

/** One end of a relation in the change-set-wide overview. */
export interface OverviewEndpoint {
  ref: string;
  title: string;
  /** `new`: a unit of this proposal · `existing`: a map object or another proposal's staged object. */
  origin: "new" | "existing";
  /** Snapshot group (index into `proposalReviewProjection`) of a new unit; `null` otherwise. */
  group: number | null;
}

export interface OverviewRelation {
  ref: string;
  relation: string;
  op: string;
  source: OverviewEndpoint;
  target: OverviewEndpoint;
  /** Both ends are new units in different snapshot groups. */
  crossSnapshot: boolean;
}

export interface OverviewGroup {
  index: number;
  artifact: string | null;
  /** Units shown in this snapshot. */
  units: number;
  /** Relations listed in this snapshot (cross-snapshot ones appear in both). */
  relations: number;
  crossRelations: number;
}

/** Change-set-wide numbers and every relation, independent of the shown snapshot. */
export interface ProposalOverview {
  units: number;
  spans: number;
  relations: number;
  debts: number;
  groups: OverviewGroup[];
  /** Every unit of the proposal with its snapshot group. */
  unitList: { ref: string; title: string; group: number }[];
  relationList: OverviewRelation[];
  /** Group shown first: the one with the most units + relations. */
  defaultGroup: number;
  /** Display title of a ref (new unit local ref or existing object id). */
  title(ref: string): string;
  /** Group of a new unit, `null` for existing objects. */
  groupOf(ref: string): number | null;
}

const shortRef = (value: string): string => value.length > 28 ? `${value.slice(0, 16)}…${value.slice(-8)}` : value;

export function proposalOverview(item: PendingChangeSet, groups: ProposalReviewProjection[] = proposalReviewProjection(item)): ProposalOverview {
  const objects = item.proposal.proposed_objects ?? [];
  const relations = item.proposal.proposed_relations ?? [];
  const unitGroup = new Map<string, number>();
  const titles = new Map<string, string>();
  groups.forEach((group, index) => group.units.forEach(unit => {
    if (!unitGroup.has(unit.unit_ref)) unitGroup.set(unit.unit_ref, index);
    titles.set(unit.unit_ref, unit.display_title ?? unit.unit_ref);
  }));
  const title = (ref: string) => titles.get(ref) ?? item.targetTitles?.[ref] ?? shortRef(ref);
  const groupOf = (ref: string) => unitGroup.get(ref) ?? null;
  const endpoint = (ref: string): OverviewEndpoint => ({ ref, title: title(ref), origin: unitGroup.has(ref) ? "new" : "existing", group: groupOf(ref) });
  const relationList = relations.map(relation => {
    const sourceRef = newRef(relation.source) ?? relation.target_id ?? "";
    const targetRef = newRef(relation.target) ?? "";
    const source = endpoint(sourceRef), target = endpoint(targetRef);
    return {
      ref: relation.local_ref, relation: relation.relation ?? relation.op, op: relation.op, source, target,
      crossSnapshot: source.group !== null && target.group !== null && source.group !== target.group,
    };
  });
  const overviewGroups = groups.map((group, index) => {
    const refs = new Set(group.units.map(unit => unit.unit_ref));
    return {
      index, artifact: group.source.artifact_id, units: group.units.length, relations: group.relations.length,
      crossRelations: group.relations.filter(relation => !refs.has(relation.source) || !refs.has(relation.target))
        .filter(relation => unitGroup.has(relation.source) && unitGroup.has(relation.target)).length,
    };
  });
  let defaultGroup = 0;
  overviewGroups.forEach(group => {
    const best = overviewGroups[defaultGroup];
    if (group.units + group.relations > best.units + best.relations) defaultGroup = group.index;
  });
  return {
    units: objects.filter(object => object.kind === "source_unit").length,
    spans: objects.filter(object => object.kind === "source_span").length,
    relations: relations.length,
    debts: proposalDebts(item).length,
    groups: overviewGroups, relationList, defaultGroup, title, groupOf,
    unitList: [...unitGroup].map(([ref, group]) => ({ ref, title: title(ref), group })),
  };
}

/** Diagnostics without a `local_ref`, grouped by the existing object they name. */
export interface TargetDiagnosticGroup {
  /** `null`: about the proposal as a whole. */
  targetId: string | null;
  title: string | null;
  diagnostics: ProposalDiagnostic[];
}

export function targetDiagnosticGroups(item: PendingChangeSet): TargetDiagnosticGroup[] {
  const groups = new Map<string | null, TargetDiagnosticGroup>();
  for (const diagnostic of item.result.diagnostics ?? []) {
    if (diagnostic.local_ref) continue;
    const targetId = diagnostic.target_id ?? null;
    let group = groups.get(targetId);
    if (!group) {
      group = { targetId, title: targetId ? item.targetTitles?.[targetId] ?? null : null, diagnostics: [] };
      groups.set(targetId, group);
    }
    group.diagnostics.push(diagnostic);
  }
  // Object groups first (by id), the proposal-level group last.
  return [...groups.values()].sort((a, b) => a.targetId === null ? 1 : b.targetId === null ? -1 : a.targetId < b.targetId ? -1 : 1);
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

/**
 * One snapshot group of a proposal in the discovery workspace's data shape,
 * opened read-only (`ReviewRunMeta.proposal`): no unit selection, no
 * Markdown patch, no registration. `contentHash` is the hash of the loaded
 * snapshot the anchors were checked against; the span ids are bound to the
 * immutable snapshot artifact, so this is the same document.
 */
export function proposalDiscoveryOutcome(item: PendingChangeSet, group: ProposalReviewProjection, contentHash: string): DiscoveryOutcome {
  const runId = `proposal:${keyOf(item)}:${group.source.artifact_id ?? "existing"}`;
  const source = group.source.artifact_id
    ? { artifact_id: group.source.artifact_id, conversion_id: group.source.conversion_id, content_hash: contentHash, input_fingerprint: item.proposalFingerprint }
    : null;
  const status = item.state === "staged" ? "completed" : "needs_attention";
  const anchorView = (anchor: SourceReviewAnchor) => ({ start: anchor.range.start, end: anchor.range.end, block_ids: anchor.block_ids, excerpt: anchor.excerpt, purpose: anchor.purpose });
  return {
    outcome: status,
    view: {
      version: "mathscape-desktop-discovery-view-1",
      status,
      complete: true,
      source,
      units: group.units.map(unit => ({
        unit_ref: unit.unit_ref, unit_kind: unit.unit_kind, title: unit.display_title ?? unit.unit_ref, display_title: unit.display_title,
        parent: unit.parent, completeness: unit.completeness, fact_status: "processed", source_excerpt: unit.source_excerpt,
        anchors: unit.anchors.map(anchorView), source_labels: [], source_title: null,
      })),
      references: [],
      relations: group.relations.map(relation => ({
        source: relation.source, target: relation.target, relation: relation.relation,
        evidence: relation.evidence.map(anchorView), bases: [], origins: [],
      })),
      issues: [],
      pending_tasks: [],
      diagnostics: [],
      attempts: [],
    },
    review: source && {
      version: "mathscape-source-review-projection-1",
      run_id: runId,
      result_fingerprint: item.proposalFingerprint,
      status,
      complete: true,
      source,
      units: group.units.map(unit => ({ ...unit, acceptance: item.state === "staged" ? unit.acceptance : "blocked" })),
      references: [],
      structure_relations: group.structure_relations,
      relations: group.relations,
      diagnostics: group.diagnostics,
      pending_tasks: [],
      acceptance: { state: item.state, selected_units: [], required_units: [], missing_targets: [], blockers: [], allowed_scopes: [] },
    },
    run: {
      run_id: runId, profile_id: "propose_changes", provider: item.proposedBy, requested_model: "",
      admission: "review_before_publish", request_timeout_seconds: null, connect_timeout_seconds: null,
      attempts: 0, input_tokens: null, output_tokens: null, total_tokens: null, elapsed_ms: 0,
    },
  };
}

/** Result of a whole-proposal decision taken in the review workspace. */
export interface ProposalDecision {
  message: string;
  /** Pending versions left in the investigation after the decision. */
  remaining: number;
}

/**
 * What the review workspace needs to act on a proposal. The backend still
 * owns every rule; this only routes the user's decision to it.
 */
export interface ProposalWorkspaceController {
  readonly investigationId: string;
  /** Opens another pending version (or another snapshot of the same one); no index ⇒ the overview's default group. */
  open(key: string, groupIndex?: number): Promise<void>;
  preview(item: PendingChangeSet): Promise<ImpactPreview>;
  /** Accepts one version through the batch accept (plan → accept, one commit). */
  accept(item: PendingChangeSet): Promise<ProposalDecision>;
  discard(item: PendingChangeSet, cascade: boolean): Promise<ProposalDecision>;
}

/** `ReviewRunMeta.proposal`: the version, its queue and the snapshot read for it. */
export interface ProposalWorkspaceMeta {
  changeSetId: string;
  version: number;
  state: string;
  proposedBy: string;
  snapshot: string | null;
  item: PendingChangeSet;
  /** The whole pending queue, for navigation, dependencies and discard previews. */
  items: PendingChangeSet[];
  groupIndex: number;
  groupCount: number;
  /** Change-set-wide totals and every relation (not just the shown snapshot). */
  overview: ProposalOverview;
  /** The snapshot from `get_change_set_snapshot`, or `null` with `documentError`. */
  document: AnchoredMarkdown | null;
  documentError: string | null;
  /** Server anchors of the shown group (verified flag, page). */
  anchors: ProposalAnchor[];
  controller: ProposalWorkspaceController;
}

/** Display label of a version state. */
export function stateLabel(state: string): string {
  return ({ staged: "未审批", blocked: "阻塞", superseded: "已替换", discarded: "已丢弃", accepted: "已接受" } as Record<string, string>)[state] ?? state;
}
