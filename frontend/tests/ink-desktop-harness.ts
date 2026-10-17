import { mockIPC } from "@tauri-apps/api/mocks";
import { DesktopInkWorkspace } from "../src/ink/desktop";
import { newDocument, projectInk, inkMetadata, type InkDocument, type InkOperation, type InkDocumentMetadata } from "../src/ink/model";
import type { StoredInkSnapshot, InkEvidenceReceipt, InkEvidenceRequest } from "../src/ink/persistence";
import { ArtifactReader } from "../src/reader/reader";
import { InspectorController } from "../src/inspector/inspector-controller";
import type { DesktopSnapshot } from "../src/artifact/types";
import { eventBus } from "../src/core/events";
import { setInkSourceCandidateProvider, type InkNoteLink } from "../src/ink/links";
import "../src/styles.css";

const a = newDocument("手稿 A");
const b = newDocument("手稿 B");
const documents = new Map<string, InkDocument>([[a.document_id, a], [b.document_id, b]]);
const sessions = new Map([[a.document_id, "session-a"], [b.document_id, "session-b"]]);
const snapshots = new Map<string, StoredInkSnapshot[]>([[a.document_id, []], [b.document_id, []]]);
const evidence = new Map<string, InkEvidenceReceipt>();
const evidenceRequests: InkEvidenceRequest[] = [];
const operationReceipts = new Map<string, unknown>();
const calls: Array<{ command: string; documentId?: string }> = [];
type NoteRow = { title_mode: "auto" | "manual"; title_revision: number; lifecycle: "active" | "trashed"; lifecycle_revision: number; trashed_at: number | null; last_used_at: number };
const notes = new Map<string, NoteRow>([[a.document_id, { title_mode: "manual", title_revision: 0, lifecycle: "active", lifecycle_revision: 0, trashed_at: null, last_used_at: 2 }], [b.document_id, { title_mode: "manual", title_revision: 0, lifecycle: "active", lifecycle_revision: 0, trashed_at: null, last_used_at: 1 }]]);
const noteRequests = new Map<string, unknown>();
const noteCalls: Array<{ command: string; request: Record<string, unknown> }> = [];
const noteState = (id: string) => { const document = documents.get(id)!; return { document_id: id, title: document.title, created_at: document.created_at, ...notes.get(id)! }; };
const summary = (document: InkDocument) => ({ document_id: document.document_id, title: document.title, created_at: document.created_at, head_seq: document.operations.length, session_count: 1, active_session_id: sessions.get(document.document_id), updated_at: document.created_at, page_count: document.pages.length, snapshot_count: snapshots.get(document.document_id)?.length ?? 0, remote_active: behavior.remoteDocumentId === document.document_id, ...notes.get(document.document_id)! });
const structuredError = (code: string, message: string, details?: Record<string, unknown>) => ({ code, message, details });
const behavior = { renameConflictTitle: "", delayedDocumentId: "", delayMs: 0, appendDelay: 0, snapshotDelay: 0, failAppend: false, appendErrorCode: "", mirrorPolls: 0, loseEvidenceAck: false, remoteDocumentId:"", pairingPending:false,epoch:1,setupStep:0,remoteConnected:true,remotePending:0,setupDocumentId:"" as string|null,handoff:"",transferConfirmation:false,pairingArgs:[] as unknown[],strictPairing:false,autoRequest:true,revokedDocument:"",failCreate:false };

// N2 links: a tiny in-memory model of ink_note_links (soft removal, one
// primary, replaces_link_id, auto titles "角色 · 目标").
const sourceLabels = new Map([["unit-thm", "定理 3.2"], ["unit-ex", "例 4"]]);
const links: InkNoteLink[] = [];
const linkRequests = new Map<string, unknown>();
const linkCalls: Array<{ command: string; request: Record<string, unknown> }> = [];
const exports: Array<{ fileName: string; contents: string }> = [];
const roleNames: Record<string, string> = { attempt: "尝试", alt_proof: "另证", check: "验算", study: "学习笔记", question: "疑问", reference: "参考" };
const retitle = (id: string) => {
  const row = notes.get(id)!, document = documents.get(id)!;
  if (row.title_mode !== "auto") return false;
  const primary = links.find(link => link.document_id === id && link.state === "active" && link.is_primary);
  const title = primary ? `${roleNames[primary.role]} · ${primary.target_label}` : "手稿 · 自动";
  if (title === document.title) return false;
  document.title = title; row.title_revision++;
  return true;
};
setInkSourceCandidateProvider(() => [...sourceLabels].map(([object_id, label]) => ({ object_id, label, kind: "SourceSemanticUnit" })));

mockIPC(async (command, payload) => {
  const args = payload as Record<string, unknown>;
  if (command === "list_ink_note_links") return links.filter(link => link.document_id === args.documentId).map(link => structuredClone(link));
  if (command === "export_ink_unsaved_writing") { exports.push({ fileName: args.fileName as string, contents: args.contents as string }); return "/tmp/" + (args.fileName as string); }
  if (["link_ink_note", "unlink_ink_note", "set_ink_note_primary_link"].includes(command)) {
    const request = args.request as Record<string, unknown> & { request_id: string; document_id: string };
    linkCalls.push({ command, request: structuredClone(request) });
    if (linkRequests.has(request.request_id)) return { ...(linkRequests.get(request.request_id) as object), replayed: true };
    if (notes.get(request.document_id)?.lifecycle === "trashed") throw structuredError("document_trashed", "手稿已在回收站中，恢复后才能继续编辑。");
    const active = () => links.filter(link => link.document_id === request.document_id && link.state === "active");
    let link: InkNoteLink | null = null, existing = false;
    if (command === "link_ink_note") {
      const target = request.target as { kind: string; object_id?: string; document_id?: string };
      const key = target.kind === "source_object" ? `src:${target.object_id}` : `ink:${target.document_id}`;
      const duplicate = active().find(item => item.target_key === key && item.role === request.role);
      if (duplicate) { link = duplicate; existing = true; }
      else {
        const replaced = request.replaces_link_id ? active().find(item => item.link_id === request.replaces_link_id) : undefined;
        if (replaced) { replaced.state = "removed"; replaced.removed_at = Date.now(); }
        const primary = Boolean(request.primary) || Boolean(replaced?.is_primary);
        if (primary) for (const item of active()) item.is_primary = false;
        if (replaced) replaced.is_primary = false;
        const id = target.kind === "source_object" ? target.object_id! : target.document_id!;
        link = { link_id: "link-" + request.request_id, document_id: request.document_id, target_kind: target.kind as "source_object", target_key: key, target_id: id, target: structuredClone(target) as InkNoteLink["target"], target_revision: null, target_label: sourceLabels.get(id) ?? documents.get(id)?.title ?? id, role: request.role as InkNoteLink["role"], is_primary: primary, state: "active", created_at: Date.now() + links.length, removed_at: null, target_status: "active" };
        links.push(link);
      }
    } else if (command === "unlink_ink_note") {
      link = active().find(item => item.link_id === request.link_id) ?? null;
      if (!link) throw structuredError("link_not_found", "关联不存在。");
      Object.assign(link, { state: "removed", removed_at: Date.now(), is_primary: false });
    } else {
      for (const item of active()) item.is_primary = item.link_id === request.link_id;
      link = active().find(item => item.link_id === request.link_id) ?? null;
    }
    const titleChanged = retitle(request.document_id);
    const receipt = { request_id: request.request_id, kind: command === "link_ink_note" ? "link" : command === "unlink_ink_note" ? "unlink" : "set_primary", note: noteState(request.document_id), link: structuredClone(link), existing, title_changed: titleChanged, replayed: false };
    linkRequests.set(request.request_id, receipt);
    return receipt;
  }
  if(command==="get_ink_lan_mirror") {
    behavior.mirrorPolls++;
    const document=documents.get(args.documentId as string)!;
    const remote=behavior.remoteDocumentId===document.document_id;
    const session={document_id:document.document_id,session_id:sessions.get(document.document_id)!,writer_id:remote?"inkpad-test":"desktop-local",writer_epoch:behavior.epoch,status:"active",device:{},started_at:document.created_at,last_seen_at:document.created_at};
    return {delta:{metadata:inkMetadata(document),operations:document.operations.filter(operation=>operation.seq>Number(args.afterSeq)),head_seq:document.operations.length,server_head:document.operations.length,writer:{document_id:document.document_id,head_seq:document.operations.length,writer_epoch:behavior.epoch,session,remote,pairing_pending:behavior.pairingPending},snapshot_count:snapshots.get(document.document_id)!.length},connected:remote&&behavior.remoteConnected,preview:null,client:remote&&behavior.remoteConnected?{pending_count:behavior.remotePending,locally_durable:true,secure_context:true,page_id:document.pages[0].id}:null};
  }
  if(command==="get_ink_lan_status"||command==="start_ink_lan")return {enabled:true,running:true,port:1443,urls:["https://127.0.0.1:1443/pad/"],certificate_file_sha256:"fixture",assets_ready:true,error:null};
  if(command==="create_ink_pairing"){behavior.pairingArgs.push(structuredClone(args));if(behavior.strictPairing&&behavior.remoteDocumentId===args.documentId&&!args.allowTransfer)throw new Error("ink invariant failed: takeover_required");behavior.pairingPending=true;return{document_id:args.documentId,expires_at:Date.now()/1000+120,urls:["https://127.0.0.1:1443/pad/#pair="+"a".repeat(64)]};}
  if(command==="start_ink_setup"){behavior.setupStep=0;behavior.setupDocumentId=(args.documentId as string|null)??null;behavior.handoff=behavior.setupDocumentId?"idle":"unbound";return{active:true,expires_at:Date.now()/1000+900,urls:["http://127.0.0.1:1442/setup/#setup="+"b".repeat(64),"http://localhost:1442/setup/#setup="+"b".repeat(64)],opened:false,certificate_downloaded:false,https_trusted:false,check:null,complete:false};}
  const setupStatus=()=>{const step=behavior.setupStep;return{active:true,expires_at:Date.now()/1000+900,urls:[],opened:step>=1,certificate_downloaded:step>=2,https_trusted:step>=3,check:step>=4?{secure_context:true,indexed_db:true,service_worker:true,user_agent:"fixture"}:null,complete:step>=4,document_id:behavior.setupDocumentId,handoff:behavior.handoff,transfer_confirmation:behavior.transferConfirmation};};
  if(command==="get_ink_setup_status"){behavior.setupStep=Math.min(4,behavior.setupStep+1);if(behavior.setupStep===4&&behavior.handoff==="idle"&&behavior.autoRequest)behavior.handoff="requested";return setupStatus();}
  if(command==="prepare_ink_setup_handoff"){behavior.pairingArgs.push({setup:true,...structuredClone(args)});if(behavior.remoteDocumentId===behavior.setupDocumentId&&!args.confirmTransfer){behavior.transferConfirmation=true;return setupStatus();}behavior.transferConfirmation=false;behavior.handoff="ready";behavior.pairingPending=true;return setupStatus();}
  if(command==="cancel_ink_setup"){behavior.setupStep=0;return;}
  if(command==="cancel_ink_pairing"){behavior.pairingPending=false;return;}
  if(command==="reclaim_ink_writer"){behavior.remoteDocumentId="";behavior.pairingPending=false;behavior.epoch++;return{document_id:args.documentId,writer_epoch:behavior.epoch,writer_id:"desktop-local",session_id:sessions.get(args.documentId as string)};}
  if (command === "list_ink_documents") {
    return [...documents.values()].filter(document => notes.get(document.document_id)!.lifecycle === "active").sort((x, y) => notes.get(y.document_id)!.last_used_at - notes.get(x.document_id)!.last_used_at).map(summary);
  }
  if (command === "list_trashed_ink_documents") {
    return [...documents.values()].filter(document => notes.get(document.document_id)!.lifecycle === "trashed").map(summary);
  }
  if (command === "get_ink_note") return documents.has(args.documentId as string) ? noteState(args.documentId as string) : null;
  if (command === "get_ink_trash_impact") {
    const id = args.documentId as string, document = documents.get(id)!;
    return { note: noteState(id), head_seq: document.operations.length, page_count: document.pages.length, snapshot_count: snapshots.get(id)!.length, evidence_count: [...evidence.values()].filter(receipt => snapshots.get(id)!.some(snapshot => snapshot.snapshot_id === receipt.snapshot_id)).length, remote_session: behavior.remoteDocumentId === id, device_credentials: behavior.remoteDocumentId === id ? 1 : 0, pairing_pending: false, blank: false, connected: behavior.remoteDocumentId === id && behavior.remoteConnected };
  }
  if (["rename_ink_note", "trash_ink_note", "restore_ink_note", "clean_blank_ink_notes"].includes(command)) {
    const request = args.request as Record<string, unknown> & { request_id: string; document_id: string };
    noteCalls.push({ command, request: structuredClone(request) });
    if (noteRequests.has(request.request_id)) return { ...(noteRequests.get(request.request_id) as object), replayed: true };
    let receipt: unknown;
    if (command === "clean_blank_ink_notes") {
      const trashed = [], skipped = [];
      for (const id of request.document_ids as string[]) {
        const row = notes.get(id), document = documents.get(id);
        if (!row || !document) { skipped.push({ document_id: id, reason: "not_found" }); continue; }
        if (document.operations.length || row.title_mode === "manual" || row.lifecycle === "trashed") { skipped.push({ document_id: id, reason: "has_ink" }); continue; }
        Object.assign(row, { lifecycle: "trashed", lifecycle_revision: row.lifecycle_revision + 1, trashed_at: Date.now() });
        trashed.push(noteState(id));
      }
      receipt = { request_id: request.request_id, trashed, skipped, replayed: false };
    } else {
      const row = notes.get(request.document_id)!, document = documents.get(request.document_id)!;
      let requiresRepairing = false;
      if (command === "rename_ink_note") {
        if (row.lifecycle === "trashed") throw structuredError("document_trashed", "手稿已在回收站中，恢复后才能继续编辑。");
        if (behavior.renameConflictTitle) { document.title = behavior.renameConflictTitle; row.title_mode = "manual"; row.title_revision++; behavior.renameConflictTitle = ""; }
        if (row.title_revision !== request.expected_title_revision) throw structuredError("title_conflict", "标题已在别处修改，请选择保留哪一个。", { document_id: request.document_id, current_title: document.title, current_title_mode: row.title_mode, current_title_revision: row.title_revision });
        const title = String(request.title).trim();
        document.title = title || "手稿 · 自动"; row.title_mode = title ? "manual" : "auto"; row.title_revision++;
      } else {
        const target = command === "trash_ink_note" ? "trashed" : "active";
        if (row.lifecycle !== target) {
          if (row.lifecycle_revision !== request.expected_lifecycle_revision) throw structuredError("lifecycle_conflict", "手稿状态已在别处变化，请刷新后重试。");
          Object.assign(row, { lifecycle: target, lifecycle_revision: row.lifecycle_revision + 1, trashed_at: target === "trashed" ? Date.now() : null });
          if (target === "trashed" && behavior.remoteDocumentId === request.document_id) { behavior.remoteDocumentId = ""; behavior.epoch++; behavior.revokedDocument = request.document_id; }
        }
        requiresRepairing = target === "active" && behavior.revokedDocument === request.document_id;
      }
      receipt = { request_id: request.request_id, kind: command.split("_")[0], note: noteState(request.document_id), replayed: false, closed_sessions: 0, revoked_devices: 0, requires_repairing: requiresRepairing };
    }
    noteRequests.set(request.request_id, receipt);
    return receipt;
  }
  if (command === "create_ink_document") {
    const request = args.request as { document: InkDocument; session_id: string; writer_id: string; request_id?: string; title_mode?: "auto" | "manual" };
    noteCalls.push({ command, request: structuredClone(request) as unknown as Record<string, unknown> });
    if (behavior.failCreate) { behavior.failCreate = false; throw structuredError("storage_failed", "injected create failure"); }
    if (request.request_id && noteRequests.has(request.request_id)) return { ...(noteRequests.get(request.request_id) as object), replayed: true };
    const document = structuredClone(request.document);
    documents.set(document.document_id,document); sessions.set(document.document_id,request.session_id); snapshots.set(document.document_id,[]);
    notes.set(document.document_id, { title_mode: request.title_mode ?? "manual", title_revision: 0, lifecycle: "active", lifecycle_revision: 0, trashed_at: null, last_used_at: Date.now() });
    const receipt = { ...request, document_id: document.document_id, writer_epoch:1,status:"active", note: noteState(document.document_id), replayed: false };
    if (request.request_id) noteRequests.set(request.request_id, receipt);
    return receipt;
  }
  if (command === "append_ink_operation") {
    const request = args.request as { request_id:string;document_id:string;expected_head:number;metadata:InkDocumentMetadata;operation:InkOperation };
    if (behavior.appendDelay) await new Promise(resolve => setTimeout(resolve,behavior.appendDelay));
    if (behavior.failAppend) throw new Error("injected save failure");
    if (behavior.appendErrorCode) throw structuredError(behavior.appendErrorCode, "injected terminal save failure");
    if (notes.get(request.document_id)?.lifecycle === "trashed") throw structuredError("document_trashed", "手稿已在回收站中，恢复后才能继续编辑。");
    if (operationReceipts.has(request.request_id)) return operationReceipts.get(request.request_id);
    const document = documents.get(request.document_id)!;
    if (document.operations.length !== request.expected_head) throw new Error("head conflict");
    // Like the store, queued metadata never changes the title (H1).
    const title = document.title;
    Object.assign(document,structuredClone(request.metadata)); document.title = title; document.operations.push(structuredClone(request.operation));
    notes.get(request.document_id)!.last_used_at = Date.now();
    const receipt = {request_id:request.request_id,document_id:request.document_id,command_id:request.operation.command_id,seq:request.operation.seq,durable:true};
    operationReceipts.set(request.request_id,receipt); return receipt;
  }
  if (command === "get_ink_document") {
    const document = documents.get(args.documentId as string);
    return document ? { document: structuredClone(document), head_seq: document.operations.length, updated_at: document.created_at, note: noteState(document.document_id) } : null;
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
    if (behavior.snapshotDelay) await new Promise(resolve => setTimeout(resolve,behavior.snapshotDelay));
    const snapshotId = args.snapshotId as string;
    return [...snapshots.values()].flat().find(snapshot => snapshot.snapshot_id === snapshotId) ?? null;
  }
  if (command === "list_ink_evidence") {
    const unique = new Map([...evidence.values()].filter(receipt => receipt.snapshot_id === args.snapshotId).map(receipt => [receipt.span_id,receipt]));
    return [...unique.values()];
  }
  if (command === "register_ink_evidence") {
    const request = args.request as InkEvidenceRequest;
    evidenceRequests.push(structuredClone(request));
    if (evidence.has(request.request_id)) return evidence.get(request.request_id);
    const signature = JSON.stringify([request.expected_artifact_id,request.page_id,[...request.stroke_ids].sort(),request.selection_mode]);
    const existing = [...evidence.values()].find(receipt => receipt.span_id === signature);
    const receipt: InkEvidenceReceipt = {...request,version:"mathscape-ink-evidence-receipt-1",artifact_id:request.expected_artifact_id,span_id:signature,revision_id:"revision",commit_id:"commit",outcome:existing ? "reused" : "created",committed_at:new Date().toISOString()};
    evidence.set(request.request_id,receipt);
    if (behavior.loseEvidenceAck) { behavior.loseEvidenceAck = false; throw new Error("响应丢失"); }
    return receipt;
  }
  if (command === "get_source_node_detail") {
    const receipt = [...evidence.values()].find(receipt => receipt.span_id === args.nodeId)!;
    const snapshot = [...snapshots.values()].flat().find(snapshot => snapshot.snapshot_id === receipt.snapshot_id)!;
    return {node_id:receipt.span_id,kind:"SourceSpan",display_title:"手稿证据",artifact_id:receipt.artifact_id,byte_range:null,excerpt:null,markdown_block_ids:[],payload:{},ink:{snapshot_id:snapshot.snapshot_id,document_id:snapshot.document_id,page_id:receipt.page_id,stroke_ids:receipt.stroke_ids}};
  }
  if (command === "get_map_object_edit_context") throw new Error("对象不存在");
  if (command === "get_artifact_content_info") {
    const snapshot = [...snapshots.values()].flat().find(snapshot => snapshot.artifact_id === args.artifactId)!;
    return {artifact_id:snapshot.artifact_id,media_type:"application/json",byte_size:snapshot.byte_size,contexts:[],ink_snapshot_id:snapshot.snapshot_id};
  }
  if (command === "create_ink_snapshot") {
    const request = args.request as { request_id: string; document_id: string; expected_head: number };
    const document = documents.get(request.document_id)!;
    const projection = projectInk(document);
    const snapshot: StoredInkSnapshot = {
      snapshot_id: "snapshot-" + request.document_id + "-" + request.expected_head,
      request_id: request.request_id,
      document_id: request.document_id,
      through_seq: request.expected_head,
      artifact_id: "artifact-" + request.document_id + "-" + request.expected_head,
      content_hash: "sha256:" + "a".repeat(64),
      byte_size: 0,
      manifest_json: JSON.stringify({
        version: "mathscape-ink-snapshot-1",
        document_id: request.document_id,
        through_document_seq: request.expected_head,
        document: structuredClone(document),
        visible_stroke_ids: projection.visible.map(stroke => stroke.id),
        point_count: projection.pointCount,
      }),
      created_at: "unix:1791676800",
    };
    const previous = snapshots.get(request.document_id)!.find(stored => stored.snapshot_id === snapshot.snapshot_id);
    if (previous) return previous;
    snapshots.get(request.document_id)!.push(snapshot);
    return snapshot;
  }
  throw new Error(`Unexpected Ink command: ${command}`);
});

const workspaceHost = document.querySelector<HTMLElement>("#ink-test-host")!;
const mapHost = document.querySelector<HTMLElement>("#ink-map-test")!;
const readerHost = document.querySelector<HTMLElement>("#ink-reader-test")!;
const reader = new ArtifactReader(readerHost);
const workspace = new DesktopInkWorkspace(workspaceHost, () => { workspaceHost.hidden = true; mapHost.hidden = false; }, () => {});
document.querySelector<HTMLButtonElement>("#ink-map-open-evidence")!.onclick = () => {
  const receipt = [...evidence.values()].at(-1)!;
  const snapshot = {pdf_artifacts:[],marker:{},landscape_graph:{nodes:[{id:"span-render",element_kind:"entity",domain_id:receipt.span_id,domain_kind:"SourceSpan",relation_kind:null,layer:"source",label:"手稿证据",summary:"快照笔迹",lifecycle:"active",parent:null,boundary:true}]}} as unknown as DesktopSnapshot;
  const inspector = new InspectorController(document.querySelector<HTMLElement>("#ink-inspector-test")!, {
    getSnapshot:() => snapshot,showToast:() => {},copyIdentifier:() => {},onFocusNode:() => {},
    onOpenReader:(id,title,inkFocus) => { readerHost.hidden=false; void reader.open(id,title,{inkFocus}); },
    onOpenReaderWithFocus:() => {},createParseForm:() => document.createElement("form"),refreshSnapshot:async() => {},canFocus:() => false,isFocusRoot:() => false,
  });
  inspector.selectNodeById("span-render");
};
eventBus.on("open-ink-document", ({documentId}) => {reader.close();readerHost.hidden=true;mapHost.hidden=true;workspaceHost.hidden=false;void workspace.openDocumentId(documentId);});
const harness = { links, linkCalls, exports, notes, noteCalls, workspace, documents: { a: a.document_id, b: b.document_id }, snapshots, calls, behavior, evidenceRequests, storedDocuments:documents };
declare global { interface Window { inkDesktopHarness: typeof harness } }
window.inkDesktopHarness = harness;
void workspace.open();
