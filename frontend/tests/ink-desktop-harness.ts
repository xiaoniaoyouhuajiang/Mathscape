import { DesktopInkWorkspace } from "../src/ink/desktop";
import { ArtifactReader } from "../src/reader/reader";
import { InspectorController } from "../src/inspector/inspector-controller";
import type { DesktopSnapshot } from "../src/artifact/types";
import { eventBus } from "../src/core/events";
import { connectInkViews } from "../src/ink/views";
import { GraphController } from "../src/graph/graph-controller";
import { a, b, documents, snapshots, evidence, evidenceRequests, calls, notes, noteCalls, behavior, links, linkCalls, exports, sourceLabels, overlayCalls } from "./ink-mock-store";
import { projectInk } from "../src/ink/model";
import "../src/styles.css";

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
// N2 map + inspector, wired exactly like the app shell (connectInkViews).
const mapSnapshot = {
  workspace_id: "w", workspace_root: "/w", artifact_count: 0, conversion_count: 0,
  marker: { configured: false, ready: false, executable: null, version: null, problem: null }, pdf_artifacts: [],
  landscape_graph: { schema_version: "mathscape.graph-projection.v1", lens: "source", edges: [], nodes: [...sourceLabels].map(([id, label]) => ({
    id: "r-" + id, element_kind: "entity", domain_id: id, domain_kind: "SourceSemanticUnit", relation_kind: null, layer: "source", label, summary: null, lifecycle: "active", parent: null, boundary: false,
  })) },
} as unknown as DesktopSnapshot;
const $id = (id: string) => document.getElementById(id)!;
let mapInspector!: InspectorController;
const graph = new GraphController({
  graphContainer: $id("n2-graph"), graphLegend: $id("n2-legend"), focusBar: $id("n2-focus-bar"), focusRootLabel: $id("n2-focus-root"), focusSummary: $id("n2-focus-summary"),
  nodeFilterGroup: $id("n2-filters"), renderModeToggle: $id("n2-render-mode") as HTMLInputElement, emptyState: $id("n2-empty"), emptyStateTitle: $id("n2-empty-title"), emptyStateMessage: $id("n2-empty-message"),
}, {
  onSelectNode: id => mapInspector.selectNodeById(id), onRequestCreateUnit: () => {}, onRequestCreateRelation: () => {},
  onBeforeRenderGraph: () => {}, onGraphRendered: () => {}, showToast: () => {}, getSnapshot: () => mapSnapshot,
});
mapInspector = new InspectorController($id("n2-detail"), {
  getSnapshot: () => mapSnapshot, showToast: () => {}, copyIdentifier: () => {}, onFocusNode: () => {},
  onOpenReader: () => {}, onOpenReaderWithFocus: () => {}, createParseForm: () => document.createElement("form"),
  refreshSnapshot: async () => {}, canFocus: () => false, isFocusRoot: () => false,
});
graph.renderCurrentGraph(mapSnapshot);
const views = connectInkViews({ applyOverlay: overlay => graph.setInkOverlay(overlay), refreshInspector: () => mapInspector.refreshIfUnlocked() });
void views.reloadOverlay();
const map = {
  overlayCalls,
  labels: () => Object.fromEntries(graph.getGraphCore()!.nodes().map(node => [node.id(), String(node.data("label"))])),
  inkNodes: () => graph.getGraphCore()!.nodes(".ink-note-node").map(node => `${node.id()}=${String(node.data("label"))}`),
  select: (id: string): void => { mapInspector.selectNodeById(id); },
};

/** N3e: page-space bounds of the visible strokes of the open document (after transforms). */
const visibleBounds = () => {
  const id = workspace.currentDocumentId();
  const document = id ? documents.get(id) : undefined;
  const points = document ? projectInk(document).visible.flatMap(stroke => stroke.points) : [];
  if (!points.length) return null;
  return { minX: Math.min(...points.map(p => p.x)), minY: Math.min(...points.map(p => p.y)), maxX: Math.max(...points.map(p => p.x)), maxY: Math.max(...points.map(p => p.y)) };
};
const visibleStrokeCount = () => {
  const id = workspace.currentDocumentId();
  const document = id ? documents.get(id) : undefined;
  return document ? projectInk(document).visible.length : 0;
};
const harness = { visibleBounds, visibleStrokeCount, map, links, linkCalls, exports, notes, noteCalls, workspace, documents: { a: a.document_id, b: b.document_id }, snapshots, calls, behavior, evidenceRequests, storedDocuments:documents };
declare global { interface Window { inkDesktopHarness: typeof harness } }
window.inkDesktopHarness = harness;
void workspace.open();
