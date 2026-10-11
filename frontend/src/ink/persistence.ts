import { safeInvoke } from "../core/ipc";
import { inkMetadata, type InkDocument, type InkOperation } from "./model";
import type { InkOperationAck, InkOperationTransport, InkOutboxItem } from "./outbox";

export interface StoredInkSession {
  session_id: string;
  document_id: string;
  writer_id: string;
  writer_epoch: number;
  status: string;
  device: unknown;
  started_at: string;
  last_seen_at: string;
}

export interface StoredInkDocumentSummary {
  document_id: string;
  title: string;
  created_at: string;
  updated_at: string;
  head_seq: number;
  session_count: number;
  active_session_id: string | null;
}

export interface StoredInkDocument {
  document: InkDocument;
  head_seq: number;
  updated_at: string;
}

export async function createInkDocument(
  document: InkDocument,
  sessionId: string,
  writerId: string,
  device: unknown,
): Promise<StoredInkSession> {
  return await safeInvoke<StoredInkSession>("create_ink_document", {
    request: { document, session_id: sessionId, writer_id: writerId, device },
  });
}

export async function startInkSession(
  documentId: string,
  sessionId: string,
  writerId: string,
  device: unknown,
): Promise<StoredInkSession> {
  return await safeInvoke<StoredInkSession>("start_ink_session", {
    request: { document_id: documentId, session_id: sessionId, writer_id: writerId, device },
  });
}

export async function listInkDocuments(): Promise<StoredInkDocumentSummary[]> {
  return await safeInvoke<StoredInkDocumentSummary[]>("list_ink_documents");
}

export async function listInkSessions(documentId: string): Promise<StoredInkSession[]> {
  return await safeInvoke<StoredInkSession[]>("list_ink_sessions", { documentId });
}

export async function openInkDocument(documentId: string): Promise<StoredInkDocument | null> {
  return await safeInvoke<StoredInkDocument | null>("get_ink_document", { documentId });
}

export async function closeInkSession(sessionId: string): Promise<void> {
  await safeInvoke<void>("close_ink_session", { sessionId });
}

export class InkDesktopTransport implements InkOperationTransport {
  async submit(item: InkOutboxItem): Promise<InkOperationAck> {
    return await safeInvoke<InkOperationAck>("append_ink_operation", {
      request: {
        request_id: item.request_id,
        document_id: item.document_id,
        session_id: item.session_id,
        writer_epoch: item.writer_epoch,
        expected_head: item.expected_head,
        metadata: item.metadata,
        operation: item.operation,
      },
    });
  }
}

export function inkOutboxItem(
  document: InkDocument,
  operation: InkOperation,
  requestId: string,
  sessionId: string,
  writerEpoch: number,
  expectedHead: number,
  clientSeq: number,
  queuedAt = new Date().toISOString(),
): InkOutboxItem {
  return {
    request_id: requestId,
    document_id: document.document_id,
    session_id: sessionId,
    writer_epoch: writerEpoch,
    expected_head: expectedHead,
    client_seq: clientSeq,
    queued_at: queuedAt,
    metadata: inkMetadata(document),
    operation,
  };
}
