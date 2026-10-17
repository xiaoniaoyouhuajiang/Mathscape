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

export type InkTitleMode = "auto" | "manual";
export type InkLifecycle = "active" | "trashed";

/** Authoritative title and lifecycle of a document (N1). */
export interface InkNoteState {
  document_id: string;
  title: string;
  title_mode: InkTitleMode;
  title_revision: number;
  lifecycle: InkLifecycle;
  lifecycle_revision: number;
  /** Unix milliseconds. */
  trashed_at: number | null;
  /** Unix milliseconds of the last ink edit. */
  last_used_at: number;
  created_at: string;
}

export interface StoredInkDocumentSummary {
  document_id: string;
  title: string;
  created_at: string;
  updated_at: string;
  head_seq: number;
  session_count: number;
  active_session_id: string | null;
  title_mode: InkTitleMode;
  title_revision: number;
  lifecycle: InkLifecycle;
  lifecycle_revision: number;
  trashed_at: number | null;
  last_used_at: number;
  page_count: number;
  snapshot_count: number;
  remote_active: boolean;
}

export interface StoredInkDocument {
  document: InkDocument;
  head_seq: number;
  updated_at: string;
  note: InkNoteState;
}

export interface InkCreateReceipt extends StoredInkSession {
  note: InkNoteState;
  request_id: string | null;
  replayed: boolean;
}

export interface InkNoteReceipt {
  request_id: string;
  kind: "rename" | "trash" | "restore";
  note: InkNoteState;
  replayed: boolean;
  closed_sessions: number;
  revoked_devices: number;
  requires_repairing: boolean;
}

export interface InkTrashImpact {
  note: InkNoteState;
  head_seq: number;
  page_count: number;
  snapshot_count: number;
  evidence_count: number;
  remote_session: boolean;
  device_credentials: number;
  pairing_pending: boolean;
  blank: boolean;
  connected: boolean;
}

export interface InkCleanBlankReceipt {
  request_id: string;
  trashed: InkNoteState[];
  skipped: Array<{ document_id: string; reason: string }>;
  replayed: boolean;
}

export function getInkNote(documentId: string): Promise<InkNoteState | null> {
  return safeInvoke("get_ink_note", { documentId });
}

export function getInkTrashImpact(documentId: string): Promise<InkTrashImpact> {
  return safeInvoke("get_ink_trash_impact", { documentId });
}

export function listTrashedInkDocuments(): Promise<StoredInkDocumentSummary[]> {
  return safeInvoke("list_trashed_ink_documents");
}

export function renameInkNote(requestId: string, documentId: string, title: string, expectedTitleRevision: number): Promise<InkNoteReceipt> {
  return safeInvoke("rename_ink_note", {
    request: { request_id: requestId, document_id: documentId, title, expected_title_revision: expectedTitleRevision },
  });
}

export function trashInkNote(requestId: string, documentId: string, expectedLifecycleRevision: number): Promise<InkNoteReceipt> {
  return safeInvoke("trash_ink_note", {
    request: { request_id: requestId, document_id: documentId, expected_lifecycle_revision: expectedLifecycleRevision },
  });
}

export function restoreInkNote(requestId: string, documentId: string, expectedLifecycleRevision: number): Promise<InkNoteReceipt> {
  return safeInvoke("restore_ink_note", {
    request: { request_id: requestId, document_id: documentId, expected_lifecycle_revision: expectedLifecycleRevision },
  });
}

export function cleanBlankInkNotes(requestId: string, documentIds: string[]): Promise<InkCleanBlankReceipt> {
  return safeInvoke("clean_blank_ink_notes", { request: { request_id: requestId, document_ids: documentIds } });
}

export interface StoredInkSnapshot {
  snapshot_id: string;
  request_id: string;
  document_id: string;
  through_seq: number;
  artifact_id: string;
  content_hash: string;
  byte_size: number;
  manifest_json: string;
  created_at: string;
}
export type StoredInkSnapshotSummary = Omit<StoredInkSnapshot, "manifest_json">;

export interface InkSelection {
  page_id: string;
  stroke_ids: string[];
  selection_mode: "whole_strokes" | "lasso";
}

export interface InkEvidenceRequest extends InkSelection {
  request_id: string;
  snapshot_id: string;
  expected_artifact_id: string;
  expected_content_hash: string;
}

export interface InkEvidenceReceipt extends InkSelection {
  version: string;
  request_id: string;
  snapshot_id: string;
  artifact_id: string;
  span_id: string;
  revision_id: string;
  commit_id: string;
  outcome: "created" | "reused";
  committed_at: string;
}

export function registerInkEvidence(request: InkEvidenceRequest): Promise<InkEvidenceReceipt> {
  return safeInvoke("register_ink_evidence", { request });
}

export function listInkEvidence(snapshotId: string): Promise<InkEvidenceReceipt[]> {
  return safeInvoke("list_ink_evidence", { snapshotId });
}

export async function createInkDocument(
  document: InkDocument,
  sessionId: string,
  writerId: string,
  device: unknown,
  options: { requestId?: string; titleMode?: InkTitleMode } = {},
): Promise<InkCreateReceipt> {
  return await safeInvoke<InkCreateReceipt>("create_ink_document", {
    request: { document, session_id: sessionId, writer_id: writerId, device, request_id: options.requestId, title_mode: options.titleMode },
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

export async function createInkSnapshot(
  requestId: string,
  documentId: string,
  expectedHead: number,
): Promise<StoredInkSnapshot> {
  return await safeInvoke<StoredInkSnapshot>("create_ink_snapshot", {
    request: { request_id: requestId, document_id: documentId, expected_head: expectedHead },
  });
}

export async function listInkSnapshots(documentId: string): Promise<StoredInkSnapshotSummary[]> {
  return await safeInvoke<StoredInkSnapshotSummary[]>("list_ink_snapshots", { documentId });
}

export async function openInkSnapshot(snapshotId: string): Promise<StoredInkSnapshot | null> {
  return await safeInvoke<StoredInkSnapshot | null>("get_ink_snapshot", { snapshotId });
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
