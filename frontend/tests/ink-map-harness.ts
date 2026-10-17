import { mockIPC } from "@tauri-apps/api/mocks";
import { GraphController } from "../src/graph/graph-controller";
import { InspectorController } from "../src/inspector/inspector-controller";
import { eventBus } from "../src/core/events";
import { setInkContextCreator, type InkLinkOverlay } from "../src/ink/links";
import type { DesktopSnapshot } from "../src/artifact/types";
import type { IncidenceNode } from "../src/graph/projection";
import "../src/styles.css";

const entity = (id: string, domainId: string, kind: string, label: string, lifecycle = "active"): IncidenceNode => ({
  id, element_kind: "entity", domain_id: domainId, domain_kind: kind, relation_kind: null, layer: "source", label, summary: label + " 摘要", lifecycle, parent: null, boundary: false,
});
const snapshot = {
  workspace_id: "w", workspace_root: "/w", artifact_count: 1, conversion_count: 1, marker: { configured: false, ready: false, executable: null, version: null, problem: null }, pdf_artifacts: [],
  landscape_graph: { schema_version: "mathscape.graph-projection.v1", lens: "source", edges: [], nodes: [
    entity("r-art", "md-art", "SourceArtifact", "数学笔记.md"),
    entity("r-thm", "unit-thm", "SourceSemanticUnit", "定理 3.2"),
    entity("r-ex", "unit-ex", "SourceSemanticUnit", "例 4"),
    entity("r-old", "unit-old", "SourceSemanticUnit", "旧引理", "superseded"),
  ] },
} as unknown as DesktopSnapshot;

const note = (id: string, title: string) => ({ document_id: id, title, title_mode: "auto", title_revision: 0, lifecycle: "active", lifecycle_revision: 0, trashed_at: null, last_used_at: 1, created_at: "unix:1" });
const linkTo = (documentId: string, targetId: string, role: string, primary: boolean) => ({ link_id: `l-${documentId}-${targetId}`, document_id: documentId, target_kind: "source_object", target_key: `src:${targetId}`, target_id: targetId, target: { kind: "source_object", object_id: targetId }, target_revision: null, target_label: targetId, role, is_primary: primary, state: "active", created_at: 1, removed_at: null, target_status: "active" });
const many = Array.from({ length: 65 }, (_, i) => `bulk-${String(i).padStart(2, "0")}`);
const overlay: InkLinkOverlay = {
  counts: [
    { target_kind: "source_object", target_id: "unit-thm", note_count: 2 },
    { target_kind: "markdown", target_id: "md-art", note_count: 1 },
  ],
  links: [
    { document_id: "note-1", title: "尝试 · 定理 3.2", last_used_at: 30, target_kind: "source_object", target_id: "unit-thm", role: "attempt", is_primary: true },
    { document_id: "note-2", title: "另证 · 定理 3.2", last_used_at: 20, target_kind: "source_object", target_id: "unit-thm", role: "alt_proof", is_primary: true },
    { document_id: "note-2", title: "另证 · 定理 3.2", last_used_at: 20, target_kind: "source_object", target_id: "unit-ex", role: "reference", is_primary: false },
    { document_id: "note-md", title: "学习笔记 · 第一章", last_used_at: 10, target_kind: "markdown", target_id: "md-art", role: "study", is_primary: true },
  ],
};
const bulk = { enabled: false };
const state = { creates: [] as unknown[], opened: [] as string[], overlayCalls: 0 };

mockIPC(async (command, payload) => {
  const args = payload as Record<string, unknown>;
  if (command === "list_ink_links_to_target") {
    if (args.targetKind === "source_object" && args.targetId === "unit-thm") return [
      { note: note("note-1", "尝试 · 定理 3.2"), link: linkTo("note-1", "unit-thm", "attempt", true) },
      { note: note("note-2", "另证 · 定理 3.2"), link: linkTo("note-2", "unit-thm", "alt_proof", true) },
    ];
    if (args.targetKind === "markdown" && args.targetId === "md-art") return [{ note: note("note-md", "学习笔记 · 第一章"), link: { ...linkTo("note-md", "md-art", "study", true), target_kind: "markdown" } }];
    return [];
  }
  if (command === "get_source_node_detail" || command === "get_map_object_edit_context") throw new Error("not in this harness");
  throw new Error("Unexpected IPC: " + command);
});

const $ = (id: string) => document.getElementById(id)!;
let inspector!: InspectorController;
const graph = new GraphController({
  graphContainer: $("graph"), graphLegend: $("legend"), focusBar: $("focus-bar"), focusRootLabel: $("focus-root"), focusSummary: $("focus-summary"),
  nodeFilterGroup: $("filters"), renderModeToggle: $("render-mode") as HTMLInputElement, emptyState: $("empty"), emptyStateTitle: $("empty-title"), emptyStateMessage: $("empty-message"),
}, {
  onSelectNode: id => inspector.selectNodeById(id), onRequestCreateUnit: () => {}, onRequestCreateRelation: () => {},
  onBeforeRenderGraph: () => {}, onGraphRendered: () => {}, showToast: () => {}, getSnapshot: () => snapshot,
  onOpenInkNote: documentId => state.opened.push(documentId),
});
inspector = new InspectorController($("detail"), {
  getSnapshot: () => snapshot, showToast: () => {}, copyIdentifier: () => {}, onFocusNode: () => {},
  onOpenReader: () => {}, onOpenReaderWithFocus: () => {}, createParseForm: () => document.createElement("form"),
  refreshSnapshot: async () => {}, canFocus: () => false, isFocusRoot: () => false,
});
setInkContextCreator(async (target, choice) => { state.creates.push({ target, choice }); });
eventBus.on("open-ink-document", ({ documentId }) => state.opened.push(documentId));

const currentOverlay = (): InkLinkOverlay => bulk.enabled
  ? { counts: [...overlay.counts, { target_kind: "source_object", target_id: "unit-ex", note_count: many.length + 1 }], links: [...overlay.links, ...many.map((id, i) => ({ document_id: id, title: id, last_used_at: 100 - i, target_kind: "source_object" as const, target_id: "unit-ex", role: "attempt" as const, is_primary: true }))] }
  : overlay;
graph.renderCurrentGraph(snapshot);
graph.setInkOverlay(currentOverlay());

const harness = {
  state, bulk,
  reload: () => { graph.renderCurrentGraph(snapshot); graph.setInkOverlay(currentOverlay()); },
  rerender: () => graph.renderCurrentGraph(snapshot),
  labels: () => Object.fromEntries(graph.getGraphCore()!.nodes().map(node => [node.id(), String(node.data("label"))])),
  inkNodes: () => graph.getGraphCore()!.nodes(".ink-note-node").map(node => node.id()),
  inkEdges: () => graph.getGraphCore()!.edges(".ink-note-edge").map(edge => [edge.source().id(), edge.target().id()]),
  tap: (id: string): void => { graph.getGraphCore()!.$id(id).emit("tap"); },
  select: (id: string) => inspector.selectNodeById(id),
};
declare global { interface Window { inkMap: typeof harness } }
window.inkMap = harness;
