import { mockIPC } from "@tauri-apps/api/mocks";
import { DesktopInkWorkspace } from "../src/ink/desktop";
import { newDocument, type InkDocument } from "../src/ink/model";
import type { StoredInkSnapshot } from "../src/ink/persistence";
import "../src/styles.css";

const a = newDocument("手稿 A");
const b = newDocument("手稿 B");
const documents = new Map<string, InkDocument>([[a.document_id, a], [b.document_id, b]]);
const sessions = new Map([[a.document_id, "session-a"], [b.document_id, "session-b"]]);
const snapshots = new Map<string, StoredInkSnapshot[]>([[a.document_id, []], [b.document_id, []]]);
const calls: Array<{ command: string; documentId?: string }> = [];
const behavior = { delayedDocumentId: "", delayMs: 0 };

mockIPC(async (command, payload) => {
  const args = payload as Record<string, unknown>;
  if (command === "list_ink_documents") {
    return [a, b].map(document => ({
      document_id: document.document_id,
      title: document.title,
      head_seq: document.operations.length,
      active_session_id: sessions.get(document.document_id),
      updated_at: document.created_at,
    }));
  }
  if (command === "get_ink_document") {
    const document = documents.get(args.documentId as string);
    return document ? { document: structuredClone(document), head_seq: document.operations.length, updated_at: document.created_at } : null;
  }
  if (command === "start_ink_session") {
    const request = args.request as { document_id: string; session_id: string; writer_id: string };
    return { ...request, writer_epoch: 1, status: "active", device: {}, started_at: a.created_at, last_seen_at: a.created_at };
  }
  if (command === "list_ink_snapshots") {
    const documentId = args.documentId as string;
    calls.push({ command, documentId });
    const result = structuredClone(snapshots.get(documentId) ?? []);
    if (behavior.delayedDocumentId === documentId) await new Promise(resolve => setTimeout(resolve, behavior.delayMs));
    return result;
  }
  if (command === "get_ink_snapshot") {
    const snapshotId = args.snapshotId as string;
    return [...snapshots.values()].flat().find(snapshot => snapshot.snapshot_id === snapshotId) ?? null;
  }
  if (command === "create_ink_snapshot") {
    const request = args.request as { request_id: string; document_id: string; expected_head: number };
    const document = documents.get(request.document_id)!;
    const snapshot: StoredInkSnapshot = {
      snapshot_id: "snapshot-" + request.document_id,
      request_id: request.request_id,
      document_id: request.document_id,
      through_seq: request.expected_head,
      artifact_id: "artifact-a",
      content_hash: "sha256:" + "a".repeat(64),
      byte_size: 0,
      manifest_json: JSON.stringify({
        version: "mathscape-ink-snapshot-1",
        document_id: request.document_id,
        through_document_seq: request.expected_head,
        document: structuredClone(document),
        visible_stroke_ids: [],
        point_count: 0,
      }),
      created_at: "unix:1791676800",
    };
    snapshots.get(request.document_id)!.push(snapshot);
    return snapshot;
  }
  throw new Error(`Unexpected Ink command: ${command}`);
});

const workspace = new DesktopInkWorkspace(document.querySelector<HTMLElement>("#ink-test-host")!, () => {}, () => {});
Object.assign(window, { inkDesktopHarness: { workspace, documents: { a: a.document_id, b: b.document_id }, snapshots, calls, behavior } });
void workspace.open();
