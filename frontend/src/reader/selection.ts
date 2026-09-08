import { invoke } from "@tauri-apps/api/core";

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
  private feedback: string | null = null;
  private status = document.createElement("p");
  private result = document.createElement("pre");
  private policy = document.createElement("select");
  private validate = document.createElement("button");
  private startButton: HTMLButtonElement;
  private endButton: HTMLButtonElement;
  private indexes: Map<string, number>;

  constructor(private data: AnchoredMarkdown, private article: HTMLElement) {
    this.indexes = new Map(data.blocks.map((b, i) => [b.id, i]));
    this.panel.className = "selection-panel";
    this.panel.setAttribute("aria-label", "来源发现选区");
    const instructions = document.createElement("p");
    instructions.textContent = "点击正文块，再设置起点和终点；列表、表格和公式按整块选择。此处仅构建发现输入，尚未调用 LLM 或入图。";
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
    this.policy.add(new Option("后续：审核后入图", "review_before_publish"));
    this.policy.add(new Option("后续：自动加入候选图（未人工审核）", "automatic_candidate"));
    this.policy.onchange = () => this.changed();
    controls.append(this.policy);
    this.validate = button("构建发现输入", () => void this.resolve());
    this.status.setAttribute("role", "status");
    this.result.className = "selection-source";
    this.result.hidden = true;
    this.panel.append(instructions, controls, this.status, this.result);
    article.addEventListener("click", this.onClick);
    article.addEventListener("keydown", this.onKey);
    this.refresh();
  }

  dispose(): void {
    this.disposed = true; this.version++;
    this.article.removeEventListener("click", this.onClick);
    this.article.removeEventListener("keydown", this.onKey);
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
    this.result.hidden = true; this.result.textContent = ""; this.refresh();
  }

  refresh(): void {
    const valid = this.first >= 0 && this.last >= this.first && this.data.blocks.length > 0;
    this.validate.disabled = !valid || this.pending;
    this.startButton.disabled = this.endButton.disabled = this.current < 0;
    this.article.querySelectorAll<HTMLElement>("[data-source-block]").forEach(block => {
      const i = this.indexes.get(block.dataset.sourceBlock ?? "") ?? -1;
      block.classList.toggle("is-current-block", i === this.current);
      block.classList.toggle("is-selected-block", valid && i >= this.first && i <= this.last);
    });
    this.status.textContent = this.feedback ?? (valid
      ? `已选择${this.whole ? "全文" : `第 ${this.first + 1}～${this.last + 1} 块`}，尚未构建发现输入。`
      : this.first >= 0 && this.last >= 0 ? "终点必须位于起点之后，请重新设置。" : "尚未选择完整范围。");
  }

  private async resolve(): Promise<void> {
    if (this.pending || this.disposed || this.first < 0 || this.last < this.first) return;
    const version = ++this.version;
    const request: SelectionRequest = {
      version: "mathscape-discovery-selection-1", snapshot: this.data.snapshot,
      selector: this.whole ? { kind: "whole_document" } : { kind: "block_range", first: this.data.blocks[this.first].id, last: this.data.blocks[this.last].id },
      admission: this.policy.value as SelectionRequest["admission"],
    };
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
}
