import type { InkDocumentMetadata, InkOperation } from "./model";

export interface InkOutboxItem {
  request_id: string;
  document_id: string;
  session_id: string;
  writer_epoch: number;
  expected_head: number;
  client_seq: number;
  queued_at: string;
  metadata: InkDocumentMetadata;
  operation: InkOperation;
  /** Set when the server refused the document for good (for example it was
   * moved to the trash). The item stays stored for recovery export but is no
   * longer submitted, so it cannot block other documents. */
  quarantined?: InkOutboxQuarantine;
}

export interface InkOutboxQuarantine {
  code: string;
  message: string;
  at: string;
}

/** Server codes after which a document's queued writes can never succeed. */
export const TERMINAL_INK_CODES = new Set(["document_trashed", "writer_epoch_conflict", "idempotency_conflict", "document_not_found"]);

function errorCodeOf(error: unknown): string | undefined {
  const code = (error as { code?: unknown } | null)?.code;
  return typeof code === "string" ? code : undefined;
}

export interface InkOperationAck {
  request_id: string;
  document_id: string;
  command_id: string;
  seq: number;
  durable: true;
  replayed?: boolean;
}

export interface InkOperationTransport {
  submit(item: InkOutboxItem): Promise<InkOperationAck>;
}

export interface InkOutboxStore {
  put(item: InkOutboxItem): Promise<void>;
  get(requestId: string): Promise<InkOutboxItem | undefined>;
  list(): Promise<InkOutboxItem[]>;
  remove(requestId: string): Promise<void>;
}

export class InkOutboxConflict extends Error {
  constructor(requestId: string) {
    super(`outbox request already exists with different content: ${requestId}`);
    this.name = "InkOutboxConflict";
  }
}

export interface InkOutboxDocumentFailure {
  document_id: string;
  error: unknown;
  /** The document's items were quarantined after a terminal server code. */
  quarantined: boolean;
}

export class InkOutboxFlushError extends Error {
  constructor(
    message: string,
    readonly sent: number,
    readonly remaining: number,
    readonly cause?: unknown,
    readonly failures: InkOutboxDocumentFailure[] = [],
  ) {
    super(message);
    this.name = "InkOutboxFlushError";
  }
}

export class MemoryInkOutboxStore implements InkOutboxStore {
  private readonly values = new Map<string, InkOutboxItem>();

  async put(item: InkOutboxItem): Promise<void> {
    this.values.set(item.request_id, structuredClone(item));
  }

  async get(requestId: string): Promise<InkOutboxItem | undefined> {
    const item = this.values.get(requestId);
    return item ? structuredClone(item) : undefined;
  }

  async list(): Promise<InkOutboxItem[]> {
    return [...this.values.values()].map(item => structuredClone(item));
  }

  async remove(requestId: string): Promise<void> {
    this.values.delete(requestId);
  }
}

class IndexedDbInkOutboxStore implements InkOutboxStore {
  private constructor(private readonly database: IDBDatabase) {}

  static async open(name = "mathscape-ink-p1", version = 1): Promise<IndexedDbInkOutboxStore> {
    const database = await new Promise<IDBDatabase>((resolve, reject) => {
      const request = indexedDB.open(name, version);
      request.onupgradeneeded = () => {
        if (!request.result.objectStoreNames.contains("operations")) {
          request.result.createObjectStore("operations", { keyPath: "request_id" });
        }
      };
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error ?? new Error("IndexedDB open failed"));
      request.onblocked = () => reject(new Error("IndexedDB open blocked"));
    });
    return new IndexedDbInkOutboxStore(database);
  }

  async put(item: InkOutboxItem): Promise<void> {
    await this.write("put", item);
  }

  async get(requestId: string): Promise<InkOutboxItem | undefined> {
    return await this.read(requestId);
  }

  async list(): Promise<InkOutboxItem[]> {
    return await new Promise<InkOutboxItem[]>((resolve, reject) => {
      const transaction = this.database.transaction("operations", "readonly");
      const request = transaction.objectStore("operations").getAll();
      request.onsuccess = () => resolve(request.result as InkOutboxItem[]);
      request.onerror = () => reject(request.error ?? new Error("IndexedDB list failed"));
    });
  }

  async remove(requestId: string): Promise<void> {
    await this.write("delete", requestId);
  }

  private async write(kind: "put" | "delete", value: InkOutboxItem | string): Promise<void> {
    await new Promise<void>((resolve, reject) => {
      const transaction = this.database.transaction("operations", "readwrite");
      const store = transaction.objectStore("operations");
      if (kind === "put") store.put(value as InkOutboxItem);
      else store.delete(value as string);
      transaction.oncomplete = () => resolve();
      transaction.onerror = () => reject(transaction.error ?? new Error("IndexedDB write failed"));
      transaction.onabort = () => reject(transaction.error ?? new Error("IndexedDB write aborted"));
    });
  }

  private async read(requestId: string): Promise<InkOutboxItem | undefined> {
    return await new Promise<InkOutboxItem | undefined>((resolve, reject) => {
      const transaction = this.database.transaction("operations", "readonly");
      const request = transaction.objectStore("operations").get(requestId);
      request.onsuccess = () => resolve(request.result as InkOutboxItem | undefined);
      request.onerror = () => reject(request.error ?? new Error("IndexedDB read failed"));
    });
  }
}

export interface InkOutboxFlushResult {
  sent: number;
  remaining: number;
  acknowledgements: InkOperationAck[];
}

export class InkOutbox {
  private activeFlush: Promise<InkOutboxFlushResult> | undefined;

  constructor(
    private readonly store: InkOutboxStore = new MemoryInkOutboxStore(),
    readonly persistent = false,
  ) {}

  static async openDefault(): Promise<InkOutbox> {
    if (typeof indexedDB === "undefined") return new InkOutbox();
    try {
      return new InkOutbox(await IndexedDbInkOutboxStore.open(), true);
    } catch {
      // Private browsing and WebViews can expose IndexedDB but reject writes.
      // The caller still gets an explicit in-memory outbox rather than a false
      // promise of refresh recovery.
      return new InkOutbox();
    }
  }

  async enqueue(item: InkOutboxItem): Promise<void> {
    const existing = await this.store.get(item.request_id);
    if (existing) {
      // A quarantine marker is local bookkeeping, not part of the request.
      const { quarantined: _marker, ...stored } = existing;
      const { quarantined: _ignored, ...incoming } = item;
      if (JSON.stringify(stored) !== JSON.stringify(incoming)) throw new InkOutboxConflict(item.request_id);
      return;
    }
    await this.store.put(item);
  }

  /** Items that will still be submitted, in client order. */
  async pending(documentId?: string): Promise<InkOutboxItem[]> {
    return this.ordered((await this.store.list()).filter(item => !item.quarantined && (!documentId || item.document_id === documentId)));
  }

  /** Items kept only for recovery after a terminal server refusal. */
  async quarantined(documentId?: string): Promise<InkOutboxItem[]> {
    return this.ordered((await this.store.list()).filter(item => !!item.quarantined && (!documentId || item.document_id === documentId)));
  }

  /** A1 (§7.2): removes items the user explicitly chose to discard, whether
   * pending or quarantined. Waits for a running flush so an item is never
   * removed while it is being submitted. Returns how many were removed. */
  async discard(requestIds: Iterable<string>): Promise<number> {
    if (this.activeFlush) await this.activeFlush.catch(() => undefined);
    let removed = 0;
    for (const requestId of requestIds) {
      if (!(await this.store.get(requestId))) continue;
      await this.store.remove(requestId);
      removed++;
    }
    return removed;
  }

  async flush(transport: InkOperationTransport): Promise<InkOutboxFlushResult> {
    if (this.activeFlush) return await this.activeFlush;
    const task = this.flushSerial(transport);
    this.activeFlush = task;
    try {
      return await task;
    } finally {
      this.activeFlush = undefined;
    }
  }

  /** Submits each document's queue in order. A failure only stops the
   * document it belongs to; other documents keep syncing. */
  private async flushSerial(transport: InkOperationTransport): Promise<InkOutboxFlushResult> {
    const acknowledgements: InkOperationAck[] = [];
    const failures: InkOutboxDocumentFailure[] = [];
    let sent = 0;
    const groups = new Map<string, InkOutboxItem[]>();
    for (const item of await this.pending()) {
      const group = groups.get(item.document_id);
      if (group) group.push(item);
      else groups.set(item.document_id, [item]);
    }
    for (const [documentId, items] of groups) {
      for (const item of items) {
        try {
          const acknowledgement = await transport.submit(item);
          if (!acknowledgement.durable || acknowledgement.request_id !== item.request_id
              || acknowledgement.document_id !== item.document_id || acknowledgement.command_id !== item.operation.command_id || acknowledgement.seq !== item.operation.seq) {
            throw new Error("server response is not a durable ACK for this request");
          }
          await this.store.remove(item.request_id);
          acknowledgements.push(acknowledgement);
          sent++;
        } catch (error) {
          const code = errorCodeOf(error);
          const terminal = !!code && TERMINAL_INK_CODES.has(code);
          if (terminal) await this.quarantine(documentId, code!, error instanceof Error ? error.message : String(error));
          failures.push({ document_id: documentId, error, quarantined: terminal });
          break;
        }
      }
    }
    if (failures.length) {
      const remaining = (await this.pending()).length;
      throw new InkOutboxFlushError("Ink outbox flush stopped; pending items were retained", sent, remaining, failures[0].error, failures);
    }
    return { sent, remaining: 0, acknowledgements };
  }

  private async quarantine(documentId: string, code: string, message: string): Promise<void> {
    const at = new Date().toISOString();
    for (const item of await this.store.list()) {
      if (item.document_id === documentId && !item.quarantined) await this.store.put({ ...item, quarantined: { code, message, at } });
    }
  }

  private ordered(items: InkOutboxItem[]): InkOutboxItem[] {
    return [...items].sort((a, b) => a.client_seq - b.client_seq || a.queued_at.localeCompare(b.queued_at) || a.request_id.localeCompare(b.request_id));
  }
}
