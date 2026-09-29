import { invoke } from "@tauri-apps/api/core";
import { askAdmission, resumeDiscoveryTask, runDiscoveryTask } from "../discovery-task";
import { markdownPatchSource, markdownRenderer } from "./markdown";
import { openReviewWorkspace } from "../review/workspace";
import { workflowStatusLabel } from "../review/labels";
import type { DiscoveryOutcome } from "../review/types";
import {
  getWorkspaceSettings,
  WORKSPACE_SETTINGS_CHANGED,
  type DiscoveryAdmissionPreference,
  type WorkspaceSettings,
} from "../workspace-settings";
import { eventBus } from "../core/events";

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

interface MarkdownPatchProposalView {
  patch_ref: string;
  issue_ref: string;
  range: TextRange;
  old_text: string;
  old_text_hash: string;
  replacement: string;
  replacement_hash: string;
  rationale: string;
  status: string;
}

interface MarkdownPatchSetView {
  version: string;
  run_id: string;
  base_snapshot: MarkdownSnapshot;
  base_input_fingerprint: string;
  patches: MarkdownPatchProposalView[];
  fingerprint: string;
}

interface MarkdownPatchReviewView {
  run_id: string;
  patch_set_id: string | null;
  input_fingerprint: string;
  status: string;
  patch_set: MarkdownPatchSetView | null;
  provider_model: string | null;
  message: string | null;
}

interface MarkdownPatchSafetyView {
  status: string;
  diagnostics: { code: string; path: string; message: string }[];
}

interface MarkdownPatchPreviewView {
  run_id: string;
  patch_set_id: string;
  patch_set_fingerprint: string;
  original: string;
  revised: string;
  safety: MarkdownPatchSafetyView;
}

interface MarkdownPatchReceiptView {
  status: string;
  patched_artifact_id: string | null;
  conversion_id: string | null;
  safety: MarkdownPatchSafetyView;
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

function markdownPatchStatusLabel(status: string): string {
  return ({
    awaiting_patch_review: "等待审核修订建议",
    no_patchable_issue: "没有可修订的转换异常",
    no_patch_proposed: "模型没有提出可验证修订",
    proposed: "等待审核修订建议",
    accepted: "修订已接受",
    applied: "修订已应用",
    rejected: "修订已拒绝",
    stale: "修订建议已过期",
    failed: "修订应用失败",
  } as Record<string, string>)[status] ?? status;
}

export type PatchReviewMode = "manual" | "workflow";

export function openDiscoveryPatchReview(
  runId: string,
  artifactId: string,
  conversionId: string | null,
  title: string,
): void {
  eventBus.emit("open-source", {
    artifactId,
    conversionId,
    title,
    patchRunId: runId,
    patchMode: "workflow",
  });
}

const DECIDED_PATCH_STATUSES = new Set(["accepted", "applied", "rejected"]);

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
  // Survives selection changes: a paused run is not tied to the current range.
  private patchReviewHost = document.createElement("section");
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
  private unsubscribeSettings?: () => void;

  constructor(private data: AnchoredMarkdown, private article: HTMLElement, patch?: { runId: string; mode: PatchReviewMode }) {
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
    button("修改设置", () => eventBus.emit("open-settings"));
    this.validate = button("构建发现输入", () => void this.resolve());
    this.discover = button("使用 LLM 发现内容", () => void this.runDiscovery());
    this.status.setAttribute("role", "status");
    this.discoveryStatus.setAttribute("aria-live", "polite");
    this.discoveryStatus.className = "discovery-status";
    this.result.className = "selection-source";
    this.result.hidden = true;
    this.discoveryResult.className = "discovery-result";
    this.discoveryResult.hidden = true;
    this.patchReviewHost.className = "markdown-patch-host";
    this.patchReviewHost.hidden = true;
    this.panel.append(
      instructions,
      controls,
      this.status,
      this.discoveryStatus,
      this.result,
      this.patchReviewHost,
      this.discoveryResult,
    );
    article.addEventListener("click", this.onClick);
    article.addEventListener("keydown", this.onKey);
    this.unsubscribeSettings = eventBus.on("workspace-settings-changed", (settings) => {
      this.onWorkspaceSettings(new CustomEvent(WORKSPACE_SETTINGS_CHANGED, { detail: settings }));
    });
    this.discoveryTimer = window.setInterval(() => void this.syncActiveDiscovery(), 1200);
    void this.syncActiveDiscovery();
    void this.loadWorkspacePolicy();
    this.refresh();
    if (patch?.mode === "manual") void this.openSidecarPatchReview(patch.runId);
    else if (patch) void this.openWorkflowPatchReview(patch.runId);
    else void this.loadPendingPatchReview();
  }

  private async openSidecarPatchReview(runId: string): Promise<void> {
    try {
      const review = await invoke<MarkdownPatchReviewView>("get_markdown_patch_review", { runId });
      if (this.disposed) return;
      this.patchReviewHost.hidden = false;
      this.patchReviewHost.replaceChildren(textElement("h3", "", "Markdown 转换校准"));
      if (review.patch_set && review.patch_set_id && review.status === "proposed") this.renderMarkdownPatchReview(review, "manual");
      else this.patchReviewHost.append(textElement("p", "markdown-patch-review-status", review.message ?? markdownPatchStatusLabel(review.status)));
    } catch (error) {
      if (!this.disposed) {
        this.patchReviewHost.hidden = false;
        this.patchReviewHost.replaceChildren(textElement("p", "markdown-patch-review-status", `无法读取修订建议：${String(error)}`));
      }
    }
  }

  private async loadPendingPatchReview(): Promise<void> {
    try {
      const review = await invoke<MarkdownPatchReviewView | null>("get_pending_markdown_patch_review", {
        artifactId: this.data.snapshot.artifact_id,
      });
      if (this.disposed || review === null || !this.patchReviewHost.hidden) return;
      this.showWorkflowPatchReview(review, "有一次内容发现已固定单元，正在等待审核 Markdown 修订。");
    } catch {
      // Recovery is best effort; the run stays paused in SQLite.
    }
  }

  private async openWorkflowPatchReview(runId: string): Promise<void> {
    try {
      const review = await invoke<MarkdownPatchReviewView>("get_markdown_patch_review", { runId });
      if (!this.disposed) this.showWorkflowPatchReview(review, "单元识别完成；在进入来源事实发现之前，请审核 Markdown 修订。");
    } catch (error) {
      if (!this.disposed) {
        this.patchReviewHost.hidden = false;
        this.patchReviewHost.replaceChildren(textElement("p", "markdown-patch-review-status", `无法读取待审核修订：${String(error)}`));
      }
    }
  }

  private showWorkflowPatchReview(review: MarkdownPatchReviewView, intro: string): void {
    this.patchReviewHost.hidden = false;
    this.patchReviewHost.replaceChildren(
      textElement("h3", "", "内容发现已暂停 · 等待审核 Markdown 修订"),
      textElement("p", "discovery-summary", intro),
    );
    if (review.patch_set && review.patch_set_id && review.status === "proposed") {
      this.renderMarkdownPatchReview(review, "workflow");
      return;
    }
    const status = textElement("p", "markdown-patch-review-status", review.message ?? markdownPatchStatusLabel(review.status));
    this.patchReviewHost.append(status);
    if (DECIDED_PATCH_STATUSES.has(review.status)) {
      const actions = textElement("div", "markdown-patch-actions", "");
      const resume = textElement("button", "", "按已记录的决定继续内容发现");
      resume.type = "button";
      resume.onclick = () => { resume.disabled = true; void this.resumeAfterPatch(review.run_id, status); };
      actions.append(resume);
      this.patchReviewHost.append(actions);
    }
  }

  private async resumeAfterPatch(runId: string, status: HTMLElement): Promise<void> {
    if (this.pending || this.activeDiscovery !== null) {
      status.textContent = "已有内容发现任务正在运行，请稍后继续。";
      return;
    }
    this.pending = true;
    this.discoveryFeedback = "正在按审核决定继续内容发现；等待审核的时间不计入总期限。";
    status.textContent = "正在继续内容发现…";
    this.refresh();
    try {
      const outcome = await resumeDiscoveryTask<DiscoveryOutcome>(runId);
      eventBus.emit("discovery-runs-changed");
      const source = outcome.view.source;
      // A run finished on the patched artifact introduces new map objects.
      if (source && source.content_hash !== this.data.snapshot.content_hash) {
        eventBus.emit("source-map-changed");
      }
      if (!this.disposed) {
        this.patchReviewHost.hidden = true;
        this.patchReviewHost.replaceChildren();
      }
      this.renderDiscovery(outcome);
    } catch (error) {
      if (!this.disposed) {
        status.textContent = `继续内容发现失败：${String(error)}`;
        this.discoveryFeedback = "继续内容发现失败：" + String(error);
        const retry = textElement("button", "discovery-anchor-button", "重试继续发现");
        retry.type = "button";
        retry.onclick = () => { retry.remove(); void this.resumeAfterPatch(runId, status); };
        status.after(retry);
      }
    } finally {
      if (!this.disposed) {
        this.pending = false;
        this.activeDiscovery = null;
        this.refresh();
        void this.syncActiveDiscovery();
      }
    }
  }

  dispose(): void {
    this.disposed = true; this.version++;
    this.article.removeEventListener("click", this.onClick);
    this.article.removeEventListener("keydown", this.onKey);
    this.unsubscribeSettings?.();
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
      ? this.activeDiscoveryText(this.activeDiscovery)
      : this.discoveryFeedback ?? "";
  }

  private activeDiscoveryText(task: ActiveDiscoveryTask): string {
    const phases: Record<string, string> = {
      starting: "正在启动",
      discovering_units: "正在识别数学单元",
      validating_units: "正在验证单元",
      reviewing_patches: "正在生成 Markdown 修订建议",
      awaiting_patch_review: "等待审核 Markdown 修订",
      applying_patches: "正在应用 Markdown 修订",
      rebuilding_units: "正在按修订后原文重新识别单元",
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
      eventBus.emit("discovery-runs-changed");
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
    const view = outcome.view;
    if (outcome.outcome === "awaiting_patch_review") {
      this.discoveryFeedback = `单元识别完成：${view.units.length} 个单元已固定；内容发现已暂停，审核 Markdown 修订后继续来源事实发现。`;
      if (outcome.run.run_id) void this.openWorkflowPatchReview(outcome.run.run_id);
      return;
    }
    this.discoveryFeedback = `两阶段内容发现${workflowStatusLabel(outcome.outcome)}：${view.units.length} 个单元、${view.relations.length} 条关系、${view.references.length} 个引用。结果尚未接受或入图，已在审核工作区打开。`;
    const open = textElement("button", "discovery-anchor-button", "打开审核工作区");
    open.type = "button";
    open.onclick = () => openReviewWorkspace(outcome);
    this.discoveryResult.replaceChildren(
      textElement("p", "discovery-summary", `分阶段发现 · ${workflowStatusLabel(outcome.outcome)} · ${view.units.length} 个单元 · ${view.pending_tasks.length} 个待完成任务`),
      open,
    );
    this.discoveryResult.hidden = false;
    openReviewWorkspace(outcome);
  }

  /**
   * The patch's enclosing physical block in the base Markdown, split around
   * the patched bytes. Null when the reader shows a different snapshot.
   */
  private patchBlockContext(patchSet: MarkdownPatchSetView, patch: MarkdownPatchProposalView): { before: string; after: string } | null {
    const base = patchSet.base_snapshot;
    const snapshot = this.data.snapshot;
    if (base.artifact_id !== snapshot.artifact_id || base.content_hash !== snapshot.content_hash) return null;
    const block = this.data.blocks.find(candidate => candidate.range.start <= patch.range.start && patch.range.end <= candidate.range.end);
    if (!block) return null;
    const bytes = new TextEncoder().encode(this.data.source);
    const decoder = new TextDecoder("utf-8", { fatal: true });
    try {
      if (decoder.decode(bytes.subarray(patch.range.start, patch.range.end)) !== patch.old_text) return null;
      return {
        before: decoder.decode(bytes.subarray(block.range.start, patch.range.start)),
        after: decoder.decode(bytes.subarray(patch.range.end, block.range.end)),
      };
    } catch {
      return null;
    }
  }

  private patchComparison(patchSet: MarkdownPatchSetView, patch: MarkdownPatchProposalView, replacement: HTMLTextAreaElement): HTMLElement {
    const context = this.patchBlockContext(patchSet, patch) ?? { before: "", after: "" };
    const md = markdownRenderer();
    const compare = textElement("div", "markdown-patch-compare", "");
    const pane = (label: string): HTMLElement => {
      const column = textElement("div", "markdown-patch-pane", "");
      const body = textElement("div", "markdown-body", "");
      column.append(textElement("span", "markdown-patch-pane-label", label), body);
      compare.append(column);
      return body;
    };
    const original = pane("原文渲染");
    const revised = pane("修订后渲染");
    const inline = textElement("pre", "markdown-patch-inline", "");
    const render = (): void => {
      // Keep the complete enclosing block so Markdown and KaTeX retain their
      // original context, but inject renderer-only anchors around the changed
      // fragment. The renderer turns those anchors into old/new highlights.
      original.innerHTML = md.render(markdownPatchSource(context.before, patch.old_text, context.after, "old"));
      revised.innerHTML = md.render(markdownPatchSource(context.before, replacement.value, context.after, "new"));
      inline.replaceChildren(
        document.createTextNode(this.tail(context.before, 80)),
        textElement("del", "", patch.old_text),
        textElement("ins", "", replacement.value),
        document.createTextNode(this.head(context.after, 80)),
      );
    };
    replacement.addEventListener("input", render);
    render();
    const wrapper = textElement("div", "markdown-patch-comparison", "");
    wrapper.append(inline, compare);
    return wrapper;
  }

  private renderMarkdownPatchReview(review: MarkdownPatchReviewView, mode: "manual" | "workflow" = "manual"): void {
    const patchSet = review.patch_set;
    if (!patchSet || !review.patch_set_id) return;
    const workflow = mode === "workflow";
    const section = textElement("section", "markdown-patch-review", "");
    const heading = textElement("div", "markdown-patch-heading", "");
    heading.append(
      textElement("h4", "", "Markdown 修订审核"),
      textElement("span", "markdown-patch-meta", `${patchSet.patches.length} 条建议 · 指纹 ${patchSet.fingerprint}`),
    );
    section.append(
      heading,
      textElement("p", "markdown-patch-help", "逐项确认模型建议。这里不会证明数学正确性；只校验原文定位、字节范围和 Markdown 结构是否保持。"),
    );
    const rows = textElement("div", "markdown-patch-list", "");
    const replacements = new Map<string, HTMLTextAreaElement>();
    const selected = new Map<string, HTMLInputElement>();
    for (const patch of patchSet.patches) {
      const row = textElement("article", "markdown-patch-item", "");
      const top = textElement("div", "markdown-patch-item-heading", "");
      const checkbox = document.createElement("input");
      checkbox.type = "checkbox";
      checkbox.checked = true;
      checkbox.setAttribute("aria-label", `接受 ${patch.patch_ref}`);
      selected.set(patch.patch_ref, checkbox);
      top.append(
        checkbox,
        textElement("strong", "", `${patch.patch_ref} · ${patch.issue_ref}`),
        textElement("span", "markdown-patch-range", `[${patch.range.start}, ${patch.range.end})`),
      );
      const old = textElement("pre", "markdown-patch-old", patch.old_text);
      const textarea = document.createElement("textarea");
      textarea.className = "markdown-patch-replacement";
      textarea.value = patch.replacement;
      textarea.rows = Math.max(2, Math.min(8, patch.replacement.split("\n").length + 1));
      textarea.setAttribute("aria-label", `${patch.patch_ref} replacement`);
      replacements.set(patch.patch_ref, textarea);
      row.append(
        top,
        textElement("span", "markdown-patch-rationale", patch.rationale),
        old,
        textarea,
        this.patchComparison(patchSet, patch, textarea),
      );
      rows.append(row);
    }
    const actions = textElement("div", "markdown-patch-actions", "");
    const previewButton = textElement("button", "", "预览校验");
    const acceptButton = textElement("button", "", workflow ? "应用选中修订并继续发现" : "应用选中修订");
    const rejectButton = textElement("button", "", workflow ? "全部拒绝并继续发现" : "全部拒绝");
    const status = textElement("p", "markdown-patch-review-status", "");
    const diff = textElement("details", "markdown-patch-diff", "");
    const diffSummary = document.createElement("summary"); diffSummary.textContent = "查看原文/修订文本";
    const original = textElement("pre", "markdown-patch-original", "");
    const revised = textElement("pre", "markdown-patch-revised", "");
    diff.append(diffSummary, original, revised);
    const values = (): Record<string, string> => Object.fromEntries(
      [...replacements.entries()].map(([ref, input]) => [ref, input.value]),
    );
    const chosen = (): string[] => [...selected.entries()]
      .filter(([, checkbox]) => checkbox.checked)
      .map(([ref]) => ref);
    const showSafety = (safety: MarkdownPatchSafetyView): void => {
      const diagnostics = safety.diagnostics.length > 0
        ? `；${safety.diagnostics.map(item => `${item.code}: ${item.message}`).join("；")}`
        : "";
      status.textContent = `${safety.status === "safe_for_b" ? "可直接衔接 Stage B" : "需要重新识别单元"}${diagnostics}`;
    };
    previewButton.onclick = async () => {
      previewButton.disabled = acceptButton.disabled = rejectButton.disabled = true;
      status.textContent = "正在进行确定性校验…";
      try {
        const preview = await invoke<MarkdownPatchPreviewView>("preview_markdown_patch", {
          request: { run_id: review.run_id, patch_set_id: review.patch_set_id, replacements: values() },
        });
        patchSet.fingerprint = preview.patch_set_fingerprint;
        const meta = heading.querySelector<HTMLElement>(".markdown-patch-meta");
        if (meta) meta.textContent = `${patchSet.patches.length} 条建议 · 指纹 ${patchSet.fingerprint}`;
        original.textContent = preview.original;
        revised.textContent = preview.revised;
        showSafety(preview.safety);
      } catch (error) {
        status.textContent = `预览失败：${String(error)}`;
      } finally {
        previewButton.disabled = acceptButton.disabled = rejectButton.disabled = false;
      }
    };
    const submit = async (acceptedPatchRefs: string[]): Promise<void> => {
      previewButton.disabled = acceptButton.disabled = rejectButton.disabled = true;
      status.textContent = acceptedPatchRefs.length > 0 ? "正在应用修订…" : "正在记录拒绝决定…";
      try {
        const receipt = await invoke<MarkdownPatchReceiptView>("submit_markdown_patch_decision", {
          request: {
            run_id: review.run_id,
            patch_set_id: review.patch_set_id,
            expected_patch_fingerprint: patchSet.fingerprint,
            accepted_patch_refs: acceptedPatchRefs,
            replacements: values(),
          },
        });
        section.classList.add("is-finished");
        acceptButton.disabled = rejectButton.disabled = true;
        // A map refresh closes this reader; the workflow path refreshes the
        // map when it reopens the reader on the patched artifact.
        if (receipt.status === "applied" && !workflow) eventBus.emit("source-map-changed");
        if (workflow) {
          status.textContent = receipt.status === "applied"
            ? `修订已写入新 Markdown：${receipt.patched_artifact_id ?? ""}；正在继续内容发现…`
            : "修订已拒绝，原 Markdown 保持不变；正在继续内容发现…";
          void this.resumeAfterPatch(review.run_id, status);
          return;
        }
        status.textContent = receipt.status === "applied"
          ? `修订已应用：${receipt.patched_artifact_id ?? "新 Markdown"}；请基于新材料重新运行来源发现。`
          : "修订已拒绝，原 Markdown 保持不变。";
      } catch (error) {
        status.textContent = `提交失败：${String(error)}`;
        previewButton.disabled = acceptButton.disabled = rejectButton.disabled = false;
      }
    };
    acceptButton.onclick = () => void submit(chosen());
    rejectButton.onclick = () => void submit([]);
    actions.append(previewButton, acceptButton, rejectButton);
    section.append(rows, actions, status, diff);
    this.patchReviewHost.append(section);
    void previewButton.click();
  }

  private head(value: string, count: number): string {
    return [...value].slice(0, count).join("");
  }

  private tail(value: string, count: number): string {
    const characters = [...value];
    return characters.slice(Math.max(0, characters.length - count)).join("");
  }
}
