/**
 * Block-level Markdown errata overlay (client IPC, pure projection/diff helpers,
 * restrained `已校对` Diff popover, and inline live-KaTeX-preview editor).
 */
import { invoke } from "@tauri-apps/api/core";
import { eventBus } from "../core/events";
import { markdownRenderer } from "./markdown";
import type { MarkdownBlock } from "./selection";

export type MarkdownErrataStatus = "active" | "reverted" | "superseded";
export type MarkdownErrataSourceKind = "manual" | "discovery_patch";

export interface MarkdownBlockErrata {
  errata_id: string;
  artifact_id: string;
  base_content_hash: string;
  first_block_id: string;
  last_block_id: string;
  covered_block_ids: string[];
  base_byte_start: number;
  base_byte_end: number;
  base_text_hash: string;
  original_text: string;
  corrected_text: string;
  status: MarkdownErrataStatus;
  source_kind: MarkdownErrataSourceKind;
  actor: string;
  note?: string | null;
  verified_pdf_page?: number | null;
  discovery_run_id?: string | null;
  patch_set_id?: string | null;
  superseded_by?: string | null;
  created_at: string;
  updated_at: string;
}

export interface SaveMarkdownErrataRequest {
  artifact_id: string;
  base_content_hash?: string | null;
  first_block_id: string;
  last_block_id: string;
  corrected_text: string;
  note?: string | null;
  verified_pdf_page?: number | null;
}

export interface SaveMarkdownErrataResponse {
  errata: MarkdownBlockErrata;
  superseded_ids: string[];
  affected_unit_ids: string[];
}

export interface RevertMarkdownErrataResponse {
  errata: MarkdownBlockErrata;
  affected_unit_ids: string[];
}

export async function listMarkdownErrata(artifactId: string): Promise<MarkdownBlockErrata[]> {
  try {
    return await invoke<MarkdownBlockErrata[]>("list_markdown_errata", { artifactId });
  } catch {
    // Graceful fallback when running in test harnesses that do not stub errata IPC.
    return [];
  }
}

export async function saveMarkdownErrata(
  request: SaveMarkdownErrataRequest,
): Promise<SaveMarkdownErrataResponse> {
  const response = await invoke<SaveMarkdownErrataResponse>("save_markdown_errata", { request });
  eventBus.emit("markdown-errata-changed", {
    artifactId: response.errata.artifact_id,
    errataId: response.errata.errata_id,
    action: "saved",
    affectedUnitIds: response.affected_unit_ids,
  });
  return response;
}

export async function revertMarkdownErrata(
  errataId: string,
): Promise<RevertMarkdownErrataResponse> {
  const response = await invoke<RevertMarkdownErrataResponse>("revert_markdown_errata", { errataId });
  eventBus.emit("markdown-errata-changed", {
    artifactId: response.errata.artifact_id,
    errataId: response.errata.errata_id,
    action: "reverted",
    affectedUnitIds: response.affected_unit_ids,
  });
  return response;
}

const PAGE_MARKER_LINE = /^\s*\{\d+\}-{16,}\s*$/;

export function containsPageMarkerLine(text: string): boolean {
  return text.split(/\r?\n/).some((line) => PAGE_MARKER_LINE.test(line));
}

export interface BlockErrataLookup {
  primaryByBlockId: Map<string, MarkdownBlockErrata>;
  hiddenSecondaryBlockIds: Set<string>;
  byAnyBlockId: Map<string, MarkdownBlockErrata>;
}

/** Indexes active errata entries by primary slot block and hidden secondary blocks. */
export function buildBlockErrataLookup(
  activeErrata: readonly MarkdownBlockErrata[],
): BlockErrataLookup {
  const primaryByBlockId = new Map<string, MarkdownBlockErrata>();
  const hiddenSecondaryBlockIds = new Set<string>();
  const byAnyBlockId = new Map<string, MarkdownBlockErrata>();

  for (const item of activeErrata) {
    if (item.status !== "active") continue;
    primaryByBlockId.set(item.first_block_id, item);
    const covered = item.covered_block_ids.length ? item.covered_block_ids : [item.first_block_id];
    covered.forEach((blockId, idx) => {
      byAnyBlockId.set(blockId, item);
      if (idx > 0 && blockId !== item.first_block_id) {
        hiddenSecondaryBlockIds.add(blockId);
      }
    });
  }
  return { primaryByBlockId, hiddenSecondaryBlockIds, byAnyBlockId };
}

export interface ErrataBlockSourceItem {
  block: MarkdownBlock;
  rawText: string;
}

export type ResolvedErrataCover =
  | {
      ok: true;
      startIdx: number;
      endIdx: number;
      firstBlockId: string;
      lastBlockId: string;
      coveredBlockIds: string[];
      originalText: string;
      initialText: string;
      overlappingErrata: MarkdownBlockErrata[];
    }
  | {
      ok: false;
      reason: string;
    };

/**
 * Mirrors backend `resolve_errata_target_range`: expands `[firstIdx..=lastIdx]`
 * to cover any overlapping active errata entries, rejects Marker page markers,
 * and composes the initial effective text for editing.
 */
export function computeErrataCoverRange(
  items: readonly ErrataBlockSourceItem[],
  firstIdx: number,
  lastIdx: number,
  activeErrata: readonly MarkdownBlockErrata[],
): ResolvedErrataCover {
  if (firstIdx < 0 || lastIdx < firstIdx || lastIdx >= items.length) {
    return { ok: false, reason: "选区范围无效，请重新选择正文块。" };
  }

  let startIdx = firstIdx;
  let endIdx = lastIdx;
  const active = activeErrata.filter((e) => e.status === "active");

  for (;;) {
    const rangeStart = items[startIdx].block.range.start;
    const rangeEnd = items[endIdx].block.range.end;
    let expanded = false;
    for (const errata of active) {
      if (errata.base_byte_start < rangeEnd && errata.base_byte_end > rangeStart) {
        const eFirst = items.findIndex((i) => i.block.id === errata.first_block_id);
        const eLast = items.findIndex((i) => i.block.id === errata.last_block_id);
        if (eFirst >= 0 && eFirst < startIdx) {
          startIdx = eFirst;
          expanded = true;
        }
        if (eLast >= 0 && eLast > endIdx) {
          endIdx = eLast;
          expanded = true;
        }
      }
    }
    if (!expanded) break;
  }

  const slice = items.slice(startIdx, endIdx + 1);
  if (slice.some((i) => i.block.kind === "page_marker")) {
    return { ok: false, reason: "所选区间包含 PDF 分页标记，请分段校对且勿跨越分页标记。" };
  }

  const lookup = buildBlockErrataLookup(active);
  const overlappingSet = new Set<MarkdownBlockErrata>();
  const rawParts: string[] = [];
  const effectiveParts: string[] = [];

  for (let i = startIdx; i <= endIdx; i++) {
    const item = items[i];
    rawParts.push(item.rawText.replace(/(?:\r?\n)+$/, ""));
    const existing = lookup.byAnyBlockId.get(item.block.id);
    if (existing) {
      overlappingSet.add(existing);
      if (existing.first_block_id === item.block.id) {
        effectiveParts.push(existing.corrected_text.replace(/(?:\r?\n)+$/, ""));
      }
    } else {
      effectiveParts.push(item.rawText.replace(/(?:\r?\n)+$/, ""));
    }
  }

  return {
    ok: true,
    startIdx,
    endIdx,
    firstBlockId: items[startIdx].block.id,
    lastBlockId: items[endIdx].block.id,
    coveredBlockIds: slice.map((i) => i.block.id),
    originalText: rawParts.join("\n\n"),
    initialText: effectiveParts.join("\n\n"),
    overlappingErrata: [...overlappingSet].sort((a, b) => a.base_byte_start - b.base_byte_start),
  };
}

export interface DiffLine {
  kind: "same" | "del" | "add";
  text: string;
}

export interface LineDiffResult {
  lines: DiffLine[];
  added: number;
  removed: number;
  changed: boolean;
}

/** Deterministic LCS line diff for compact OCR vs corrected text comparison. */
export function computeLineDiff(original: string, revised: string): LineDiffResult {
  const norm = (s: string) => s.replace(/\r\n/g, "\n").replace(/\n+$/, "");
  const aStr = norm(original);
  const bStr = norm(revised);
  if (aStr === bStr) {
    const sameLines = aStr ? aStr.split("\n").map((text): DiffLine => ({ kind: "same", text })) : [];
    return { lines: sameLines, added: 0, removed: 0, changed: false };
  }
  const a = aStr ? aStr.split("\n") : [];
  const b = bStr ? bStr.split("\n") : [];
  const m = a.length;
  const n = b.length;

  // Guard against huge blocks with a fast fallback.
  if (m * n > 40_000) {
    const lines: DiffLine[] = [
      ...a.map((text): DiffLine => ({ kind: "del", text })),
      ...b.map((text): DiffLine => ({ kind: "add", text })),
    ];
    return { lines, added: b.length, removed: a.length, changed: true };
  }

  const dp: number[][] = Array.from({ length: m + 1 }, () => new Array<number>(n + 1).fill(0));
  for (let i = m - 1; i >= 0; i--) {
    for (let j = n - 1; j >= 0; j--) {
      dp[i][j] = a[i] === b[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }

  const lines: DiffLine[] = [];
  let i = 0;
  let j = 0;
  let added = 0;
  let removed = 0;
  while (i < m && j < n) {
    if (a[i] === b[j]) {
      lines.push({ kind: "same", text: a[i] });
      i++;
      j++;
    } else if (dp[i + 1][j] >= dp[i][j + 1]) {
      lines.push({ kind: "del", text: a[i++] });
      removed++;
    } else {
      lines.push({ kind: "add", text: b[j++] });
      added++;
    }
  }
  while (i < m) {
    lines.push({ kind: "del", text: a[i++] });
    removed++;
  }
  while (j < n) {
    lines.push({ kind: "add", text: b[j++] });
    added++;
  }
  return { lines, added, removed, changed: added > 0 || removed > 0 };
}

let activeDiffPopover: { el: HTMLElement; close: () => void } | null = null;

export function closeErrataDiffPopover(): void {
  activeDiffPopover?.close();
}

export function openErrataDiffPopover(options: {
  anchor: HTMLElement;
  errata: MarkdownBlockErrata;
  onEdit: () => void;
  onRevert: () => Promise<void>;
}): void {
  closeErrataDiffPopover();
  const { anchor, errata, onEdit, onRevert } = options;

  const pop = document.createElement("div");
  pop.className = "errata-diff-popover";
  pop.setAttribute("role", "dialog");
  pop.setAttribute("aria-label", "人工校对记录与原 OCR 对比");
  const stop = (e: Event) => e.stopPropagation();
  pop.addEventListener("click", stop);
  pop.addEventListener("pointerdown", stop);
  pop.addEventListener("keydown", stop);

  const header = document.createElement("div");
  header.className = "errata-diff-header";
  const blockCount = errata.covered_block_ids.length;
  const titleText = blockCount > 1 ? `已人工校对 · 合并 ${blockCount} 块` : "已人工校对";
  const titleEl = document.createElement("strong");
  titleEl.textContent = titleText;
  const closeBtn = document.createElement("button");
  closeBtn.type = "button";
  closeBtn.className = "errata-diff-close";
  closeBtn.textContent = "✕";
  closeBtn.setAttribute("aria-label", "关闭对比浮层");
  header.append(titleEl, closeBtn);
  pop.append(header);

  if (errata.note) {
    const noteEl = document.createElement("p");
    noteEl.className = "errata-diff-note";
    noteEl.textContent = `备注：${errata.note}`;
    pop.append(noteEl);
  }

  const diff = computeLineDiff(errata.original_text, errata.corrected_text);
  const diffBox = document.createElement("div");
  diffBox.className = "errata-diff-view";
  for (const line of diff.lines) {
    const row = document.createElement("div");
    row.className = `errata-diff-line is-${line.kind}`;
    const prefix = line.kind === "del" ? "− " : line.kind === "add" ? "+ " : "  ";
    row.textContent = prefix + line.text;
    diffBox.append(row);
  }
  pop.append(diffBox);

  const statusEl = document.createElement("p");
  statusEl.className = "errata-diff-status";
  statusEl.hidden = true;
  pop.append(statusEl);

  const actions = document.createElement("div");
  actions.className = "errata-diff-actions";
  const editBtn = document.createElement("button");
  editBtn.type = "button";
  editBtn.className = "errata-btn-primary";
  editBtn.textContent = "继续编辑";
  const revertBtn = document.createElement("button");
  revertBtn.type = "button";
  revertBtn.className = "errata-btn-danger";
  revertBtn.textContent = "恢复原 OCR";
  actions.append(editBtn, revertBtn);
  pop.append(actions);

  const close = () => {
    pop.remove();
    window.removeEventListener("pointerdown", onOutside, true);
    window.removeEventListener("keydown", onEsc, true);
    if (activeDiffPopover?.el === pop) activeDiffPopover = null;
  };
  const onOutside = (event: Event) => {
    const t = event.target as Node | null;
    if (t && (pop.contains(t) || anchor.contains(t))) return;
    close();
  };
  const onEsc = (event: KeyboardEvent) => {
    if (event.key === "Escape") {
      event.preventDefault();
      close();
    }
  };
  closeBtn.onclick = close;
  editBtn.onclick = () => {
    close();
    onEdit();
  };
  revertBtn.onclick = async () => {
    revertBtn.disabled = editBtn.disabled = true;
    statusEl.hidden = false;
    statusEl.textContent = "正在恢复原始 OCR…";
    try {
      await onRevert();
      close();
    } catch (error) {
      statusEl.textContent = `恢复失败：${error instanceof Error ? error.message : String(error)}`;
      revertBtn.disabled = editBtn.disabled = false;
    }
  };

  document.body.append(pop);
  const rect = anchor.getBoundingClientRect();
  const popWidth = Math.min(460, window.innerWidth - 24);
  pop.style.width = `${popWidth}px`;
  const left = Math.max(12, Math.min(rect.right - popWidth, window.innerWidth - popWidth - 12));
  const top = Math.min(rect.bottom + 6, window.innerHeight - 260);
  pop.style.left = `${left}px`;
  pop.style.top = `${Math.max(12, top)}px`;

  window.addEventListener("pointerdown", onOutside, true);
  window.addEventListener("keydown", onEsc, true);
  activeDiffPopover = { el: pop, close };
}

export interface InlineErrataOpenArgs {
  anchorElement: HTMLElement;
  artifactId: string;
  baseContentHash: string;
  target: Extract<ResolvedErrataCover, { ok: true }>;
  verifiedPdfPage?: number | null;
  onOpenPdf?: () => void;
  onSaved: (response: SaveMarkdownErrataResponse) => Promise<void> | void;
  onReverted?: (response: RevertMarkdownErrataResponse) => Promise<void> | void;
}

/**
 * Manages at most one inline Markdown/KaTeX errata editor card inside an article.
 * Keeps editing strictly in place (zero page jump, zero lost scroll position) and
 * adapts automatically between wide reader (side-by-side source + live KaTeX preview)
 * and narrow `#ink-split-reader` (stacked source + live preview).
 */
export class InlineErrataEditorController {
  private currentCard: HTMLElement | null = null;
  private currentAnchor: HTMLElement | null = null;

  isOpen(): boolean {
    return this.currentCard !== null;
  }

  close(): void {
    this.currentCard?.remove();
    this.currentCard = null;
    this.currentAnchor?.classList.remove("is-errata-editing");
    this.currentAnchor = null;
  }

  open(args: InlineErrataOpenArgs): void {
    closeErrataDiffPopover();
    this.close();

    const { anchorElement, artifactId, baseContentHash, target, verifiedPdfPage, onOpenPdf, onSaved, onReverted } = args;
    const card = document.createElement("section");
    card.className = "errata-inline-editor";
    card.setAttribute("role", "region");
    card.setAttribute("aria-label", "就地校对 Markdown 块");

    const stopBubble = (e: Event) => e.stopPropagation();
    card.addEventListener("click", stopBubble);
    card.addEventListener("pointerdown", stopBubble);
    card.addEventListener("contextmenu", stopBubble);

    const header = document.createElement("div");
    header.className = "errata-editor-header";
    const titleGroup = document.createElement("div");
    titleGroup.className = "errata-editor-title-group";
    const blockCount = target.coveredBlockIds.length;
    const title = document.createElement("strong");
    title.textContent = blockCount > 1 ? `✎ 合并校对 ${blockCount} 块` : "✎ 校对当前块";
    const diffBadge = document.createElement("span");
    diffBadge.className = "errata-editor-diff-badge";
    diffBadge.textContent = "未修改";
    titleGroup.append(title, diffBadge);

    const headerActions = document.createElement("div");
    headerActions.className = "errata-editor-header-actions";
    if (onOpenPdf) {
      const pdfBtn = document.createElement("button");
      pdfBtn.type = "button";
      pdfBtn.className = "errata-btn-ghost";
      pdfBtn.textContent = verifiedPdfPage !== undefined && verifiedPdfPage !== null
        ? `对照 PDF 第 ${verifiedPdfPage + 1} 页 ↗`
        : "对照 PDF 原页 ↗";
      pdfBtn.title = "查看该块对应的原始 PDF 扫描页";
      pdfBtn.onclick = () => onOpenPdf();
      headerActions.append(pdfBtn);
    }
    const hint = document.createElement("span");
    hint.className = "errata-editor-shortcut-hint";
    hint.textContent = "⌘/Ctrl+Enter 保存 · Esc 取消";
    headerActions.append(hint);
    header.append(titleGroup, headerActions);

    const grid = document.createElement("div");
    grid.className = "errata-editor-grid";

    const sourceCol = document.createElement("div");
    sourceCol.className = "errata-editor-col";
    const sourceLabel = document.createElement("label");
    sourceLabel.className = "errata-editor-col-label";
    sourceLabel.textContent = "Markdown / LaTeX 源码";
    const textarea = document.createElement("textarea");
    textarea.className = "errata-editor-input";
    textarea.setAttribute("aria-label", "校对后的 Markdown 源码");
    textarea.spellcheck = false;
    textarea.value = target.initialText;
    textarea.rows = Math.max(4, Math.min(14, target.initialText.split("\n").length + 2));

    const noteInput = document.createElement("input");
    noteInput.type = "text";
    noteInput.className = "errata-editor-note-input";
    noteInput.placeholder = "可选备注（例如：核对 PDF 原页，修正下标 n）";
    noteInput.setAttribute("aria-label", "勘误备注");
    if (target.overlappingErrata.length === 1 && target.overlappingErrata[0].note) {
      noteInput.value = target.overlappingErrata[0].note;
    }
    sourceCol.append(sourceLabel, textarea, noteInput);

    const previewCol = document.createElement("div");
    previewCol.className = "errata-editor-col errata-editor-preview-col";
    const previewLabel = document.createElement("div");
    previewLabel.className = "errata-editor-col-label";
    previewLabel.textContent = "实时渲染预览（Markdown + KaTeX）";
    const katexWarn = document.createElement("div");
    katexWarn.className = "errata-katex-warning";
    katexWarn.hidden = true;
    const previewBox = document.createElement("div");
    previewBox.className = "errata-editor-preview markdown-body";
    previewBox.setAttribute("aria-live", "polite");
    previewCol.append(previewLabel, katexWarn, previewBox);

    grid.append(sourceCol, previewCol);

    const footer = document.createElement("div");
    footer.className = "errata-editor-footer";
    const errorEl = document.createElement("p");
    errorEl.className = "errata-editor-error";
    errorEl.hidden = true;

    const btnGroup = document.createElement("div");
    btnGroup.className = "errata-editor-actions";

    const cancelBtn = document.createElement("button");
    cancelBtn.type = "button";
    cancelBtn.className = "errata-btn-ghost";
    cancelBtn.textContent = "取消";

    const saveBtn = document.createElement("button");
    saveBtn.type = "button";
    saveBtn.className = "errata-btn-primary";
    saveBtn.textContent = "保存校对";

    if (target.overlappingErrata.length === 1 && onReverted) {
      const existing = target.overlappingErrata[0];
      const revertBtn = document.createElement("button");
      revertBtn.type = "button";
      revertBtn.className = "errata-btn-danger";
      revertBtn.textContent = "恢复原 OCR";
      revertBtn.onclick = async () => {
        saveBtn.disabled = cancelBtn.disabled = revertBtn.disabled = true;
        errorEl.hidden = true;
        try {
          const res = await revertMarkdownErrata(existing.errata_id);
          this.close();
          await onReverted(res);
        } catch (err) {
          errorEl.hidden = false;
          errorEl.textContent = `恢复失败：${err instanceof Error ? err.message : String(err)}`;
          saveBtn.disabled = cancelBtn.disabled = revertBtn.disabled = false;
        }
      };
      btnGroup.append(revertBtn);
    }

    btnGroup.append(cancelBtn, saveBtn);
    footer.append(errorEl, btnGroup);
    card.append(header, grid, footer);

    const md = markdownRenderer();
    let saving = false;

    const updatePreviewAndState = () => {
      const val = textarea.value;
      const trimmed = val.trim();
      previewBox.innerHTML = trimmed ? md.render(val, {}) : '<p class="errata-preview-empty">（正文为空）</p>';
      previewBox.querySelectorAll("a").forEach((link) => {
        link.removeAttribute("href");
      });

      const katexErrors = previewBox.querySelectorAll(".katex-error");
      if (katexErrors.length > 0) {
        katexWarn.hidden = false;
        katexWarn.textContent = `检测到 ${katexErrors.length} 处公式语法异常，请核对预览后再保存`;
      } else {
        katexWarn.hidden = true;
        katexWarn.textContent = "";
      }

      const diff = computeLineDiff(target.originalText, val);
      const changedFromInitial = val.replace(/\r\n/g, "\n").trimEnd() !== target.initialText.replace(/\r\n/g, "\n").trimEnd();
      const noteChanged =
        (target.overlappingErrata.length === 1 ? (target.overlappingErrata[0].note ?? "") : "") !==
        noteInput.value.trim();

      if (diff.changed) {
        diffBadge.textContent = `相对原 OCR：+${diff.added} / −${diff.removed} 行`;
        diffBadge.classList.add("has-changes");
      } else {
        diffBadge.textContent = "与原 OCR 一致";
        diffBadge.classList.remove("has-changes");
      }

      if (!trimmed) {
        errorEl.hidden = false;
        errorEl.textContent = "校对后的正文不能为空。";
        saveBtn.disabled = true;
      } else if (containsPageMarkerLine(val)) {
        errorEl.hidden = false;
        errorEl.textContent = "不能包含或伪造 Marker 分页标记行（{N}----------------）。";
        saveBtn.disabled = true;
      } else {
        errorEl.hidden = true;
        errorEl.textContent = "";
        saveBtn.disabled = saving || (!changedFromInitial && !noteChanged);
      }
    };

    const doSave = async () => {
      if (saving || saveBtn.disabled) return;
      saving = true;
      saveBtn.disabled = true;
      saveBtn.textContent = "保存中…";
      errorEl.hidden = true;
      try {
        const res = await saveMarkdownErrata({
          artifact_id: artifactId,
          base_content_hash: baseContentHash,
          first_block_id: target.firstBlockId,
          last_block_id: target.lastBlockId,
          corrected_text: textarea.value,
          note: noteInput.value.trim() || null,
          verified_pdf_page: verifiedPdfPage ?? null,
        });
        this.close();
        await onSaved(res);
      } catch (err) {
        saving = false;
        saveBtn.textContent = "保存校对";
        errorEl.hidden = false;
        errorEl.textContent = `保存失败：${err instanceof Error ? err.message : String(err)}`;
        updatePreviewAndState();
      }
    };

    textarea.addEventListener("input", updatePreviewAndState);
    noteInput.addEventListener("input", updatePreviewAndState);
    cancelBtn.onclick = () => this.close();
    saveBtn.onclick = () => void doSave();
    card.addEventListener("keydown", (event) => {
      event.stopPropagation();
      if (event.key === "Escape") {
        event.preventDefault();
        this.close();
      } else if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
        event.preventDefault();
        void doSave();
      }
    });

    anchorElement.classList.add("is-errata-editing");
    anchorElement.insertAdjacentElement("afterend", card);
    this.currentCard = card;
    this.currentAnchor = anchorElement;
    updatePreviewAndState();
    textarea.focus();
    textarea.setSelectionRange(textarea.value.length, textarea.value.length);
    card.scrollIntoView({ behavior: "smooth", block: "nearest" });
  }
}
