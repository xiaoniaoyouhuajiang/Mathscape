import { InkCanvas, type CanvasMetrics, type InkTool } from "./canvas";
import {
  INK_TITLE_MAX,
  appendInk,
  autoInkTitle,
  encodeInk,
  inkId,
  newDocument,
  normalizeInkTitle,
  parseInkDocument,
  parseInkSnapshotManifest,
  projectInk,
  type CaptureSegment,
  type InkBrush,
  type InkDocument,
  type InkPage,
  type InkStroke,
} from "./model";
import {
  InkDesktopTransport,
  cleanBlankInkNotes,
  createInkSnapshot,
  createInkDocument,
  getInkNote,
  getInkTrashImpact,
  inkOutboxItem,
  listInkDocuments,
  listInkSessions,
  listInkSnapshots,
  listTrashedInkDocuments,
  openInkDocument,
  renameInkNote,
  restoreInkNote,
  startInkSession,
  openInkSnapshot,
  trashInkNote,
  type InkNoteReceipt,
  type InkNoteState,
  type StoredInkDocumentSummary,
  type StoredInkSnapshot,
} from "./persistence";
import { InkOutbox, InkOutboxFlushError, type InkOutboxItem } from "./outbox";
import { InkEvidencePanel } from "./evidence-panel";
import { eventBus } from "../core/events";
import type { InkSelection } from "./persistence";
import { errorCode, safeInvoke } from "../core/ipc";
import { InkLanPanel,inkLanMirror,type InkWriterPresence,type InkWriterView } from "./lan";

/** Older hosts return only the first fields; N1 adds the note columns. */
type SessionSummary = Pick<StoredInkDocumentSummary, "document_id" | "title" | "head_seq" | "active_session_id" | "updated_at"> & Partial<StoredInkDocumentSummary>;

interface DialogChoice { id: string; label: string; primary?: boolean }

/** How long the "撤销" bar stays after moving a document to the trash. */
const UNDO_TRASH_MS = 10_000;

interface SessionRecord {
  session_id: string;
  document_id: string;
  writer_id: string;
  writer_epoch: number;
  status: string;
}

function element<T extends HTMLElement>(root: HTMLElement, selector: string): T {
  return root.querySelector<T>(selector)!;
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function displayMillis(value: number | null | undefined): string {
  return value ? new Date(value).toLocaleString() : "";
}

function displayTimestamp(value: string): string {
  const unix = /^unix:(\d+)$/.exec(value);
  const date = unix ? new Date(Number(unix[1]) * 1000) : new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

function download(bytes: Blob, name: string): void {
  const url = URL.createObjectURL(bytes);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = name;
  document.body.append(anchor);
  anchor.click();
  anchor.remove();
  URL.revokeObjectURL(url);
}

type PersistenceState = "loading" | "saved" | "pending" | "syncing" | "error";

export class DesktopInkWorkspace {
  private documentState: InkDocument | undefined;
  private currentPage = "";
  private currentSession: SessionRecord | undefined;
  private currentSnapshot: StoredInkSnapshot | undefined;
  private readOnly = false;
  private currentSegment: CaptureSegment | undefined;
  private board: InkCanvas | undefined;
  private outbox: InkOutbox | undefined;
  private readonly transport = new InkDesktopTransport();
  private readonly writerId = "desktop-local";
  private tool: InkTool = "pen";
  private logicalHead = 0;
  private listReload: Promise<void> | undefined;
  private listReloadAgain = false;
  private sessionSummaries: SessionSummary[] = [];
  private snapshotListRevision = 0;
  private persistenceState: PersistenceState = "loading";
  private pendingCount = 0;
  private brushColor = "#b9e3d8";
  private brushWidth = 4;
  private fingerWriting = false;
  private metrics: CanvasMetrics = { pointerEvents: 0, coalescedSamples: 0, cancelled: 0, lastPointer: "—", frameMs: 0, activePoints: 0 };
  private rendered = false;
  private selectedStrokeIds: string[] = [];
  private selectionMode: InkSelection["selection_mode"] = "lasso";
  private evidencePanel: InkEvidencePanel | undefined;
  private enqueueTail: Promise<void> = Promise.resolve();
  private bufferedOperations = new Map<string, InkOutboxItem>();
  private activeFlush: Promise<boolean> | undefined;
  private readonly confirmedHeads = new Map<string, number>();
  private navigationRevision = 0;
  private mirrorMode=false;
  private remotePreview:InkStroke|undefined;
  private writerView:InkWriterView|undefined;
  private writerPresence:InkWriterPresence|undefined;
  private lanPanel:InkLanPanel|undefined;
  private mirrorTimer:ReturnType<typeof setInterval>|undefined;
  private mirrorPolling=false;
  private snapshotCount=-1;
  private followPadPage=true;
  /** Authoritative title and lifecycle of the open document. */
  private note: InkNoteState | undefined;
  private trashedSummaries: SessionSummary[] = [];
  private listView: "active" | "trash" = "active";
  /** A specific document was requested; do not auto-open the first one. */
  private targetDocumentId: string | undefined;
  /** Read-only view of a trashed document. */
  private trashedView = false;
  /** Reused on retry so a lost reply never creates a second document (D12). */
  private pendingCreate: { requestId: string; sessionId: string; document: InkDocument } | undefined;
  private renameRequest: { requestId: string; documentId: string; title: string; expected: number } | undefined;
  private titleOriginal = "";
  private titleCancelled = false;
  private titleBusy = false;
  private undoTimer: ReturnType<typeof setTimeout> | undefined;
  private dialogResolve: ((value: string | null) => void) | undefined;

  constructor(
    private readonly host: HTMLElement,
    private readonly onClose: () => void,
    private readonly showToast: (message: string, kind: "success" | "error") => void,
  ) {}

  async open(): Promise<void> {
    this.host.hidden = false;
    if (!this.rendered) {
      this.rendered = true;
      this.render();
      this.outbox = await InkOutbox.openDefault();
      if (!this.outbox.persistent) this.status("本地恢复队列不可用；当前窗口关闭前请保持应用运行。", true);
    }
    await this.refreshDocuments(true);
    if(!this.mirrorTimer)this.mirrorTimer=setInterval(()=>void this.refreshMirror(),300);
  }

  close(): void {
    this.navigationRevision++;
    if(this.mirrorTimer){clearInterval(this.mirrorTimer);this.mirrorTimer=undefined;}
    this.board?.resetTransientInteraction();
    void this.flush();
    this.onClose();
  }

  async openDocumentId(documentId: string): Promise<void> {
    this.targetDocumentId = documentId;
    try { await this.open(); } finally { this.targetDocumentId = undefined; }
    const summary = this.sessionSummaries.find(item => item.document_id === documentId);
    if (summary) { await this.openDocument(summary); return; }
    const trashed = this.trashedSummaries.find(item => item.document_id === documentId);
    if (trashed) { await this.openTrashed(trashed); return; }
    this.status("手稿不存在或已被移除。", true);
  }

  private render(): void {
    this.host.innerHTML = [
      '<div class="ink-desktop-shell">',
      '<header class="ink-desktop-header"><div class="ink-document-heading"><span class="eyebrow">INK DOCUMENT</span><h2 id="ink-document-heading">数学手稿</h2><div class="ink-state-row"><span id="ink-mode-badge" class="ink-state-badge">未打开</span><span id="ink-editability-badge" class="ink-state-badge">—</span><span id="ink-save-badge" class="ink-state-badge">正在加载</span></div></div>',
      '<div id="ink-undo-bar" class="ink-undo-bar" role="status" hidden><span id="ink-undo-text"></span><button id="ink-undo-trash" class="secondary-button" type="button">撤销</button></div><div class="ink-desktop-actions"><span id="ink-persistence-status" class="ink-persistence-status">正在加载本地手稿…</span><button id="ink-lan" class="secondary-button" type="button">平板书写</button><button id="ink-continue-current" class="secondary-button" type="button" hidden>继续编辑当前手稿</button><button id="ink-save-now" class="secondary-button" type="button">立即保存</button><button id="ink-snapshot" class="secondary-button" type="button">保存快照</button><button id="ink-close" class="secondary-button" type="button">返回地图</button></div></header>',
      '<div class="ink-desktop-body"><aside class="ink-session-panel"><div class="ink-session-heading"><strong>手稿</strong><button id="ink-new-document" class="secondary-button" type="button">新建</button></div><div class="ink-session-tabs" role="tablist" aria-label="手稿列表"><button id="ink-tab-active" role="tab" type="button" aria-selected="true">最近手稿</button><button id="ink-tab-trash" role="tab" type="button" aria-selected="false">回收站</button></div><div class="ink-session-tools"><button id="ink-import-recovery" class="secondary-button" type="button">导入恢复草稿</button><button id="ink-clean-blank" class="secondary-button" type="button">清理空白稿</button></div><div id="ink-session-list" class="ink-session-list"><p>正在读取…</p></div><div class="ink-snapshot-heading"><strong>历史快照</strong></div><div id="ink-snapshot-list" class="ink-session-list"><p>请选择手稿。</p></div></aside>',
      '<main class="ink-desktop-main"><nav class="ink-desktop-toolbar" aria-label="手稿工具"><div class="ink-tool-buttons">',
      '<button id="desktop-ink-pen" class="is-active" type="button">笔</button><button id="desktop-ink-erase" type="button">整笔擦除</button><button id="desktop-ink-partial_erase" type="button">局部擦除</button><button id="desktop-ink-select" type="button">套索选择</button><button id="desktop-ink-undo" type="button" disabled>撤销</button><button id="desktop-ink-redo" type="button" disabled>重做</button>',
      '</div><div class="ink-brush-controls"><label title="之后创建的笔迹使用此颜色">颜色<input id="desktop-ink-color" type="color" value="#b9e3d8" aria-label="笔迹颜色"></label><label title="之后创建的笔迹使用此笔宽">笔宽<input id="desktop-ink-width" type="range" min="1" max="16" step="0.5" value="4" aria-label="笔迹宽度"><output id="desktop-ink-width-value">4</output></label><label class="ink-check-control"><input id="desktop-ink-finger" type="checkbox">手指书写</label></div><div class="ink-page-controls"><label id="desktop-ink-follow-label" hidden><input id="desktop-ink-follow-pad" type="checkbox" checked>跟随平板页</label><select id="desktop-ink-page" aria-label="手稿页面"></select><button id="desktop-ink-add-page" class="secondary-button" type="button">新增页面</button><button id="desktop-ink-zoom-out" class="secondary-button" type="button" aria-label="缩小">−</button><button id="desktop-ink-fit" class="secondary-button" type="button">整页</button><button id="desktop-ink-zoom-in" class="secondary-button" type="button" aria-label="放大">＋</button></div></nav>',
      '<div id="ink-trash-banner" class="ink-trash-banner" role="status" hidden><span>这份手稿在回收站中，只能查看和导出。</span><button id="ink-trash-restore" class="secondary-button" type="button">恢复手稿</button></div><div id="desktop-ink-evidence" hidden></div><div id="ink-empty-state" class="ink-empty-state" hidden><p>还没有打开的手稿。</p><button id="ink-empty-create" class="secondary-button" type="button">新建手稿</button></div><div id="desktop-ink-board" class="ink-desktop-board" aria-label="数学手稿画板"></div><footer class="ink-desktop-footer"><input id="desktop-ink-title" class="ink-desktop-title" maxlength="'+INK_TITLE_MAX+'" value="" placeholder="留空恢复自动标题" aria-label="手稿标题" title="点击改名；回车保存，Esc 取消" readonly><span id="desktop-ink-status">请选择一个手稿或新建手稿。</span><span id="desktop-ink-summary">—</span><div class="ink-export-actions"><button id="desktop-ink-export-json" class="secondary-button" type="button">导出 Ink JSON</button><button id="desktop-ink-export-png" class="secondary-button" type="button">导出 PNG</button></div></footer></main></div><section id="ink-lan-panel" hidden></section><div id="ink-dialog" class="ink-dialog" role="dialog" aria-modal="true" aria-labelledby="ink-dialog-text" hidden><div class="ink-dialog-box"><p id="ink-dialog-text"></p><div id="ink-dialog-actions" class="ink-dialog-actions"></div></div></div></div>',
    ].join("");
    element<HTMLButtonElement>(this.host, "#ink-close").addEventListener("click", () => this.close());
    element<HTMLButtonElement>(this.host, "#ink-save-now").addEventListener("click", () => void this.flush("手动保存完成。"));
    element<HTMLButtonElement>(this.host, "#ink-snapshot").addEventListener("click", () => void this.createSnapshot());
    this.lanPanel=new InkLanPanel(element(this.host,"#ink-lan-panel"),documentId=>this.preparePad(documentId),()=>this.writerView,()=>this.refreshMirror(true),()=>this.writerView?.remote?this.writerPresence:undefined);
    element<HTMLButtonElement>(this.host,"#ink-lan").onclick=()=>{if(this.documentState)void this.lanPanel!.open(this.documentState.document_id).catch(error=>this.status(errorText(error),true));};
    element<HTMLButtonElement>(this.host, "#ink-continue-current").addEventListener("click", () => void this.continueCurrentDocument());
    element<HTMLButtonElement>(this.host, "#ink-new-document").addEventListener("click", () => void this.createDocument());
    element<HTMLButtonElement>(this.host, "#ink-empty-create").addEventListener("click", () => void this.createDocument());
    element<HTMLButtonElement>(this.host, "#ink-tab-active").addEventListener("click", () => this.showList("active"));
    element<HTMLButtonElement>(this.host, "#ink-tab-trash").addEventListener("click", () => this.showList("trash"));
    element<HTMLButtonElement>(this.host, "#ink-clean-blank").addEventListener("click", () => void this.cleanBlank());
    element<HTMLButtonElement>(this.host, "#ink-trash-restore").addEventListener("click", () => { if (this.note) void this.restoreDocument(this.note.document_id, this.note.lifecycle_revision); });
    const title = element<HTMLInputElement>(this.host, "#desktop-ink-title");
    title.addEventListener("focus", () => { this.titleOriginal = title.value; this.titleCancelled = false; });
    title.addEventListener("keydown", event => {
      if (event.key === "Enter") { event.preventDefault(); title.blur(); }
      else if (event.key === "Escape") { event.preventDefault(); this.titleCancelled = true; title.value = this.titleOriginal; title.blur(); }
    });
    title.addEventListener("blur", () => { if (this.titleCancelled) { this.titleCancelled = false; return; } void this.commitTitle(); });
    this.host.addEventListener("keydown", event => { if (event.key === "Escape" && this.dialogResolve) { event.preventDefault(); this.closeDialog(null); } });
    element<HTMLButtonElement>(this.host,"#ink-import-recovery").onclick=()=>void this.importRecovery();
    element<HTMLButtonElement>(this.host, "#desktop-ink-add-page").addEventListener("click", () => this.addPage());
    element<HTMLSelectElement>(this.host, "#desktop-ink-page").addEventListener("change", event => {
      const value = (event.target as HTMLSelectElement).value;
      if (!this.documentState || !value) return;
      this.board?.resetTransientInteraction();
      this.currentPage = value;
      if(this.mirrorMode){this.followPadPage=false;element<HTMLInputElement>(this.host,"#desktop-ink-follow-pad").checked=false;}
      this.board?.setSelection([]);
      this.board?.fit();
      this.renderSummary();
    });
    element<HTMLInputElement>(this.host,"#desktop-ink-follow-pad").onchange=event=>{this.followPadPage=(event.target as HTMLInputElement).checked;};
    element<HTMLInputElement>(this.host, "#desktop-ink-color").addEventListener("input", event => {
      this.brushColor = (event.target as HTMLInputElement).value;
    });
    element<HTMLInputElement>(this.host, "#desktop-ink-width").addEventListener("input", event => {
      this.brushWidth = Number((event.target as HTMLInputElement).value);
      element<HTMLOutputElement>(this.host, "#desktop-ink-width-value").value = String(this.brushWidth);
    });
    element<HTMLInputElement>(this.host, "#desktop-ink-finger").addEventListener("change", event => {
      this.fingerWriting = (event.target as HTMLInputElement).checked;
    });
    element<HTMLButtonElement>(this.host, "#desktop-ink-zoom-out").addEventListener("click", () => this.board?.zoom(1 / 1.3));
    element<HTMLButtonElement>(this.host, "#desktop-ink-fit").addEventListener("click", () => this.board?.fit());
    element<HTMLButtonElement>(this.host, "#desktop-ink-zoom-in").addEventListener("click", () => this.board?.zoom(1.3));
    element<HTMLButtonElement>(this.host, "#desktop-ink-export-json").addEventListener("click", () => this.exportJson());
    element<HTMLButtonElement>(this.host, "#desktop-ink-export-png").addEventListener("click", () => void this.exportPng());
    for (const type of ["pen", "erase", "partial_erase", "select"] as const) {
      element<HTMLButtonElement>(this.host, "#desktop-ink-" + type).addEventListener("click", () => {
        this.board?.resetTransientInteraction();
        this.setTool(type);
      });
    }
    for (const kind of ["undo", "redo"] as const) {
      element<HTMLButtonElement>(this.host, "#desktop-ink-" + kind).addEventListener("click", () => this.history(kind));
    }
  }

  /**
   * Reloads both lists. Overlapping calls coalesce into one more reload rather
   * than being dropped, so a list never stays stale after trash or restore.
   * Only opening the panel picks a document automatically; after the open
   * document is trashed the empty state stays until the user chooses (§6).
   */
  private async refreshDocuments(autoOpen = false): Promise<void> {
    await this.reloadLists();
    if (this.documentState) return;
    const first = this.sessionSummaries[0];
    if (autoOpen && first && !this.targetDocumentId) await this.openDocument(first);
    else if (!this.targetDocumentId) this.renderEmpty();
  }

  private reloadLists(): Promise<void> {
    if (this.listReload) { this.listReloadAgain = true; return this.listReload; }
    const task = (async () => {
      do {
        this.listReloadAgain = false;
        try {
          const [documents, trashed] = await Promise.all([
            listInkDocuments(),
            listTrashedInkDocuments().catch(() => [] as SessionSummary[]),
          ]);
          // The store lists active documents only; older hosts may still mix them.
          this.sessionSummaries = documents.filter(item => item.lifecycle !== "trashed");
          this.trashedSummaries = trashed;
          this.renderSessionList(this.sessionSummaries);
        } catch (error) {
          this.status("读取手稿失败：" + errorText(error), true);
        }
      } while (this.listReloadAgain);
    })();
    this.listReload = task.finally(() => { this.listReload = undefined; });
    return this.listReload;
  }


  private showList(view: "active" | "trash"): void {
    this.listView = view;
    this.renderSessionList(this.sessionSummaries);
  }

  private renderSessionList(documents: SessionSummary[]): void {
    const list = element(this.host, "#ink-session-list");
    const trashView = this.listView === "trash";
    element(this.host, "#ink-tab-active").setAttribute("aria-selected", String(!trashView));
    element(this.host, "#ink-tab-trash").setAttribute("aria-selected", String(trashView));
    element(this.host, "#ink-tab-active").classList.toggle("is-active", !trashView);
    element(this.host, "#ink-tab-trash").classList.toggle("is-active", trashView);
    element<HTMLButtonElement>(this.host, "#ink-clean-blank").hidden = trashView;
    const items = trashView ? this.trashedSummaries : documents;
    list.replaceChildren();
    if (!items.length) {
      const empty = document.createElement("p");
      empty.textContent = trashView ? "回收站是空的。" : "还没有手稿。点击“新建”开始书写。";
      list.append(empty);
      return;
    }
    for (const summary of items) {
      const row = document.createElement("div");
      row.className = "ink-session-row";
      const button = document.createElement("button");
      button.type = "button";
      button.className = "ink-session-item";
      const isCurrentDocument = summary.document_id === this.documentState?.document_id;
      button.classList.toggle("is-active", isCurrentDocument && !this.readOnly);
      button.classList.toggle("is-context", isCurrentDocument && this.readOnly);
      if (isCurrentDocument) button.setAttribute("aria-current", this.readOnly ? "location" : "true");
      button.innerHTML = '<strong></strong><span></span>';
      button.querySelector("strong")!.textContent = summary.title;
      button.querySelector("span")!.textContent = trashView
        ? "移到回收站于 " + displayMillis(summary.trashed_at)
        : summary.head_seq + " 个已保存操作 · " + (summary.last_used_at ? displayMillis(summary.last_used_at) : displayTimestamp(summary.updated_at));
      button.addEventListener("click", () => void (trashView ? this.openTrashed(summary) : this.openDocument(summary)));
      const action = document.createElement("button");
      action.type = "button";
      action.className = "ink-session-action";
      if (trashView) {
        action.textContent = "恢复";
        action.setAttribute("aria-label", "恢复");
        action.title = "恢复「" + summary.title + "」";
        action.addEventListener("click", () => void this.restoreDocument(summary.document_id, summary.lifecycle_revision ?? 0));
      } else {
        action.textContent = "🗑";
        action.setAttribute("aria-label", "移到回收站");
        action.title = "将「" + summary.title + "」移到回收站";
        action.addEventListener("click", () => void this.trashDocument(summary));
      }
      row.append(button, action);
      list.append(row);
    }
  }

  private renderEmpty(): void {
    const empty = !this.documentState;
    element(this.host, "#ink-empty-state").hidden = !empty;
    element(this.host, "#desktop-ink-board").hidden = empty;
    if (!empty) return;
    element(this.host, "#ink-document-heading").textContent = "数学手稿";
    element(this.host, "#ink-mode-badge").textContent = "未打开";
    element(this.host, "#ink-editability-badge").textContent = "—";
    element(this.host, "#ink-save-badge").textContent = "—";
    const title = element<HTMLInputElement>(this.host, "#desktop-ink-title");
    title.value = ""; title.readOnly = true;
    for (const selector of ["#ink-lan", "#ink-save-now", "#ink-snapshot", "#desktop-ink-export-json", "#desktop-ink-export-png"]) element<HTMLButtonElement>(this.host, selector).disabled = true;
    element(this.host, "#ink-trash-banner").hidden = true;
    element(this.host, "#ink-snapshot-list").replaceChildren();
    element(this.host, "#desktop-ink-status").textContent = this.sessionSummaries.length ? "请选择一个手稿，或新建手稿。" : "还没有手稿。点击“新建手稿”开始。";
  }

  /** Clears the open document, e.g. after it moved to the trash. */
  private unmountDocument(): void {
    this.navigationRevision++;
    this.board?.finish();
    this.board?.dispose();
    this.board = undefined;
    this.evidencePanel?.dispose();
    this.evidencePanel = undefined;
    element(this.host, "#desktop-ink-evidence").hidden = true;
    this.documentState = undefined;
    this.currentSession = undefined;
    this.currentSnapshot = undefined;
    this.note = undefined;
    this.trashedView = false;
    this.readOnly = false;
    this.mirrorMode = false;
    this.writerView = undefined;
    this.renderEmpty();
  }

  /** Small in-page dialog; resolves to the chosen id or `null` (Esc). */
  private choose(text: string, choices: DialogChoice[]): Promise<string | null> {
    this.closeDialog(null);
    const dialog = element(this.host, "#ink-dialog");
    element(this.host, "#ink-dialog-text").textContent = text;
    const actions = element(this.host, "#ink-dialog-actions");
    actions.replaceChildren();
    for (const choice of choices) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = choice.primary ? "primary-button" : "secondary-button";
      button.textContent = choice.label;
      button.dataset.choice = choice.id;
      button.addEventListener("click", () => this.closeDialog(choice.id));
      actions.append(button);
    }
    dialog.hidden = false;
    actions.querySelector<HTMLButtonElement>("button")?.focus();
    return new Promise(resolve => { this.dialogResolve = resolve; });
  }

  private closeDialog(value: string | null): void {
    const resolve = this.dialogResolve;
    this.dialogResolve = undefined;
    element(this.host, "#ink-dialog").hidden = true;
    resolve?.(value);
  }

  /** Applies the server note to the open document, list and header. */
  private applyNote(note: InkNoteState): void {
    for (const summary of [...this.sessionSummaries, ...this.trashedSummaries]) {
      if (summary.document_id !== note.document_id) continue;
      summary.title = note.title; summary.title_mode = note.title_mode; summary.title_revision = note.title_revision;
      summary.lifecycle = note.lifecycle; summary.lifecycle_revision = note.lifecycle_revision; summary.trashed_at = note.trashed_at;
    }
    if (this.documentState?.document_id === note.document_id) {
      this.note = note;
      this.documentState.title = note.title;
      const title = element<HTMLInputElement>(this.host, "#desktop-ink-title");
      if (document.activeElement !== title) title.value = note.title;
      this.renderViewState();
    }
    this.renderSessionList(this.sessionSummaries);
  }

  private async commitTitle(): Promise<void> {
    const input = element<HTMLInputElement>(this.host, "#desktop-ink-title");
    const note = this.note;
    if (!note || input.readOnly || this.titleBusy || this.documentState?.document_id !== note.document_id) return;
    let requested: string;
    try { requested = normalizeInkTitle(input.value) ?? ""; }
    catch (error) { input.value = note.title; this.status(errorText(error), true); return; }
    if (requested === note.title || (!requested && note.title_mode === "auto")) { input.value = note.title; return; }
    await this.submitRename(note.document_id, requested, note.title_revision);
  }

  private async submitRename(documentId: string, title: string, expected: number): Promise<void> {
    const previous = this.renameRequest;
    const request = previous && previous.documentId === documentId && previous.title === title && previous.expected === expected
      ? previous : { requestId: inkId(), documentId, title, expected };
    this.renameRequest = request;
    this.titleBusy = true;
    try {
      const receipt = await renameInkNote(request.requestId, documentId, title, expected);
      this.renameRequest = undefined;
      this.applyNote(receipt.note);
      this.status(title ? "标题已更新。" : "已恢复自动标题。", false);
    } catch (error) {
      if (errorCode(error) === "title_conflict") {
        this.renameRequest = undefined;
        const details = (error as { details?: Record<string, unknown> }).details ?? {};
        const current = String(details.current_title ?? "");
        const revision = Number(details.current_title_revision ?? expected);
        const choice = await this.choose(`标题已在别处改为「${current}」。要保留哪一个？`, [
          { id: "theirs", label: `保留「${current}」` },
          { id: "mine", label: `使用「${title || "自动标题"}」`, primary: true },
        ]);
        if (choice === "mine") { this.titleBusy = false; await this.submitRename(documentId, title, revision); return; }
        const latest = await getInkNote(documentId).catch(() => null);
        if (latest) this.applyNote(latest);
      } else {
        if (this.note) element<HTMLInputElement>(this.host, "#desktop-ink-title").value = this.note.title;
        this.status("改名失败：" + errorText(error), true);
      }
    } finally {
      this.titleBusy = false;
    }
  }

  /** Items for the document that are not yet confirmed by the store. */
  private async pendingFor(documentId: string): Promise<number> {
    const stored = this.outbox ? (await this.outbox.pending(documentId)).length : 0;
    const buffered = [...this.bufferedOperations.values()].filter(item => item.document_id === documentId).length;
    return stored + buffered;
  }

  /** D6: sync first, report what is affected, then trash and offer undo. */
  private async trashDocument(summary: SessionSummary): Promise<void> {
    const documentId = summary.document_id;
    if (this.documentState?.document_id === documentId) this.board?.finish();
    for (;;) {
      await this.flush();
      const pending = await this.pendingFor(documentId);
      if (!pending) break;
      const choice = await this.choose(`这份手稿还有 ${pending} 个操作没有保存到电脑。`, [
        { id: "retry", label: "重试保存", primary: true },
        { id: "force", label: "仍移到回收站（未保存操作留在本机恢复队列）" },
        { id: "cancel", label: "取消" },
      ]);
      if (choice === "retry") continue;
      if (choice !== "force") return;
      break;
    }
    let impact;
    try { impact = await getInkTrashImpact(documentId); }
    catch (error) { this.status("读取手稿状态失败：" + errorText(error), true); return; }
    const lines = [`将「${impact.note.title}」移到回收站？`, `${impact.page_count} 页 · ${impact.head_seq} 个已保存操作 · ${impact.snapshot_count} 个快照 · ${impact.evidence_count} 条来源证据。快照和证据仍可查看。`];
    const tablet = impact.remote_session || impact.connected || impact.device_credentials > 0 || impact.pairing_pending;
    if (impact.remote_session || impact.connected) {
      let padPending: number | null = null;
      try { padPending = (await inkLanMirror(documentId, impact.head_seq)).client?.pending_count ?? null; } catch { /* Presence is advisory. */ }
      if (padPending) {
        const choice = await this.choose(`平板上还有 ${padPending} 个操作没有同步到电脑。建议先在平板上点“立即同步”。`, [
          { id: "cancel", label: "先去平板同步", primary: true },
          { id: "force", label: "仍移到回收站（平板保留未同步内容，可导出恢复草稿）" },
        ]);
        if (choice !== "force") return;
      } else if (!impact.connected) {
        lines.push("平板当前离线，可能还有未同步的内容留在平板上。");
      }
    }
    if (tablet) lines.push("平板连接会被关闭；恢复后需要在电脑重新配对才能继续书写。");
    const confirmed = await this.choose(lines.join("\n"), [
      { id: "trash", label: "移到回收站", primary: true },
      { id: "cancel", label: "取消" },
    ]);
    if (confirmed !== "trash") return;
    try {
      const receipt = await trashInkNote(inkId(), documentId, impact.note.lifecycle_revision);
      if (this.documentState?.document_id === documentId) this.unmountDocument();
      await this.refreshDocuments();
      this.showUndo(receipt);
      const quarantined = this.outbox ? (await this.outbox.quarantined(documentId)).length + (await this.outbox.pending(documentId)).length : 0;
      this.status(`已将「${receipt.note.title}」移到回收站。` + (quarantined ? `${quarantined} 个未保存操作保留在本机恢复队列。` : ""), false);
    } catch (error) {
      this.status("移到回收站失败：" + errorText(error), true);
      await this.refreshDocuments();
    }
  }

  private showUndo(receipt: InkNoteReceipt): void {
    const bar = element(this.host, "#ink-undo-bar");
    element(this.host, "#ink-undo-text").textContent = `「${receipt.note.title}」已移到回收站。`;
    bar.hidden = false;
    if (this.undoTimer) clearTimeout(this.undoTimer);
    const hide = () => { bar.hidden = true; this.undoTimer = undefined; };
    this.undoTimer = setTimeout(hide, UNDO_TRASH_MS);
    element<HTMLButtonElement>(this.host, "#ink-undo-trash").onclick = () => {
      hide();
      if (this.undoTimer) clearTimeout(this.undoTimer);
      // Undo is a normal restore (§13.5): sessions and credentials stay closed.
      void this.restoreDocument(receipt.note.document_id, receipt.note.lifecycle_revision, true);
    };
  }

  private async restoreDocument(documentId: string, expectedRevision: number, reopen = false): Promise<void> {
    try {
      const receipt = await restoreInkNote(inkId(), documentId, expectedRevision);
      const viewing = this.trashedView && this.documentState?.document_id === documentId;
      if (viewing) this.unmountDocument();
      await this.refreshDocuments();
      const summary = this.sessionSummaries.find(item => item.document_id === documentId);
      if (summary && (viewing || reopen || !this.documentState)) { this.listView = "active"; await this.openDocument(summary); }
      this.status(`「${receipt.note.title}」已恢复。` + (receipt.requires_repairing ? "之前连接的平板需要在电脑重新配对后才能继续书写。" : ""), false);
    } catch (error) {
      this.status("恢复手稿失败：" + errorText(error), true);
      await this.refreshDocuments();
    }
  }

  /** D10: never touches the open document, local pending writes or devices. */
  private async cleanBlank(): Promise<void> {
    const candidates: string[] = [];
    for (const summary of this.sessionSummaries) {
      if (summary.document_id === this.documentState?.document_id) continue;
      if (summary.title_mode !== "auto" || summary.head_seq !== 0 || (summary.snapshot_count ?? 0) > 0 || summary.remote_active) continue;
      if (await this.pendingFor(summary.document_id)) continue;
      candidates.push(summary.document_id);
    }
    if (!candidates.length) { this.status("没有可清理的空白稿。当前打开的手稿、写过内容或改过名的手稿不会被清理。", false); return; }
    const choice = await this.choose(`将 ${candidates.length} 份空白稿移到回收站？当前打开的手稿、有平板连接的手稿不会被清理。`, [
      { id: "clean", label: "移到回收站", primary: true },
      { id: "cancel", label: "取消" },
    ]);
    if (choice !== "clean") return;
    try {
      const receipt = await cleanBlankInkNotes(inkId(), candidates);
      await this.refreshDocuments();
      this.status(`已将 ${receipt.trashed.length} 份空白稿移到回收站。` + (receipt.skipped.length ? `${receipt.skipped.length} 份因状态变化被跳过。` : ""), false);
    } catch (error) {
      this.status("清理空白稿失败：" + errorText(error), true);
    }
  }

  private async openTrashed(summary: SessionSummary): Promise<void> {
    const revision = ++this.navigationRevision;
    this.board?.finish();
    try {
      await this.flush();
      const stored = await openInkDocument(summary.document_id);
      if (revision !== this.navigationRevision) return;
      if (!stored) throw new Error("手稿不存在或已被移除");
      this.note = stored.note;
      if (stored.note && stored.note.lifecycle !== "trashed") {
        await this.refreshDocuments();
        const active = this.sessionSummaries.find(item => item.document_id === summary.document_id);
        if (active) { await this.openDocument(active); return; }
      }
      this.listView = "trash";
      await this.mountDocument(stored.document, undefined, stored.head_seq, true, undefined, revision, true);
      this.status("这份手稿在回收站中，只能查看；恢复后才能继续书写。", false);
    } catch (error) {
      this.status("打开手稿失败：" + errorText(error), true);
    }
  }

  private async createDocument(): Promise<void> {
    const revision = ++this.navigationRevision;
    this.board?.finish();
    await this.flush();
    this.status("正在创建本地手稿…", false);
    if (!this.pendingCreate) {
      const draft = newDocument();
      draft.title = autoInkTitle(draft.created_at);
      this.pendingCreate = { requestId: inkId(), sessionId: inkId(), document: draft };
    }
    const pending = this.pendingCreate;
    try {
      const receipt = await createInkDocument(structuredClone(pending.document), pending.sessionId, this.writerId, { source: "desktop" }, { requestId: pending.requestId, titleMode: "auto" });
      if (this.pendingCreate === pending) this.pendingCreate = undefined;
      if (revision !== this.navigationRevision) return;
      const documentState = structuredClone(pending.document);
      if (receipt.note) documentState.title = receipt.note.title;
      this.note = receipt.note;
      this.listView = "active";
      await this.mountDocument(documentState, receipt, 0, false, undefined, revision);
      this.status("手稿已创建，书写后会自动保存。", false);
      await this.refreshDocuments();
    } catch (error) {
      // The same request id is reused on the next attempt.
      this.status("创建手稿失败，可重试：" + errorText(error), true);
    }
  }

  private async openDocument(summary: SessionSummary): Promise<void> {
    const revision = ++this.navigationRevision;
    this.board?.finish();
    this.status("正在恢复手稿…", false);
    try {
      await this.flush();
      const stored = await openInkDocument(summary.document_id);
      if (revision !== this.navigationRevision) return;
      if (!stored) throw new Error("手稿不存在或已被移除");
      if (stored.note?.lifecycle === "trashed") { await this.openTrashed(summary); return; }
      this.note = stored.note ?? this.summaryNote(summary, stored.document);
      const mirror=await inkLanMirror(summary.document_id,stored.head_seq);
      if(revision!==this.navigationRevision)return;
      this.writerView=mirror.delta.writer;this.writerPresence={connected:mirror.connected,pending:mirror.client?.pending_count??null};
      if(mirror.delta.writer.remote||mirror.delta.writer.pairing_pending){
        await this.mountDocument(stored.document,mirror.delta.writer.session??undefined,stored.head_seq,true,undefined,revision);
        this.status("电脑显示手稿镜像，平板书写后会自动同步。",false);return;
      }
      const sessions = summary.active_session_id ? [] : await listInkSessions(summary.document_id);
      const sessionId = mirror.delta.writer.session?.session_id ?? sessions.find(session=>session.writer_id===this.writerId&&session.writer_epoch===mirror.delta.writer.writer_epoch)?.session_id ?? inkId();
      const session = await startInkSession(summary.document_id, sessionId, this.writerId, { source: "desktop" });
      if (revision !== this.navigationRevision) return;
      await this.mountDocument(stored.document, session, stored.head_seq, false, undefined, revision);
      this.status("手稿已恢复；新操作会自动进入本地保存队列。", false);
    } catch (error) {
      this.status("恢复手稿失败：" + errorText(error), true);
    }
  }

  /** Fallback note for hosts that predate N1. */
  private summaryNote(summary: SessionSummary, document: InkDocument): InkNoteState {
    return { document_id: summary.document_id, title: document.title, title_mode: summary.title_mode ?? "manual", title_revision: summary.title_revision ?? 0, lifecycle: "active", lifecycle_revision: summary.lifecycle_revision ?? 0, trashed_at: null, last_used_at: summary.last_used_at ?? 0, created_at: document.created_at };
  }

  private async openSnapshot(snapshotId: string): Promise<void> {
    const revision = ++this.navigationRevision;
    try {
      this.board?.finish();
      await this.flush();
      const stored = await openInkSnapshot(snapshotId);
      if (revision !== this.navigationRevision) return;
      if (!stored) throw new Error("快照不存在或已被移除");
      const manifest = parseInkSnapshotManifest(JSON.parse(stored.manifest_json));
      await this.mountDocument(manifest.document, undefined, manifest.through_document_seq, true, stored, revision);
      this.status("当前正在查看只读历史快照。", false);
    } catch (error) {
      this.status("打开快照失败：" + errorText(error), true);
    }
  }

  private async mountDocument(documentState: InkDocument, session: SessionRecord | undefined, head: number, readOnly = false, snapshot?: StoredInkSnapshot, revision = this.navigationRevision, trashed = false): Promise<void> {
    const localHead = !readOnly && session ? await this.replayPending(documentState, session.session_id, head) : head;
    if (revision !== this.navigationRevision) return;
    this.board?.finish();
    this.board?.dispose();
    this.evidencePanel?.dispose();
    this.evidencePanel = undefined;
    this.selectedStrokeIds = [];
    this.documentState = documentState;
    this.currentPage = documentState.pages[0]?.id ?? "";
    this.currentSession = session;
    if(this.writerView?.document_id!==documentState.document_id)this.writerView=undefined;
    this.currentSnapshot = snapshot;
    this.readOnly = readOnly;
    this.trashedView = trashed;
    this.mirrorMode=readOnly&&!snapshot&&!trashed;
    if (this.note?.document_id !== documentState.document_id) this.note = undefined;
    element(this.host, "#ink-empty-state").hidden = true;
    element(this.host, "#desktop-ink-board").hidden = false;
    element(this.host, "#ink-trash-banner").hidden = !trashed;
    for (const selector of ["#desktop-ink-export-json", "#desktop-ink-export-png"]) element<HTMLButtonElement>(this.host, selector).disabled = false;
    element(this.host,"#desktop-ink-follow-label").hidden=!this.mirrorMode;
    this.remotePreview=undefined;
    this.snapshotCount=-1;
    this.setTool(readOnly ? "select" : "pen");
    this.currentSegment = undefined;
    this.logicalHead = localHead;
    this.confirmedHeads.set(documentState.document_id, head);
    this.renderSessionList(this.sessionSummaries);
    element<HTMLInputElement>(this.host, "#desktop-ink-title").value = documentState.title;
    this.renderPages();
    this.board = new InkCanvas(element(this.host, "#desktop-ink-board"), {
      document: () => this.documentState!,
      page: () => this.documentState!.pages.find(page => page.id === this.currentPage)!,
      segmentId: () => this.segmentId(),
      brush: () => this.brush(),
      tool: () => this.tool,
      fingerWriting: () => this.fingerWriting,
      readOnly,
      externalPreview:()=>this.remotePreview,
      onStroke: stroke => this.commit({ kind: "stroke", stroke }),
      onErase: strokeIds => this.commit({ kind: "erase", stroke_ids: strokeIds }),
      onPartialErase: (sourceStrokeId, fragments) => this.commit(fragments.length ? { kind: "partial_erase", source_stroke_id: sourceStrokeId, fragments } : { kind: "erase", stroke_ids: [sourceStrokeId] }),
      onTransform: (strokeIds, matrix) => this.commit({ kind: "transform", stroke_ids: strokeIds, matrix }),
      onSelection: ids => { this.selectedStrokeIds = ids; this.selectionMode = "lasso"; this.evidencePanel?.update(); this.renderSummary(); },
      onMessage: message => this.status(message, false),
      onMetrics: metrics => { this.metrics = metrics; this.renderSummary(); },
    });
    this.board.refresh();
    this.renderSummary();
    this.setReadOnlyControls(readOnly);
    const evidenceHost = element(this.host, "#desktop-ink-evidence");
    evidenceHost.hidden = !snapshot;
    evidenceHost.replaceChildren();
    if (snapshot) this.evidencePanel = new InkEvidencePanel(evidenceHost, snapshot, () => ({ page_id: this.currentPage, stroke_ids: this.selectedStrokeIds, selection_mode: this.selectionMode }), selected => {
      this.board?.resetTransientInteraction();
      this.currentPage = selected.page_id;
      this.renderPages();
      this.board?.fit();
      this.board?.setSelection(selected.stroke_ids);
      this.selectionMode = selected.selection_mode;
      this.evidencePanel?.update();
    });
    this.renderViewState();
    void this.refreshSnapshots();
    if (!readOnly) {
      this.persistenceState = "saved";
      void this.flush();
    }
  }

  private setReadOnlyControls(readOnly: boolean): void {
    for (const selector of ["#desktop-ink-pen", "#desktop-ink-erase", "#desktop-ink-partial_erase", "#desktop-ink-undo", "#desktop-ink-redo", "#desktop-ink-add-page", "#ink-save-now", "#ink-snapshot"]) {
      element<HTMLButtonElement>(this.host, selector).disabled = readOnly;
    }
    element<HTMLButtonElement>(this.host, "#desktop-ink-select").disabled = false;
    for (const selector of ["#desktop-ink-color", "#desktop-ink-width", "#desktop-ink-finger"]) {
      element<HTMLInputElement>(this.host, selector).disabled = readOnly;
    }
    element<HTMLButtonElement>(this.host, "#ink-continue-current").hidden = !this.currentSnapshot;
    element<HTMLButtonElement>(this.host,"#ink-lan").disabled=!!this.currentSnapshot||this.trashedView;
    element<HTMLButtonElement>(this.host,"#ink-snapshot").disabled=!!this.currentSnapshot||this.trashedView;
    element<HTMLButtonElement>(this.host,"#ink-save-now").disabled=readOnly;
    // Renaming is note metadata, so it also works while a tablet holds the lease.
    element<HTMLInputElement>(this.host,"#desktop-ink-title").readOnly=!this.note||!!this.currentSnapshot||this.trashedView;
    element<HTMLButtonElement>(this.host,"#ink-snapshot").textContent=this.mirrorMode?"保存已同步快照":"保存快照";
  }

  private async refreshSnapshots(): Promise<void> {
    const documentId = this.documentState?.document_id;
    if (!documentId) return;
    const revision = ++this.snapshotListRevision;
    const list = element(this.host, "#ink-snapshot-list");
    list.replaceChildren();
    const loading = document.createElement("p");
    loading.textContent = "正在读取历史快照…";
    list.append(loading);
    try {
      const snapshots = await listInkSnapshots(documentId);
      if (revision !== this.snapshotListRevision || this.documentState?.document_id !== documentId) return;
      list.replaceChildren();
      if (!snapshots.length) {
        const empty = document.createElement("p");
        empty.textContent = "还没有历史快照。";
        list.append(empty);
        return;
      }
      for (const snapshot of snapshots) {
        const button = document.createElement("button");
        button.type = "button";
        button.className = "ink-session-item";
        button.classList.toggle("is-active", snapshot.snapshot_id === this.currentSnapshot?.snapshot_id);
        button.innerHTML = "<strong></strong><span></span>";
        button.querySelector("strong")!.textContent = "head " + snapshot.through_seq;
        button.querySelector("span")!.textContent = (snapshot.snapshot_id === this.currentSnapshot?.snapshot_id ? "正在查看 · " : "") + displayTimestamp(snapshot.created_at);
        button.addEventListener("click", () => void this.openSnapshot(snapshot.snapshot_id));
        list.append(button);
      }
    } catch (error) {
      if (revision !== this.snapshotListRevision || this.documentState?.document_id !== documentId) return;
      list.replaceChildren();
      this.status("读取历史快照失败：" + errorText(error), true);
    }
  }

  private async replayPending(documentState: InkDocument, sessionId: string, head: number): Promise<number> {
    if (!this.outbox) return head;
    let logicalHead = head;
    const pending = [...await this.outbox.pending(), ...this.bufferedOperations.values()]
      .filter(item => item.document_id === documentState.document_id && item.session_id === sessionId)
      .sort((a, b) => a.client_seq - b.client_seq);
    for (const item of pending) {
      if (item.operation.seq <= head || item.operation.seq !== logicalHead + 1) continue;
      // The queued title is a stale copy; the store's note owns the title (H1).
      documentState.version = item.metadata.version;
      documentState.exported_at = item.metadata.exported_at;
      documentState.pages = structuredClone(item.metadata.pages);
      documentState.capture_segments = structuredClone(item.metadata.capture_segments);
      documentState.operations.push(structuredClone(item.operation));
      parseInkDocument(documentState);
      logicalHead = item.operation.seq;
    }
    return logicalHead;
  }

  private segmentId(): string {
    if (!this.documentState) throw new Error("没有打开手稿");
    if (!this.currentSegment) {
      this.currentSegment = {
        id: inkId(),
        started_at: new Date().toISOString(),
        user_agent: navigator.userAgent,
        secure_context: window.isSecureContext,
        coalesced_events: typeof PointerEvent.prototype.getCoalescedEvents === "function",
        pointer_raw_update: "onpointerrawupdate" in window,
        device_pixel_ratio: window.devicePixelRatio || 1,
        time_origin_ms: performance.timeOrigin,
        pressure_capability: "unknown",
        tilt_capability: "unknown",
      };
      this.documentState.capture_segments.push(this.currentSegment);
    }
    return this.currentSegment.id;
  }

  private brush(): InkBrush {
    return { renderer: "perfect-freehand@1.2.3", color: this.brushColor, width: this.brushWidth, smoothing: 0.35, streamline: 0.15, thinning: 0.4, simulate_pressure: false };
  }

  private commit(command: Parameters<typeof appendInk>[1]): void {
    if (this.readOnly || !this.documentState || !this.currentSession || !this.outbox) return;
    try {
      appendInk(this.documentState, command);
      const operation = this.documentState.operations.at(-1)!;
      const item = inkOutboxItem(this.documentState, operation, inkId(), this.currentSession.session_id, this.currentSession.writer_epoch, this.logicalHead, operation.seq);
      this.logicalHead = operation.seq;
      this.persistenceState = "pending";
      this.pendingCount = Math.max(1, this.logicalHead - (this.confirmedHeads.get(this.documentState.document_id) ?? 0));
      this.renderViewState();
      this.bufferedOperations.set(item.request_id, item);
      this.enqueueTail = this.enqueueTail.then(async () => {
        try { await this.outbox!.enqueue(item); this.bufferedOperations.delete(item.request_id); }
        catch (error) { this.persistenceState = "error"; this.status("本地保存队列失败：" + errorText(error), true); }
      });
      void this.enqueueTail.then(() => this.flush());
      this.board?.refresh();
      this.renderSummary();
    } catch (error) {
      this.status(errorText(error), true);
    }
  }

  private async flush(message?: string): Promise<boolean> {
    if (!this.outbox) return false;
    if (message && this.bufferedOperations.size) {
      for (const item of this.bufferedOperations.values()) {
        this.enqueueTail = this.enqueueTail.then(async () => {
          try { await this.outbox!.enqueue(item); this.bufferedOperations.delete(item.request_id); }
          catch { /* The original item stays available for an explicit retry. */ }
        });
      }
    }
    if (this.activeFlush) return this.activeFlush;
    const task = this.flushAll(message);
    this.activeFlush = task;
    try { return await task; }
    finally { if (this.activeFlush === task) this.activeFlush = undefined; }
  }

  private async flushAll(message?: string): Promise<boolean> {
    if (!this.outbox) return false;
    const documentId = this.documentState?.document_id;
    if (!this.readOnly) { this.persistenceState = "syncing"; this.renderViewState(); }
    try {
      let sent = 0;
      for (;;) {
        await this.enqueueTail;
        if (this.bufferedOperations.size) throw new Error("仍有操作未成功写入本地恢复队列，请重试立即保存。");
        const result = await this.outbox.flush({ submit: async item => {
          const ack = await this.transport.submit(item);
          if (ack.durable && ack.request_id === item.request_id && ack.document_id === item.document_id && ack.seq === item.operation.seq && ack.command_id === item.operation.command_id) {
            this.confirmedHeads.set(ack.document_id, Math.max(this.confirmedHeads.get(ack.document_id) ?? 0, ack.seq));
            const summary = this.sessionSummaries.find(summary => summary.document_id === ack.document_id);
            if (summary) { summary.head_seq = ack.seq; summary.updated_at = new Date().toISOString(); }
          }
          return ack;
        }});
        sent += result.sent;
        await this.enqueueTail;
        if (!(await this.outbox.pending()).length && !this.bufferedOperations.size) break;
      }
      await this.updatePendingState("saved");
      this.renderSessionList(this.sessionSummaries);
      if (!this.readOnly && this.documentState?.document_id === documentId && (sent || message)) this.status(message ?? "桌面已保存所有已完成操作。", false);
      return true;
    } catch (error) {
      // Each document syncs on its own; a failure elsewhere does not mark the
      // open document as failed (§13.7).
      const failures = error instanceof InkOutboxFlushError ? error.failures : [];
      const own = failures.find(failure => failure.document_id === documentId);
      if (failures.length && !own && documentId && !(await this.outbox.pending(documentId)).length && ![...this.bufferedOperations.values()].some(item => item.document_id === documentId)) {
        await this.updatePendingState("saved");
        this.renderSessionList(this.sessionSummaries);
        return true;
      }
      await this.updatePendingState("error");
      if (!this.readOnly && this.documentState?.document_id === documentId) {
        if (own?.quarantined || errorCode(own?.error) === "document_trashed") this.status("这份手稿已移到回收站，未保存的操作保留在本机恢复队列。", true);
        else this.status("保存暂时失败，待处理操作已保留；可稍后重试。" + errorText(own?.error ?? error), true);
      }
      return false;
    }
  }

  private async createSnapshot(): Promise<void> {
    if (!this.documentState || this.currentSnapshot) return;
    this.board?.finish();
    const documentId = this.documentState.document_id;
    const expectedHead = this.logicalHead;
    const saved = await this.flush();
    if (!saved) return;
    try {
      const snapshot = await createInkSnapshot(inkId(), documentId, expectedHead);
      eventBus.emit("source-map-changed", { reason: "ink-snapshot" });
      this.status("快照已保存，截止 head " + snapshot.through_seq + "。", false);
      await this.refreshSnapshots();
    } catch (error) {
      this.status("保存快照失败：" + errorText(error), true);
    }
  }

  private async continueCurrentDocument(): Promise<void> {
    if (!this.documentState) return;
    const id = this.documentState.document_id;
    const summary = this.sessionSummaries.find(item => item.document_id === id);
    if (summary) { await this.openDocument(summary); return; }
    const trashed = this.trashedSummaries.find(item => item.document_id === id);
    if (trashed) await this.openTrashed(trashed);
    else this.status("当前手稿列表尚未加载完成。", true);
  }

  private exportJson(): void {
    if (!this.documentState) return;
    try {
      const json = encodeInk(this.documentState);
      const suffix = this.readOnly ? `snapshot-head-${this.logicalHead}` : "current";
      download(new Blob([json], { type: "application/json" }), `mathscape-ink-${this.documentState.document_id}-${suffix}.json`);
      this.status(this.readOnly ? "历史快照 Ink JSON 已生成下载。" : "当前手稿 Ink JSON 已生成下载。", false);
    } catch (error) {
      this.status("Ink JSON 导出失败：" + errorText(error), true);
    }
  }

  private async exportPng(): Promise<void> {
    if (!this.board || !this.documentState) return;
    try {
      const canvas = this.board.png();
      const blob = await new Promise<Blob | null>(resolve => canvas.toBlob(resolve, "image/png"));
      if (!blob) throw new Error("PNG 生成失败");
      const suffix = this.readOnly ? `snapshot-head-${this.logicalHead}` : "current";
      download(blob, `mathscape-ink-${this.documentState.document_id}-${suffix}.png`);
      this.status("当前页面 PNG 预览已生成下载；完整编辑历史请使用 Ink JSON。", false);
    } catch (error) {
      this.status("PNG 导出失败：" + errorText(error), true);
    }
  }

  private addPage(): void {
    if (!this.documentState) return;
    const page: InkPage = { id: inkId(), width: 1200, height: 1600 };
    this.documentState.pages.push(page);
    this.commit({ kind: "page_add", page });
    this.currentPage = page.id;
    this.renderPages();
    this.board?.fit();
  }

  private setTool(tool: InkTool): void {
    this.tool = tool;
    for (const value of ["pen", "erase", "partial_erase", "select"] as const) {
      const button = element<HTMLButtonElement>(this.host, "#desktop-ink-" + value);
      button.classList.toggle("is-active", value === tool);
    }
  }

  private history(kind: "undo" | "redo"): void {
    if (!this.documentState) return;
    const target = projectInk(this.documentState)[kind].at(-1);
    if (!target) return;
    this.commit({ kind, target_command_id: target });
  }

  private renderPages(): void {
    if (!this.documentState) return;
    const select = element<HTMLSelectElement>(this.host, "#desktop-ink-page");
    select.replaceChildren();
    this.documentState.pages.forEach((page, index) => {
      const option = document.createElement("option");
      option.value = page.id;
      option.textContent = "第 " + (index + 1) + " 页";
      select.append(option);
    });
    select.value = this.currentPage;
  }

  private renderSummary(): void {
    if (!this.documentState) return;
    const projection = projectInk(this.documentState);
    element<HTMLButtonElement>(this.host, "#desktop-ink-undo").disabled = this.readOnly || !projection.undo.length;
    element<HTMLButtonElement>(this.host, "#desktop-ink-redo").disabled = this.readOnly || !projection.redo.length;
    element(this.host, "#desktop-ink-summary").textContent = projection.visible.length + " 笔 · " + (projection.pointCount + this.metrics.activePoints) + " 点 · head " + this.logicalHead;
    this.renderViewState();
  }

  private async updatePendingState(preferred: PersistenceState): Promise<void> {
    if (!this.documentState || !this.outbox || this.readOnly) return;
    const documentId = this.documentState.document_id;
    const sessionId = this.currentSession?.session_id;
    const pending = (await this.outbox.pending()).filter(item => item.document_id === documentId && (!sessionId || item.session_id === sessionId));
    if (this.documentState?.document_id !== documentId || this.currentSession?.session_id !== sessionId || this.readOnly) return;
    const buffered = [...this.bufferedOperations.values()].filter(item => item.document_id === documentId).length;
    this.pendingCount = Math.max(pending.length + buffered, this.logicalHead - (this.confirmedHeads.get(documentId) ?? 0));
    this.persistenceState = this.pendingCount ? (preferred === "error" ? "error" : "pending") : "saved";
    this.renderViewState();
  }

  private status(message: string, error: boolean): void {
    element(this.host, "#desktop-ink-status").textContent = message;
    if (error) {
      this.renderViewState();
      this.showToast(message, "error");
    } else {
      element(this.host, "#ink-persistence-status").textContent = message;
    }
  }

  private renderViewState(): void {
    if (!this.documentState) return;
    element(this.host, "#ink-document-heading").textContent = this.documentState.title;
    const mode = element(this.host, "#ink-mode-badge");
    const editable = element(this.host, "#ink-editability-badge");
    const saved = element(this.host, "#ink-save-badge");
    mode.textContent = this.currentSnapshot ? "历史快照" : this.trashedView ? "回收站" : this.mirrorMode?"平板镜像":"当前手稿";
    mode.className = "ink-state-badge " + (this.currentSnapshot ? "is-snapshot" : "is-current");
    editable.textContent = this.mirrorMode?"电脑只读":this.readOnly ? "只读" : "可编辑";
    editable.className = "ink-state-badge " + (this.readOnly ? "is-readonly" : "is-editable");
    if (this.trashedView) {
      saved.textContent = "已保存 · head " + this.logicalHead;
      saved.className = "ink-state-badge is-snapshot";
    } else if (this.readOnly && this.currentSnapshot) {
      saved.textContent = "截止 head " + this.currentSnapshot.through_seq + " · " + displayTimestamp(this.currentSnapshot.created_at);
      saved.className = "ink-state-badge is-snapshot";
    } else if(this.mirrorMode){
      saved.textContent="电脑已保存 · head "+this.logicalHead;
      saved.className="ink-state-badge is-saved";
    } else {
      const labels: Record<PersistenceState, string> = {
        loading: "正在加载",
        saved: "桌面已保存 · head " + this.logicalHead,
        pending: "待保存 " + this.pendingCount + " 个操作",
        syncing: "保存中…",
        error: "保存失败 · 待处理 " + this.pendingCount,
      };
      saved.textContent = labels[this.persistenceState];
      saved.className = "ink-state-badge " + (this.persistenceState === "saved" ? "is-saved" : this.persistenceState === "error" ? "is-error" : "is-pending");
    }
  }

  /** With `documentId`, a setup hand-off may target a manuscript that is not
   * open for editing; local writes are flushed and the service reads its head. */
  private async preparePad(documentId?:string):Promise<{documentId:string;head:number|null}> {
    if(documentId&&(this.documentState?.document_id!==documentId||this.currentSnapshot)){
      this.board?.finish();if(!await this.flush())throw new Error("请先完成本机保存，再交给平板书写。");
      return {documentId,head:null};
    }
    if(!this.documentState||this.currentSnapshot)throw new Error("请先打开当前可编辑手稿。");
    this.board?.finish();if(!await this.flush())throw new Error("请先完成本机保存，再交给平板书写。");
    return {documentId:this.documentState.document_id,head:this.logicalHead};
  }
  private async importRecovery():Promise<void> {
    try{
      this.board?.finish();await this.flush();
      const session=await safeInvoke<SessionRecord|null>("choose_ink_recovery",{requestId:inkId()});
      if(!session)return;
      const stored=await openInkDocument(session.document_id);if(!stored)throw new Error("恢复草稿不存在。");
      this.note=stored.note;this.listView="active";this.navigationRevision++;await this.mountDocument(stored.document,session,stored.head_seq);
      await this.refreshDocuments();this.status("恢复文件已导入为独立草稿，原手稿与旧会话保持原有记录。",false);
    }catch(error){this.status("导入恢复草稿失败："+errorText(error),true);}
  }
  private async refreshMirror(force=false):Promise<void> {
    if(!this.documentState||this.currentSnapshot||this.trashedView||this.mirrorPolling)return;
    this.mirrorPolling=true;const id=this.documentState.document_id,revision=this.navigationRevision;
    try {
      const after=Math.min(this.logicalHead,this.confirmedHeads.get(id)??this.logicalHead);
      const mirror=await inkLanMirror(id,after);
      if(this.documentState?.document_id!==id||revision!==this.navigationRevision||this.currentSnapshot)return;
      this.writerView=mirror.delta.writer;this.writerPresence={connected:mirror.connected,pending:mirror.client?.pending_count??null};
      const note=mirror.delta.note;
      if(note&&note.document_id===id&&this.note&&note.title_revision>this.note.title_revision)this.applyNote(note);
      if(mirror.delta.snapshot_count!==this.snapshotCount){this.snapshotCount=mirror.delta.snapshot_count;void this.refreshSnapshots();}
      const observe=mirror.delta.writer.remote||mirror.delta.writer.pairing_pending;
      if(!observe){
        if(this.mirrorMode){const summary=this.sessionSummaries.find(item=>item.document_id===id);if(summary)await this.openDocument(summary);}
        return;
      }
      if(!this.mirrorMode){
        const stored=await openInkDocument(id);if(stored&&revision===this.navigationRevision)await this.mountDocument(stored.document,mirror.delta.writer.session??undefined,stored.head_seq,true);
      }else{
        for(const operation of mirror.delta.operations){if(operation.seq>this.documentState!.operations.length){if(operation.seq!==this.documentState!.operations.length+1)throw new Error("镜像历史出现序号缺口，请重新打开手稿。");this.documentState!.operations.push(operation);}}
        Object.assign(this.documentState!,mirror.delta.metadata);
        if(this.note)this.documentState!.title=this.note.title;
        this.logicalHead=this.documentState!.operations.length;this.confirmedHeads.set(id,this.logicalHead);
        if(!this.documentState!.pages.some(page=>page.id===this.currentPage))this.currentPage=this.documentState!.pages[0].id;
        const oldPage=this.currentPage;
        const padPage=mirror.preview?.page_id??mirror.client?.page_id;
        if(this.followPadPage&&padPage&&this.documentState!.pages.some(page=>page.id===padPage))this.currentPage=padPage;
        this.renderPages();this.remotePreview=mirror.preview??undefined;if(this.currentPage!==oldPage)this.board?.fit();else this.board?.refresh();this.renderSummary();
        const summary=this.sessionSummaries.find(item=>item.document_id===id);if(summary){summary.head_seq=mirror.delta.server_head;summary.updated_at=new Date().toISOString();this.renderSessionList(this.sessionSummaries);}
      }
      element<HTMLButtonElement>(this.host,"#ink-snapshot").disabled=this.logicalHead!==mirror.delta.server_head;
      this.status(mirror.delta.writer.pairing_pending?"等待平板扫码。":mirror.connected?`平板已连接，电脑显示镜像；平板待同步 ${mirror.client?.pending_count??0} 个操作。`:"平板暂时离线；已同步笔迹保留，恢复连接后继续同步。",false);
    }catch(error){if(force)this.status(errorText(error),true);}
    finally{this.mirrorPolling=false;}
  }
}
