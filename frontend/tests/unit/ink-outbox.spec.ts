import { describe, expect, it } from "vitest";
import trace from "../../../testdata/ink/p0/trace.json";
import { parseInkDocument } from "../../src/ink/model";
import {
  InkOutbox,
  InkOutboxConflict,
  InkOutboxFlushError,
  MemoryInkOutboxStore,
  type InkOutboxItem,
  type InkOperationTransport,
} from "../../src/ink/outbox";

const document = parseInkDocument(structuredClone(trace));
const operation = document.operations[0];

function item(requestId: string, clientSeq: number, expectedHead = clientSeq - 1): InkOutboxItem {
  return {
    request_id: requestId,
    document_id: document.document_id,
    session_id: "session-1",
    writer_epoch: 1,
    expected_head: expectedHead,
    client_seq: clientSeq,
    queued_at: `2026-10-10T00:00:0${clientSeq}.000Z`,
    metadata: {
      version: document.version,
      document_id: document.document_id,
      title: document.title,
      created_at: document.created_at,
      exported_at: document.exported_at,
      pages: document.pages,
      capture_segments: document.capture_segments,
    },
    operation: { ...operation, seq: clientSeq, command_id: "command-" + clientSeq },
  };
}

describe("P1 Ink outbox", () => {
  it("keeps client order and removes only durable acknowledgements", async () => {
    const outbox = new InkOutbox(new MemoryInkOutboxStore());
    await outbox.enqueue(item("r2", 2));
    await outbox.enqueue(item("r1", 1));
    const sent: string[] = [];
    const result = await outbox.flush({
      submit: async value => {
        sent.push(value.request_id);
        return { request_id: value.request_id, document_id: value.document_id, command_id: value.operation.command_id, seq: value.client_seq, durable: true };
      },
    });
    expect(sent).toEqual(["r1", "r2"]);
    expect(result.sent).toBe(2);
    expect(await outbox.pending()).toEqual([]);
  });

  it("retains the failed item and later items for recovery", async () => {
    const outbox = new InkOutbox(new MemoryInkOutboxStore());
    await outbox.enqueue(item("r1", 1));
    await outbox.enqueue(item("r2", 2));
    let attempts = 0;
    await expect(outbox.flush({
      submit: async value => {
        attempts++;
        if (value.request_id === "r1") throw new Error("offline");
        return { request_id: value.request_id, document_id: value.document_id, command_id: value.operation.command_id, seq: value.client_seq, durable: true };
      },
    })).rejects.toBeInstanceOf(InkOutboxFlushError);
    expect(attempts).toBe(1);
    expect((await outbox.pending()).map(value => value.request_id)).toEqual(["r1", "r2"]);
  });

  it("rejects a request id reused for different content", async () => {
    const outbox = new InkOutbox(new MemoryInkOutboxStore());
    await outbox.enqueue(item("same", 1));
    await expect(outbox.enqueue({ ...item("same", 1), expected_head: 99 })).rejects.toBeInstanceOf(InkOutboxConflict);
  });

  it("coalesces concurrent flush calls", async () => {
    const outbox = new InkOutbox(new MemoryInkOutboxStore());
    await outbox.enqueue(item("r1", 1));
    let submissions = 0;
    const transport: InkOperationTransport = {
      submit: async value => {
        submissions++;
        await new Promise(resolve => setTimeout(resolve, 5));
        return { request_id: value.request_id, document_id: value.document_id, command_id: value.operation.command_id, seq: value.client_seq, durable: true as const };
      },
    };
    const [first, second] = await Promise.all([outbox.flush(transport), outbox.flush(transport)]);
    expect(first).toEqual(second);
    expect(submissions).toBe(1);
  });

  it("retains an item when the ACK belongs to another document or sequence", async () => {
    const outbox = new InkOutbox(new MemoryInkOutboxStore());
    await outbox.enqueue(item("r1", 1));
    await expect(outbox.flush({ submit: async value => ({ request_id: value.request_id, document_id: "wrong-document", command_id: value.operation.command_id, seq: 1, durable: true }) })).rejects.toBeInstanceOf(InkOutboxFlushError);
    await expect(outbox.flush({ submit: async value => ({ request_id: value.request_id, document_id: value.document_id, command_id: value.operation.command_id, seq: 2, durable: true }) })).rejects.toBeInstanceOf(InkOutboxFlushError);
    expect((await outbox.pending()).map(value => value.request_id)).toEqual(["r1"]);
  });
});
