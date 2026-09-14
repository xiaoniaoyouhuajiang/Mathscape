import { invoke } from "@tauri-apps/api/core";
import { askAdmission, runDiscoveryTask } from "../discovery-task";
import {
  getWorkspaceSettings,
  WORKSPACE_SETTINGS_CHANGED,
  type DiscoveryAdmissionPreference,
  type WorkspaceSettings,
} from "../workspace-settings";

export interface TextRange { start: number; end: number }
export interface MarkdownSnapshot {
  artifact_id: string; conversion_id: string | null; content_hash: string;
  anchor_scheme: string; fingerprint: string;
}
export interface MarkdownBlock { id: string; range: TextRange; kind: string }
export interface AnchoredMarkdown { snapshot: MarkdownSnapshot; source: string; blocks: MarkdownBlock[] }
export type SourceSelector = { kind: "whole_document" } | { kind: "block_range"; first: string; last: string }
  | { kind: "quote_range"; first: string; start_quote: string; last: string; end_quote: string };
export interface SelectionRequest {
  version: "mathscape-discovery-selection-1";
  snapshot: MarkdownSnapshot;
  selector: SourceSelector;
  admission: "review_before_publish" | "automatic_candidate";
}
export type DiscoveryInputSegment = {
  segment_kind: "block"; block_id: string; structural_kind: string;
  physical_range: TextRange; source_range: TextRange; text: string;
} | { segment_kind: "gap"; source_range: TextRange; text: string };
export interface DiscoveryInput {
  version: "mathscape-discovery-input-2";
  snapshot: MarkdownSnapshot;
  selection_fingerprint: string;
  fingerprint: string;
  requested: { source_range: TextRange; segments: DiscoveryInputSegment[] };
  context: { source_range: TextRange; segments: DiscoveryInputSegment[] }[];
  boundary: {
    at_document_start: boolean; at_document_end: boolean;
  };
}

interface ActiveDiscoveryTask {
  artifact_id: string;
  input_fingerprint: string;
  profile_id: string;
  request_timeout_seconds: number;
  phase: string;
  attempt: number;
  batch_index: number | null;
  batch_total: number | null;
  unit_count: number;
  completed_tasks: number;
  total_tasks: number;
  cancellation_requested: boolean;
}

interface DiscoveryAnchorView {
  start: number;
  end: number;
  block_ids: string[];
  excerpt: string;
  purpose?: string;
}

interface LocatedTextView {
  fact_ref: string;
  text: string;
  anchor: DiscoveryAnchorView;
  normalized: boolean;
  precision_review: boolean;
}

interface DiscoveryUnitView {
  unit_ref: string;
  unit_kind: string;
  title: string;
  display_title: string | null;
  parent: string | null;
  completeness: string;
  fact_status: "processed" | "pending" | "requires_unit_revision";
  source_excerpt: string;
  anchors: DiscoveryAnchorView[];
  source_labels: LocatedTextView[];
  source_title: LocatedTextView | null;
}

interface DiscoveryReferenceView {
  owner: string;
  text: string;
  anchor: DiscoveryAnchorView;
  status: "unresolved" | "candidate" | "resolved_local" | "ambiguous";
  candidates: string[];
  reason: string;
  origins: string[];
}

interface DiscoveryRelationView {
  source: string;
  target: string;
  relation: string;
  evidence: DiscoveryAnchorView[];
  bases: string[];
  origins: string[];
}

interface DiscoveryIssueView {
  issue_ref: string;
  stage: "unit_discovery" | "source_facts";
  owner: string | null;
  kind: string;
  observation: string;
  requires_unit_revision: boolean;
  evidence: DiscoveryAnchorView[];
}

interface DiscoveryDiagnosticView {
  report: number;
  reports: number[];
  occurrences: number;
  code: string;
  path: string;
  message: string;
  checked: boolean;
}

interface DesktopDiscoveryView {
  version: "mathscape-desktop-discovery-view-1";
  status: WorkflowStatus;
  complete: boolean;
  source: null | { artifact_id: string; conversion_id: string | null; content_hash: string; input_fingerprint: string };
  units: DiscoveryUnitView[];
  references: DiscoveryReferenceView[];
  relations: DiscoveryRelationView[];
  issues: DiscoveryIssueView[];
  pending_tasks: { task_ref: string; owner: string | null }[];
  diagnostics: DiscoveryDiagnosticView[];
  attempts: { attempt: number; stage: string; repair: boolean; status: WorkflowStatus; error: string | null; total_tokens: number | null; elapsed_ms: number | null }[];
}

interface DiscoveryRun {
  run_id: string;
  profile_id: string;
  provider: string;
  requested_model: string;
  admission: SelectionRequest["admission"];
  request_timeout_seconds: number;
  connect_timeout_seconds: number;
  attempts: number;
  input_tokens: number | null;
  output_tokens: number | null;
  total_tokens: number | null;
  elapsed_ms: number;
}

type WorkflowStatus = "running" | "completed" | "partial" | "needs_attention" | "failed" | "cancelled";
type DiscoveryOutcome = {
  outcome: WorkflowStatus;
  view: DesktopDiscoveryView;
  review: SourceReviewProjection | null;
  run: DiscoveryRun;
};

interface SourceReviewProjection {
  version: "mathscape-source-review-projection-1";
  run_id: string;
  result_fingerprint: string;
  status: WorkflowStatus;
  complete: boolean;
  source: { artifact_id: string; conversion_id: string | null; content_hash: string; input_fingerprint: string };
  units: SourceReviewUnit[];
  references: { reference_ref: string; owner: string; text: string; anchor: DiscoveryAnchorView; status: string; candidates: string[]; reason: string; origins: string[] }[];
  structure_relations: { relation_ref: string; parent: string; child: string }[];
  relations: { relation_ref: string; source: string; target: string; relation: string; evidence: DiscoveryAnchorView[]; bases: string[]; origins: string[] }[];
  diagnostics: { diagnostic_ref: string; severity: string; stage: string; code: string; path: string; message: string; checked: boolean; related_units: string[] }[];
  pending_tasks: { task_ref: string; owner: string | null; reason: string | null }[];
  acceptance: { state: string; selected_units: string[]; required_units: string[]; missing_targets: string[]; blockers: string[]; allowed_scopes: unknown[] };
}

interface DiscoveryAcceptanceOutcome {
  state: "registered";
  decision_id: string;
  run_id: string;
  result_fingerprint: string;
  plan_fingerprint: string;
  selected_units: string[];
  partial: boolean;
  span_count: number;
  unit_count: number;
  reference_count: number;
  relation_count: number;
  pending_task_count: number;
  already_accepted: boolean;
  commit_id: string;
  created_object_count: number;
  reused_object_count: number;
  already_registered: boolean;
}

interface SourceReviewUnit {
  unit_ref: string;
  unit_kind: string;
  display_title: string | null;
  source_title: { text: string; anchor: DiscoveryAnchorView } | null;
  source_labels: { text: string; anchor: DiscoveryAnchorView }[];
  parent: string | null;
  completeness: string;
  fact_status: string;
  acceptance: string;
  anchors: DiscoveryAnchorView[];
  source_excerpt: string;
}

function textElement<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className: string,
  text: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  node.textContent = text;
  return node;
}

export function markdownDocument(id: string, conversion: string | null): Promise<AnchoredMarkdown> {
  return invoke("get_markdown_document", { artifactId: id, conversionId: conversion });
}

/** UI selects only backend-issued blocks. Never derive offsets from KaTeX/DOM text. */
export class MarkdownSelection {
  readonly panel = document.createElement("section");
  private current = -1;
  private first = -1;
  private last = -1;
  private whole = false;
  private version = 0;
  private disposed = false;
  private pending = false;
  private launching = false;
  private feedback: string | null = null;
  private discoveryFeedback: string | null = null;
  private activeDiscovery: ActiveDiscoveryTask | null = null;
  private pollingDiscovery = false;
  private discoveryTimer: number;
  private status = document.createElement("p");
  private result = document.createElement("pre");
  private discoveryStatus = document.createElement("p");
  private discoveryResult = document.createElement("section");
  private anchorPreview = document.createElement("section");
  private discoverySource: DesktopDiscoveryView["source"] = null;
  private policy = document.createElement("select");
  private policyTouched = false;
  private askEachRun = false;
  private policyLabel = document.createElement("span");
  private discoveryTimeoutSeconds = 600;
  private validate = document.createElement("button");
  private discover: HTMLButtonElement;
  private startButton: HTMLButtonElement;
  private endButton: HTMLButtonElement;
  private indexes: Map<string, number>;

  constructor(private data: AnchoredMarkdown, private article: HTMLElement) {
    this.indexes = new Map(data.blocks.map((b, i) => [b.id, i]));
    this.panel.className = "selection-panel";
    this.panel.setAttribute("aria-label", "来源发现选区");
    const instructions = document.createElement("p");
    instructions.textContent = "点击正文块，再设置起点和终点；列表、表格和公式按整块选择。可先检查发现输入，再调用 LLM；有效提案也不会在此处直接入图。";
    const controls = document.createElement("div"); controls.className = "reader-controls";
    const button = (text: string, run: () => void): HTMLButtonElement => {
      const b = document.createElement("button"); b.type = "button"; b.textContent = text;
      b.onclick = run; controls.append(b); return b;
    };
    this.startButton = button("设置起点", () => { this.first = this.current; this.whole = false; this.changed(); });
    this.endButton = button("设置终点", () => { this.last = this.current; this.whole = false; this.changed(); });
    button("选择全文", () => { this.whole = true; this.first = 0; this.last = data.blocks.length - 1; this.changed(); });
    button("清除选择", () => { this.current = this.first = this.last = -1; this.whole = false; this.changed(); });
    this.policy.setAttribute("aria-label", "后续入图方式");
    this.policy.add(new Option("本次运行：请选择审核方式", ""));
    this.policy.add(new Option("后续：审核后入图", "review_before_publish"));
    this.policy.add(new Option("后续：自动加入候选图（未人工审核）", "automatic_candidate"));
    this.policy.value = "review_before_publish";
    this.policy.onchange = () => { this.policyTouched = true; this.changed(); };
    controls.append(this.policyLabel);
    button("修改设置", () => window.dispatchEvent(new Event("mathscape:open-settings")));
    this.validate = button("构建发现输入", () => void this.resolve());
    this.discover = button("使用 LLM 发现内容", () => void this.runDiscovery());
    this.status.setAttribute("role", "status");
    this.discoveryStatus.setAttribute("aria-live", "polite");
    this.discoveryStatus.className = "discovery-status";
    this.result.className = "selection-source";
    this.result.hidden = true;
    this.discoveryResult.className = "discovery-result";
    this.discoveryResult.hidden = true;
    this.anchorPreview.className = "discovery-anchor-preview";
    this.anchorPreview.hidden = true;
    this.panel.append(
      instructions,
      controls,
      this.status,
      this.discoveryStatus,
      this.result,
      this.anchorPreview,
      this.discoveryResult,
    );
    article.addEventListener("click", this.onClick);
    article.addEventListener("keydown", this.onKey);
    window.addEventListener(WORKSPACE_SETTINGS_CHANGED, this.onWorkspaceSettings);
    this.discoveryTimer = window.setInterval(() => void this.syncActiveDiscovery(), 1200);
    void this.syncActiveDiscovery();
    void this.loadWorkspacePolicy();
    this.refresh();
  }

  dispose(): void {
    this.disposed = true; this.version++;
    this.article.removeEventListener("click", this.onClick);
    this.article.removeEventListener("keydown", this.onKey);
    window.removeEventListener(WORKSPACE_SETTINGS_CHANGED, this.onWorkspaceSettings);
    window.clearInterval(this.discoveryTimer);
    this.clearAnchorHighlight();
    this.panel.remove();
  }

  private onClick = (event: Event): void => {
    const block = (event.target as Element).closest<HTMLElement>("[data-source-block]");
    if (!block || !this.article.contains(block)) return;
    this.current = this.indexes.get(block.dataset.sourceBlock ?? "") ?? -1;
    this.refresh();
  };
  private onKey = (event: KeyboardEvent): void => {
    if ((event.key === "Enter" || event.key === " ") && (event.target as HTMLElement).hasAttribute("data-source-block")) {
      event.preventDefault(); this.onClick(event);
    }
  };

  private changed(): void {
    this.version++; this.pending = false; this.feedback = null;
    this.discoveryFeedback = null;
    this.result.hidden = true; this.result.textContent = ""; this.refresh();
    this.discoveryResult.hidden = true; this.discoveryResult.replaceChildren();
    this.discoverySource = null;
    this.anchorPreview.hidden = true; this.anchorPreview.replaceChildren();
    this.clearAnchorHighlight();
  }

  private onWorkspaceSettings = (event: Event): void => {
    this.applyWorkspaceSettings((event as CustomEvent<WorkspaceSettings>).detail);
  };

  private async loadWorkspacePolicy(): Promise<void> {
    try {
      const settings = await getWorkspaceSettings();
      if (!this.disposed) {
        this.discoveryTimeoutSeconds = settings.discovery_timeout_seconds;
        if (!this.policyTouched) {
          this.applyAdmissionPreference(settings.discovery_admission);
        }
      }
    } catch {
      // The safe review-before-publish default remains selected.
    }
  }

  private applyWorkspaceSettings(settings: WorkspaceSettings): void {
    this.discoveryTimeoutSeconds = settings.discovery_timeout_seconds;
    this.applyAdmissionPreference(settings.discovery_admission);
  }

  private applyAdmissionPreference(preference: DiscoveryAdmissionPreference): void {
    this.askEachRun = preference === "ask_each_run";
    this.policy.value = this.askEachRun ? "review_before_publish" : preference;
    this.policyLabel.textContent = this.askEachRun ? "审核方式：运行时询问" : preference === "automatic_candidate" ? "审核方式：自动候选" : "审核方式：人工审核";
    this.policyTouched = false;
    this.refresh();
  }

  refresh(): void {
    const valid = this.first >= 0 && this.last >= this.first && this.data.blocks.length > 0;
    const admissionSelected = this.policy.value === "review_before_publish"
      || this.policy.value === "automatic_candidate";
    this.validate.disabled = !valid || !admissionSelected || this.pending;
    this.discover.disabled = !valid || !admissionSelected || this.pending || this.launching || this.activeDiscovery !== null;
    this.policy.disabled = this.pending || this.activeDiscovery !== null;
    this.startButton.disabled = this.endButton.disabled = this.current < 0;
    this.article.querySelectorAll<HTMLElement>("[data-source-block]").forEach(block => {
      const i = this.indexes.get(block.dataset.sourceBlock ?? "") ?? -1;
      block.classList.toggle("is-current-block", i === this.current);
      block.classList.toggle("is-selected-block", valid && i >= this.first && i <= this.last);
    });
    this.status.textContent = this.feedback ?? (valid
      ? admissionSelected
        ? `已选择${this.whole ? "全文" : `第 ${this.first + 1}～${this.last + 1} 块`}，尚未构建发现输入。`
        : "已选择原文范围；请为本次运行选择审核方式。"
      : this.first >= 0 && this.last >= 0 ? "终点必须位于起点之后，请重新设置。" : "尚未选择完整范围。");
    this.discoveryStatus.textContent = this.activeDiscovery !== null
      ? this.activeDiscoveryText(this.activeDiscovery)
      : this.discoveryFeedback ?? "";
  }

  private activeDiscoveryText(task: ActiveDiscoveryTask): string {
    const phases: Record<string, string> = {
      starting: "正在启动",
      discovering_units: "正在识别数学单元",
      validating_units: "正在验证单元",
      planning_facts: "正在规划来源事实任务",
      discovering_facts: "正在发现标签、引用与关系",
      validating_facts: "正在验证来源事实",
      repairing: "正在进行一次有界修订",
      assembling: "正在组装结果",
      cancelling: "正在取消",
      finishing: "正在结束任务",
    };
    const batch = task.batch_index && task.batch_total ? ` · 批次 ${task.batch_index}/${task.batch_total}` : "";
    const facts = task.total_tasks > 0 ? ` · 事实任务 ${task.completed_tasks}/${task.total_tasks}` : "";
    return `${phases[task.phase] ?? task.phase}${batch}${facts} · ${task.profile_id} · 总期限 ${task.request_timeout_seconds} 秒`;
  }

  private async resolve(): Promise<void> {
    if (this.pending || this.disposed || this.first < 0 || this.last < this.first) return;
    const version = ++this.version;
    const request = this.selectionRequest();
    if (request === null) return;
    this.pending = true; this.feedback = "后端正在核对原文锚点…"; this.refresh();
    try {
      const result = await invoke<DiscoveryInput>("prepare_discovery_input", { request });
      if (this.disposed || version !== this.version) return;
      const range = result.requested.source_range;
      const blockCount = result.requested.segments.filter(segment => segment.segment_kind === "block").length;
      this.feedback = `发现输入已构建：${blockCount} 块、${range.end - range.start} 字节，原文区间 [${range.start}, ${range.end})。尚未启动 LLM 或入图。`;
      this.result.textContent = result.requested.segments.map(segment => segment.text).join("");
      this.result.hidden = false;
    } catch (error) {
      if (!this.disposed && version === this.version) this.feedback = "发现输入构建失败：" + String(error);
    } finally {
      if (!this.disposed && version === this.version) { this.pending = false; this.refresh(); }
    }
  }

  private selectionRequest(): SelectionRequest | null {
    if (this.first < 0 || this.last < this.first || this.data.blocks.length === 0) return null;
    if (this.policy.value !== "review_before_publish" && this.policy.value !== "automatic_candidate") {
      this.feedback = "请先为本次运行选择审核方式。";
      this.refresh();
      return null;
    }
    return {
      version: "mathscape-discovery-selection-1",
      snapshot: this.data.snapshot,
      selector: this.whole
        ? { kind: "whole_document" }
        : {
            kind: "block_range",
            first: this.data.blocks[this.first].id,
            last: this.data.blocks[this.last].id,
          },
      admission: this.policy.value as SelectionRequest["admission"],
    };
  }

  private async syncActiveDiscovery(): Promise<void> {
    if (this.disposed || this.pending || this.pollingDiscovery) return;
    this.pollingDiscovery = true;
    try {
      const active = await invoke<ActiveDiscoveryTask | null>("get_active_discovery");
      if (this.disposed || this.pending) return;
      if (JSON.stringify(active) !== JSON.stringify(this.activeDiscovery)) {
        this.activeDiscovery = active;
        this.refresh();
      }
    } catch {
      // Keep the last known state across transient IPC failures.
    } finally {
      this.pollingDiscovery = false;
    }
  }

  private async runDiscovery(): Promise<void> {
    if (this.pending || this.launching || this.disposed || this.activeDiscovery !== null) return;
    this.launching = true; this.refresh();
    // Read the latest policy even if a settings event was missed.
    try { this.applyWorkspaceSettings(await getWorkspaceSettings()); }
    catch { this.launching = false; this.discoveryFeedback = "无法读取运行设置，请稍后重试。"; this.refresh(); return; }
    if (this.disposed) return;
    if (this.askEachRun) {
      const admission = await askAdmission();
      if (admission === null || this.disposed) { this.launching = false; this.refresh(); return; }
      this.policy.value = admission;
    }
    this.launching = false;
    const selection = this.selectionRequest();
    if (selection === null) return;
    const version = ++this.version;
    this.pending = true;
    this.discoveryFeedback = `LLM 正在分析所选原文并流式返回结果；最长等待 ${this.discoveryTimeoutSeconds} 秒，完整 JSON 验证通过后才会形成待查看提案。`;
    this.discoveryResult.hidden = true;
    this.discoveryResult.replaceChildren();
    this.refresh();
    try {
      const outcome = await runDiscoveryTask<DiscoveryOutcome>({ selection, profile_id: null });
      if (this.disposed || version !== this.version) return;
      this.renderDiscovery(outcome);
    } catch (error) {
      if (!this.disposed && version === this.version) {
        this.discoveryFeedback = "内容发现无法启动：" + String(error);
      }
    } finally {
      if (!this.disposed && version === this.version) {
        this.pending = false;
        this.activeDiscovery = null;
        this.refresh();
        void this.syncActiveDiscovery();
      }
    }
  }

  private renderDiscovery(outcome: DiscoveryOutcome): void {
    this.discoveryResult.replaceChildren();
    this.discoveryResult.hidden = false;
    const view = outcome.view;
    this.discoverySource = view.source;
    const units = view.units;
    const unitTitles = new Map(units.map(unit => [unit.unit_ref, unit.title]));
    const relations = view.relations;
    const references = view.references;
    const pending = view.pending_tasks;
    const statusLabels: Record<WorkflowStatus, string> = {
      running: "运行中", completed: "完成", partial: "部分完成",
      needs_attention: "需要处理", failed: "失败", cancelled: "已取消",
    };
    this.discoveryFeedback = `两阶段内容发现${statusLabels[outcome.outcome]}：${units.length} 个单元、${relations.length} 条关系、${references.length} 个引用。结果尚未接受或入图。`;
    this.discoveryResult.append(
      textElement("h3", "", `分阶段发现 · ${statusLabels[outcome.outcome]}`),
      textElement(
        "p",
        "discovery-summary",
        `${units.length} 个单元 · ${relations.length} 条关系 · ${references.length} 个引用 · ${pending.length} 个待完成任务`,
      ),
    );
    if (units.length > 0) {
      const cards = textElement("div", "discovery-units", "");
      for (const unit of units) {
        const card = textElement("article", "discovery-unit", "");
        const heading = textElement("div", "discovery-unit-heading", "");
        heading.append(
          textElement("strong", "", unit.title),
          this.anchorButtons("定位原文", unit.anchors),
        );
        card.append(
          heading,
          textElement("span", "discovery-kind", `${this.unitKindLabel(unit.unit_kind)} · ${this.completenessLabel(unit.completeness)} · ${this.factStatusLabel(unit.fact_status)}${unit.parent ? ` · 隶属 ${unitTitles.get(unit.parent) ?? unit.parent}` : ""}`),
          textElement("p", "", unit.source_excerpt),
        );
        if (unit.source_labels.length > 0) {
          const labels = textElement("div", "discovery-facts", "");
          unit.source_labels.forEach(label => labels.append(this.factChip("标签", label)));
          card.append(labels);
        }
        if (unit.source_title) card.append(this.factChip("原文标题", unit.source_title));
        cards.append(card);
      }
      this.discoveryResult.append(cards);
    }

    if (references.length > 0) {
      const list = textElement("div", "discovery-reference-list", "");
      for (const reference of references) {
        const item = textElement("article", `discovery-reference reference-${reference.status}`, "");
        const target = reference.candidates.length > 0 ? ` · 候选 ${reference.candidates.map(candidate => unitTitles.get(candidate) ?? candidate).join("、")}` : "";
        item.append(
          textElement("strong", "", `${reference.text} · ${this.referenceStatusLabel(reference.status)}`),
          textElement("p", "", `${unitTitles.get(reference.owner) ?? reference.owner}${target} · ${reference.reason}`),
          this.anchorButton("查看引用原文", reference.anchor),
        );
        list.append(item);
      }
      this.discoveryResult.append(textElement("h4", "", "来源引用"), list);
    }

    if (relations.length > 0) {
      const list = textElement("ul", "discovery-relations", "");
      for (const relation of relations) {
        const item = document.createElement("li");
        item.append(document.createTextNode(`${unitTitles.get(relation.source) ?? relation.source} —${this.relationLabel(relation.relation)}→ ${unitTitles.get(relation.target) ?? relation.target}：${relation.bases.join("；")}`));
        if (relation.evidence.length > 0) item.append(" ", this.anchorButtons("证据", relation.evidence));
        list.append(item);
      }
      this.discoveryResult.append(textElement("h4", "", "来源关系"), list);
    }

    if (view.issues.length > 0) {
      const list = textElement("ul", "discovery-issues", "");
      for (const issue of view.issues) {
        const item = document.createElement("li");
        item.append(document.createTextNode(`${issue.stage === "unit_discovery" ? "单元识别" : "来源事实"} · ${this.issueKindLabel(issue.kind)}${issue.owner ? ` · ${unitTitles.get(issue.owner) ?? issue.owner}` : ""}：${issue.observation}`));
        if (issue.evidence.length > 0) item.append(" ", this.anchorButtons("证据", issue.evidence));
        list.append(item);
      }
      this.discoveryResult.append(textElement("h4", "", "来源问题"), list);
    }

    if (view.diagnostics.length > 0) {
      const list = textElement("ul", "discovery-issues", "");
      view.diagnostics.forEach(item => {
        const repeated = item.occurrences > 1
          ? `（重复 ${item.occurrences} 次，报告 ${item.reports.join("、")}）`
          : "";
        list.append(textElement("li", "", `${item.path} · ${item.code}：${item.message}${repeated}`));
      });
      this.discoveryResult.append(textElement("h4", "", "确定性诊断"), list);
    }
    if (pending.length > 0) {
      this.discoveryResult.append(textElement("p", "discovery-pending", `尚有 ${pending.length} 个事实任务未完成：${pending.map(task => task.owner ?? task.task_ref).join("、")}`));
    }
    if (outcome.review) this.discoveryResult.append(this.renderSourceReview(outcome.review, unitTitles));
    this.discoveryResult.append(this.jsonDetails("查看完整展示数据", view));
    this.discoveryResult.append(this.runSummary(outcome.run));
  }

  private renderSourceReview(review: SourceReviewProjection, unitTitles: Map<string, string>): HTMLElement {
    const section = textElement("section", "source-review-panel", "");
    const heading = textElement("div", "source-review-heading", "");
    heading.append(
      textElement("h4", "", "来源审核视图"),
      textElement("span", `source-review-state state-${review.acceptance.state}`, `审核状态：${review.acceptance.state}`),
    );
    section.append(heading);
    const selected = new Set(review.acceptance.selected_units);
    const eligible = new Set(review.units
      .filter(unit => unit.acceptance !== "blocked")
      .map(unit => unit.unit_ref));
    if (selected.size === 0) eligible.forEach(unit => selected.add(unit));
    const unitByRef = new Map(review.units.map(unit => [unit.unit_ref, unit]));
    const checkboxes = new Map<string, HTMLInputElement>();
    const state = textElement("p", "source-review-selection-state", "");
    const feedback = textElement("p", "source-review-action-feedback", "");
    const accept = textElement("button", "source-review-accept", "接受选中内容");
    accept.type = "button";
    const tree = textElement("div", "source-review-tree", "");
    const depth = (unit: SourceReviewUnit): number => {
      let value = 0;
      let parent = unit.parent;
      const seen = new Set<string>();
      while (parent && !seen.has(parent)) {
        seen.add(parent); value += 1; parent = unitByRef.get(parent)?.parent ?? null;
      }
      return value;
    };
    for (const unit of review.units) {
      const item = textElement("article", `source-review-unit state-${unit.acceptance}`, "");
      item.tabIndex = 0;
      item.title = "悬停或聚焦以高亮对应原文";
      item.onmouseenter = () => unit.anchors[0] && this.focusAnchor(unit.anchors[0]);
      item.onfocus = () => unit.anchors[0] && this.focusAnchor(unit.anchors[0]);
      item.style.setProperty("--source-review-depth", String(depth(unit)));
      const selector = textElement("label", "source-review-selector", "");
      const checkbox = document.createElement("input");
      checkbox.type = "checkbox";
      checkbox.checked = selected.has(unit.unit_ref);
      checkbox.disabled = unit.acceptance === "blocked";
      checkbox.setAttribute("aria-label", `选择来源单元 ${unit.display_title ?? unit.unit_ref}`);
      checkboxes.set(unit.unit_ref, checkbox);
      selector.append(checkbox, document.createTextNode(" 纳入来源登记计划"));
      const parent = unit.parent ? ` · 隶属 ${unitTitles.get(unit.parent) ?? unit.parent}` : "";
      item.append(
        selector,
        textElement("strong", "", unit.display_title ?? unit.unit_ref),
        textElement("span", "source-review-meta", `${this.unitKindLabel(unit.unit_kind)} · ${unit.fact_status} · ${unit.acceptance}${parent}`),
        textElement("p", "", unit.source_excerpt),
      );
      checkbox.onchange = () => {
        if (checkbox.checked) {
          selected.add(unit.unit_ref);
          this.closeSourceReviewSelection(selected, review, eligible);
        } else {
          selected.delete(unit.unit_ref);
          this.removeSourceReviewDependents(selected, review, unit.unit_ref);
        }
        updateSelection();
      };
      tree.append(item);
    }
    section.append(state, tree);
    if (review.relations.length > 0) {
      const relations = textElement("ul", "source-review-relations", "");
      for (const relation of review.relations) {
        const item = document.createElement("li");
        item.onmouseenter = () => relation.evidence[0] && this.focusAnchor(relation.evidence[0]);
        item.append(document.createTextNode(`${unitTitles.get(relation.source) ?? relation.source} —${this.relationLabel(relation.relation)}→ ${unitTitles.get(relation.target) ?? relation.target}`));
        relations.append(item);
      }
      section.append(textElement("h5", "", "可审核来源关系"), relations);
    }
    if (review.acceptance.blockers.length > 0) {
      section.append(textElement("p", "source-review-blocked", `以下单元需修订，未被默认选中：${review.acceptance.blockers.join("、")}`));
    }
    const actions = textElement("div", "source-review-actions", "");
    const selectAll = textElement("button", "", "选择全部可接受单元");
    selectAll.type = "button";
    selectAll.onclick = () => {
      selected.clear(); eligible.forEach(unit => selected.add(unit));
      this.closeSourceReviewSelection(selected, review, eligible);
      updateSelection();
    };
    const clear = textElement("button", "", "清空选择");
    clear.type = "button";
    clear.onclick = () => { selected.clear(); updateSelection(); };
    actions.append(selectAll, clear, accept);
    section.append(actions, feedback);

    let accepting = false;
    let accepted = review.acceptance.state === "accepted" || review.acceptance.state === "registered";
    const missingDependencies = (): string[] => {
      const missing = new Set<string>();
      for (const ref of selected) {
        const parent = unitByRef.get(ref)?.parent;
        if (parent && !selected.has(parent)) missing.add(parent);
      }
      for (const relation of review.relations) {
        if (selected.has(relation.source) && !selected.has(relation.target)) {
          missing.add(relation.target);
        }
      }
      return [...missing];
    };
    const updateSelection = (): void => {
      checkboxes.forEach((checkbox, ref) => {
        checkbox.checked = selected.has(ref);
        checkbox.disabled = accepting || accepted || !eligible.has(ref);
      });
      const missing = missingDependencies();
      state.textContent = missing.length > 0
        ? `已选择 ${selected.size} 个单元；仍缺少依赖：${missing.join("、")}`
        : `已选择 ${selected.size} 个单元；父级与来源关系端点闭合。`;
      accept.disabled = accepting || accepted || selected.size === 0 || missing.length > 0;
      selectAll.disabled = accepting || accepted || eligible.size === 0;
      clear.disabled = accepting || accepted || selected.size === 0;
    };
    accept.onclick = async () => {
      if (accept.disabled) return;
      accepting = true; feedback.textContent = "后端正在重新绑定原文并验证接受范围…"; updateSelection();
      try {
        const outcome = await invoke<DiscoveryAcceptanceOutcome>("accept_discovery_result", {
          request: {
            run_id: review.run_id,
            expected_result_fingerprint: review.result_fingerprint,
            selection: { kind: "explicit_units", unit_refs: [...selected].sort() },
          },
        });
        accepted = true;
        review.acceptance.state = "registered";
        review.acceptance.selected_units = outcome.selected_units;
        outcome.selected_units.forEach(ref => {
          const unit = unitByRef.get(ref); if (unit) unit.acceptance = "registered";
        });
        heading.querySelector(".source-review-state")!.textContent = "审核状态：registered";
        feedback.textContent = `${outcome.already_registered ? "已存在相同来源提交" : "审核并注册完成"}：${outcome.unit_count} 个单元、${outcome.span_count} 个来源范围、${outcome.relation_count} 条关系已原子写入来源图；新建 ${outcome.created_object_count}、复用 ${outcome.reused_object_count}${outcome.partial ? "；这是部分发现结果" : ""}。`;
        this.discoveryFeedback = `内容发现结果已注册：${outcome.unit_count} 个来源单元已写入正式来源图。`;
        section.classList.add("is-accepted");
        window.dispatchEvent(new Event("mathscape:source-map-changed"));
      } catch (error) {
        feedback.textContent = "接受失败：" + String(error);
      } finally {
        accepting = false; updateSelection(); this.refresh();
      }
    };
    this.closeSourceReviewSelection(selected, review, eligible);
    updateSelection();
    return section;
  }

  private closeSourceReviewSelection(selected: Set<string>, review: SourceReviewProjection, eligible: Set<string>): void {
    const units = new Map(review.units.map(unit => [unit.unit_ref, unit]));
    let changed = true;
    while (changed) {
      changed = false;
      for (const ref of [...selected]) {
        const parent = units.get(ref)?.parent;
        if (parent && eligible.has(parent) && !selected.has(parent)) {
          selected.add(parent); changed = true;
        }
      }
      for (const relation of review.relations) {
        if (selected.has(relation.source) && eligible.has(relation.target) && !selected.has(relation.target)) {
          selected.add(relation.target); changed = true;
        }
      }
    }
  }

  private removeSourceReviewDependents(selected: Set<string>, review: SourceReviewProjection, removed: string): void {
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

  private factChip(kind: string, fact: LocatedTextView): HTMLElement {
    const chip = textElement("span", "discovery-fact", `${kind}：${fact.text}`);
    if (fact.normalized) chip.title = "定位经过安全排版规范化";
    if (fact.precision_review) chip.classList.add("needs-precision-review");
    chip.append(" ", this.anchorButton("定位", fact.anchor));
    return chip;
  }

  private unitKindLabel(kind: string): string {
    return ({ section: "章节", definition: "定义", statement: "命题", proof: "证明", problem: "习题", hint: "提示", solution: "解答", example: "例子", remark: "注记", method: "方法", notation: "记号", narrative: "叙述" } as Record<string, string>)[kind] ?? kind;
  }

  private completenessLabel(value: string): string {
    return ({ complete: "内容完整", left_truncated: "左侧截断", right_truncated: "右侧截断", internally_missing: "内部缺失", uncertain: "边界待确认" } as Record<string, string>)[value] ?? value;
  }

  private factStatusLabel(value: DiscoveryUnitView["fact_status"]): string {
    return ({ processed: "来源事实已验证", pending: "来源事实待发现", requires_unit_revision: "需要修订单元" } as Record<string, string>)[value] ?? value;
  }

  private referenceStatusLabel(value: DiscoveryReferenceView["status"]): string {
    return ({ unresolved: "未解析", candidate: "存在候选", resolved_local: "已解析到本地单元", ambiguous: "存在歧义" } as Record<string, string>)[value] ?? value;
  }

  private relationLabel(value: string): string {
    return ({ proves: "证明", solves: "解答", hints: "提示", explains: "解释", cites: "引用", applies_method: "应用方法" } as Record<string, string>)[value] ?? value;
  }

  private issueKindLabel(value: string): string {
    return ({ left_truncated: "左侧截断", right_truncated: "右侧截断", internally_missing: "内部缺失", uncertain_boundary: "边界待确认", uncertain_parent: "父级待确认", ambiguous_reference: "引用有歧义", suspected_conversion_error: "疑似转换错误", missing_context: "缺少上下文", uncertain_classification: "分类待确认" } as Record<string, string>)[value] ?? value;
  }

  private anchorButton(label: string, anchor: DiscoveryAnchorView | undefined): HTMLButtonElement {
    const button = textElement("button", "discovery-anchor-button", label);
    button.type = "button";
    button.disabled = anchor === undefined;
    if (anchor) button.onclick = () => this.focusAnchor(anchor);
    return button;
  }

  private anchorButtons(label: string, anchors: DiscoveryAnchorView[]): HTMLElement {
    const actions = textElement("span", "discovery-anchor-actions", "");
    anchors.forEach((anchor, index) => actions.append(
      this.anchorButton(anchors.length === 1 ? label : `${label} ${index + 1}`, anchor),
    ));
    return actions;
  }

  private focusAnchor(anchor: DiscoveryAnchorView): void {
    const expectedConversion = this.data.snapshot.conversion_id;
    if (this.discoverySource === null
      || this.discoverySource.artifact_id !== this.data.snapshot.artifact_id
      || this.discoverySource.conversion_id !== expectedConversion
      || this.discoverySource.content_hash !== this.data.snapshot.content_hash) {
      this.discoveryFeedback = "发现结果不属于当前 Markdown 快照，拒绝应用来源锚点。";
      this.refresh();
      return;
    }
    const source = new TextEncoder().encode(this.data.source);
    if (!Number.isSafeInteger(anchor.start) || !Number.isSafeInteger(anchor.end)
      || anchor.start < 0 || anchor.end <= anchor.start || anchor.end > source.length) {
      this.discoveryFeedback = "来源锚点已经失效，无法定位到当前 Markdown 快照。";
      this.refresh();
      return;
    }
    const blocks = this.data.blocks.filter(block => block.range.start < anchor.end && block.range.end > anchor.start);
    if (blocks.length === 0) {
      this.discoveryFeedback = "来源锚点不属于当前 Markdown 选区。";
      this.refresh();
      return;
    }
    this.clearAnchorHighlight();
    blocks.forEach(block => this.article.querySelector<HTMLElement>(`[data-source-block="${CSS.escape(block.id)}"]`)?.classList.add("is-discovery-anchor-block"));
    const first = this.article.querySelector<HTMLElement>(`[data-source-block="${CSS.escape(blocks[0].id)}"]`);
    first?.scrollIntoView({ behavior: "smooth", block: "center" });

    const decoder = new TextDecoder("utf-8", { fatal: true });
    const contextStart = blocks[0].range.start;
    const contextEnd = blocks[blocks.length - 1].range.end;
    const before = decoder.decode(source.subarray(contextStart, anchor.start));
    const selected = decoder.decode(source.subarray(anchor.start, anchor.end));
    const after = decoder.decode(source.subarray(anchor.end, contextEnd));
    const pre = document.createElement("pre");
    pre.append(
      document.createTextNode(this.tail(before, 240)),
      Object.assign(document.createElement("mark"), { textContent: selected }),
      document.createTextNode(this.head(after, 240)),
    );
    this.anchorPreview.replaceChildren(
      textElement("strong", "", `Markdown 原文范围 [${anchor.start}, ${anchor.end})`),
      pre,
    );
    this.anchorPreview.hidden = false;
  }

  private clearAnchorHighlight(): void {
    this.article.querySelectorAll(".is-discovery-anchor-block").forEach(block => block.classList.remove("is-discovery-anchor-block"));
  }

  private head(value: string, count: number): string {
    return [...value].slice(0, count).join("");
  }

  private tail(value: string, count: number): string {
    const characters = [...value];
    return characters.slice(Math.max(0, characters.length - count)).join("");
  }

  private jsonDetails(label: string, value: unknown): HTMLDetailsElement {
    const details = document.createElement("details");
    details.className = "discovery-json";
    details.append(
      textElement("summary", "", label),
      textElement("pre", "", JSON.stringify(value, null, 2)),
    );
    return details;
  }

  private runSummary(run: DiscoveryRun): HTMLElement {
    const usage = run.total_tokens === null || run.total_tokens === undefined
      ? "token 数未知"
      : `${run.total_tokens} tokens`;
    const admission = run.admission === "automatic_candidate" ? "自动候选策略（尚未登记）" : "等待人工审核";
    return textElement(
      "p",
      "discovery-run",
      `${run.profile_id} · ${run.requested_model} · ${run.attempts} 次调用 · ${usage} · ${run.elapsed_ms} ms · 总期限 ${run.request_timeout_seconds} 秒 · ${admission}`,
    );
  }
}
