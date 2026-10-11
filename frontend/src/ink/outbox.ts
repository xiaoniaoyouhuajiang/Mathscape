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

export class InkOutboxFlushError extends Error {
  constructor(
    message: string,
    readonly sent: number,
    readonly remaining: number,
    readonly cause?: unknown,
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
      if (JSON.stringify(existing) !== JSON.stringify(item)) throw new InkOutboxConflict(item.request_id);
      return;
    }
    await this.store.put(item);
  }

  async pending(): Promise<InkOutboxItem[]> {
    return this.ordered(await this.store.list());
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

  private async flushSerial(transport: InkOperationTransport): Promise<InkOutboxFlushResult> {
    const acknowledgements: InkOperationAck[] = [];
    let sent = 0;
    const pending = this.ordered(await this.store.list());
    for (const item of pending) {
      try {
        const acknowledgement = await transport.submit(item);
        if (!acknowledgement.durable || acknowledgement.request_id !== item.request_id) {
          throw new Error("server response is not a durable ACK for this request");
        }
        await this.store.remove(item.request_id);
        acknowledgements.push(acknowledgement);
        sent++;
      } catch (error) {
        const remaining = (await this.store.list()).length;
        throw new InkOutboxFlushError("Ink outbox flush stopped; pending items were retained", sent, remaining, error);
      }
    }
    return { sent, remaining: 0, acknowledgements };
  }

  private ordered(items: InkOutboxItem[]): InkOutboxItem[] {
    return [...items].sort((a, b) => a.client_seq - b.client_seq || a.queued_at.localeCompare(b.queued_at) || a.request_id.localeCompare(b.request_id));
  }
}
