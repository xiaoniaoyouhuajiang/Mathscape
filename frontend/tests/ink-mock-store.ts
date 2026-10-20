// In-memory stand-in for the desktop Ink commands, shared by the Ink
// workspace harness (ink-desktop.html) and the full app harness (app.html).
import { mockIPC } from "@tauri-apps/api/mocks";
import { newDocument, projectInk, inkMetadata, type InkDocument, type InkOperation, type InkDocumentMetadata } from "../src/ink/model";
import type { StoredInkSnapshot, InkEvidenceReceipt, InkEvidenceRequest } from "../src/ink/persistence";
import { setInkSourceCandidateProvider, type InkNoteLink } from "../src/ink/links";

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
const behavior = { preferredHost: null as string|null, pushOffline:false, pushNeedsTakeover:false, pushArgs:[] as unknown[], noLanAddress: false, renameConflictTitle: "", delayedDocumentId: "", delayMs: 0, appendDelay: 0, snapshotDelay: 0, failAppend: false, appendErrorCode: "", mirrorPolls: 0, loseEvidenceAck: false, remoteDocumentId:"", pairingPending:false,epoch:1,setupStep:0,remoteConnected:true,remotePending:0,setupDocumentId:"" as string|null,handoff:"",transferConfirmation:false,pairingArgs:[] as unknown[],strictPairing:false,autoRequest:true,revokedDocument:"",failCreate:false,recoveryFile:"", snapshotLabelConflict: null as string|null, snapshotLabelDelay: 0 };
const snapshotLabelRequests = new Map<string, unknown>();
const snapshotLabelCalls: Array<Record<string, unknown>> = [];

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

// The queries filter by note lifecycle like the store (§14.6).
const liveLinks = () => links.filter(link => link.state === "active" && notes.get(link.document_id)?.lifecycle === "active");
const overlayCalls = { count: 0 };

mockIPC(async (command, payload) => {
  const args = payload as Record<string, unknown>;
  if (command === "get_ink_link_overlay") {
    overlayCalls.count++;
    const live = liveLinks().filter(link => link.target_kind === "source_object");
    const counts = new Map<string, Set<string>>();
    for (const link of live) counts.set(link.target_id, (counts.get(link.target_id) ?? new Set()).add(link.document_id));
    return {
      counts: [...counts].map(([target_id, notes]) => ({ target_kind: "source_object", target_id, note_count: notes.size })),
      links: live.map(link => ({ document_id: link.document_id, title: documents.get(link.document_id)!.title, last_used_at: notes.get(link.document_id)!.last_used_at, target_kind: "source_object", target_id: link.target_id, role: link.role, is_primary: link.is_primary })),
    };
  }
  if (command === "list_ink_links_to_target") {
    return liveLinks().filter(link => link.target_kind === args.targetKind && link.target_id === args.targetId).map(link => ({ note: noteState(link.document_id), link: structuredClone(link) }));
  }
  if (command === "choose_ink_recovery") {
    // Stands in for the file picker + Rust rebuild (covered by
    // apps/mathscape-desktop/tests/ink_recovery_import.rs): the picked file is
    // `behavior.recoveryFile`; an unsaved-operations export is rebuilt on the
    // stored prefix each queue was written against.
    const file = JSON.parse(behavior.recoveryFile) as { format?: string; document_id: string; items?: Array<{ session_id: string; expected_head: number; client_seq: number; metadata: InkDocumentMetadata; operation: InkOperation }> } & InkDocument;
    const drafts: InkDocument[] = [];
    if (file.format === "mathscape-ink-unsaved-operations") {
      const original = documents.get(file.document_id)!;
      const queues = new Map<string, NonNullable<typeof file.items>>();
      for (const item of file.items!) queues.set(item.session_id, [...(queues.get(item.session_id) ?? []), item]);
      for (const items of queues.values()) {
        items.sort((x, y) => x.client_seq - y.client_seq);
        const operations = original.operations.slice(0, items[0].expected_head).map(op => structuredClone(op));
        for (const item of items) operations.push({ ...structuredClone(item.operation), seq: operations.length + 1 });
        drafts.push({ ...structuredClone(original), ...structuredClone(items.at(-1)!.metadata), operations });
      }
    } else drafts.push(structuredClone(file));
    const created = drafts.map((draft, index) => {
      const id = `recovery-${String(args.requestId)}-${index}`;
      const document = { ...draft, document_id: id, title: `${draft.title} · 恢复草稿` };
      documents.set(id, document); sessions.set(id, `recovery-session-${id}`); snapshots.set(id, []);
      notes.set(id, { title_mode: "manual", title_revision: 0, lifecycle: "active", lifecycle_revision: 0, trashed_at: null, last_used_at: Date.now() + index });
      return { document_id: id, session_id: `recovery-session-${id}`, writer_id: "desktop-local", writer_epoch: 1, status: "active", device: {}, started_at: document.created_at, last_seen_at: document.created_at };
    });
    return { source: file.format ? "unsaved_operations" : "document", original_document_id: file.document_id, sessions: created };
  }
  if (command === "create_ink_note_from_context") {
    // §6.5: one transaction creates an auto-titled note and its primary link.
    const request = args.request as { request_id: string; document: InkDocument; session_id: string; target: { kind: "source_object"; object_id: string }; role: InkNoteLink["role"]; title: string | null };
    const document = structuredClone(request.document);
    documents.set(document.document_id, document); sessions.set(document.document_id, request.session_id); snapshots.set(document.document_id, []);
    notes.set(document.document_id, { title_mode: request.title ? "manual" : "auto", title_revision: 0, lifecycle: "active", lifecycle_revision: 0, trashed_at: null, last_used_at: Date.now() });
    if (request.title) document.title = request.title;
    const id = request.target.object_id;
    const link: InkNoteLink = { link_id: "link-" + request.request_id, document_id: document.document_id, target_kind: "source_object", target_key: `src:${id}`, target_id: id, target: structuredClone(request.target) as InkNoteLink["target"], target_revision: null, target_label: sourceLabels.get(id) ?? id, role: request.role, is_primary: true, state: "active", created_at: Date.now(), removed_at: null, target_status: "active" };
    links.push(link);
    retitle(document.document_id);
    return { document_id: document.document_id, session_id: request.session_id, writer_id: "desktop-local", writer_epoch: 1, status: "active", device: {}, started_at: document.created_at, last_seen_at: document.created_at, note: noteState(document.document_id), link: structuredClone(link), request_id: request.request_id, replayed: false };
  }
  if (command === "list_ink_note_links") return links.filter(link => link.document_id === args.documentId).map(link => structuredClone(link));
  if (command === "export_ink_unsaved_writing") { exports.push({ fileName: args.fileName as string, contents: args.contents as string }); return "/tmp/" + (args.fileName as string); }
  if (["link_ink_note", "unlink_ink_note", "set_ink_note_primary_link"].includes(command)) {
    const request = args.request as Record<string, unknown> & { request_id: string; document_id: string };
    linkCalls.push({ command, request: structuredClone(request) });
    if (linkRequests.has(request.request_id)) return { ...(linkRequests.get(request.request_id) as object), replayed: true };
    if (notes.get(request.document_id)?.lifecycle === "trashed") throw structuredError("document_trashed", "手稿已在回收站中，恢复后才能继续编辑。");
    const active = () => links.filter(link => link.document_id === request.document_id && link.state === "active");
    let link: InkNoteLink | null = null, existing = false, merged = false, primaryChanged = false;
    if (command === "link_ink_note") {
      const target = request.target as { kind: string; object_id?: string; document_id?: string };
      const key = target.kind === "source_object" ? `src:${target.object_id}` : `ink:${target.document_id}`;
      const duplicate = active().find(item => item.target_key === key && item.role === request.role);
      if (duplicate) {
        // Same D12 merge semantics as the store.
        link = duplicate; existing = true;
        const replaced = request.replaces_link_id && request.replaces_link_id !== duplicate.link_id ? active().find(item => item.link_id === request.replaces_link_id) : undefined;
        let promote = Boolean(request.primary) && !duplicate.is_primary;
        if (replaced) { promote ||= replaced.is_primary && !duplicate.is_primary; primaryChanged = replaced.is_primary; Object.assign(replaced, { state: "removed", removed_at: Date.now(), is_primary: false }); merged = true; }
        if (promote) { for (const item of active()) item.is_primary = false; duplicate.is_primary = true; primaryChanged = true; }
      }
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
    const receipt = { request_id: request.request_id, kind: command === "link_ink_note" ? "link" : command === "unlink_ink_note" ? "unlink" : "set_primary", note: noteState(request.document_id), link: structuredClone(link), existing, merged, primary_changed: primaryChanged, title_changed: titleChanged, replayed: false };
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
  if(command==="get_ink_lan_status"||command==="start_ink_lan")return {enabled:true,running:true,port:1443,urls:["https://192.168.31.20:1443/pad/","https://100.90.0.2:1443/pad/","https://127.0.0.1:1443/pad/","https://localhost:1443/pad/"],url_kinds:["lan","vpn","loopback","loopback"],preferred_host:behavior.preferredHost||null,certificate_file_sha256:"fixture",assets_ready:true,error:null};
  if(command==="set_ink_lan_preferred_host"){behavior.preferredHost=(args.host as string|null|undefined)||null;return{enabled:true,port:1443,preferred_host:behavior.preferredHost};}
  if(command==="push_ink_to_tablet"){behavior.pushArgs.push(structuredClone(args));if(behavior.pushOffline)return{pushed:false,online:0,requires_takeover:false,expires_at:null,document_id:args.documentId,title:null,message:"平板未连接。请用二维码配对，或确认平板已打开这份电脑上的手稿。"};if(behavior.pushNeedsTakeover&&!args.allowTransfer)return{pushed:false,online:1,requires_takeover:true,expires_at:null,document_id:args.documentId,title:"手稿",message:"这份手稿正在由平板书写。推送会转交写入权，对方未同步的内容会被隔离，可以导出恢复草稿。"};return{pushed:true,online:1,requires_takeover:false,expires_at:Math.floor(Date.now()/1000)+120,document_id:args.documentId,title:"手稿",message:"已向 1 台在线平板发送切换请求。"};}
  if(command==="create_ink_pairing"){behavior.pairingArgs.push(structuredClone(args));if(behavior.strictPairing&&behavior.remoteDocumentId===args.documentId&&!args.allowTransfer)throw new Error("ink invariant failed: takeover_required");behavior.pairingPending=true;return{document_id:args.documentId,expires_at:Date.now()/1000+120,urls:["https://192.168.31.20:1443/pad/#pair="+"a".repeat(64),"https://100.90.0.2:1443/pad/#pair="+"a".repeat(64),"https://127.0.0.1:1443/pad/#pair="+"a".repeat(64)],url_kinds:behavior.noLanAddress?["vpn","vpn","loopback"]:["lan","vpn","loopback"]};}
  if(command==="start_ink_setup"){behavior.setupStep=0;behavior.setupDocumentId=(args.documentId as string|null)??null;behavior.handoff=behavior.setupDocumentId?"idle":"unbound";return{active:true,expires_at:Date.now()/1000+900,urls:["http://192.168.31.20:1442/setup/#setup="+"b".repeat(64),"http://100.90.0.2:1442/setup/#setup="+"b".repeat(64),"http://127.0.0.1:1442/setup/#setup="+"b".repeat(64),"http://localhost:1442/setup/#setup="+"b".repeat(64)],url_kinds:["lan","vpn","loopback","loopback"],opened:false,certificate_downloaded:false,https_trusted:false,check:null,complete:false};}
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
    const extraDetail = extraCommands.get(command);
    if (extraDetail) return extraDetail(args);
    const receipt = [...evidence.values()].find(receipt => receipt.span_id === args.nodeId)!;
    const snapshot = [...snapshots.values()].flat().find(snapshot => snapshot.snapshot_id === receipt.snapshot_id)!;
    return {node_id:receipt.span_id,kind:"SourceSpan",display_title:"手稿证据",artifact_id:receipt.artifact_id,byte_range:null,excerpt:null,markdown_block_ids:[],payload:{},ink:{snapshot_id:snapshot.snapshot_id,document_id:snapshot.document_id,page_id:receipt.page_id,stroke_ids:receipt.stroke_ids}};
  }
  if (command === "get_map_object_edit_context") throw new Error("对象不存在");
  if (command === "get_artifact_content_info") {
    const snapshot = [...snapshots.values()].flat().find(snapshot => snapshot.artifact_id === args.artifactId);
    if (snapshot) return {artifact_id:snapshot.artifact_id,media_type:"application/json",byte_size:snapshot.byte_size,contexts:[],ink_snapshot_id:snapshot.snapshot_id};
    const extraInfo = extraCommands.get("get_artifact_content_info");
    if (extraInfo) return extraInfo(args);
    throw new Error("artifact not found: " + String(args.artifactId));
  }
  if (command === "rename_ink_snapshot") {
    // Mirrors ink_snapshot_label.rs: trimmed, ≤ 40 chars, optimistic
    // label_revision, idempotent request ids; bytes/hash never change.
    const request = args.request as { request_id: string; snapshot_id: string; label: string; expected_label_revision: number };
    snapshotLabelCalls.push(structuredClone(request));
    if (behavior.snapshotLabelDelay) await new Promise(resolve => setTimeout(resolve, behavior.snapshotLabelDelay));
    if (snapshotLabelRequests.has(request.request_id)) return { ...(snapshotLabelRequests.get(request.request_id) as object), replayed: true };
    const snapshot = [...snapshots.values()].flat().find(item => item.snapshot_id === request.snapshot_id);
    if (!snapshot) throw structuredError("snapshot_not_found", "快照不存在或已被移除。");
    if (notes.get(snapshot.document_id)?.lifecycle === "trashed") throw structuredError("document_trashed", "手稿已在回收站中，恢复后才能继续编辑。");
    if (behavior.snapshotLabelConflict !== null) { snapshot.label = behavior.snapshotLabelConflict || null; snapshot.label_revision = (snapshot.label_revision ?? 0) + 1; behavior.snapshotLabelConflict = null; }
    if ((snapshot.label_revision ?? 0) !== request.expected_label_revision) throw structuredError("snapshot_label_conflict", "快照名称已在别处修改，请选择保留哪一个。", { snapshot_id: snapshot.snapshot_id, current_label: snapshot.label ?? null, current_label_revision: snapshot.label_revision ?? 0 });
    const label = request.label.trim();
    if ([...label].length > 40) throw structuredError("snapshot_label_invalid", "快照名称最多 40 个字符");
    if ((label || null) !== (snapshot.label ?? null)) { snapshot.label = label || null; snapshot.label_revision = (snapshot.label_revision ?? 0) + 1; }
    const { manifest_json: _manifest, ...summary } = snapshot;
    void _manifest;
    const receipt = { request_id: request.request_id, snapshot: structuredClone(summary), replayed: false };
    snapshotLabelRequests.set(request.request_id, receipt);
    return receipt;
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
      label: null,
      label_revision: 0,
    };
    const previous = snapshots.get(request.document_id)!.find(stored => stored.snapshot_id === snapshot.snapshot_id);
    if (previous) return previous;
    snapshots.get(request.document_id)!.push(snapshot);
    return snapshot;
  }
  const extra = extraCommands.get(command);
  if (extra) return extra(args);
  throw new Error(`Unexpected Ink command: ${command}`);
});


/** Extra commands a harness adds (e.g. the app shell's snapshot). */
export const extraCommands = new Map<string, (args: Record<string, unknown>) => unknown>();
export { snapshotLabelCalls };
export { a, b, documents, sessions, snapshots, evidence, evidenceRequests, calls, notes, noteCalls, noteState, behavior, links, linkCalls, exports, sourceLabels, overlayCalls };
