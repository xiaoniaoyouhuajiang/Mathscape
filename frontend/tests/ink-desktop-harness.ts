import { mockIPC } from "@tauri-apps/api/mocks";
import { DesktopInkWorkspace } from "../src/ink/desktop";
import { newDocument, projectInk, inkMetadata, type InkDocument, type InkOperation, type InkDocumentMetadata } from "../src/ink/model";
import type { StoredInkSnapshot, InkEvidenceReceipt, InkEvidenceRequest } from "../src/ink/persistence";
import { ArtifactReader } from "../src/reader/reader";
import { InspectorController } from "../src/inspector/inspector-controller";
import type { DesktopSnapshot } from "../src/artifact/types";
import { eventBus } from "../src/core/events";
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
const behavior = { delayedDocumentId: "", delayMs: 0, appendDelay: 0, snapshotDelay: 0, failAppend: false, loseEvidenceAck: false, remoteDocumentId:"", pairingPending:false,epoch:1,setupStep:0,remoteConnected:true,remotePending:0,setupDocumentId:"" as string|null,handoff:"",transferConfirmation:false,pairingArgs:[] as unknown[],strictPairing:false,autoRequest:true };

mockIPC(async (command, payload) => {
  const args = payload as Record<string, unknown>;
  if(command==="get_ink_lan_mirror") {
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
    return [...documents.values()].map(document => ({
      document_id: document.document_id,
      title: document.title,
      head_seq: document.operations.length,
      active_session_id: sessions.get(document.document_id),
      updated_at: document.created_at,
    }));
  }
  if (command === "create_ink_document") {
    const request = args.request as { document: InkDocument; session_id: string; writer_id: string };
    const document = structuredClone(request.document);
    documents.set(document.document_id,document); sessions.set(document.document_id,request.session_id); snapshots.set(document.document_id,[]);
    return { ...request, document_id: document.document_id, writer_epoch:1,status:"active" };
  }
  if (command === "append_ink_operation") {
    const request = args.request as { request_id:string;document_id:string;expected_head:number;metadata:InkDocumentMetadata;operation:InkOperation };
    if (behavior.appendDelay) await new Promise(resolve => setTimeout(resolve,behavior.appendDelay));
    if (behavior.failAppend) throw new Error("injected save failure");
    if (operationReceipts.has(request.request_id)) return operationReceipts.get(request.request_id);
    const document = documents.get(request.document_id)!;
    if (document.operations.length !== request.expected_head) throw new Error("head conflict");
    Object.assign(document,structuredClone(request.metadata)); document.operations.push(structuredClone(request.operation));
    const receipt = {request_id:request.request_id,document_id:request.document_id,command_id:request.operation.command_id,seq:request.operation.seq,durable:true};
    operationReceipts.set(request.request_id,receipt); return receipt;
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
const harness = { workspace, documents: { a: a.document_id, b: b.document_id }, snapshots, calls, behavior, evidenceRequests, storedDocuments:documents };
declare global { interface Window { inkDesktopHarness: typeof harness } }
window.inkDesktopHarness = harness;
void workspace.open();
