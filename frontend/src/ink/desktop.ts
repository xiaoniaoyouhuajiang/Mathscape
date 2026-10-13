import { InkCanvas, type CanvasMetrics, type InkTool } from "./canvas";
import {
  appendInk,
  encodeInk,
  inkId,
  newDocument,
  parseInkDocument,
  parseInkSnapshotManifest,
  projectInk,
  type CaptureSegment,
  type InkBrush,
  type InkDocument,
  type InkPage,
} from "./model";
import {
  InkDesktopTransport,
  createInkSnapshot,
  createInkDocument,
  inkOutboxItem,
  listInkDocuments,
  listInkSessions,
  listInkSnapshots,
  openInkDocument,
  startInkSession,
  openInkSnapshot,
  type StoredInkSnapshot,
} from "./persistence";
import { InkOutbox, type InkOutboxItem } from "./outbox";
import { InkEvidencePanel } from "./evidence-panel";
import { eventBus } from "../core/events";
import type { InkSelection } from "./persistence";

interface SessionSummary {
  document_id: string;
  title: string;
  head_seq: number;
  active_session_id: string | null;
  updated_at: string;
}

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
  private loading = false;
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
    await this.refreshDocuments();
  }

  close(): void {
    this.navigationRevision++;
    this.board?.resetTransientInteraction();
    void this.flush();
    this.onClose();
  }

  async openDocumentId(documentId: string): Promise<void> {
    await this.open();
    const summary = this.sessionSummaries.find(item => item.document_id === documentId);
    if (summary) await this.openDocument(summary);
    else this.status("手稿不存在或已被移除。", true);
  }

  private render(): void {
    this.host.innerHTML = [
      '<div class="ink-desktop-shell">',
      '<header class="ink-desktop-header"><div class="ink-document-heading"><span class="eyebrow">INK DOCUMENT</span><h2 id="ink-document-heading">数学手稿</h2><div class="ink-state-row"><span id="ink-mode-badge" class="ink-state-badge">未打开</span><span id="ink-editability-badge" class="ink-state-badge">—</span><span id="ink-save-badge" class="ink-state-badge">正在加载</span></div></div>',
      '<div class="ink-desktop-actions"><span id="ink-persistence-status" class="ink-persistence-status">正在加载本地手稿…</span><button id="ink-continue-current" class="secondary-button" type="button" hidden>继续编辑当前手稿</button><button id="ink-save-now" class="secondary-button" type="button">立即保存</button><button id="ink-snapshot" class="secondary-button" type="button">保存快照</button><button id="ink-close" class="secondary-button" type="button">返回地图</button></div></header>',
      '<div class="ink-desktop-body"><aside class="ink-session-panel"><div class="ink-session-heading"><strong>手稿会话</strong><button id="ink-new-document" class="secondary-button" type="button">新建</button></div><div id="ink-session-list" class="ink-session-list"><p>正在读取…</p></div><div class="ink-snapshot-heading"><strong>历史快照</strong></div><div id="ink-snapshot-list" class="ink-session-list"><p>请选择手稿。</p></div></aside>',
      '<main class="ink-desktop-main"><nav class="ink-desktop-toolbar" aria-label="手稿工具"><div class="ink-tool-buttons">',
      '<button id="desktop-ink-pen" class="is-active" type="button">笔</button><button id="desktop-ink-erase" type="button">整笔擦除</button><button id="desktop-ink-partial_erase" type="button">局部擦除</button><button id="desktop-ink-select" type="button">套索选择</button><button id="desktop-ink-undo" type="button" disabled>撤销</button><button id="desktop-ink-redo" type="button" disabled>重做</button>',
      '</div><div class="ink-brush-controls"><label title="之后创建的笔迹使用此颜色">颜色<input id="desktop-ink-color" type="color" value="#b9e3d8" aria-label="笔迹颜色"></label><label title="之后创建的笔迹使用此笔宽">笔宽<input id="desktop-ink-width" type="range" min="1" max="16" step="0.5" value="4" aria-label="笔迹宽度"><output id="desktop-ink-width-value">4</output></label><label class="ink-check-control"><input id="desktop-ink-finger" type="checkbox">手指书写</label></div><div class="ink-page-controls"><select id="desktop-ink-page" aria-label="手稿页面"></select><button id="desktop-ink-add-page" class="secondary-button" type="button">新增页面</button><button id="desktop-ink-zoom-out" class="secondary-button" type="button" aria-label="缩小">−</button><button id="desktop-ink-fit" class="secondary-button" type="button">整页</button><button id="desktop-ink-zoom-in" class="secondary-button" type="button" aria-label="放大">＋</button></div></nav>',
      '<div id="desktop-ink-evidence" hidden></div><div id="desktop-ink-board" class="ink-desktop-board" aria-label="数学手稿画板"></div><footer class="ink-desktop-footer"><input id="desktop-ink-title" class="ink-desktop-title" maxlength="200" value="数学手稿" aria-label="手稿标题" readonly><span id="desktop-ink-status">请选择一个手稿或新建手稿。</span><span id="desktop-ink-summary">—</span><div class="ink-export-actions"><button id="desktop-ink-export-json" class="secondary-button" type="button">导出 Ink JSON</button><button id="desktop-ink-export-png" class="secondary-button" type="button">导出 PNG</button></div></footer></main></div></div>',
    ].join("");
    element<HTMLButtonElement>(this.host, "#ink-close").addEventListener("click", () => this.close());
    element<HTMLButtonElement>(this.host, "#ink-save-now").addEventListener("click", () => void this.flush("手动保存完成。"));
    element<HTMLButtonElement>(this.host, "#ink-snapshot").addEventListener("click", () => void this.createSnapshot());
    element<HTMLButtonElement>(this.host, "#ink-continue-current").addEventListener("click", () => void this.continueCurrentDocument());
    element<HTMLButtonElement>(this.host, "#ink-new-document").addEventListener("click", () => void this.createDocument());
    element<HTMLButtonElement>(this.host, "#desktop-ink-add-page").addEventListener("click", () => this.addPage());
    element<HTMLSelectElement>(this.host, "#desktop-ink-page").addEventListener("change", event => {
      const value = (event.target as HTMLSelectElement).value;
      if (!this.documentState || !value) return;
      this.board?.resetTransientInteraction();
      this.currentPage = value;
      this.board?.setSelection([]);
      this.board?.fit();
      this.renderSummary();
    });
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

  private async refreshDocuments(): Promise<void> {
    if (this.loading) return;
    this.loading = true;
    try {
      const documents = await listInkDocuments();
      this.sessionSummaries = documents;
      this.renderSessionList(documents);
      if (!this.documentState) {
        const first = documents[0];
        if (first) await this.openDocument(first);
        else await this.createDocument();
      }
    } catch (error) {
      this.status("读取手稿失败：" + errorText(error), true);
    } finally {
      this.loading = false;
    }
  }

  private renderSessionList(documents: SessionSummary[]): void {
    const list = element(this.host, "#ink-session-list");
    list.replaceChildren();
    if (!documents.length) {
      const empty = document.createElement("p");
      empty.textContent = "还没有本地手稿。";
      list.append(empty);
      return;
    }
    for (const summary of documents) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "ink-session-item";
      const isCurrentDocument = summary.document_id === this.documentState?.document_id;
      button.classList.toggle("is-active", isCurrentDocument && !this.readOnly);
      button.classList.toggle("is-context", isCurrentDocument && this.readOnly);
      if (isCurrentDocument) button.setAttribute("aria-current", this.readOnly ? "location" : "true");
      button.innerHTML = '<strong></strong><span></span>';
      button.querySelector("strong")!.textContent = summary.title;
      button.querySelector("span")!.textContent = summary.head_seq + " 个已保存操作 · " + displayTimestamp(summary.updated_at);
      button.addEventListener("click", () => void this.openDocument(summary));
      list.append(button);
    }
  }

  private async createDocument(): Promise<void> {
    const revision = ++this.navigationRevision;
    this.board?.finish();
    await this.flush();
    this.status("正在创建本地手稿…", false);
    const documentState = newDocument();
    try {
      const session = await createInkDocument(documentState, inkId(), this.writerId, { source: "desktop" });
      if (revision !== this.navigationRevision) return;
      await this.mountDocument(documentState, session, 0, false, undefined, revision);
      this.status("手稿已创建，书写后会自动保存。", false);
      await this.refreshDocuments();
    } catch (error) {
      this.status("创建手稿失败：" + errorText(error), true);
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
      const sessions = summary.active_session_id ? [] : await listInkSessions(summary.document_id);
      const sessionId = summary.active_session_id ?? sessions[0]?.session_id ?? inkId();
      const session = await startInkSession(summary.document_id, sessionId, this.writerId, { source: "desktop" });
      if (revision !== this.navigationRevision) return;
      await this.mountDocument(stored.document, session, stored.head_seq, false, undefined, revision);
      this.status("手稿已恢复；新操作会自动进入本地保存队列。", false);
    } catch (error) {
      this.status("恢复手稿失败：" + errorText(error), true);
    }
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

  private async mountDocument(documentState: InkDocument, session: SessionRecord | undefined, head: number, readOnly = false, snapshot?: StoredInkSnapshot, revision = this.navigationRevision): Promise<void> {
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
    this.currentSnapshot = snapshot;
    this.readOnly = readOnly;
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
    element<HTMLButtonElement>(this.host, "#ink-continue-current").hidden = !readOnly;
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
      documentState.version = item.metadata.version;
      documentState.title = item.metadata.title;
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
      await this.updatePendingState("error");
      if (!this.readOnly && this.documentState?.document_id === documentId) this.status("保存暂时失败，待处理操作已保留；可稍后重试。" + errorText(error), true);
      return false;
    }
  }

  private async createSnapshot(): Promise<void> {
    if (!this.documentState || this.readOnly) return;
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
    const summary = this.sessionSummaries.find(item => item.document_id === this.documentState!.document_id);
    if (summary) await this.openDocument(summary);
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
    mode.textContent = this.readOnly ? "历史快照" : "当前手稿";
    mode.className = "ink-state-badge " + (this.readOnly ? "is-snapshot" : "is-current");
    editable.textContent = this.readOnly ? "只读" : "可编辑";
    editable.className = "ink-state-badge " + (this.readOnly ? "is-readonly" : "is-editable");
    if (this.readOnly && this.currentSnapshot) {
      saved.textContent = "截止 head " + this.currentSnapshot.through_seq + " · " + displayTimestamp(this.currentSnapshot.created_at);
      saved.className = "ink-state-badge is-snapshot";
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
}
