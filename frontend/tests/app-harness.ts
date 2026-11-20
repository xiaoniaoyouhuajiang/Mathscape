// The real app shell (src/main.ts) over the in-memory Ink store: map ↔ 手写
// workspace switching, the shared overlay cache and the inspector run exactly
// as in the desktop app; only the Tauri commands are mocked.
import type { Core } from "cytoscape";
import type { InvestigationSnapshot } from "../src/artifact/types";
import { mockWindows } from "@tauri-apps/api/mocks";
import { a, b, behavior, documents, extraCommands, links, noteCalls, notes, snapshots, snapshotLabelCalls, sourceLabels } from "./ink-mock-store";

const mdSource = "# 正确表述\n\n设 $a_n$ 为数列。\n";
const snapshot = {
  workspace_id: "w", workspace_root: "/w", artifact_count: 1, conversion_count: 0,
  marker: { configured: false, ready: false, executable: null, version: null, problem: null },
  lean: { root: null, source: null, root_exists: false, profiles: [], problem: "未找到 Lean profile 目录" },
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
// A-PR3: inside the sandbox the map also holds one unit with a canonical
// stable id, so cite chips and the read-path echo have something to hit.
const citeUnitId = "mathscape:source-unit/" + "ab12".repeat(16);
const citeNode = { id: "r-cite-unit", element_kind: "entity", domain_id: citeUnitId, domain_kind: "SourceSemanticUnit", relation_kind: null, layer: "source", label: "引理 1.4", summary: null, lifecycle: "active", parent: null, boundary: false };
// Composer @ mentions: a second canonical node so the menu has something to filter.
const citeArtId = "mathscape:artifact/" + "cd34".repeat(16);
const citeArtNode = { id: "r-cite-art", element_kind: "entity", domain_id: citeArtId, domain_kind: "SourceArtifact", relation_kind: null, layer: "source", label: "紧集讲义.pdf", summary: null, lifecycle: "active", parent: null, boundary: false };
const richMapNodes = [
  { id: "r-meta-1", element_kind: "entity", domain_id: "meta-1", domain_kind: "SourceArtifact", relation_kind: null, layer: "source", label: "notes · PDF 1-2 页 · 解析元数据", summary: "application/json · 128 bytes · available", lifecycle: "active", parent: null, boundary: false },
  { id: "r-span-1", element_kind: "entity", domain_id: "span-1", domain_kind: "SourceSpan", relation_kind: null, layer: "source", label: "来源范围", summary: "Markdown 字节范围", lifecycle: "active", parent: null, boundary: true },
  { id: "r-sem-t", element_kind: "entity", domain_id: "sem-t", domain_kind: "SemanticEntity", relation_kind: null, layer: "global_semantic", label: "理论：实数与序列", summary: "理论", lifecycle: "active", parent: null, boundary: false },
  { id: "r-sem-c", element_kind: "entity", domain_id: "sem-c", domain_kind: "SemanticEntity", relation_kind: null, layer: "global_semantic", label: "概念：收敛数列", summary: "概念", lifecycle: "active", parent: null, boundary: false },
  { id: "rel-loc-1", element_kind: "relation", domain_id: "rel:loc:1", domain_kind: "Evidence", relation_kind: "located_in", layer: "source", label: "来源于 Markdown", summary: null, lifecycle: "active", parent: null, boundary: false },
  { id: "rel-cs-1", element_kind: "relation", domain_id: "rel:cs:1", domain_kind: "SourceUnitRelation", relation_kind: "contains_span", layer: "source", label: "对应原文", summary: null, lifecycle: "active", parent: null, boundary: false },
  { id: "rel-it-1", element_kind: "relation", domain_id: "rel:it:1", domain_kind: "SemanticEntity", relation_kind: "in_theory", layer: "global_semantic", label: "属于理论", summary: null, lifecycle: "active", parent: null, boundary: false },
  { id: "rel-al-1", element_kind: "relation", domain_id: "rel:al:1", domain_kind: "Alignment", relation_kind: "aligns", layer: "alignment", label: "说的就是它", summary: null, lifecycle: "active", parent: null, boundary: false },
];
const richMapEdges = [
  { id: "e-loc-in", source: "r-span-1", target: "rel-loc-1", relation_id: "rel:loc:1", role: "span", ordinal: null, direction: "into_relation", directed: true },
  { id: "e-loc-out", source: "rel-loc-1", target: "r-md-1", relation_id: "rel:loc:1", role: "artifact", ordinal: null, direction: "out_of_relation", directed: true },
  { id: "e-cs-in", source: "r-cite-unit", target: "rel-cs-1", relation_id: "rel:cs:1", role: "unit", ordinal: null, direction: "into_relation", directed: true },
  { id: "e-cs-out", source: "rel-cs-1", target: "r-span-1", relation_id: "rel:cs:1", role: "span", ordinal: null, direction: "out_of_relation", directed: true },
  { id: "e-it-in", source: "r-sem-c", target: "rel-it-1", relation_id: "rel:it:1", role: "member", ordinal: null, direction: "into_relation", directed: true },
  { id: "e-it-out", source: "rel-it-1", target: "r-sem-t", relation_id: "rel:it:1", role: "theory", ordinal: null, direction: "out_of_relation", directed: true },
  { id: "e-al-in", source: "r-cite-unit", target: "rel-al-1", relation_id: "rel:al:1", role: "source", ordinal: null, direction: "into_relation", directed: true },
  { id: "e-al-out", source: "rel-al-1", target: "r-sem-c", relation_id: "rel:al:1", role: "target", ordinal: null, direction: "out_of_relation", directed: true },
];
extraCommands.set("get_desktop_snapshot", () => {
  const custom = (window as unknown as { __CUSTOM_BUNDLE__?: { snapshot: unknown } }).__CUSTOM_BUNDLE__;
  if (custom?.snapshot) return structuredClone(custom.snapshot);
  const value = structuredClone(snapshot);
  if (sessionStorage.getItem("harness.sandbox") === "1") value.landscape_graph.nodes.push(structuredClone(citeNode), structuredClone(citeArtNode));
  if (sessionStorage.getItem("harness.richMap") === "1") {
    (value.landscape_graph.nodes as unknown[]).push(structuredClone(citeNode), ...structuredClone(richMapNodes));
    (value.landscape_graph.edges as unknown[]).push(...structuredClone(richMapEdges));
  }
  // Same backend projection/marks contract as real investigation maps.
  if (sessionStorage.getItem("harness.themeStates") === "1") {
    (value.landscape_graph.nodes as unknown[]).push(
      { id: "r-theme-candidate", element_kind: "entity", domain_id: "theme-candidate", domain_kind: "SemanticEntity", relation_kind: null, layer: "global_semantic", label: "有界性", summary: "概念", lifecycle: "active", parent: null, boundary: false },
      { id: "r-theme-debt", element_kind: "entity", domain_id: "theme-debt", domain_kind: "Debt", relation_kind: null, layer: "global_semantic", label: "Debt：待补假设", summary: "待补紧性条件", lifecycle: "active", parent: null, boundary: false },
    );
    (value as typeof value & { investigation: InvestigationSnapshot }).investigation = {
      investigation_id: "theme-investigation",
      staged: { "theme-candidate": { staged: true, change_set_id: "theme-proposal", change_set_version: 1, change_sets: ["theme-proposal@1"], replaces_revision: null } },
      conflicts: [],
    };
  }
  // Sync ensure: while the harness "Marker" runs, the PDF node says 解析中.
  if (sandboxRound.parsingNow) {
    const pdf = value.landscape_graph.nodes.find((node: { id: string }) => node.id === "r-pdf-notes");
    if (pdf) (pdf as { summary: string | null }).summary = "PDF · 解析中 第3–4页";
  }
  return value;
});
for (const command of ["list_discovery_runs", "list_parse_jobs", "list_review_items"]) extraCommands.set(command, () => []);
extraCommands.set("get_llm_settings_summary", () => ({ configured: false }));
const mdBytes = new TextEncoder().encode(mdSource);
function tinySamplePdf(): Uint8Array {
  const s1 = "0 0 0 rg BT /F1 14 Tf 24 340 Td (Page 1 Original Statement) Tj ET\n";
  const s2 = "0 0 0 rg BT /F1 14 Tf 24 340 Td (Page 2 Formula C_n^r + C_n^{r+1}) Tj ET\n";
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Resources << /Font << /F1 7 0 R >> >> /Contents 5 0 R >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Resources << /Font << /F1 7 0 R >> >> /Contents 6 0 R >>",
    `<< /Length ${s1.length} >>\nstream\n${s1}\nendstream`,
    `<< /Length ${s2.length} >>\nstream\n${s2}\nendstream`,
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
  ];
  let data = "%PDF-1.4\n";
  const offsets = [0];
  objects.forEach((body, i) => { offsets.push(data.length); data += `${i + 1} 0 obj\n${body}\nendobj\n`; });
  const xref = data.length;
  data += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  offsets.slice(1).forEach((offset) => { data += `${String(offset).padStart(10, "0")} 00000 n \n`; });
  data += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF`;
  return new TextEncoder().encode(data);
}
const pdfBytes = tinySamplePdf();
const errataState = {
  items: [] as Array<{
    errata_id: string; artifact_id: string; base_content_hash: string;
    first_block_id: string; last_block_id: string; covered_block_ids: string[];
    base_byte_start: number; base_byte_end: number; base_text_hash: string;
    original_text: string; corrected_text: string; status: "active" | "reverted" | "superseded";
    source_kind: "manual" | "discovery_patch"; actor: string; note: string | null;
    verified_pdf_page: number | null; created_at: string; updated_at: string;
  }>,
  reviews: new Map<string, "acknowledged" | "sent_to_sandbox">(),
  seq: 1,
};
const syncMockMarkdownLinkLabels = () => {
  const active = errataState.items.find((e) => e.artifact_id === "md-1" && e.status === "active");
  const label = active ? active.corrected_text.replace(/^#+\s*/, "").trim().slice(0, 32) : "正确表述";
  for (const link of links) {
    if (link.target_kind === "markdown" && link.target_id === "md-1") {
      link.target_label = label;
      if (link.target.kind === "markdown") {
        link.target.label = label;
        link.target.excerpt = active ? active.corrected_text : mdSource;
      }
    }
  }
};
extraCommands.set("list_markdown_errata", (args) =>
  structuredClone(errataState.items.filter((e) => e.artifact_id === String(args.artifactId) && e.status === "active")),
);
extraCommands.set("save_markdown_errata", (args) => {
  const req = args.request as { artifact_id: string; first_block_id: string; last_block_id: string; corrected_text: string; note?: string | null; verified_pdf_page?: number | null };
  const superseded: string[] = [];
  for (const prev of errataState.items) {
    if (prev.artifact_id === req.artifact_id && prev.status === "active") {
      prev.status = "superseded";
      superseded.push(prev.errata_id);
    }
  }
  const entry = {
    errata_id: `mathscape:markdown-errata/app-${errataState.seq++}`,
    artifact_id: req.artifact_id,
    base_content_hash: "sha256:test",
    first_block_id: req.first_block_id,
    last_block_id: req.last_block_id,
    covered_block_ids: [req.first_block_id],
    base_byte_start: 0,
    base_byte_end: mdBytes.length,
    base_text_hash: "sha256:orig",
    original_text: mdSource,
    corrected_text: req.corrected_text,
    status: "active" as const,
    source_kind: "manual" as const,
    actor: "mathscape:actor/desktop-user",
    note: req.note ?? null,
    verified_pdf_page: req.verified_pdf_page ?? null,
    created_at: "2026-11-19T00:00:00Z",
    updated_at: "2026-11-19T00:00:00Z",
  };
  errataState.items.push(entry);
  syncMockMarkdownLinkLabels();
  return { errata: structuredClone(entry), superseded_ids: superseded, affected_unit_ids: ["unit-thm"] };
});
extraCommands.set("revert_markdown_errata", (args) => {
  const target = errataState.items.find((e) => e.errata_id === String(args.errataId));
  if (!target) throw new Error("未找到勘误记录");
  target.status = "reverted";
  syncMockMarkdownLinkLabels();
  return { errata: structuredClone(target), affected_unit_ids: ["unit-thm"] };
});
extraCommands.set("mark_errata_unit_reviewed", (args) => {
  const unitId = String(args.unitId);
  const errataIds = (args.errataIds as string[]) ?? [];
  const decision = String(args.decision) as "acknowledged" | "sent_to_sandbox";
  for (const eid of errataIds) errataState.reviews.set(`${unitId}:${eid}`, decision);
  return errataIds.length;
});
extraCommands.set("list_unreviewed_errata_units", () => {
  const active = errataState.items.filter((e) => e.artifact_id === "md-1" && e.status === "active");
  const unreviewed = active.filter((e) => errataState.reviews.get(`unit-thm:${e.errata_id}`) !== "acknowledged");
  return unreviewed.length ? { "unit-thm": unreviewed.length } : {};
});
const contentFailures = new Set<string>();
const contentRequests: string[] = [];
const detailRequests: string[] = [];
const sourceDetailBehavior = { delayMs: 0, failIds: [] as string[] };
extraCommands.set("get_source_node_detail", async (args) => {
  const id = String(args.nodeId);
  detailRequests.push(id);
  if (sourceDetailBehavior.delayMs) await new Promise(resolve => setTimeout(resolve, sourceDetailBehavior.delayMs));
  if (sourceDetailBehavior.failIds.includes(id)) throw new Error("来源对象读取失败");
  const active = errataState.items.filter((e) => e.artifact_id === "md-1" && e.status === "active");
  const unreviewed = active.filter((e) => errataState.reviews.get(`${id}:${e.errata_id}`) !== "acknowledged");
  const effectiveExcerpt = active.length ? active[active.length - 1].corrected_text : mdSource;
  return {
    node_id: id, kind: "SourceSemanticUnit", display_title: "定理 3.2", unit_kind: "statement", parent: null,
    artifact_id: "md-1", byte_range: { start: 0, end: mdBytes.length }, excerpt: effectiveExcerpt,
    markdown_block_ids: ["b1"], payload: {},
    pdf_backlink: { kind: "exact_pdf_pages", pdf_artifact_id: "pdf-notes", pdf_page_indices: [1], marker_page_ids: [1] },
    errata_review: unreviewed.length ? {
      unit_id: id,
      items: unreviewed.map((e) => ({
        errata_id: e.errata_id,
        artifact_id: e.artifact_id,
        first_block_id: e.first_block_id,
        last_block_id: e.last_block_id,
        original_text: e.original_text,
        corrected_text: e.corrected_text,
        source_kind: e.source_kind,
        updated_at: e.updated_at,
        sent_to_sandbox: errataState.reviews.get(`${id}:${e.errata_id}`) === "sent_to_sandbox",
      })),
    } : undefined,
  };
});
extraCommands.set("get_source_backlink", () => ({
  kind: "exact_pdf_pages", pdf_artifact_id: "pdf-notes", pdf_page_indices: [1], marker_page_ids: [1],
}));
const harnessStudyMarks = new Map<string, string>();
extraCommands.set("set_study_marks", (args) => {
  const updates = (args.updates as Array<{ object_id: string; status: string }>) ?? [];
  for (const u of updates) harnessStudyMarks.set(u.object_id, u.status);
  return updates.map((u) => ({ object_id: u.object_id, status: u.status, updated_at: "unix:1", note: null }));
});
extraCommands.set("semantic_learning_view", () => {
  const custom = (window as unknown as { __CUSTOM_BUNDLE__?: { learningView: unknown } }).__CUSTOM_BUNDLE__;
  if (custom?.learningView) return structuredClone(custom.learningView);
  return {
  materials: [
    {
      artifact_id: "md-1",
      units: [
        {
          unit_id: citeUnitId,
          unit_title: "引理 1.4",
          alignment_id: "rel:al:1",
          entity_id: "sem-c",
          entity_label: "概念：收敛数列",
          relation: "expresses",
          label: "新的",
          seen_in: [],
          only_differs_by: ["仅在度量空间下给出"],
        },
      ],
      new_conventions: [],
      conventions_detail: [],
      open_debts: [],
      outline_units: [
        {
          unit_id: "unit-thm",
          unit_title: "定理 3.2",
          unit_kind: "section",
          parent_unit: null,
          study_status: harnessStudyMarks.get("unit-thm") ?? "unread",
          category: "unaligned",
          targets: [],
        },
        {
          unit_id: citeUnitId,
          unit_title: "引理 1.4",
          unit_kind: "statement",
          parent_unit: "unit-thm",
          study_status: harnessStudyMarks.get(citeUnitId) ?? "unread",
          category: "fresh",
          targets: [
            {
              alignment_id: "rel:al:1",
              entity_id: "sem-c",
              entity_label: "概念：收敛数列",
              relation: "expresses",
              only_differs_by: ["仅在度量空间下给出"],
              seen_in: [],
              mastered_in: [],
            },
          ],
        },
        {
          unit_id: "unit-ex",
          unit_title: "例 4",
          unit_kind: "example",
          parent_unit: "unit-thm",
          study_status: harnessStudyMarks.get("unit-ex") ?? "unread",
          category: "unaligned",
          targets: [],
        },
      ],
    },
  ],
  };
});
const longParts = Array.from({ length: 30 }, (_, i) => `## 第 ${i + 1} 节\n\n这是第 ${i + 1} 段用于验证长文档滚动锚定不漂移的正文内容，包含公式 $a_{${i + 1}} = ${i + 1}^2$。\n\n`);
const longSource = longParts.join("");
const longBytes = new TextEncoder().encode(longSource);
const longBlocks = (() => {
  const out: Array<{ id: string; kind: string; range: { start: number; end: number } }> = [];
  let cursor = 0;
  longParts.forEach((part, idx) => {
    const len = new TextEncoder().encode(part).length;
    out.push({ id: `lb-${idx + 1}`, kind: "text", range: { start: cursor, end: cursor + len } });
    cursor += len;
  });
  return out;
})();

extraCommands.set("get_artifact_content_info", (args) => {
  const id = String(args.artifactId);
  contentRequests.push(id);
  if (contentFailures.has(id)) throw new Error("Artifact 不存在");
  if (id === "md-1") return { artifact_id: id, media_type: "text/markdown", byte_size: mdBytes.length, contexts: [{ conversion_id: "c1", paths: [{ path: "notes.md", artifact_id: "md-1" }] }] };
  if (id === "md-long") return { artifact_id: id, media_type: "text/markdown", byte_size: longBytes.length, contexts: [{ conversion_id: "c-long", paths: [{ path: "long.md", artifact_id: "md-long" }] }] };
  if (id === "pdf-notes") return { artifact_id: id, media_type: "application/pdf", byte_size: pdfBytes.length, contexts: [] };
  throw new Error("artifact not found: " + id);
});
extraCommands.set("read_artifact_bytes", (args) => {
  const id = String(args.artifactId);
  const offset = Number(args.offset ?? 0);
  const length = Number(args.length ?? 0);
  if (id === "md-1") return mdBytes.slice(offset, offset + length).buffer;
  if (id === "md-long") return longBytes.slice(offset, offset + length).buffer;
  if (id === "pdf-notes") return pdfBytes.slice(offset, offset + length).buffer;
  return new ArrayBuffer(0);
});
extraCommands.set("get_markdown_document", (args) => {
  if (String(args.artifactId) === "md-long") {
    return {
      schema_version: "mathscape.anchored-markdown.v1",
      artifact_id: "md-long",
      conversion_id: "c-long",
      source: longSource,
      blocks: longBlocks,
      snapshot: { artifact_id: "md-long", conversion_id: "c-long", content_hash: "sha256:long", anchor_scheme: "mathscape-markdown-blocks-1", fingerprint: "fp-md-long", byte_size: longBytes.length },
    };
  }
  return {
    schema_version: "mathscape.anchored-markdown.v1",
    artifact_id: "md-1",
    conversion_id: "c1",
    source: mdSource,
    blocks: [{ id: "b1", kind: "heading", range: { start: 0, end: mdBytes.length } }],
    snapshot: { artifact_id: "md-1", conversion_id: "c1", content_hash: "sha256:test", anchor_scheme: "mathscape-markdown-blocks-1", fingerprint: "fp-md-1", byte_size: mdBytes.length },
  };
});

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
// Dialogue threads share the one sandbox copy; only the active one takes messages.
const dialogueThreads = { threads: [{ thread_id: "d1", created_at: "2026-10-31T09:00:00+08:00" }], active: "d1", rounds: 0 };
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
let harnessModel = "gpt-test";
// Placeholder catalog in the real models_cache.json shape (levels + default).
const levels = (...ids: string[]) => ids.map((id) => ({ id, description: `harness ${id}` }));
const harnessModels: { id: string; displayName: string; reasoningEfforts?: { id: string; description: string }[]; defaultReasoningEffort?: string }[] = [
  { id: "gpt-test", displayName: "GPT Test", reasoningEfforts: levels("low", "medium", "high", "xhigh", "max", "ultra"), defaultReasoningEffort: "low" },
  { id: "gpt-alt", displayName: "GPT Alt", reasoningEfforts: levels("low", "medium", "high", "xhigh", "max"), defaultReasoningEffort: "medium" },
];
let harnessEffort: string | null = "high";
const harnessEntry = () => harnessModels.find((model) => model.id === harnessModel);
const agentStatus = () => {
  const entry = harnessEntry();
  const efforts = entry?.reasoningEfforts ?? [];
  return {
    backend: "codex",
    codexProgram: "codex",
    mcpProgram: "mathscape",
    tools: ["1", "2", "3", "4", "5", "6", "7", "8"],
    probes: [{ capability: "codex", state: "available", found_version: "0.124.0", required: null, install_hint: null }],
    available: true,
    model: harnessModel,
    models: harnessModels,
    modelSelection: true,
    effort: harnessEffort,
    efforts,
    effortSelection: efforts.length > 0,
  };
};
extraCommands.set("sandbox_agent_status", () => agentStatus());
extraCommands.set("sandbox_set_model", (args) => {
  const id = String(args.modelId ?? "").trim();
  if (!id) throw new Error("model id required");
  harnessModel = id;
  if (!harnessModels.some((model) => model.id === id)) {
    harnessModels.push({ id, displayName: id });
  }
  // Same rule as the Codex adapter: an effort the new model does not list
  // falls back to that model's default.
  const entry = harnessEntry();
  if (harnessEffort && entry?.reasoningEfforts?.length && !entry.reasoningEfforts.some((e) => e.id === harnessEffort)) {
    harnessEffort = entry.defaultReasoningEffort ?? harnessEffort;
  }
  sandboxCalls.push({ command: "set_model", args });
  return agentStatus();
});
extraCommands.set("sandbox_set_effort", (args) => {
  const effort = String(args.effort ?? "").trim();
  const entry = harnessEntry();
  if (!effort) throw new Error("effort required");
  if (entry?.reasoningEfforts?.length && !entry.reasoningEfforts.some((e) => e.id === effort)) {
    throw new Error(`model ${harnessModel} does not support reasoning effort ${effort}`);
  }
  harnessEffort = effort;
  sandboxCalls.push({ command: "set_effort", args });
  return agentStatus();
});
/** `cite`: the round reads the canonical unit with graph.get and answers
 * with its id plus LaTeX (A-PR3 chips / echo / KaTeX). */
/** `act` (Map Scene Player second slice): the round instead runs one
 * Act tool — search typewriter, pdf.outline 目录 window, ink/pdf.page_image 页图 window. */
type SandboxAct = "" | "search" | "outline" | "outline_empty" | "page_image" | "page_image_meta" | "pdf_page_image" | "pdf_page_image_meta" | "parsing";
/** `script` (fourth slice): the round replays real tool steps (from
 * tests/fixtures/scene-tool-events.json) with their own call / result times. */
interface ScriptStep { id: string; tool: string; args: unknown; result?: unknown; status?: string; callAt: number; resultAt?: number }
const sandboxRound: { delayMs: number; gapMs: number; cite: boolean; act: SandboxAct; parsingNow: boolean; script: ScriptStep[] | null } = { delayMs: 30, gapMs: 0, cite: false, act: "", parsingNow: false, script: null };
/** 1×1 PNG for the page-image window. */
const tinyPng = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
function actStep(act: Exclude<SandboxAct, "">): { tool: string; args: Record<string, unknown>; result: Record<string, unknown> } {
  switch (act) {
    case "search": {
      const structured = { status: "ok", query: "紧集 开覆盖", total: 2, groups: [
        { kind: "source_unit", hits: [{ id: "h1", target: { kind: "source_object", object_id: citeUnitId } }] },
        { kind: "artifact", hits: [{ id: "h2", target: { kind: "artifact", artifact_id: "pdf-notes" } }] },
      ] };
      return { tool: "workspace.search", args: { query: "紧集 开覆盖" }, result: { content: [{ type: "text", text: JSON.stringify(structured) }], structuredContent: structured, isError: false } };
    }
    case "outline":
    case "outline_empty": {
      const entries = act === "outline" ? [
        { title: "第一章 拓扑空间", level: 1, pdf_page_index: 0 },
        { title: "1.1 紧集", level: 2, pdf_page_index: 1 },
      ] : [];
      const structured = { status: "ok", artifact_id: "pdf-notes", entries, total_entries: entries.length, total_pages: 2 };
      return { tool: "pdf.outline", args: { artifact_id: "pdf-notes" }, result: { content: [{ type: "text", text: JSON.stringify(structured) }], structuredContent: structured, isError: false } };
    }
    case "parsing": {
      // Scanned pages: material.read blocks while Marker parses (sync ensure).
      const structured = {
        status: "ok", artifact_id: "pdf-notes", content_type: "markdown", total_pages: 12,
        details: {
          ensure: { status: "expanded", requested: { from: 3, to: 4 }, marker_range: { from: 3, to: 4 } },
          map_signal: { kind: "parsing", label: "解析中", node_id: "pdf-notes", pages: { from: 3, to: 4 }, phase: "done", waited_ms: 0 },
        },
      };
      return { tool: "material.read", args: { artifact_id: "pdf-notes", locator: { pdf_pages: { from: 3, to: 4 } } }, result: { content: [{ type: "text", text: JSON.stringify(structured) }], structuredContent: structured, isError: false } };
    }
    case "page_image":
    case "page_image_meta": {
      const meta = { status: "ok", truncated: false, node_id: "ink-doc-1", source: { kind: "note", node_id: "ink-doc-1", document_id: "ink-doc-1", note_title: "手稿 A" }, page: { id: "p0", index: 0, width: 800, height: 1000 }, page_count: 3, image: { format: "png", width: 1, height: 1, scale: 1 } };
      const content: unknown[] = [{ type: "text", text: JSON.stringify(meta) }];
      if (act === "page_image") content.push({ type: "image", data: tinyPng, mimeType: "image/png" });
      return { tool: "ink.page_image", args: { node_id: "ink-doc-1", page_index: 0 }, result: { content, isError: false } };
    }
    case "pdf_page_image":
    case "pdf_page_image_meta": {
      const meta = { status: "ok", truncated: false, artifact_id: "pdf-notes", page: 2, page_count: 5, page_size: { width: 612, height: 792, unit: "pt", rotation: 0 }, image: { format: "png", width: 1, height: 1, scale: 1, bytes: 68, sha256: "x" }, renderer: "hayro-0.8" };
      const content: unknown[] = [{ type: "text", text: JSON.stringify(meta) }];
      if (act === "pdf_page_image") content.push({ type: "image", data: tinyPng, mimeType: "image/png" });
      return { tool: "pdf.page_image", args: { artifact_id: "pdf-notes", page: 2 }, result: { content, isError: false } };
    }
  }
}
const sessionDialogue = (args: Record<string, unknown>) => ({ sandboxSessionId: String(args.sandboxSessionId), transcript: structuredClone(transcript), events: structuredClone(sandboxEvents), activeRoundId: null, threads: structuredClone(dialogueThreads.threads), activeThreadId: dialogueThreads.active });
extraCommands.set("sandbox_session_dialogue", sessionDialogue);
extraCommands.set("sandbox_new_dialogue_thread", (args) => {
  sandboxCalls.push({ command: "new_thread", args });
  const used = transcript.some((entry) => entry.thread_id === dialogueThreads.active && entry.role === "user");
  if (used) {
    const id = `d${dialogueThreads.threads.length + 1}`;
    dialogueThreads.threads.push({ thread_id: id, created_at: new Date().toISOString() });
    dialogueThreads.active = id;
  }
  return sessionDialogue(args);
});
extraCommands.set("sandbox_send_message", (args) => {
  sandboxCalls.push({ command: "send", args });
  const id = String(args.sandboxSessionId);
  const rid = `r${++dialogueThreads.rounds}`;
  const thread = dialogueThreads.active;
  const userEntry = { ts: new Date().toISOString(), role: "user", text: String(args.text), round_id: rid, thread_id: thread };
  transcript.push(userEntry);
  // Stream the round after the command resolves, as the Rust pump does.
  // The tool call lands first so the dialogue can show an in-progress card
  // before the answer.
  const event = (seq: number, type: string, payload: Record<string, unknown>) => {
    const value = { round_id: rid, turn_id: "t1", seq, ts: "2026-10-31T09:01:00+08:00", backend: "codex", type, payload };
    sandboxEvents.push(value);
    emitDialogue({ kind: "agent_event", sandbox_session_id: id, event: value });
  };
  const gap = sandboxRound.gapMs > 0 ? sandboxRound.gapMs : Math.min(400, Math.max(80, sandboxRound.delayMs));
  if (sandboxRound.script) {
    const script = sandboxRound.script;
    let seq = 0;
    event(seq++, "turn_start", { input_ref: "x" });
    const started = new Map<string, number>();
    const timeline: Array<{ at: number; run: () => void }> = [];
    for (const step of script) {
      timeline.push({ at: step.callAt, run: () => {
        started.set(step.id, Date.now());
        const raw = { type: "mcpToolCall", id: step.id, server: "mathscape", tool: step.tool, arguments: step.args, startedAtMs: Date.now() };
        event(seq++, "tool_call", { call_id: step.id, tool: step.tool, server: "mathscape", args_summary: "", raw });
      } });
      if (step.resultAt !== undefined) timeline.push({ at: step.resultAt, run: () => {
        const duration = Date.now() - (started.get(step.id) ?? Date.now());
        const raw = { type: "mcpToolCall", id: step.id, server: "mathscape", tool: step.tool, arguments: step.args, status: "completed", result: step.result, durationMs: duration };
        event(seq++, "tool_result", { call_id: step.id, status: step.status ?? "ok", duration_ms: duration, raw });
      } });
    }
    const end = Math.max(0, ...timeline.map((item) => item.at)) + 120;
    for (const item of timeline) setTimeout(item.run, item.at);
    setTimeout(() => {
      event(seq++, "turn_end", { outcome: "completed" });
      const reply = { ts: new Date().toISOString(), role: "agent", text: "完成。", round_id: rid, outcome: "completed", thread_id: thread };
      transcript.push(reply);
      emitDialogue({ kind: "transcript", sandbox_session_id: id, entry: reply });
      emitDialogue({ kind: "round_finished", sandbox_session_id: id, round_id: rid });
    }, end);
    return { roundId: rid, userEntry };
  }
  setTimeout(() => {
    const tools = ["workspace.search", "material.read", "material.grep", "pdf.outline", "graph.get", "graph.neighborhood", "ink.page_image", "pdf.page_image"];
    const mcp = { mcp: { server: "mathscape", status: "ready", tools, expected: tools, missing: [], error: null } };
    event(0, "turn_start", { input_ref: "x" });
    event(1, "tool_call", { call_id: "mcp-startup", tool: "mcp.startup", server: "codex", args_summary: "mathscape", raw: mcp });
    event(2, "tool_result", { call_id: "mcp-startup", status: "ok", duration_ms: 40, raw: mcp });
    if (sandboxRound.act) {
      const { tool, args } = actStep(sandboxRound.act);
      const raw = { type: "mcpToolCall", id: "c1", server: "mathscape", tool, arguments: args };
      if (sandboxRound.act === "parsing") sandboxRound.parsingNow = true;
      event(3, "tool_call", { call_id: "c1", tool, server: "mathscape", args_summary: "", raw });
    } else if (sandboxRound.cite) {
      const raw = { type: "mcpToolCall", id: "c1", server: "mathscape", tool: "graph.get", arguments: { node_id: citeUnitId } };
      event(3, "tool_call", { call_id: "c1", tool: "graph.get", server: "mathscape", args_summary: `node_id=${citeUnitId}`, raw });
    } else {
      event(3, "tool_call", { call_id: "c1", tool: "workspace.search", server: "mathscape", args_summary: "query=紧集" });
    }
  }, sandboxRound.delayMs);
  setTimeout(() => {
    if (sandboxRound.act) {
      const { tool, args, result } = actStep(sandboxRound.act);
      const raw = { type: "mcpToolCall", id: "c1", server: "mathscape", tool, arguments: args, status: "completed", result };
      sandboxRound.parsingNow = false;
      event(4, "tool_result", { call_id: "c1", status: "ok", duration_ms: 12, raw });
    } else if (sandboxRound.cite) {
      const result = { structuredContent: { status: "ok", node: { id: citeUnitId, kind: "source_unit", label: "引理 1.4", content_ref: null } }, isError: false };
      const raw = { type: "mcpToolCall", id: "c1", server: "mathscape", tool: "graph.get", arguments: { node_id: citeUnitId }, status: "completed", result };
      event(4, "tool_result", { call_id: "c1", status: "ok", duration_ms: 12, raw });
    } else {
      event(4, "tool_result", { call_id: "c1", status: "ok", duration_ms: 12 });
    }
    event(5, "usage", { input_tokens: 120, output_tokens: 30 });
    event(6, "turn_end", { outcome: "completed" });
    const text = sandboxRound.cite
      ? `依据 ${citeUnitId}，有 $\\sum_{n=1}^{\\infty} a_n < \\infty$。未知对象 ${"f".repeat(64)}。`
      : "找到 1 条：定理 3.2。";
    const reply = { ts: new Date().toISOString(), role: "agent", text, round_id: rid, outcome: "completed", thread_id: thread };
    transcript.push(reply);
    emitDialogue({ kind: "transcript", sandbox_session_id: id, entry: reply });
    emitDialogue({ kind: "round_finished", sandbox_session_id: id, round_id: rid });
  }, sandboxRound.delayMs + gap);
  return { roundId: rid, userEntry };
});
extraCommands.set("sandbox_cancel_round", () => false);


// Phase2 proposal review: a queue of one staged proposal over a snapshot
// that only the sandbox copy has (served by get_change_set_snapshot).
const proposalSource = "# 引理 C\n\n设 $x^2 \\ge 0$ 对一切实数成立。\n";
const proposalBytes = new TextEncoder().encode(proposalSource);
const proposalQuote = "设 $x^2 \\ge 0$ 对一切实数成立。";
const proposalStart = new TextEncoder().encode(proposalSource.slice(0, proposalSource.indexOf(proposalQuote))).length;
const proposalEnd = proposalStart + new TextEncoder().encode(proposalQuote).length;
const proposalReview = { failSnapshot: "", calls: [] as Array<{ command: string; args: Record<string, unknown> }>, accepted: false };
const proposalItem = () => ({
  changeSetId: "mathscape:change-set/demo", version: 3, state: "staged", investigationId: "sandbox:sbx_demo", roundId: "r1",
  proposedBy: "agent:codex", createdAt: "2026-11-10T09:00:00+08:00", proposalFingerprint: "fp-demo", baseCommit: null,
  proposal: {
    rationale: "登记引理 C",
    proposed_objects: [
      { local_ref: "s", kind: "source_span", op: "create", payload: { artifact_id: "md-sbx", md_byte_range: { start: proposalStart, end: proposalEnd }, granularity: "inline", quote: proposalQuote } },
      { local_ref: "u", kind: "source_unit", op: "create", payload: { kind: "lemma", display_title: "引理 C", parent: null, source_spans: ["$new:s"], completeness: "complete" } },
    ],
  },
  result: {
    debts: [{ kind: "unresolved", value: { id: "chapter-pages-1-7", kind: "locator", summary: "PDF 第 2–7 页所在快照缺少分页标记" } }],
    diagnostics: [{ level: "warn", code: "head_integrity", message: "工作区已有对象存在问题；不阻塞", local_ref: null, target_id: "mathscape:source-unit/old" }],
  },
  dependencies: [],
  anchors: [{
    span_ref: "s", span_id: "mathscape:source-span/demo", origin: "new", artifact: "md-sbx", conversion: null,
    range: { start: proposalStart, end: proposalEnd }, block_ids: ["pb2"], page: { pdf_page_index: 1, page_number: 2 },
    granularity: "inline", excerpt: proposalQuote, verified: true, compiled: true,
  }],
  history: [
    { version: 2, state: "superseded", createdAt: "2026-11-10T08:30:00+08:00", diagnostics: [{ code: "invalid_locator", message: "x" }] },
    { version: 1, state: "superseded", createdAt: "2026-11-10T08:00:00+08:00", diagnostics: [] },
  ],
  targetTitles: { "mathscape:source-unit/old": "旧定理" },
});
// Multi-snapshot proposal (sessionStorage harness.multiSnapshot = "1"): units in
// three snapshots, two cross-snapshot relations and one from an existing object.
const multiSnapshotItem = () => {
  const spanOf = (ref: string, artifact: string) => ({ local_ref: ref, kind: "source_span", op: "create", payload: { artifact_id: artifact, md_byte_range: { start: proposalStart, end: proposalEnd }, granularity: "inline", quote: proposalQuote } });
  const unitOf = (ref: string, title: string, span: string) => ({ local_ref: ref, kind: "source_unit", op: "create", payload: { kind: "lemma", display_title: title, parent: null, source_spans: [`$new:${span}`], completeness: "complete" } });
  const relOf = (ref: string, relation: string, source: string, target: string) => ({ local_ref: ref, kind: "source_relation", op: "create", relation, source, target, evidence_spans: [] });
  const anchorOf = (ref: string, artifact: string) => ({
    span_ref: ref, span_id: `mathscape:source-span/${ref}`, origin: "new", artifact, conversion: null,
    range: { start: proposalStart, end: proposalEnd }, block_ids: ["pb2"], page: null,
    granularity: "inline", excerpt: proposalQuote, verified: true, compiled: true,
  });
  return {
    ...proposalItem(),
    changeSetId: "mathscape:change-set/multi", version: 2, proposalFingerprint: "fp-multi", history: [],
    proposal: {
      rationale: "习题 8 前置知识",
      proposed_objects: [
        spanOf("sa", "md-a"), spanOf("sb", "md-a"), spanOf("sc", "md-b"), spanOf("sd", "md-c"),
        unitOf("a", "定义 1", "sa"), unitOf("b", "定理 2", "sb"), unitOf("c", "例 3", "sc"), unitOf("d", "推论 4", "sd"),
      ],
      proposed_relations: [
        relOf("r_ab", "explains", "$new:a", "$new:b"),
        relOf("r_bc", "applies_method", "$new:b", "$new:c"),
        relOf("r_cd", "applies_method", "$new:c", "$new:d"),
        relOf("r_old", "explains", "mathscape:source-unit/old", "$new:a"),
      ],
    },
    result: { debts: [{ kind: "unresolved", value: { kind: "locator", summary: "页码待核" } }], diagnostics: [] },
    anchors: [anchorOf("sa", "md-a"), anchorOf("sb", "md-a"), anchorOf("sc", "md-b"), anchorOf("sd", "md-c")],
  };
};
extraCommands.set("get_change_set_investigation", (args) => {
  proposalReview.calls.push({ command: "investigation", args });
  return args.sandboxSessionId ? `sandbox:${String(args.sandboxSessionId)}` : (sessionStorage.getItem("harness.sandbox") === "1" ? "sandbox:sbx_demo" : null);
});
extraCommands.set("list_pending_change_sets", (args) => {
  const failure = sessionStorage.getItem("harness.failQueue");
  if (failure) throw new Error(failure);
  return listPendingChangeSets(args);
});
const listPendingChangeSets = (args: Record<string, unknown>) => ({
  // Only specs that opt in (sessionStorage harness.proposals) see a pending proposal.
  investigationId: String(args.investigationId), baseCommit: null,
  items: proposalReview.accepted || sessionStorage.getItem("harness.proposals") !== "1" ? []
    : [sessionStorage.getItem("harness.multiSnapshot") === "1" ? multiSnapshotItem() : proposalItem()],
});
extraCommands.set("get_change_set_snapshot", (args) => {
  proposalReview.calls.push({ command: "snapshot", args });
  const failure = proposalReview.failSnapshot || sessionStorage.getItem("harness.failSnapshot");
  if (failure) throw new Error(failure);
  const heading = new TextEncoder().encode("# 引理 C").length;
  return {
    conversionId: null,
    document: {
      source: proposalSource,
      blocks: [
        { id: "pb1", kind: "heading", range: { start: 0, end: heading } },
        { id: "pb2", kind: "paragraph", range: { start: proposalStart, end: proposalBytes.length - 1 } },
      ],
      snapshot: { content_hash: "sha256:sbx", byte_size: proposalBytes.length },
    },
  };
});
extraCommands.set("preview_change_set_impact", () => ({
  changeSetId: "mathscape:change-set/demo", version: 3, plannedWith: [],
  impact: { plan_fingerprint: "p", status: "ready", base_commit: null, direct_changes: [{ object_id: "a", kind: "SourceSpan", action: "create" }, { object_id: "b", kind: "SourceSemanticUnit", action: "create" }], affected_objects: [], diagnostics: [] },
}));
extraCommands.set("plan_change_set_batch", (args) => {
  proposalReview.calls.push({ command: "plan", args });
  return { investigationId: String(args.investigationId), baseCommit: null, selections: args.selections, impact: { plan_fingerprint: "p", status: "ready", base_commit: null, direct_changes: [], affected_objects: [], diagnostics: [] }, perChangeSet: [], batchPlanFingerprint: "bp" };
});
extraCommands.set("accept_change_set_batch", (args) => {
  proposalReview.calls.push({ command: "accept", args });
  proposalReview.accepted = true;
  return { commitId: "c", accepted: args.selections, receipt: {} };
});

await import("../src/main");

const core = (): Core => (document.querySelector("#graph") as unknown as { _cyreg: { cy: Core } } | null)?._cyreg?.cy
  ?? ([...document.querySelectorAll<HTMLElement>("div")].find(div => (div as unknown as { _cyreg?: unknown })._cyreg) as unknown as { _cyreg: { cy: Core } })._cyreg.cy;
const appHarness = {
  documents: { a: a.document_id, b: b.document_id }, notes, links, behavior, storedDocuments: documents, noteCalls, unexpected,
  windowEvents, requestWindowClose,
  sandboxCalls,
  sandboxRound,
  proposalReview,
  contentFailures, contentRequests, detailRequests, sourceDetailBehavior, searchBehavior, snapshots, snapshotLabelCalls,
  /** Cytoscape state of a node: selected flag and distance from the canvas centre. */
  nodeView: (id: string) => {
    const cy = core(), node = cy.getElementById(id);
    if (node.empty()) return null;
    // cy.center() centres the bounding box (node + label), so compare that.
    const box = node.renderedBoundingBox({});
    return { selected: node.selected(), dx: (box.x1 + box.x2) / 2 - cy.width() / 2, dy: (box.y1 + box.y2) / 2 - cy.height() / 2, width: cy.width(), height: cy.height() };
  },
  graphPaint: () => ({
    nodes: core().nodes().map(node => ({ id: node.id(), type: node.data("nodeType"), staged: node.hasClass("is-staged"), label: node.data("label"), shape: node.style("shape"), border: node.style("border-style"), borderColor: node.style("border-color"), color: node.style("color"), labelBackground: node.style("text-background-opacity"), position: node.position() })),
    edges: core().edges().map(edge => ({ id: edge.id(), color: edge.style("line-color") })),
  }),
  labels: () => Object.fromEntries(core().nodes().map(node => [node.id(), String(node.data("label"))])),
  inkNodes: () => core().nodes(".ink-note-node").map(node => `${node.id()}=${String(node.data("label"))}`),
  selectNode: (id: string) => { core().getElementById(id).emit("tap"); },
  nodeClasses: (id: string) => { const node = core().getElementById(id); return node.empty() ? null : node.classes(); },
  citeUnitId,
  citeArtId,
  panTo: (x: number, y: number) => { core().pan({ x, y }); },
  /** Model position (rounded), to check the scene clump moves and restores nodes. */
  nodePosition: (id: string) => { const node = core().getElementById(id); if (node.empty()) return null; const p = node.position(); return { x: Math.round(p.x), y: Math.round(p.y) }; },
  /** Place a node (sets up a sparse focus before a scene). */
  setNodePosition: (id: string, x: number, y: number) => { core().getElementById(id).position({ x, y }); },
  /** Proposal scene ghosts (transient Cytoscape elements). */
  ghosts: () => core().nodes(".scene-ghost").map(node => ({ id: node.id(), label: String(node.data("label")), classes: node.classes(), x: node.position("x"), y: node.position("y") })),
  ghostEdges: () => core().edges(".scene-ghost-edge").map(edge => ({ id: edge.id(), label: String(edge.data("label")), source: edge.source().id(), target: edge.target().id(), classes: edge.classes() })),
  withClass: (cls: string) => core().elements(`.${cls}`).map(element => element.id()),
  /** Rounded zoom/pan, to check the scene player restores the camera. */
  viewport: () => { const cy = core(), pan = cy.pan(); return { zoom: Math.round(cy.zoom() * 1000) / 1000, x: Math.round(pan.x), y: Math.round(pan.y) }; },
};
declare global { interface Window { appHarness: typeof appHarness } }
window.appHarness = appHarness;
