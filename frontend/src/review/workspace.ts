import { invoke } from "@tauri-apps/api/core";
import { markdownRenderer } from "../reader/markdown";
import { markdownDocument, type AnchoredMarkdown } from "../reader/selection";
import { getSourceBacklink, type SourceBacklinkView } from "../reader/backlink";
import { checkAnchor, rawAnchorPreview, renderAnchorFragment, reviewAnchor, type ReviewAnchor } from "./anchor-fragment";
import {
  acceptanceLabel, anchorPurposeLabel, completenessLabel, factStatusLabel, issueKindLabel,
  referenceStatusLabel, relationLabel, unitKindLabel, workflowStatusLabel,
} from "./labels";
import { ReviewGraph } from "./review-graph";
import type { DiscoveryAcceptanceOutcome, DiscoveryOutcome, DiscoveryRun, ReviewRunMeta, SourceReviewProjection } from "./types";
import { eventBus } from "../core/events";
import "./review.css";
import { openSourceRegistrationReview } from "../discovery/registration-workspace";

export const OPEN_REVIEW_EVENT = "mathscape:open-review";
export type { OpenReviewDetail } from "../core/events";

export function openReviewWorkspace(outcome: DiscoveryOutcome, meta?: ReviewRunMeta): void {
  eventBus.emit("open-review", { outcome, meta });
}

/** Larger results start as a layered list; a graph of that size is unreadable. */
const GRAPH_UNIT_LIMIT = 40;

interface WUnit {
  ref: string; kind: string; title: string; parent: string | null; completeness: string;
  factStatus: string; acceptance: string | null; anchors: ReviewAnchor[]; excerpt: string;
  facts: { kind: string; text: string; anchor: ReviewAnchor }[];
}
interface WReference { id: string; owner: string; text: string; status: string; candidates: string[]; reason: string; anchor: ReviewAnchor }
interface WRelation { id: string; source: string; target: string; relation: string; bases: string[]; evidence: ReviewAnchor[] }
interface WIssue { id: string; owner: string | null; kind: string; stages: string; observation: string; occurrences: number; evidence: ReviewAnchor[] }
interface WDiagnostic { id: string; severity: string; code: string; path: string; message: string; related: string[] }
interface Model {
  units: WUnit[]; unitByRef: Map<string, WUnit>; references: WReference[]; relations: WRelation[];
  structure: { parent: string; child: string }[]; issues: WIssue[]; diagnostics: WDiagnostic[];
  pending: { task_ref: string; owner: string | null; reason: string | null }[];
}

type TargetKind = "unit" | "fact" | "reference" | "relation" | "issue" | "diagnostic";
interface Target {
  kind: TargetKind; id: string; label: string; anchors: ReviewAnchor[];
  units: string[]; relation: string | null; note?: string;
}
type Tab = "overview" | "units" | "references" | "relations" | "diagnostics" | "graph";

interface Draft { fingerprint: string; selected: string[]; target: string | null; tab: Tab }

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className = "", text = ""): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  node.textContent = text;
  return node;
}

function button(text: string, onclick: () => void, className = ""): HTMLButtonElement {
  const node = el("button", className, text);
  node.type = "button";
  node.onclick = onclick;
  return node;
}

function shortId(value: string): string {
  return value.length > 30 ? `${value.slice(0, 14)}…${value.slice(-10)}` : value;
}

function buildModel(outcome: DiscoveryOutcome): Model {
  const view = outcome.view;
  const review = outcome.review;
  const titles = new Map(view.units.map(unit => [unit.unit_ref, unit.title]));
  let units: WUnit[];
  let references: WReference[];
  let relations: WRelation[];
  let structure: Model["structure"];
  let diagnostics: WDiagnostic[];
  let pending: Model["pending"];
  if (review) {
    units = review.units.map(unit => ({
      ref: unit.unit_ref, kind: unit.unit_kind, title: titles.get(unit.unit_ref) ?? unit.display_title ?? unit.unit_ref,
      parent: unit.parent, completeness: unit.completeness, factStatus: unit.fact_status, acceptance: unit.acceptance,
      anchors: unit.anchors.map(anchor => reviewAnchor(anchor)), excerpt: unit.source_excerpt,
      facts: [
        ...(unit.source_title ? [{ kind: "原文标题", text: unit.source_title.text, anchor: reviewAnchor(unit.source_title.anchor) }] : []),
        ...unit.source_labels.map(label => ({ kind: "标签", text: label.text, anchor: reviewAnchor(label.anchor) })),
      ],
    }));
    references = review.references.map(reference => ({
      id: reference.reference_ref, owner: reference.owner, text: reference.text, status: reference.status,
      candidates: reference.candidates, reason: reference.reason, anchor: reviewAnchor(reference.anchor),
    }));
    relations = review.relations.map(relation => ({
      id: relation.relation_ref, source: relation.source, target: relation.target, relation: relation.relation,
      bases: relation.bases, evidence: relation.evidence.map(anchor => reviewAnchor(anchor)),
    }));
    structure = review.structure_relations.map(({ parent, child }) => ({ parent, child }));
    diagnostics = review.diagnostics.map(item => ({
      id: item.diagnostic_ref, severity: item.severity, code: item.code, path: item.path, message: item.message, related: item.related_units,
    }));
    pending = review.pending_tasks;
  } else {
    units = view.units.map(unit => ({
      ref: unit.unit_ref, kind: unit.unit_kind, title: unit.title, parent: unit.parent, completeness: unit.completeness,
      factStatus: unit.fact_status, acceptance: null, anchors: unit.anchors.map(anchor => reviewAnchor(anchor, "unit")),
      excerpt: unit.source_excerpt,
      facts: [
        ...(unit.source_title ? [{ kind: "原文标题", text: unit.source_title.text, anchor: reviewAnchor(unit.source_title.anchor, "source_title") }] : []),
        ...unit.source_labels.map(label => ({ kind: "标签", text: label.text, anchor: reviewAnchor(label.anchor, "source_label") })),
      ],
    }));
    references = view.references.map((reference, index) => ({
      id: `reference-${index}`, owner: reference.owner, text: reference.text, status: reference.status,
      candidates: reference.candidates, reason: reference.reason, anchor: reviewAnchor(reference.anchor, "reference"),
    }));
    relations = view.relations.map((relation, index) => ({
      id: `relation-${index}`, source: relation.source, target: relation.target, relation: relation.relation,
      bases: relation.bases, evidence: relation.evidence.map(anchor => reviewAnchor(anchor, "relation_evidence")),
    }));
    structure = view.units.filter(unit => unit.parent).map(unit => ({ parent: unit.parent!, child: unit.unit_ref }));
    diagnostics = view.diagnostics.map((item, index) => ({
      id: `diagnostic-${index}`, severity: item.code.startsWith("warning_") ? "warning" : "error",
      code: item.code, path: item.path, message: item.message, related: [],
    }));
    pending = view.pending_tasks.map(task => ({ ...task, reason: null }));
  }
  const issues = view.issues.map(issue => ({
    id: issue.issue_ref, owner: issue.owner, kind: issue.kind,
    stages: (issue.stages ?? [issue.stage]).map(stage => stage === "unit_discovery" ? "单元识别" : "来源事实").join("、"),
    observation: issue.observation, occurrences: issue.occurrences ?? 1,
    evidence: issue.evidence.map(anchor => reviewAnchor(anchor, "diagnostic_evidence")),
  }));
  return { units, unitByRef: new Map(units.map(unit => [unit.ref, unit])), references, relations, structure, issues, diagnostics, pending };
}

/**
 * Review workspace for one discovery run: result lists on the left, the
 * exact Markdown context of the current anchor on the right. It overlays
 * the map and reader, and survives both being refreshed.
 */
export class DiscoveryReviewWorkspace {
  private outcome: DiscoveryOutcome | null = null;
  private meta: ReviewRunMeta = {};
  private model: Model | null = null;
  private doc: AnchoredMarkdown | null = null;
  private bytes = new Uint8Array();
  private stale: string | null = null;
  private generation = 0;
  private tab: Tab = "overview";
  private locked: Target | null = null;
  private lockedAnchor = 0;
  private hovering = false;
  private rawMode = false;
  private selected = new Set<string>();
  private eligible = new Set<string>();
  private accepted = false;
  private accepting = false;
  private graph: ReviewGraph | null = null;
  private graphMode = false;
  private evidenceMode = false;
  private readonly md = markdownRenderer();

  private header = el("header", "review-header");
  private tabs = el("nav", "review-tabs");
  private list = el("div", "review-list");
  private footer = el("footer", "review-footer");
  private preview = el("section", "review-preview");
  private returnChip = el("div", "review-return-chip");

  constructor(private host: HTMLElement) {
    host.classList.add("review-workspace");
    host.setAttribute("aria-label", "内容发现审核工作区");
    host.hidden = true;
    const left = el("section", "review-results");
    left.append(this.tabs, this.list, this.footer);
    this.preview.setAttribute("aria-label", "来源预览");
    const body = el("div", "review-body");
    body.append(left, this.preview);
    host.append(this.header, body);
    this.returnChip.hidden = true;
    this.returnChip.append(
      button("返回审核工作区", () => this.resume(), "review-return-open"),
      button("×", () => this.close(), "review-return-close"),
    );
    this.returnChip.querySelector(".review-return-close")!.setAttribute("aria-label", "关闭审核工作区");
    (host.parentElement ?? document.body).append(this.returnChip);
    eventBus.on("reader-opened", () => { if (this.isOpen()) this.suspend(); });
    eventBus.on("discovery-runs-changed",()=>void this.refreshRegistrationState());
  }
  private async refreshRegistrationState():Promise<void> {
    const runId=this.outcome?.run.run_id;
    if(!runId)return;
    const generation=this.generation;
    try {
      const summaries=await invoke<import("../discovery/types").DiscoveryRunSummaryView[]>("list_discovery_runs",{request:{artifact_id:null}});
      const summary=summaries.find(s=>s.run_id===runId);
      if(generation!==this.generation)return;
      if(summary&&summary.registrations?.length===0&&summary.review_status!==this.meta.review_status) {
        if(summary.review_status==="discarded") {
          this.meta.review_status="discarded";this.renderHeader();this.renderFooter();this.syncCheckboxes();
        }
        const envelope=await invoke<import("../discovery/types").DiscoveryReviewEnvelopeView>("get_discovery_review",{runId});
        if(generation!==this.generation)return;
        await this.open(envelope.outcome,{...this.meta,review_status:envelope.review_status,disposition:envelope.disposition,registrations:envelope.registrations});return;
      }
      const registrations=await invoke<import("../discovery/registration-types").RegistrationSummary[]>("list_discovery_registrations",{runId});
      if(generation!==this.generation||registrations.length===0)return;
      this.meta.registrations=registrations;
      this.meta.review_status=registrations.every(r=>r.state==="withdrawn")?"withdrawn":"registered";
      this.accepted=true;
      if(this.outcome?.review){this.outcome.review.acceptance.state=this.meta.review_status==="withdrawn"?"blocked":"registered";}
      if(this.model)this.model.units.forEach(u=>{u.acceptance=this.meta.review_status==="withdrawn"?"blocked":u.acceptance;});
      this.renderHeader();this.renderFooter();this.syncCheckboxes();
    }catch { /* The ledger can retry; do not discard an open review/draft on read failure. */ }
  }

  isOpen(): boolean { return !this.host.hidden; }

  async open(outcome: DiscoveryOutcome, meta: ReviewRunMeta = {}): Promise<void> {
    const generation = ++this.generation;
    this.graph?.destroy(); this.graph = null;
    this.outcome = outcome;
    this.meta = meta;
    this.model = buildModel(outcome);
    this.doc = null; this.bytes = new Uint8Array(); this.stale = null;
    this.locked = null; this.lockedAnchor = 0; this.rawMode = false;
    this.graphMode = this.model.units.length < GRAPH_UNIT_LIMIT; this.evidenceMode = false;
    const review = outcome.review;
    this.accepted = review?.acceptance.state === "registered"
      || meta.review_status === "registered" || meta.review_status === "withdrawn";
    this.eligible = new Set(review?.units.filter(unit => unit.acceptance !== "blocked").map(unit => unit.unit_ref) ?? []);
    this.selected = new Set(review?.acceptance.selected_units ?? []);
    if (this.selected.size === 0) this.eligible.forEach(ref => this.selected.add(ref));
    this.tab = "overview";
    const draft = this.readDraft();
    if (draft && !this.accepted) {
      this.selected = new Set(draft.selected.filter(ref => this.eligible.has(ref)));
      this.tab = draft.tab;
    }
    this.host.hidden = false;
    this.returnChip.hidden = true;
    this.renderHeader();
    this.renderTabs();
    this.renderList();
    this.renderFooter();
    this.preview.replaceChildren(el("p", "review-preview-empty", "正在读取来源 Markdown…"));
    const source = outcome.view.source;
    if (source === null) {
      this.stale = "发现结果没有记录来源快照。";
    } else {
      try {
        const doc = await markdownDocument(source.artifact_id, source.conversion_id);
        if (generation !== this.generation) return;
        if (doc.snapshot.content_hash !== source.content_hash) {
          this.stale = `当前 Markdown 内容哈希 ${shortId(doc.snapshot.content_hash)} 与运行时 ${shortId(source.content_hash)} 不一致；锚点已失效，不做近似定位。`;
        } else {
          this.doc = doc;
          this.bytes = new TextEncoder().encode(doc.source);
        }
      } catch (error) {
        if (generation !== this.generation) return;
        this.stale = `无法读取运行时的 Markdown 快照：${String(error)}`;
      }
    }
    this.renderHeader();
    this.renderFooter();
    const restored = draft?.target ? this.findTarget(draft.target) : null;
    const first = this.model.units.find(unit => unit.anchors.length > 0);
    this.lock(restored ?? (first ? this.unitTarget(first) : null), false);
  }

  suspend(): void {
    if (this.outcome === null) return;
    this.host.hidden = true;
    this.returnChip.hidden = false;
    const label = this.returnChip.querySelector<HTMLElement>(".review-return-open");
    if (label) label.textContent = `返回审核工作区 · ${this.meta.title ?? "内容发现"}`;
  }

  resume(): void {
    if (this.outcome === null) return;
    this.host.hidden = false;
    this.returnChip.hidden = true;
    this.graph?.resize();
  }

  close(): void {
    this.generation++;
    this.graph?.destroy(); this.graph = null;
    this.outcome = null; this.model = null; this.doc = null;
    this.host.hidden = true;
    this.returnChip.hidden = true;
  }

  // ---------------------------------------------------------------- state

  private draftKey(): string | null {
    return this.outcome?.review ? `mathscape:review-draft:${this.outcome.review.run_id}` : null;
  }

  private readDraft(): Draft | null {
    const key = this.draftKey();
    if (!key) return null;
    try {
      const draft = JSON.parse(localStorage.getItem(key) ?? "null") as Draft | null;
      return draft?.fingerprint === this.outcome!.review!.result_fingerprint ? draft : null;
    } catch { return null; }
  }

  /** A presentation draft only; the backend re-validates any acceptance. */
  private saveDraft(): void {
    // A read-only historical/discarded view has no eligible selection. Do not
    // replace the user's actual pre-discard review draft with that empty set.
    if(this.accepted||this.meta.review_status==="discarded"||this.meta.review_status==="withdrawn")return;
    const key = this.draftKey();
    if (!key) return;
    const draft: Draft = {
      fingerprint: this.outcome!.review!.result_fingerprint,
      selected: [...this.selected].sort(),
      target: this.locked ? `${this.locked.kind}:${this.locked.id}` : null,
      tab: this.tab,
    };
    try { localStorage.setItem(key, JSON.stringify(draft)); } catch { /* Storage may be unavailable. */ }
  }

  private title(ref: string): string {
    return this.model?.unitByRef.get(ref)?.title ?? ref;
  }

  private unitTarget(unit: WUnit): Target {
    return { kind: "unit", id: unit.ref, label: unit.title, anchors: unit.anchors, units: [unit.ref], relation: null };
  }

  private referenceTarget(reference: WReference): Target {
    return {
      kind: "reference", id: reference.id, label: `引用「${reference.text}」`, anchors: [reference.anchor],
      units: [reference.owner, ...reference.candidates], relation: null,
    };
  }

  private relationTarget(relation: WRelation): Target {
    return {
      kind: "relation", id: relation.id,
      label: `${this.title(relation.source)} —${relationLabel(relation.relation)}→ ${this.title(relation.target)}`,
      anchors: relation.evidence, units: [relation.source, relation.target], relation: relation.id,
      note: relation.evidence.length === 0 ? "该关系没有原文证据范围。" : undefined,
    };
  }

  private issueTarget(issue: WIssue): Target {
    const owner = issue.owner ? this.model!.unitByRef.get(issue.owner) : undefined;
    if (issue.evidence.length > 0 || !owner?.anchors[0]) {
      return {
        kind: "issue", id: issue.id, label: `${issueKindLabel(issue.kind)}：${issue.observation}`,
        anchors: issue.evidence, units: issue.owner ? [issue.owner] : [], relation: null,
        note: issue.evidence.length === 0 ? "该问题没有原文范围，也没有可回退的单元。" : undefined,
      };
    }
    return {
      kind: "issue", id: issue.id, label: `${issueKindLabel(issue.kind)}：${issue.observation}`,
      anchors: [owner.anchors[0]], units: [owner.ref], relation: null,
      note: `问题未提供独立原文范围；显示所属单元「${owner.title}」的第一个锚点。`,
    };
  }

  private diagnosticTarget(diagnostic: WDiagnostic): Target {
    const unit = diagnostic.related.map(ref => this.model!.unitByRef.get(ref)).find(candidate => candidate?.anchors[0]);
    return {
      kind: "diagnostic", id: diagnostic.id, label: `${diagnostic.code}：${diagnostic.message}`,
      anchors: unit ? [unit.anchors[0]] : [], units: diagnostic.related, relation: null,
      note: unit
        ? `诊断未提供独立原文范围；显示相关单元「${unit.title}」的第一个锚点。`
        : "诊断未提供独立原文范围，也没有相关单元可回退。",
    };
  }

  private findTarget(key: string): Target | null {
    const model = this.model!;
    const [kind, ...rest] = key.split(":");
    const id = rest.join(":");
    if (kind === "unit") { const unit = model.unitByRef.get(id); return unit ? this.unitTarget(unit) : null; }
    if (kind === "reference") { const item = model.references.find(r => r.id === id); return item ? this.referenceTarget(item) : null; }
    if (kind === "relation") { const item = model.relations.find(r => r.id === id); return item ? this.relationTarget(item) : null; }
    if (kind === "issue") { const item = model.issues.find(r => r.id === id); return item ? this.issueTarget(item) : null; }
    if (kind === "diagnostic") { const item = model.diagnostics.find(r => r.id === id); return item ? this.diagnosticTarget(item) : null; }
    return null;
  }

  private lock(target: Target | null, save = true): void {
    this.locked = target;
    this.lockedAnchor = 0;
    this.hovering = false;
    this.syncCurrent();
    this.renderPreview(target, 0);
    if (save) this.saveDraft();
  }

  private hover(target: Target): void {
    this.hovering = true;
    this.syncCurrent(target);
    this.renderPreview(target, 0, true);
  }

  private unhover(): void {
    if (!this.hovering) return;
    this.hovering = false;
    this.syncCurrent();
    this.renderPreview(this.locked, this.lockedAnchor);
  }

  private syncCurrent(target = this.locked): void {
    const key = target ? `${target.kind}:${target.id}` : "";
    this.list.querySelectorAll<HTMLElement>("[data-target]").forEach(item => {
      item.classList.toggle("is-current", item.dataset.target === key);
    });
    this.graph?.setCurrent(target?.units ?? [], target?.relation ?? null);
  }

  private bindTarget(item: HTMLElement, target: () => Target): void {
    const resolved = target();
    item.dataset.target = `${resolved.kind}:${resolved.id}`;
    item.tabIndex = 0;
    item.addEventListener("click", event => {
      if ((event.target as HTMLElement).closest("input, button:not(.review-item-main)")) return;
      this.lock(target());
    });
    item.addEventListener("keydown", event => {
      if (event.target === item && (event.key === "Enter" || event.key === " ")) { event.preventDefault(); this.lock(target()); }
    });
    item.addEventListener("mouseenter", () => this.hover(target()));
    item.addEventListener("mouseleave", () => this.unhover());
  }

  // --------------------------------------------------------------- header

  private renderHeader(): void {
    const outcome = this.outcome!;
    const source = outcome.view.source;
    const title = el("div", "review-title");
    title.append(el("span", "eyebrow", "REVIEW"), el("h2", "", `内容发现审核 · ${this.meta.title ?? source?.artifact_id ?? "未知材料"}`));
    const facts = [
      `流水线：${workflowStatusLabel(outcome.outcome)}`,
      ...(this.meta.review_status ? [`审核：${acceptanceLabel(this.meta.review_status)}`] : []),
      `运行 ${shortId(outcome.run.run_id)}`,
      ...(outcome.review ? [`结果指纹 ${shortId(outcome.review.result_fingerprint)}`] : []),
      ...(this.meta.updated_at ? [`更新于 ${this.meta.updated_at}`] : []),
    ];
    const meta = el("p", "review-meta", facts.join(" · "));
    const state = el("span", `review-state ${this.stale ? "is-stale" : "is-live"}`, this.stale ? "锚点失效" : "锚点有效");
    if (this.stale) state.title = this.stale;
    const actions = el("div", "review-header-actions");
    const back = button("返回选区", () => this.openSelection());
    back.disabled = this.stale !== null || !source?.requested_range || this.doc === null;
    const open = button("打开原文", () => this.openSource(this.currentAnchor()));
    open.disabled = source === null;
    actions.append(back, open, button("返回地图", () => this.suspend()), button("关闭", () => this.close()));
    this.header.replaceChildren(title, state, actions, meta);
  }

  private currentAnchor(): ReviewAnchor | undefined {
    return this.locked?.anchors[this.lockedAnchor];
  }

  private openSource(anchor?: ReviewAnchor): void {
    const source = this.outcome?.view.source;
    if (!source) return;
    this.suspend();
    eventBus.emit("open-source", {
      artifactId: source.artifact_id,
      conversionId: source.conversion_id,
      title: this.meta.title ?? "来源 Markdown",
      focus: anchor && this.stale === null ? { start: anchor.start, end: anchor.end, blockIds: anchor.block_ids } : undefined,
    });
  }

  private openSelection(): void {
    const range = this.outcome?.view.source?.requested_range;
    if (!range || !this.doc) return;
    const blockIds = this.doc.blocks
      .filter(block => block.range.start < range.end && block.range.end > range.start)
      .map(block => block.id);
    this.openSource({ start: range.start, end: range.end, block_ids: blockIds, excerpt: "", purpose: "selection" });
  }

  // ----------------------------------------------------------------- tabs

  private renderTabs(): void {
    const model = this.model!;
    const tabs: [Tab, string][] = [
      ["overview", "总览"],
      ["units", `单元 ${model.units.length}`],
      ["references", `引用 ${model.references.length}`],
      ["relations", `关系 ${model.relations.length}`],
      ["diagnostics", `诊断 ${model.issues.length + model.diagnostics.length}`],
      ["graph", "局部结构图"],
    ];
    this.tabs.setAttribute("role", "tablist");
    this.tabs.replaceChildren(...tabs.map(([tab, label]) => {
      const item = button(label, () => { this.tab = tab; this.renderTabs(); this.renderList(); this.saveDraft(); }, "review-tab");
      item.setAttribute("role", "tab");
      item.setAttribute("aria-selected", String(tab === this.tab));
      return item;
    }));
  }

  private renderList(): void {
    this.graph?.destroy(); this.graph = null;
    this.list.replaceChildren();
    this.list.scrollTop = 0;
    ({
      overview: () => this.renderOverview(),
      units: () => this.renderUnits(),
      references: () => this.renderReferences(),
      relations: () => this.renderRelations(),
      diagnostics: () => this.renderDiagnostics(),
      graph: () => this.renderGraph(),
    })[this.tab]();
    this.syncCurrent(this.hovering ? undefined : this.locked);
  }

  private renderOverview(): void {
    const outcome = this.outcome!;
    const model = this.model!;
    const unresolved = model.references.filter(reference => reference.status === "unresolved").length;
    const summary = el("div", "review-summary");
    const stat = (value: number, label: string) => {
      const box = el("div", "review-stat");
      box.append(el("strong", "", String(value)), el("span", "", label));
      return box;
    };
    summary.append(
      stat(model.units.length, "单元"), stat(model.references.length, "引用"), stat(unresolved, "未解析引用"),
      stat(model.relations.length, "关系"), stat(model.issues.length + model.diagnostics.length, "问题与诊断"),
      stat(model.pending.length, "待完成任务"),
    );
    this.list.append(
      summary,
      el("p", "review-note", `两阶段内容发现${workflowStatusLabel(outcome.outcome)}。${this.meta.review_status==="discarded"?"结果已放弃，仅供查看；可从发现记录点击恢复审核。":this.meta.review_status==="withdrawn"?"该登记已撤回；这是原始发现结果，不代表活动地图。":this.accepted?"已保存登记；部分对象可能已撤回或后续修改，请以登记回执为准。":"结果尚未登记。"}审核图中的对象都是候选，不是当前正式地图对象。`),
    );
    if (model.pending.length > 0) {
      this.list.append(el("p", "review-warning review-pending", `尚有 ${model.pending.length} 个事实任务未完成：${model.pending.map(task => task.owner ? this.title(task.owner) : task.task_ref).join("、")}`));
    }
    const blockers = outcome.review?.acceptance.blockers ?? [];
    if (blockers.length > 0) {
      this.list.append(el("p", "review-warning", `以下单元需修订，未被默认选中：${blockers.map(ref => this.title(ref)).join("、")}`));
    }
    if (this.stale) this.list.append(el("p", "review-warning", this.stale));
    if (model.issues.some(issue => issue.kind === "suspected_conversion_error") && outcome.outcome !== "awaiting_patch_review") {
      this.list.append(this.patchSidecar());
    }
    this.list.append(this.runSummary(outcome.run));
    const details = el("details", "review-json");
    details.append(el("summary", "", "查看完整展示数据"), el("pre", "", JSON.stringify(outcome.view, null, 2)));
    this.list.append(details);
  }

  private patchSidecar(): HTMLElement {
    const outcome = this.outcome!;
    const box = el("div", "review-patch-sidecar");
    const status = el("span", "review-note", "仅处理转换异常，不会自动改写原文。");
    const run = button("生成 Markdown 修订建议", async () => {
      run.disabled = true;
      status.textContent = "正在生成修订建议…";
      try {
        const review = await invoke<{ status: string; message: string | null; patch_set: unknown; patch_set_id: string | null }>(
          "run_markdown_patch", { request: { run_id: outcome.run.run_id, profile_id: null } },
        );
        status.textContent = review.message ?? review.status;
        const source = outcome.view.source;
        if (review.patch_set && review.patch_set_id && source) {
          this.suspend();
          eventBus.emit("open-source", {
            artifactId: source.artifact_id,
            conversionId: source.conversion_id,
            title: this.meta.title ?? "Markdown 修订审核",
            patchRunId: outcome.run.run_id,
            patchMode: "manual",
          });
        }
      } catch (error) {
        status.textContent = `修订建议失败：${String(error)}`;
      } finally {
        run.disabled = false;
      }
    });
    box.append(el("h4", "", "Markdown 转换校准"), run, status);
    return box;
  }

  private runSummary(run: DiscoveryRun): HTMLElement {
    const usage = run.total_tokens === null || run.total_tokens === undefined ? "token 数未知" : `${run.total_tokens} tokens`;
    const admission = run.admission === "automatic_candidate" ? "自动候选策略（尚未登记）" : "等待人工审核";
    const timeout = run.request_timeout_seconds === null ? "原运行期限未记录" : `总期限 ${run.request_timeout_seconds} 秒`;
    return el("p", "review-run", `${run.profile_id} · ${run.requested_model} · ${run.attempts} 次调用 · ${usage} · ${run.elapsed_ms} ms · ${timeout} · ${admission}`);
  }

  private renderMath(text: string, className: string): HTMLElement {
    const node = el("div", `${className} markdown-body`);
    node.innerHTML = this.md.render(text);
    node.querySelectorAll("a").forEach(link => link.removeAttribute("href"));
    return node;
  }

  private depth(unit: WUnit): number {
    let value = 0;
    let parent = unit.parent;
    const seen = new Set<string>();
    while (parent && !seen.has(parent)) {
      seen.add(parent); value += 1; parent = this.model!.unitByRef.get(parent)?.parent ?? null;
    }
    return value;
  }

  private renderUnits(): void {
    const selectable = this.outcome!.review !== null;
    for (const unit of this.model!.units) {
      const item = el("article", `review-item review-unit state-${unit.acceptance ?? "none"}`);
      item.style.setProperty("--review-depth", String(this.depth(unit)));
      const heading = el("div", "review-item-heading");
      if (selectable) {
        const checkbox = document.createElement("input");
        checkbox.type = "checkbox";
        checkbox.className = "review-unit-checkbox";
        checkbox.dataset.unit = unit.ref;
        checkbox.setAttribute("aria-label", `选择来源单元 ${unit.title}`);
        checkbox.onchange = () => this.toggleUnit(unit.ref, checkbox.checked);
        heading.append(checkbox);
      }
      heading.append(el("strong", "", unit.title));
      const meta = [
        unitKindLabel(unit.kind), completenessLabel(unit.completeness), factStatusLabel(unit.factStatus),
        ...(unit.acceptance ? [acceptanceLabel(unit.acceptance)] : []),
        ...(unit.parent ? [`隶属 ${this.title(unit.parent)}`] : []),
      ].join(" · ");
      item.append(heading, el("span", "review-item-meta", meta), this.renderMath(unit.excerpt, "review-excerpt"));
      if (unit.facts.length > 0) {
        const facts = el("div", "review-facts");
        unit.facts.forEach((fact, index) => {
          const chip = button(`${fact.kind}：${fact.text}`, () => this.lock({
            kind: "fact", id: `${unit.ref}#${index}`, label: `${fact.kind}「${fact.text}」`, anchors: [fact.anchor], units: [unit.ref], relation: null,
          }), "review-fact");
          facts.append(chip);
        });
        item.append(facts);
      }
      this.bindTarget(item, () => this.unitTarget(unit));
      this.list.append(item);
    }
    this.syncCheckboxes();
  }

  private renderReferences(): void {
    if (this.model!.references.length === 0) { this.list.append(el("p", "review-empty", "本次运行没有来源引用。")); return; }
    for (const reference of this.model!.references) {
      const item = el("article", `review-item review-reference reference-${reference.status}`);
      const candidates = reference.candidates.length > 0 ? ` · 候选 ${reference.candidates.map(ref => this.title(ref)).join("、")}` : "";
      item.append(
        el("strong", "", `${reference.text} · ${referenceStatusLabel(reference.status)}`),
        el("span", "review-item-meta", `${this.title(reference.owner)}${candidates} · ${reference.reason}`),
      );
      this.bindTarget(item, () => this.referenceTarget(reference));
      this.list.append(item);
    }
  }

  private renderRelations(): void {
    if (this.model!.relations.length === 0 && this.model!.structure.length === 0) {
      this.list.append(el("p", "review-empty", "本次运行没有来源关系。"));
      return;
    }
    for (const relation of this.model!.relations) {
      const item = el("article", "review-item review-relation");
      item.append(
        el("strong", "", `${this.title(relation.source)} —${relationLabel(relation.relation)}→ ${this.title(relation.target)}`),
        el("span", "review-item-meta", `${relation.evidence.length} 处证据${relation.bases.length > 0 ? ` · ${relation.bases.join("；")}` : ""}`),
      );
      this.bindTarget(item, () => this.relationTarget(relation));
      this.list.append(item);
    }
    if (this.model!.structure.length > 0) {
      this.list.append(el("h4", "review-subheading", "结构关系"));
      const list = el("ul", "review-structure");
      this.model!.structure.forEach(edge => list.append(el("li", "", `${this.title(edge.parent)} ⊃ ${this.title(edge.child)}`)));
      this.list.append(list);
    }
  }

  private renderDiagnostics(): void {
    const model = this.model!;
    if (model.issues.length === 0 && model.diagnostics.length === 0) {
      this.list.append(el("p", "review-empty", "没有来源问题或确定性诊断。"));
      return;
    }
    if (model.issues.length > 0) {
      this.list.append(el("h4", "review-subheading", "来源问题"));
      for (const issue of model.issues) {
        const item = el("article", "review-item review-issue");
        const repeated = issue.occurrences > 1 ? `（聚合 ${issue.occurrences} 条）` : "";
        item.append(
          el("strong", "", `${issue.stages} · ${issueKindLabel(issue.kind)}${repeated}`),
          el("span", "review-item-meta", `${issue.owner ? `${this.title(issue.owner)} · ` : ""}${issue.observation}`),
        );
        if (issue.evidence.length === 0) item.append(el("span", "review-fallback", "无独立原文范围"));
        this.bindTarget(item, () => this.issueTarget(issue));
        this.list.append(item);
      }
    }
    if (model.diagnostics.length > 0) {
      this.list.append(el("h4", "review-subheading", "确定性诊断"));
      for (const diagnostic of model.diagnostics) {
        const item = el("article", `review-item review-diagnostic severity-${diagnostic.severity}`);
        item.append(
          el("strong", "", `${diagnostic.severity === "error" ? "验证错误" : diagnostic.severity === "warning" ? "非阻塞警告" : "提示"} · ${diagnostic.code}`),
          el("span", "review-item-meta", `${diagnostic.path} · ${diagnostic.message}`),
          el("span", "review-fallback", "诊断未提供独立原文范围"),
        );
        this.bindTarget(item, () => this.diagnosticTarget(diagnostic));
        this.list.append(item);
      }
    }
  }

  private renderGraph(): void {
    const model = this.model!;
    const controls = el("div", "review-graph-controls");
    const toggle = button(this.graphMode ? "切换为分层列表" : "切换为图模式", () => { this.graphMode = !this.graphMode; this.renderList(); });
    controls.append(toggle);
    this.list.append(controls);
    if (!this.graphMode) {
      if (model.units.length >= GRAPH_UNIT_LIMIT) {
        this.list.append(el("p", "review-note", `本次结果有 ${model.units.length} 个单元，默认以分层列表显示。`));
      }
      const tree = el("div", "review-layered");
      for (const unit of model.units) {
        const row = el("div", `review-layered-row state-${this.graphState(unit)}`, unit.title);
        row.style.setProperty("--review-depth", String(this.depth(unit)));
        this.bindTarget(row, () => this.unitTarget(unit));
        tree.append(row);
      }
      this.list.append(tree);
      return;
    }
    const evidenceLabel = el("label", "review-evidence-toggle");
    const evidence = document.createElement("input");
    evidence.type = "checkbox";
    evidence.checked = this.evidenceMode;
    evidenceLabel.append(evidence, document.createTextNode(" 证据模式（显示原文范围标记）"));
    controls.append(evidenceLabel);
    const legend = el("div", "review-graph-legend");
    [["selectable", "可接受"], ["blocked", "阻塞"], ["pending", "待完成事实"], ["registered", "已登记"]]
      .forEach(([state, label]) => legend.append(el("span", `review-legend-item state-${state}`, label)));
    legend.append(el("span", "review-legend-item is-selected", "已选中入图"), el("span", "review-legend-item is-current", "当前回链"));
    const container = el("div", "review-graph");
    this.list.append(legend, container);
    const evidenceItems = () => model.units.flatMap(unit => unit.anchors.map((_, index) => ({
      id: `${unit.ref}#${index}`, unit: unit.ref, label: `${unit.title} · 原文 ${index + 1}`,
    })));
    this.graph = new ReviewGraph(
      container,
      model.units.map(unit => ({ ref: unit.ref, label: unit.title, state: this.graphState(unit) })),
      model.structure,
      model.relations.map(relation => ({ id: relation.id, source: relation.source, target: relation.target, label: relationLabel(relation.relation) })),
      {
        unit: (ref, hover) => {
          const unit = model.unitByRef.get(ref);
          if (unit) (hover ? this.hover(this.unitTarget(unit)) : this.lock(this.unitTarget(unit)));
        },
        relation: (id, hover) => {
          const relation = model.relations.find(candidate => candidate.id === id);
          if (relation) (hover ? this.hover(this.relationTarget(relation)) : this.lock(this.relationTarget(relation)));
        },
        evidence: id => {
          const [ref, index] = id.split("#");
          const unit = model.unitByRef.get(ref);
          if (!unit) return;
          this.lock(this.unitTarget(unit));
          this.showAnchor(Number(index));
        },
        leave: () => this.unhover(),
      },
    );
    this.graph.setSelected(this.selected);
    if (this.evidenceMode) this.graph.setEvidence(evidenceItems());
    evidence.onchange = () => {
      this.evidenceMode = evidence.checked;
      this.graph?.setEvidence(this.evidenceMode ? evidenceItems() : []);
    };
  }

  private graphState(unit: WUnit): "selectable" | "blocked" | "pending" | "registered" {
    if (unit.acceptance === "blocked") return "blocked";
    if (unit.acceptance === "registered" || unit.acceptance === "accepted") return "registered";
    if (unit.factStatus !== "processed") return "pending";
    return "selectable";
  }

  // -------------------------------------------------------------- preview

  private showAnchor(index: number): void {
    if (!this.locked || index < 0 || index >= this.locked.anchors.length) return;
    this.lockedAnchor = index;
    this.renderPreview(this.locked, index);
  }

  private renderPreview(target: Target | null, index: number, transient = false): void {
    this.preview.classList.toggle("is-transient", transient);
    if (target === null) {
      this.preview.replaceChildren(el("p", "review-preview-empty", "点击左侧的单元、引用、关系或诊断以查看原文。"));
      return;
    }
    const anchor = target.anchors[index];
    const heading = el("div", "review-preview-heading");
    heading.append(
      el("span", "review-preview-purpose", anchor ? anchorPurposeLabel(anchor.purpose) : "无原文范围"),
      el("strong", "review-preview-label", target.label),
    );
    const controls = el("div", "review-preview-controls");
    if (target.anchors.length > 1 && !transient) {
      target.anchors.forEach((_, i) => {
        const item = button(`证据 ${i + 1}`, () => this.showAnchor(i), "review-anchor-tab");
        item.setAttribute("aria-pressed", String(i === index));
        controls.append(item);
      });
    }
    const mode = button(this.rawMode ? "查看渲染" : "查看原文", () => { this.rawMode = !this.rawMode; this.renderPreview(this.locked, this.lockedAnchor); });
    mode.disabled = anchor === undefined || this.doc === null;
    const open = button("在阅读器中打开", () => this.openSource(anchor));
    open.disabled = anchor === undefined || this.doc === null;
    controls.append(mode, open);
    const pdf = button("PDF 原页", () => void this.openPdfBacklink(anchor), "review-pdf-backlink");
    pdf.disabled = anchor === undefined || this.doc === null || this.stale !== null;
    controls.append(pdf);
    const children: HTMLElement[] = [heading, controls];
    if (target.note) children.push(el("p", "review-fallback-note", target.note));
    if (this.stale) {
      children.push(el("p", "review-stale", this.stale));
    } else if (anchor === undefined) {
      children.push(el("p", "review-preview-empty", "该条目没有可定位的原文范围。"));
    } else if (this.doc === null) {
      children.push(el("p", "review-preview-empty", "正在读取来源 Markdown…"));
    } else {
      const check = checkAnchor(this.doc, this.bytes, anchor);
      if (!check.ok) {
        children.push(el("p", "review-stale", `锚点失效：${check.reason}不做近似定位。`));
      } else if (this.rawMode) {
        children.push(rawAnchorPreview(anchor, check));
      } else {
        const raw = el("details", "review-raw-details");
        raw.append(el("summary", "", `查看原文 · UTF-8 [${anchor.start}, ${anchor.end})`), rawAnchorPreview(anchor, check));
        children.push(renderAnchorFragment(this.doc, this.bytes, anchor, check.blocks), raw);
      }
    }
    this.preview.replaceChildren(...children);
  }

  private async openPdfBacklink(anchor?: ReviewAnchor): Promise<void> {
    const source = this.outcome?.view.source;
    if (!source || !anchor || this.stale !== null) return;
    try {
      const backlink = await getSourceBacklink({
        artifact_id: source.artifact_id,
        conversion_id: source.conversion_id,
        start: anchor.start,
        end: anchor.end,
        block_ids: anchor.block_ids,
      });
      if (backlink.kind === "markdown_only" || !backlink.pdf_artifact_id || !backlink.pdf_page_indices?.length) {
        this.preview.append(el("p", "review-fallback-note", backlink.reason ?? "当前锚点没有 PDF 页映射。"));
        return;
      }
      const pages = backlink.pdf_page_indices;
      if (backlink.kind === "candidate_pdf_pages" && pages.length > 1) {
        const note = el("p", "review-fallback-note", backlink.reason ?? "该锚点对应多个候选 PDF 页面，请选择。");
        const choices = el("div", "review-preview-controls");
        pages.forEach((page) => {
          choices.append(button("打开 PDF 第 " + (page + 1) + " 页", () => this.openPdfPage(backlink.pdf_artifact_id!, page)));
        });
        this.preview.append(note, choices);
        return;
      }
      if (backlink.kind === "candidate_pdf_pages") {
        this.preview.append(el("p", "review-fallback-note", backlink.reason ?? "当前页码是候选定位。"));
      }
      this.openPdfPage(backlink.pdf_artifact_id, pages[0]);
    } catch (error) {
      this.preview.append(el("p", "review-stale", "PDF 回链失败：" + String(error)));
    }
  }

  private openPdfPage(artifactId: string, pageIndex: number): void {
    this.suspend();
    eventBus.emit("open-source", {
      artifactId,
      title: this.meta.title ?? "来源 PDF",
      pdfPageIndex: pageIndex,
    });
  }

  // ------------------------------------------------------------ acceptance

  private toggleUnit(ref: string, checked: boolean): void {
    const review = this.outcome!.review!;
    if (checked) {
      this.selected.add(ref);
      closeSelection(this.selected, review, this.eligible);
    } else {
      this.selected.delete(ref);
      removeDependents(this.selected, review, ref);
    }
    this.syncCheckboxes();
    this.renderFooter();
    this.saveDraft();
  }

  private syncCheckboxes(): void {
    this.list.querySelectorAll<HTMLInputElement>(".review-unit-checkbox").forEach(checkbox => {
      const ref = checkbox.dataset.unit!;
      checkbox.checked = this.selected.has(ref);
      checkbox.disabled = this.accepting || this.accepted || this.meta.review_status==="discarded" || !this.eligible.has(ref) || this.stale !== null;
    });
    this.graph?.setSelected(this.selected);
  }

  private missingDependencies(review: SourceReviewProjection): string[] {
    const units = new Map(review.units.map(unit => [unit.unit_ref, unit]));
    const missing = new Set<string>();
    for (const ref of this.selected) {
      const parent = units.get(ref)?.parent;
      if (parent && !this.selected.has(parent)) missing.add(parent);
    }
    for (const relation of review.relations) {
      if (this.selected.has(relation.source) && !this.selected.has(relation.target)) missing.add(relation.target);
    }
    return [...missing];
  }

  private renderFooter(): void {
    const review = this.outcome?.review;
    if (!review) {
      this.footer.replaceChildren(el("p", "review-note", "该运行没有可接受的来源审核投影。"));
      return;
    }
    const missing = this.missingDependencies(review);
    const blocked = this.accepting || this.accepted || this.meta.review_status==="discarded" || this.stale !== null;
    const state = el("p", "review-selection-state", this.meta.review_status==="discarded"?"结果已放弃，不能接受。请先从发现记录恢复审核。":this.accepted
      ? this.meta.review_status==="withdrawn"?"登记已撤回；当前查看的是原始发现结果。重新采纳请启动新的发现。":"已保存登记；活动状态与历史请从登记回执查看。"
      : missing.length > 0
        ? `已选择 ${this.selected.size} 个单元；仍缺少依赖：${missing.map(ref => this.title(ref)).join("、")}`
        : `已选择 ${this.selected.size} 个单元；父级与来源关系端点闭合。`);
    const selectAll = button("选择全部可接受单元", () => {
      this.selected = new Set(this.eligible);
      closeSelection(this.selected, review, this.eligible);
      this.syncCheckboxes(); this.renderFooter(); this.saveDraft();
    });
    selectAll.disabled = blocked || this.eligible.size === 0;
    const clear = button("清空选择", () => { this.selected.clear(); this.syncCheckboxes(); this.renderFooter(); this.saveDraft(); });
    clear.disabled = blocked || this.selected.size === 0;
    const accept = button("接受选中内容", () => void this.accept(review), "review-accept");
    accept.disabled = blocked || this.selected.size === 0 || missing.length > 0;
    const actions = el("div", "review-footer-actions");
    actions.append(selectAll, clear, accept);
    const previous = this.footer.querySelector(".review-action-feedback")?.textContent ?? "";
    const feedback = el("p", "review-action-feedback", previous);
    feedback.setAttribute("role", "status");
    this.footer.replaceChildren(state, actions, feedback);
  }

  private async accept(review: SourceReviewProjection): Promise<void> {
    const generation = this.generation;
    this.accepting = true;
    this.renderFooter();
    this.footer.querySelector(".review-action-feedback")!.textContent = "后端正在重新绑定原文并验证接受范围…";
    this.syncCheckboxes();
    let message: string;
    try {
      const outcome = await invoke<DiscoveryAcceptanceOutcome>("accept_discovery_result", {
        request: {
          run_id: review.run_id,
          expected_result_fingerprint: review.result_fingerprint,
          selection: { kind: "explicit_units", unit_refs: [...this.selected].sort() },
        },
      });
      if (generation !== this.generation) return;
      if(outcome.state==="registration_review_required"&&outcome.registration_plan) {
        openSourceRegistrationReview(outcome.registration_plan,()=>{
          if(generation!==this.generation)return;
          this.accepted=true;this.meta.review_status="registered";
          review.acceptance.state="registered";this.renderHeader();this.renderFooter();this.syncCheckboxes();
        });
        this.accepting=false;this.renderFooter();this.syncCheckboxes();
        this.footer.querySelector(".review-action-feedback")!.textContent="已接受发现结果；需要审查重新激活或内容替换，尚未登记。关闭后可从发现记录继续。";
        eventBus.emit("discovery-runs-changed");return;
      }
      if(outcome.state==="withdrawn") {
        this.accepted=true;this.meta.review_status="withdrawn";this.accepting=false;this.renderHeader();this.renderFooter();this.syncCheckboxes();
        this.footer.querySelector(".review-action-feedback")!.textContent="这是已撤回登记的历史回执；如需重新采纳，请启动新的发现。";return;
      }
      this.accepted = true;
      review.acceptance.state = "registered";
      review.acceptance.selected_units = outcome.selected_units;
      const registered = new Set(outcome.selected_units);
      review.units.forEach(unit => { if (registered.has(unit.unit_ref)) unit.acceptance = "registered"; });
      this.model!.units.forEach(unit => { if (registered.has(unit.ref)) unit.acceptance = "registered"; });
      this.meta.review_status = "registered";
      try { localStorage.removeItem(this.draftKey()!); } catch { /* Storage may be unavailable. */ }
      message = `${outcome.already_registered ? "已存在相同来源提交" : "审核并注册完成"}：${outcome.unit_count} 个单元、${outcome.span_count} 个来源范围、${outcome.relation_count} 条关系已原子写入来源图；新建 ${outcome.created_object_count}、复用 ${outcome.reused_object_count}${outcome.partial ? "；这是部分发现结果" : ""}。`;
      eventBus.emit("source-map-changed");
      eventBus.emit("discovery-runs-changed");
    } catch (error) {
      if (generation !== this.generation) return;
      message = "接受失败：" + String(error);
    }
    this.accepting = false;
    this.renderHeader();
    this.renderFooter();
    this.footer.querySelector(".review-action-feedback")!.textContent = message;
    if (this.tab === "units" || this.tab === "graph") this.renderList(); else this.syncCheckboxes();
  }
}

function closeSelection(selected: Set<string>, review: SourceReviewProjection, eligible: Set<string>): void {
  const units = new Map(review.units.map(unit => [unit.unit_ref, unit]));
  let changed = true;
  while (changed) {
    changed = false;
    for (const ref of [...selected]) {
      const parent = units.get(ref)?.parent;
      if (parent && eligible.has(parent) && !selected.has(parent)) { selected.add(parent); changed = true; }
    }
    for (const relation of review.relations) {
      if (selected.has(relation.source) && eligible.has(relation.target) && !selected.has(relation.target)) {
        selected.add(relation.target); changed = true;
      }
    }
  }
}

function removeDependents(selected: Set<string>, review: SourceReviewProjection, removed: string): void {
  const removedSet = new Set([removed]);
  let changed = true;
  while (changed) {
    changed = false;
    for (const unit of review.units) {
      if (selected.has(unit.unit_ref) && unit.parent && removedSet.has(unit.parent)) {
        selected.delete(unit.unit_ref); removedSet.add(unit.unit_ref); changed = true;
      }
    }
    for (const relation of review.relations) {
      if (selected.has(relation.source) && removedSet.has(relation.target)) {
        selected.delete(relation.source); removedSet.add(relation.source); changed = true;
      }
    }
  }
}

/** Mounts the single workspace instance and routes open requests to it. */
export function mountReviewWorkspace(host: HTMLElement): DiscoveryReviewWorkspace {
  const workspace = new DiscoveryReviewWorkspace(host);
  eventBus.on("open-review", (detail) => {
    void workspace.open(detail.outcome, detail.meta);
  });
  return workspace;
}
