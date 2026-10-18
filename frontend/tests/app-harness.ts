// The real app shell (src/main.ts) over the in-memory Ink store: map ↔ 手写
// workspace switching, the shared overlay cache and the inspector run exactly
// as in the desktop app; only the Tauri commands are mocked.
import type { Core } from "cytoscape";
import { a, b, behavior, documents, extraCommands, links, noteCalls, notes, sourceLabels } from "./ink-mock-store";

const snapshot = {
  workspace_id: "w", workspace_root: "/w", artifact_count: 0, conversion_count: 0,
  marker: { configured: false, ready: false, executable: null, version: null, problem: null }, pdf_artifacts: [],
  landscape_graph: { schema_version: "mathscape.graph-projection.v1", lens: "source", edges: [], nodes: [...sourceLabels].map(([id, label]) => ({
    id: "r-" + id, element_kind: "entity", domain_id: id, domain_kind: "SourceSemanticUnit", relation_kind: null, layer: "source", label, summary: null, lifecycle: "active", parent: null, boundary: false,
  })) },
};
const unexpected: string[] = [];
extraCommands.set("get_desktop_snapshot", () => structuredClone(snapshot));
for (const command of ["list_discovery_runs", "list_parse_jobs", "list_review_items"]) extraCommands.set(command, () => []);
extraCommands.set("get_llm_settings_summary", () => ({ configured: false }));

await import("../src/main");

const core = (): Core => (document.querySelector("#graph") as unknown as { _cyreg: { cy: Core } } | null)?._cyreg?.cy
  ?? ([...document.querySelectorAll<HTMLElement>("div")].find(div => (div as unknown as { _cyreg?: unknown })._cyreg) as unknown as { _cyreg: { cy: Core } })._cyreg.cy;
const appHarness = {
  documents: { a: a.document_id, b: b.document_id }, notes, links, behavior, storedDocuments: documents, noteCalls, unexpected,
  labels: () => Object.fromEntries(core().nodes().map(node => [node.id(), String(node.data("label"))])),
  inkNodes: () => core().nodes(".ink-note-node").map(node => `${node.id()}=${String(node.data("label"))}`),
  selectNode: (id: string) => { core().getElementById(id).emit("tap"); },
};
declare global { interface Window { appHarness: typeof appHarness } }
window.appHarness = appHarness;
