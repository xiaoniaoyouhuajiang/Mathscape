// The real app shell (src/main.ts) over the in-memory Ink store: map ↔ 手写
// workspace switching, the shared overlay cache and the inspector run exactly
// as in the desktop app; only the Tauri commands are mocked.
import type { Core } from "cytoscape";
import { a, b, behavior, documents, extraCommands, links, noteCalls, notes, sourceLabels } from "./ink-mock-store";

const mdSource = "# 正确表述\n\n设 $a_n$ 为数列。\n";
const snapshot = {
  workspace_id: "w", workspace_root: "/w", artifact_count: 1, conversion_count: 0,
  marker: { configured: false, ready: false, executable: null, version: null, problem: null },
  pdf_artifacts: [{ artifact_id: "pdf-notes", filename: "notes.pdf", page_count: 2 }],
  landscape_graph: { schema_version: "mathscape.graph-projection.v1", lens: "source", edges: [], nodes: [
    ...[...sourceLabels].map(([id, label]) => ({
      id: "r-" + id, element_kind: "entity", domain_id: id, domain_kind: "SourceSemanticUnit", relation_kind: null, layer: "source", label, summary: null, lifecycle: "active", parent: null, boundary: false,
    })),
    { id: "r-pdf-notes", element_kind: "entity", domain_id: "pdf-notes", domain_kind: "SourceArtifact", relation_kind: null, layer: "source", label: "notes.pdf", summary: null, lifecycle: "active", parent: null, boundary: false },
    { id: "r-md-1", element_kind: "entity", domain_id: "md-1", domain_kind: "SourceArtifact", relation_kind: null, layer: "source", label: "正确表述.md", summary: null, lifecycle: "active", parent: null, boundary: false },
  ] },
};
const unexpected: string[] = [];
extraCommands.set("get_desktop_snapshot", () => structuredClone(snapshot));
for (const command of ["list_discovery_runs", "list_parse_jobs", "list_review_items"]) extraCommands.set(command, () => []);
extraCommands.set("get_llm_settings_summary", () => ({ configured: false }));
const mdBytes = new TextEncoder().encode(mdSource);
extraCommands.set("get_artifact_content_info", (args) => {
  const id = String(args.artifactId);
  if (id === "md-1") return { artifact_id: id, media_type: "text/markdown", byte_size: mdBytes.length, contexts: [{ conversion_id: "c1", paths: [{ path: "notes.md", artifact_id: "md-1" }] }] };
  if (id === "pdf-notes") return { artifact_id: id, media_type: "application/pdf", byte_size: 100, contexts: [] };
  throw new Error("artifact not found: " + id);
});
extraCommands.set("read_artifact_bytes", (args) => {
  const id = String(args.artifactId);
  const offset = Number(args.offset ?? 0);
  const length = Number(args.length ?? 0);
  if (id === "md-1") return mdBytes.slice(offset, offset + length).buffer;
  return new ArrayBuffer(0);
});
extraCommands.set("get_markdown_document", () => ({
  schema_version: "mathscape.anchored-markdown.v1",
  artifact_id: "md-1",
  conversion_id: "c1",
  source: mdSource,
  blocks: [{ id: "b1", kind: "heading", range: { start: 0, end: mdBytes.length } }],
  snapshot: { content_hash: "sha256:test", byte_size: mdBytes.length },
}));

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
