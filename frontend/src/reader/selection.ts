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
}

interface DiscoveryUnit {
  temp_id: string;
  unit_kind: string;
  source_label: string | null;
  source_title: string | null;
  display_title: string | null;
  selectors: unknown[];
  parent: string | null;
  completeness: string;
  source_excerpt: string;
}

interface DiscoveryRelation {
  source: string;
  target: string;
  relation: string;
  evidence: unknown[];
  basis: string;
}

interface DiscoveryIssue {
  kind: string;
  unit: string | null;
  evidence: unknown[];
  observation: string;
  context_request: unknown | null;
}

interface DiscoveryProposalView {
  status: "complete" | "partial";
  input_fingerprint: string;
  units: DiscoveryUnit[];
  relations: DiscoveryRelation[];
  references: unknown[];
  issues: DiscoveryIssue[];
  coverage: unknown[];
}

interface DiscoveryRun {
  profile_id: string;
  provider: string;
  requested_model: string;
  admission: SelectionRequest["admission"];
  request_timeout_seconds: number;
  connect_timeout_seconds: number;
  response: null | {
    provider_model: string;
    finish_reason: string | null;
    input_tokens: number | null;
    output_tokens: number | null;
    total_tokens: number | null;
    elapsed_ms: number;
  };
}

type DiscoveryOutcome =
  | { outcome: "valid"; proposal: DiscoveryProposalView; run: DiscoveryRun }
  | {
      outcome: "invalid_proposal";
      proposal: unknown | null;
      rejection_kind: string;
      message: string;
      diagnostics: { code: string; path: string; message: string }[];
      run: DiscoveryRun;
    }
  | { outcome: "failed"; error_kind: string; message: string; run: DiscoveryRun };

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
    this.panel.append(
      instructions,
      controls,
      this.status,
      this.discoveryStatus,
      this.result,
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
      ? `内容发现正在流式接收：${this.activeDiscovery.profile_id} · 最长等待 ${this.activeDiscovery.request_timeout_seconds} 秒`
      : this.discoveryFeedback ?? "";
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
    if (outcome.outcome === "valid") {
      const proposal = outcome.proposal;
      this.discoveryFeedback = `内容发现完成：${proposal.units.length} 个单元、${proposal.relations.length} 条关系，状态 ${proposal.status}。结果尚未接受或入图。`;
      this.discoveryResult.append(
        textElement("h3", "", "发现提案"),
        textElement(
          "p",
          "discovery-summary",
          `${proposal.units.length} 个单元 · ${proposal.relations.length} 条关系 · ${proposal.references.length} 个引用 · ${proposal.issues.length} 个问题`,
        ),
      );
      const units = textElement("div", "discovery-units", "");
      for (const unit of proposal.units) {
        const title = unit.source_label ?? unit.source_title ?? unit.display_title ?? unit.temp_id;
        const card = textElement("article", "discovery-unit", "");
        card.append(
          textElement("strong", "", title),
          textElement(
            "span",
            "discovery-kind",
            `${unit.unit_kind} · ${unit.completeness}${unit.parent ? ` · parent ${unit.parent}` : ""}`,
          ),
          textElement("p", "", unit.source_excerpt),
        );
        units.append(card);
      }
      this.discoveryResult.append(units);
      if (proposal.relations.length > 0) {
        const relations = textElement("ul", "discovery-relations", "");
        for (const relation of proposal.relations) {
          relations.append(
            textElement(
              "li",
              "",
              `${relation.source} —${relation.relation}→ ${relation.target}：${relation.basis}`,
            ),
          );
        }
        this.discoveryResult.append(textElement("h4", "", "原文明示关系"), relations);
      }
      if (proposal.issues.length > 0) {
        const issues = textElement("ul", "discovery-issues", "");
        proposal.issues.forEach(issue => issues.append(
          textElement("li", "", `${issue.kind}${issue.unit ? ` · ${issue.unit}` : ""}：${issue.observation}`),
        ));
        this.discoveryResult.append(textElement("h4", "", "待处理问题"), issues);
      }
      this.discoveryResult.append(this.jsonDetails("查看完整发现结果", proposal));
    } else if (outcome.outcome === "invalid_proposal") {
      this.discoveryFeedback = `模型已返回，但提案被拒绝：${outcome.message}`;
      this.discoveryResult.append(
        textElement("h3", "", `提案未通过 · ${outcome.rejection_kind}`),
      );
      if (outcome.diagnostics.length > 0) {
        const diagnostics = textElement("ul", "discovery-issues", "");
        outcome.diagnostics.forEach(item => diagnostics.append(
          textElement("li", "", `${item.path} · ${item.code}：${item.message}`),
        ));
        this.discoveryResult.append(diagnostics);
      }
      if (outcome.proposal !== null) {
        this.discoveryResult.append(this.jsonDetails("查看模型提案", outcome.proposal));
      }
    } else {
      this.discoveryFeedback = `LLM 调用失败（${outcome.error_kind}）：${outcome.message}`;
      this.discoveryResult.append(textElement("h3", "", "内容发现调用失败"));
    }
    this.discoveryResult.append(this.runSummary(outcome.run));
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
    const response = run.response;
    const model = response?.provider_model ?? run.requested_model;
    const usage = response?.total_tokens === null || response?.total_tokens === undefined
      ? "token 数未知"
      : `${response.total_tokens} tokens`;
    const elapsed = response === null ? "未收到响应" : `${response.elapsed_ms} ms`;
    return textElement(
      "p",
      "discovery-run",
      `${run.profile_id} · ${model} · ${usage} · ${elapsed} · 总超时 ${run.request_timeout_seconds} 秒`,
    );
  }
}
