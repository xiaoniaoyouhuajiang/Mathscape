// The real app shell (src/main.ts) over the in-memory Ink store: map ↔ 手写
// workspace switching, the shared overlay cache and the inspector run exactly
// as in the desktop app; only the Tauri commands are mocked.
import type { Core } from "cytoscape";
import { mockWindows } from "@tauri-apps/api/mocks";
import { a, b, behavior, documents, extraCommands, links, noteCalls, notes, snapshots, snapshotLabelCalls, sourceLabels } from "./ink-mock-store";

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
const contentFailures = new Set<string>();
const contentRequests: string[] = [];
const detailRequests: string[] = [];
const sourceDetailBehavior = { delayMs: 0, failIds: [] as string[] };
extraCommands.set("get_source_node_detail", async (args) => {
  const id = String(args.nodeId);
  detailRequests.push(id);
  if (sourceDetailBehavior.delayMs) await new Promise(resolve => setTimeout(resolve, sourceDetailBehavior.delayMs));
  if (sourceDetailBehavior.failIds.includes(id)) throw new Error("来源对象读取失败");
  return {
    node_id: id, kind: "SourceSemanticUnit", display_title: "定理 3.2", unit_kind: "statement", parent: null,
    artifact_id: "md-1", byte_range: { start: 0, end: mdBytes.length }, excerpt: mdSource,
    markdown_block_ids: ["b1"], payload: {},
  };
});
extraCommands.set("get_artifact_content_info", (args) => {
  const id = String(args.artifactId);
  contentRequests.push(id);
  if (contentFailures.has(id)) throw new Error("Artifact 不存在");
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

// N3d: a stand-in for search_workspace. Ranking is covered by the Rust
// tests; this returns real target shapes so navigation runs end to end.
const searchBehavior = { delayMs: 0, delays: {} as Record<string, number>, failNext: "", calls: [] as Array<{ text: string; include_trashed?: boolean }> };
const materials = [{ id: "pdf-notes", title: "notes" }, { id: "md-1", title: "正确表述.md" }];
extraCommands.set("search_workspace", async (args) => {
  const query = args.query as { text: string; include_trashed?: boolean };
  searchBehavior.calls.push(structuredClone(query));
  const delay = searchBehavior.delays[query.text] ?? searchBehavior.delayMs;
  if (delay) await new Promise(resolve => setTimeout(resolve, delay));
  if (searchBehavior.failNext) { const message = searchBehavior.failNext; searchBehavior.failNext = ""; throw new Error(message); }
  const needle = query.text.trim().toLowerCase();
  const hit = (kind: string, id: string, title: string, subtitle: string, target: unknown, trashed = false) => {
    const at = title.toLowerCase().indexOf(needle);
    return at < 0 ? [] : [{ kind, id, title, subtitle, matched_alias: null, score: 50, highlights: [[at, at + needle.length]], trashed, target }];
  };
  const groups = [
    { kind: "note", label: "手稿", hits: [...documents.values()].filter(d => query.include_trashed || notes.get(d.document_id)?.lifecycle !== "trashed")
      .flatMap(d => hit("note", d.document_id, d.title, notes.get(d.document_id)?.lifecycle === "trashed" ? "手稿 · 回收站" : "手稿", { kind: "ink_note", document_id: d.document_id }, notes.get(d.document_id)?.lifecycle === "trashed")) },
    { kind: "source_unit", label: "来源单元", hits: [...sourceLabels, ["unit-ghost", "定理 9.9（不在地图）"]].flatMap(([id, label]) => hit("source_unit", id, label, "命题", { kind: "source_object", object_id: id })) },
    { kind: "material", label: "材料", hits: materials.flatMap(m => hit("material", m.id, m.title, "材料", { kind: "artifact", artifact_id: m.id })) },
    { kind: "snapshot", label: "命名快照", hits: [...snapshots.values()].flat().filter(sn => sn.label && (query.include_trashed || notes.get(sn.document_id)?.lifecycle !== "trashed"))
      .flatMap(sn => hit("snapshot", sn.snapshot_id, sn.label!, `${documents.get(sn.document_id)!.title} · 第 ${sn.through_seq} 步`, { kind: "ink_snapshot", snapshot_id: sn.snapshot_id, artifact_id: sn.artifact_id, document_id: sn.document_id })) },
  ].filter(group => needle && group.hits.length).map(group => ({ ...group, total: group.hits.length }));
  return { query: query.text, total: groups.reduce((sum, group) => sum + group.total, 0), groups };
});

// H5: the Tauri window. getCurrentWindow().onCloseRequested registers through
// plugin:event|listen; a test fires the close request with requestWindowClose()
// and checks whether the app let the window be destroyed.
mockWindows("main");
const windowEvents = { listeners: [] as Array<{ event: string; handler: number }>, destroyed: 0 };
extraCommands.set("plugin:event|listen", (args) => {
  const handler = Number(args.handler);
  windowEvents.listeners.push({ event: String(args.event), handler });
  return handler;
});
extraCommands.set("plugin:event|unlisten", () => null);
extraCommands.set("plugin:window|destroy", () => { windowEvents.destroyed += 1; return null; });
const requestWindowClose = () => {
  const internals = (window as unknown as { __TAURI_INTERNALS__: { runCallback: (id: number, data: unknown) => void } }).__TAURI_INTERNALS__;
  const targets = windowEvents.listeners.filter(listener => listener.event === "tauri://close-requested");
  for (const { handler } of targets) internals.runCallback(handler, { event: "tauri://close-requested", id: handler, payload: null });
  return targets.length;
};

// S1 sandbox: the in-window binding survives the reload the switch triggers
// (sessionStorage), like DesktopState survives a webview reload.
const sandboxSession = { sandbox_session_id: "sbx_demo", state: "active", checkpoint_id: "ckpt_demo", checkpoint_digest: null,
  created_at: "2026-10-31T09:00:00+08:00", last_exported_at: null, export_count: 0, discarded_at: null, workspace_root: "/sandbox/sessions/sbx_demo/workspace" };
const sandboxCalls: Array<{ command: string; args: Record<string, unknown> }> = [];
const workspaceStatus = () => {
  const inSandbox = sessionStorage.getItem("harness.sandbox") === "1";
  return {
    motherWorkspaceRoot: "/w",
    currentWorkspaceRoot: inSandbox ? sandboxSession.workspace_root : "/w",
    sandbox: inSandbox ? { sandboxSessionId: "sbx_demo", checkpointId: "ckpt_demo", workspaceRoot: sandboxSession.workspace_root } : null,
  };
};
const transcript: Array<Record<string, unknown>> = [];
const sandboxEvents: Array<Record<string, unknown>> = [];
const emitDialogue = (payload: Record<string, unknown>) => {
  const internals = (window as unknown as { __TAURI_INTERNALS__: { runCallback: (id: number, data: unknown) => void } }).__TAURI_INTERNALS__;
  for (const { handler } of windowEvents.listeners.filter(listener => listener.event === "sandbox://dialogue")) {
    internals.runCallback(handler, { event: "sandbox://dialogue", id: handler, payload });
  }
};
extraCommands.set("sandbox_workspace_status", () => workspaceStatus());
extraCommands.set("sandbox_overview", () => ({
  sandboxRoot: "/sandbox", defaultExportDir: "/sandbox/exports", currentWorkspaceRoot: workspaceStatus().currentWorkspaceRoot,
  workspace: workspaceStatus(),
  checkpoints: [{ checkpoint_id: "ckpt_demo", label: "demo", root: "/w", registered_at: "2026-10-31T08:59:00+08:00" }],
  sessions: [sandboxSession],
}));
extraCommands.set("sandbox_enter_session", (args) => { sandboxCalls.push({ command: "enter", args }); sessionStorage.setItem("harness.sandbox", "1"); return workspaceStatus(); });
extraCommands.set("sandbox_return_to_mother", (args) => { sandboxCalls.push({ command: "return", args }); sessionStorage.removeItem("harness.sandbox"); return workspaceStatus(); });
extraCommands.set("sandbox_agent_status", () => ({ backend: "codex", codexProgram: "codex", mcpProgram: "mathscape", tools: ["1", "2", "3", "4", "5", "6", "7"],
  probes: [{ capability: "codex", state: "available", found_version: "0.124.0", required: null, install_hint: null }], available: true }));
extraCommands.set("sandbox_session_dialogue", (args) => ({ sandboxSessionId: String(args.sandboxSessionId), transcript: structuredClone(transcript), events: structuredClone(sandboxEvents), activeRoundId: null }));
extraCommands.set("sandbox_send_message", (args) => {
  sandboxCalls.push({ command: "send", args });
  const id = String(args.sandboxSessionId);
  const userEntry = { ts: new Date().toISOString(), role: "user", text: String(args.text), round_id: "r1" };
  transcript.push(userEntry);
  // Stream the round after the command resolves, as the Rust pump does.
  setTimeout(() => {
    const event = (seq: number, type: string, payload: Record<string, unknown>) => {
      const value = { round_id: "r1", turn_id: "t1", seq, ts: "2026-10-31T09:01:00+08:00", backend: "codex", type, payload };
      sandboxEvents.push(value);
      emitDialogue({ kind: "agent_event", sandbox_session_id: id, event: value });
    };
    event(0, "turn_start", { input_ref: "x" });
    event(1, "tool_call", { call_id: "c1", tool: "workspace.search", server: "mathscape", args_summary: "query=紧集" });
    event(2, "tool_result", { call_id: "c1", status: "ok", duration_ms: 12 });
    event(3, "usage", { input_tokens: 120, output_tokens: 30 });
    event(4, "turn_end", { outcome: "completed" });
    const reply = { ts: new Date().toISOString(), role: "agent", text: "找到 1 条：定理 3.2。", round_id: "r1", outcome: "completed" };
    transcript.push(reply);
    emitDialogue({ kind: "transcript", sandbox_session_id: id, entry: reply });
    emitDialogue({ kind: "round_finished", sandbox_session_id: id, round_id: "r1" });
  }, 30);
  return { roundId: "r1", userEntry };
});
extraCommands.set("sandbox_cancel_round", () => false);

await import("../src/main");

const core = (): Core => (document.querySelector("#graph") as unknown as { _cyreg: { cy: Core } } | null)?._cyreg?.cy
  ?? ([...document.querySelectorAll<HTMLElement>("div")].find(div => (div as unknown as { _cyreg?: unknown })._cyreg) as unknown as { _cyreg: { cy: Core } })._cyreg.cy;
const appHarness = {
  documents: { a: a.document_id, b: b.document_id }, notes, links, behavior, storedDocuments: documents, noteCalls, unexpected,
  windowEvents, requestWindowClose,
  sandboxCalls,
  contentFailures, contentRequests, detailRequests, sourceDetailBehavior, searchBehavior, snapshots, snapshotLabelCalls,
  /** Cytoscape state of a node: selected flag and distance from the canvas centre. */
  nodeView: (id: string) => {
    const cy = core(), node = cy.getElementById(id);
    if (node.empty()) return null;
    // cy.center() centres the bounding box (node + label), so compare that.
    const box = node.renderedBoundingBox({});
    return { selected: node.selected(), dx: (box.x1 + box.x2) / 2 - cy.width() / 2, dy: (box.y1 + box.y2) / 2 - cy.height() / 2, width: cy.width(), height: cy.height() };
  },
  labels: () => Object.fromEntries(core().nodes().map(node => [node.id(), String(node.data("label"))])),
  inkNodes: () => core().nodes(".ink-note-node").map(node => `${node.id()}=${String(node.data("label"))}`),
  selectNode: (id: string) => { core().getElementById(id).emit("tap"); },
  panTo: (x: number, y: number) => { core().pan({ x, y }); },
};
declare global { interface Window { appHarness: typeof appHarness } }
window.appHarness = appHarness;
