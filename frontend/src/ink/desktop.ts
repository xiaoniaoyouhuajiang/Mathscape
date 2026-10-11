import { InkCanvas, type CanvasMetrics, type InkTool } from "./canvas";
import {
  appendInk,
  inkId,
  newDocument,
  parseInkDocument,
  projectInk,
  type CaptureSegment,
  type InkBrush,
  type InkDocument,
  type InkPage,
} from "./model";
import {
  InkDesktopTransport,
  createInkDocument,
  inkOutboxItem,
  listInkDocuments,
  listInkSessions,
  openInkDocument,
  startInkSession,
} from "./persistence";
import { InkOutbox, type InkOutboxFlushError } from "./outbox";

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

export class DesktopInkWorkspace {
  private documentState: InkDocument | undefined;
  private currentPage = "";
  private currentSession: SessionRecord | undefined;
  private currentSegment: CaptureSegment | undefined;
  private board: InkCanvas | undefined;
  private outbox: InkOutbox | undefined;
  private readonly transport = new InkDesktopTransport();
  private readonly writerId = "desktop-local";
  private tool: InkTool = "pen";
  private logicalHead = 0;
  private loading = false;
  private sessionSummaries: SessionSummary[] = [];
  private metrics: CanvasMetrics = { pointerEvents: 0, coalescedSamples: 0, cancelled: 0, lastPointer: "—", frameMs: 0, activePoints: 0 };
  private rendered = false;

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
    this.board?.resetTransientInteraction();
    this.onClose();
  }

  private render(): void {
    this.host.innerHTML = [
      '<div class="ink-desktop-shell">',
      '<header class="ink-desktop-header"><div><span class="eyebrow">INK DOCUMENT</span><h2>数学手稿</h2></div>',
      '<div class="ink-desktop-actions"><span id="ink-persistence-status" class="ink-persistence-status">正在加载本地手稿…</span><button id="ink-save-now" class="secondary-button" type="button">立即保存</button><button id="ink-close" class="secondary-button" type="button">返回地图</button></div></header>',
      '<div class="ink-desktop-body"><aside class="ink-session-panel"><div class="ink-session-heading"><strong>手稿会话</strong><button id="ink-new-document" class="secondary-button" type="button">新建</button></div><div id="ink-session-list" class="ink-session-list"><p>正在读取…</p></div></aside>',
      '<main class="ink-desktop-main"><nav class="ink-desktop-toolbar" aria-label="手稿工具"><div class="ink-tool-buttons">',
      '<button id="desktop-ink-pen" class="is-active" type="button">笔</button><button id="desktop-ink-erase" type="button">整笔擦除</button><button id="desktop-ink-partial_erase" type="button">局部擦除</button><button id="desktop-ink-select" type="button">套索选择</button><button id="desktop-ink-undo" type="button" disabled>撤销</button><button id="desktop-ink-redo" type="button" disabled>重做</button>',
      '</div><div class="ink-page-controls"><select id="desktop-ink-page" aria-label="手稿页面"></select><button id="desktop-ink-add-page" class="secondary-button" type="button">新增页面</button></div></nav>',
      '<div id="desktop-ink-board" class="ink-desktop-board" aria-label="数学手稿画板"></div><footer class="ink-desktop-footer"><input id="desktop-ink-title" class="ink-desktop-title" maxlength="200" value="数学手稿" aria-label="手稿标题" readonly><span id="desktop-ink-status">请选择一个手稿或新建手稿。</span><span id="desktop-ink-summary">—</span></footer></main></div></div>',
    ].join("");
    element<HTMLButtonElement>(this.host, "#ink-close").addEventListener("click", () => this.close());
    element<HTMLButtonElement>(this.host, "#ink-save-now").addEventListener("click", () => void this.flush("手动保存完成。"));
    element<HTMLButtonElement>(this.host, "#ink-new-document").addEventListener("click", () => void this.createDocument());
    element<HTMLButtonElement>(this.host, "#desktop-ink-add-page").addEventListener("click", () => this.addPage());
    element<HTMLSelectElement>(this.host, "#desktop-ink-page").addEventListener("change", event => {
      const value = (event.target as HTMLSelectElement).value;
      if (!this.documentState || !value) return;
      this.board?.resetTransientInteraction();
      this.currentPage = value;
      this.board?.fit();
      this.renderSummary();
    });
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
      button.classList.toggle("is-active", summary.document_id === this.documentState?.document_id);
      button.innerHTML = '<strong></strong><span></span>';
      button.querySelector("strong")!.textContent = summary.title;
      button.querySelector("span")!.textContent = summary.head_seq + " 个已保存操作 · " + new Date(summary.updated_at).toLocaleString();
      button.addEventListener("click", () => void this.openDocument(summary));
      list.append(button);
    }
  }

  private async createDocument(): Promise<void> {
    this.status("正在创建本地手稿…", false);
    const documentState = newDocument();
    try {
      const session = await createInkDocument(documentState, inkId(), this.writerId, { source: "desktop" });
      await this.mountDocument(documentState, session, 0);
      this.status("手稿已创建，书写后会自动保存。", false);
      await this.refreshDocuments();
    } catch (error) {
      this.status("创建手稿失败：" + errorText(error), true);
    }
  }

  private async openDocument(summary: SessionSummary): Promise<void> {
    this.status("正在恢复手稿…", false);
    try {
      await this.flush();
      const stored = await openInkDocument(summary.document_id);
      if (!stored) throw new Error("手稿不存在或已被移除");
      const sessions = summary.active_session_id ? [] : await listInkSessions(summary.document_id);
      const sessionId = summary.active_session_id ?? sessions[0]?.session_id ?? inkId();
      const session = await startInkSession(summary.document_id, sessionId, this.writerId, { source: "desktop" });
      await this.mountDocument(stored.document, session, stored.head_seq);
      this.status("手稿已恢复；新操作会自动进入本地保存队列。", false);
    } catch (error) {
      this.status("恢复手稿失败：" + errorText(error), true);
    }
  }

  private async mountDocument(documentState: InkDocument, session: SessionRecord, head: number): Promise<void> {
    this.board?.dispose();
    this.documentState = documentState;
    this.currentPage = documentState.pages[0]?.id ?? "";
    this.currentSession = session;
    this.currentSegment = undefined;
    this.logicalHead = head;
    await this.replayPending(documentState, session.session_id, head);
    this.renderSessionList(this.sessionSummaries);
    element<HTMLInputElement>(this.host, "#desktop-ink-title").value = documentState.title;
    this.renderPages();
    this.board = new InkCanvas(element(this.host, "#desktop-ink-board"), {
      document: () => this.documentState!,
      page: () => this.documentState!.pages.find(page => page.id === this.currentPage)!,
      segmentId: () => this.segmentId(),
      brush: () => this.brush(),
      tool: () => this.tool,
      fingerWriting: () => false,
      onStroke: stroke => this.commit({ kind: "stroke", stroke }),
      onErase: strokeIds => this.commit({ kind: "erase", stroke_ids: strokeIds }),
      onPartialErase: (sourceStrokeId, fragments) => this.commit(fragments.length ? { kind: "partial_erase", source_stroke_id: sourceStrokeId, fragments } : { kind: "erase", stroke_ids: [sourceStrokeId] }),
      onTransform: (strokeIds, matrix) => this.commit({ kind: "transform", stroke_ids: strokeIds, matrix }),
      onSelection: () => this.renderSummary(),
      onMessage: message => this.status(message, false),
      onMetrics: metrics => { this.metrics = metrics; this.renderSummary(); },
    });
    this.board.refresh();
    this.renderSummary();
    void this.flush();
  }

  private async replayPending(documentState: InkDocument, sessionId: string, head: number): Promise<void> {
    if (!this.outbox) return;
    const pending = (await this.outbox.pending())
      .filter(item => item.document_id === documentState.document_id && item.session_id === sessionId)
      .sort((a, b) => a.client_seq - b.client_seq);
    for (const item of pending) {
      if (item.operation.seq <= head || item.operation.seq !== this.logicalHead + 1) continue;
      documentState.version = item.metadata.version;
      documentState.title = item.metadata.title;
      documentState.exported_at = item.metadata.exported_at;
      documentState.pages = structuredClone(item.metadata.pages);
      documentState.capture_segments = structuredClone(item.metadata.capture_segments);
      documentState.operations.push(structuredClone(item.operation));
      parseInkDocument(documentState);
      this.logicalHead = item.operation.seq;
    }
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
    return { renderer: "perfect-freehand@1.2.3", color: "#b9e3d8", width: 4, smoothing: 0.35, streamline: 0.15, thinning: 0.4, simulate_pressure: false };
  }

  private commit(command: Parameters<typeof appendInk>[1]): void {
    if (!this.documentState || !this.currentSession || !this.outbox) return;
    try {
      appendInk(this.documentState, command);
      const operation = this.documentState.operations.at(-1)!;
      const item = inkOutboxItem(this.documentState, operation, inkId(), this.currentSession.session_id, this.currentSession.writer_epoch, this.logicalHead, operation.seq);
      this.logicalHead = operation.seq;
      void this.outbox.enqueue(item).then(() => this.flush()).catch(error => this.status("本地保存队列失败：" + errorText(error), true));
      this.board?.refresh();
      this.renderSummary();
    } catch (error) {
      this.status(errorText(error), true);
    }
  }

  private async flush(message?: string): Promise<void> {
    if (!this.outbox) return;
    this.status("正在保存到本地 Workspace…", false);
    try {
      const result = await this.outbox.flush(this.transport);
      if (result.sent || message) this.status(message ?? "已保存 " + result.sent + " 个操作。", false);
    } catch (error) {
      const failure = error as InkOutboxFlushError;
      this.status("保存暂时失败，已保留 " + (failure.remaining ?? "待处理") + " 个操作；可稍后重试。", true);
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
    element<HTMLButtonElement>(this.host, "#desktop-ink-undo").disabled = !projection.undo.length;
    element<HTMLButtonElement>(this.host, "#desktop-ink-redo").disabled = !projection.redo.length;
    element(this.host, "#desktop-ink-summary").textContent = projection.visible.length + " 笔 · " + (projection.pointCount + this.metrics.activePoints) + " 点 · head " + this.logicalHead;
  }

  private status(message: string, error: boolean): void {
    element(this.host, "#desktop-ink-status").textContent = message;
    element(this.host, "#ink-persistence-status").textContent = error ? "本地保存异常" : message;
    if (error) this.showToast(message, "error");
  }
}
