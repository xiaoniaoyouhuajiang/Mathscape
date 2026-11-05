import { describe, expect, it } from "vitest";
import {
  callArgs,
  compileRoundScenes,
  compileSceneAct,
  compileToolScene,
  isParsingSummary,
  parseArgsSummary,
  parsingScene,
  readPdfPages,
  resolveScene,
  resultImage,
  resultPayload,
  sceneHasFocus,
  sceneToolName,
} from "../../src/sandbox/scene/compile";
import { OUTLINE_ROWS_CAP, SCENE_FOCUS_CAP, SCENE_TOOLS, type SceneResolve } from "../../src/sandbox/scene/types";
import type { AgentEventLike } from "../../src/sandbox/format";

const UNIT = `mathscape:source-unit/${"a".repeat(64)}`;
const ART = `mathscape:artifact/${"b".repeat(64)}`;
const ART2 = `mathscape:artifact/${"c".repeat(64)}`;

/** A tool_call + tool_result pair in the real adapter shape. */
function step(tool: string, args: Record<string, unknown>, result?: unknown, opts: { id?: string; text?: boolean; status?: string; image?: Record<string, unknown> | null } = {}) {
  const id = opts.id ?? "c1";
  const call: AgentEventLike = {
    type: "tool_call",
    round_id: "r1",
    payload: { call_id: id, tool, server: "mathscape", args_summary: "", raw: { tool, server: "mathscape", arguments: args, id } },
  };
  const content = opts.text
    ? { content: [{ type: "text", text: JSON.stringify(result) }, ...(opts.image === null ? [] : [{ type: "image", data: "…", mimeType: "image/png", ...opts.image }])] }
    : { content: [{ type: "text", text: JSON.stringify(result) }], structuredContent: result };
  const done: AgentEventLike | undefined = result === undefined ? undefined : {
    type: "tool_result",
    round_id: "r1",
    payload: { call_id: id, status: opts.status ?? "ok", duration_ms: 5, raw: { tool, arguments: args, result: content, id, status: "completed" } },
  };
  return { call, done };
}

/** Every id resolves to `n:<id>` labelled `L:<id>`, except ids in `off`. */
const resolver = (off: string[] = []): SceneResolve => (id) => (off.includes(id) ? null : { nodeId: `n:${id}`, label: `L:${id.slice(-4)}` });

describe("sceneToolName", () => {
  it("normalizes bare, server-prefixed and mcp__ names; skips startup and others", () => {
    for (const tool of SCENE_TOOLS) expect(sceneToolName(tool)).toBe(tool);
    expect(sceneToolName("mathscape.graph.get")).toBe("graph.get");
    expect(sceneToolName("mcp__mathscape__graph_neighborhood")).toBe("graph.neighborhood");
    expect(sceneToolName("mcp.startup")).toBeNull();
    expect(sceneToolName("shell")).toBeNull();
  });
});

describe("args / payload extraction", () => {
  it("parses adapter args_summary when raw arguments are missing", () => {
    expect(parseArgsSummary("depth=2 direction=out node_id=abc")).toEqual({ depth: 2, direction: "out", node_id: "abc" });
    expect(callArgs({ type: "tool_call", payload: { call_id: "x", args_summary: "query=紧集 limit_per_group=5" } })).toEqual({ query: "紧集", limit_per_group: 5 });
  });

  it("reads structuredContent, else the JSON text block (ink.page_image)", () => {
    const { done } = step("ink.page_image", {}, { status: "ok", node_id: "n" }, { text: true });
    expect(resultPayload(done)).toEqual({ status: "ok", node_id: "n" });
    const { done: other } = step("graph.get", {}, { status: "ok", node: null });
    expect(resultPayload(other)).toEqual({ status: "ok", node: null });
  });
});

describe("per-tool compile", () => {
  it("workspace.search: query title, no seeds, hits by target", () => {
    const { call, done } = step("workspace.search", { query: "紧集" }, {
      query: "紧集", total: 3, status: "ok",
      groups: [
        { kind: "source_unit", hits: [{ id: "h1", target: { kind: "source_object", object_id: UNIT } }, { id: "h2", target: { kind: "map_node", node_id: "node-7" } }] },
        { kind: "artifact", hits: [{ id: "h3", target: { kind: "artifact", artifact_id: ART } }] },
      ],
    });
    const running = compileToolScene(call)!;
    expect(running.status).toBe("running");
    expect(running.seeds).toEqual([]);
    const draft = compileToolScene(call, done)!;
    expect(draft.layers).toEqual([[UNIT, "node-7", ART]]);
    const scene = resolveScene(draft, resolver());
    expect(scene.title).toBe("搜索 · 紧集");
    expect(scene.summary).toBe("命中 3 条 · 地图上 3 个");
  });

  it("material.read: focus is the artifact, locator in the title", () => {
    const { call, done } = step("material.read", { artifact_id: ART, locator: { pdf_pages: { from: 3, to: 4 } } }, {
      status: "ok", artifact_id: ART, content_type: "pdf", total_pages: 12, truncated: true,
    });
    const scene = resolveScene(compileToolScene(call, done)!, resolver());
    expect(scene.seeds).toEqual([`n:${ART}`]);
    expect(scene.layers).toEqual([]);
    expect(scene.title).toBe("阅读材料 · L:bbbb · 第 3–4 页");
    expect(scene.summary).toContain("共 12 页 · 已截断");
  });

  it("material.grep: seeds are the searched materials, hits only the matched ones", () => {
    const { call, done } = step("material.grep", { artifact_ids: [ART, ART2], pattern: "compact" }, {
      status: "ok", truncated: false,
      hits: [{ artifact_id: ART2, snippet: "x" }, { artifact_id: ART2, snippet: "y" }],
    });
    const scene = resolveScene(compileToolScene(call, done)!, resolver());
    expect(scene.title).toBe("材料检索 · compact");
    expect(scene.seeds).toEqual([`n:${ART}`, `n:${ART2}`]);
    expect(scene.layers).toEqual([[`n:${ART2}`]]);
    expect(scene.summary).toBe("1 份材料命中 2 处 · 地图上 2 个");
  });

  it("pdf.outline: focus the artifact, entries in the summary", () => {
    const { call, done } = step("pdf.outline", { artifact_id: ART }, { status: "ok", artifact_id: ART, entries: [], total_entries: 14, total_pages: 80 });
    const scene = resolveScene(compileToolScene(call, done)!, resolver());
    expect(scene.title).toBe("PDF 目录 · L:bbbb");
    expect(scene.summary).toBe("14 条目录 · 80 页 · 地图上 1 个");
  });

  it("graph.get: the asked node is the seed and turns into the hit", () => {
    const { call, done } = step("graph.get", { node_id: UNIT }, {
      status: "ok", node: { id: UNIT, canonical_id: UNIT, kind: "source_unit", entity_kind: "定理", label: "定理 3.2" },
    });
    const scene = resolveScene(compileToolScene(call, done)!, resolver());
    expect(scene.title).toBe("查看节点 · 定理 3.2");
    expect(scene.seeds).toEqual([`n:${UNIT}`]);
    expect(scene.layers).toEqual([[`n:${UNIT}`]]);
    // While running the title falls back to the map label.
    expect(resolveScene(compileToolScene(call)!, resolver()).title).toBe("查看节点 · L:aaaa");
  });

  it("graph.neighborhood: seeds + one layer per depth (逐层亮)", () => {
    const { call, done } = step("graph.neighborhood", { node_id: "s", depth: 3, direction: "out" }, {
      status: "ok", seed_ids: ["s"], applied: { depth: 3, direction: "out" },
      nodes: [
        { id: "s", depth: 0 }, { id: "a", depth: 1 }, { id: "b", depth: 1 },
        { id: "c", depth: 2 }, { id: "d", depth: 3 },
      ],
      edges: [{ id: "e1" }, { id: "e2" }],
    });
    const draft = compileToolScene(call, done)!;
    expect(draft.seeds).toEqual(["s"]);
    expect(draft.layers).toEqual([["a", "b"], ["c"], ["d"]]);
    const scene = resolveScene(draft, resolver(["c"]));
    expect(scene.title).toBe("邻域 · L:s · 深度 3 · 出边");
    // An off-map layer drops out; the remaining layers keep their order.
    expect(scene.layers).toEqual([["n:a", "n:b"], ["n:d"]]);
    expect(scene.offMap).toBe(1);
    expect(scene.summary).toBe("5 个节点 · 2 条边 · 3 层 · 地图上 4 个");
  });

  it("graph.neighborhood: caps the focus, keeping seeds and shallow layers", () => {
    const nodes = Array.from({ length: 90 }, (_, i) => ({ id: `x${i}`, depth: i < 50 ? 1 : 2 }));
    const { call, done } = step("graph.neighborhood", { seed_ids: ["s1", "s2"] }, { status: "ok", seed_ids: ["s1", "s2"], nodes, edges: [] });
    const scene = resolveScene(compileToolScene(call, done)!, resolver());
    expect(scene.seeds).toEqual(["n:s1", "n:s2"]);
    expect(scene.layers[0]).toHaveLength(50);
    expect(scene.layers[1]).toHaveLength(SCENE_FOCUS_CAP - 52);
    expect(scene.capped).toBe(true);
    expect(scene.title).toBe("邻域 · 2 个种子 · 深度 1 · 双向");
    expect(scene.summary).toContain(`地图上 ${SCENE_FOCUS_CAP} 个（上限 ${SCENE_FOCUS_CAP}）`);
  });

  it("ink.page_image: note title + page, focus the snapshot/artifact when on the map", () => {
    const { call, done } = step("ink.page_image", { node_id: "ink-node", page_index: 1 }, {
      status: "ok", node_id: "ink-node",
      source: { kind: "ink_snapshot", node_id: "ink-node", snapshot_id: "snap", artifact_id: ART, note_title: "笔记 A" },
      page: { id: "p2", index: 1 }, page_count: 3,
    }, { text: true });
    const scene = resolveScene(compileToolScene(call, done)!, resolver(["snap", "ink-node"]));
    expect(scene.title).toBe("手稿页图 · 笔记 A · 第 2/3 页");
    expect(scene.seeds).toEqual([`n:${ART}`]);
  });

  it("an off-map result is announced but has no focus", () => {
    const { call, done } = step("graph.get", { node_id: UNIT }, { status: "ok", node: { id: UNIT, label: "x" } });
    const scene = resolveScene(compileToolScene(call, done)!, resolver([UNIT]));
    expect(sceneHasFocus(scene)).toBe(false);
    expect(scene.summary).toBe("x · 不在当前地图上");
  });

  it("errors carry the message", () => {
    const { call } = step("material.read", { artifact_id: ART });
    const failed: AgentEventLike = { type: "tool_result", round_id: "r1", payload: { call_id: "c1", status: "error", raw: { error: { message: "artifact not found" } } } };
    const draft = compileToolScene(call, failed)!;
    expect(draft.status).toBe("error");
    expect(draft.summary).toBe("调用失败：artifact not found");
  });
});

describe("compileRoundScenes", () => {
  it("one scene per tool step in call order; startup and unknown tools skipped", () => {
    const a = step("graph.get", { node_id: UNIT }, { status: "ok", node: { id: UNIT, label: "定理" } }, { id: "a" });
    const b = step("workspace.search", { query: "q" }, undefined, { id: "b" });
    const startup: AgentEventLike = { type: "tool_call", round_id: "r1", payload: { call_id: "mcp-startup", tool: "mcp.startup" } };
    const shell: AgentEventLike = { type: "tool_call", round_id: "r1", payload: { call_id: "sh", tool: "shell" } };
    const drafts = compileRoundScenes([startup, a.call, shell, b.call, a.done!]);
    expect(drafts.map((draft) => [draft.callId, draft.tool, draft.status])).toEqual([
      ["a", "graph.get", "ok"],
      ["b", "workspace.search", "running"],
    ]);
  });
});

const PNG = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
const INK_META = {
  status: "ok", node_id: "ink-node",
  source: { kind: "ink_snapshot", node_id: "ink-node", document_id: "doc-1", note_title: "笔记 A" },
  page: { id: "p2", index: 1 }, page_count: 3, image: { width: 800, height: 1000 },
};

describe("Act overlays (第二刀)", () => {
  it("workspace.search: typewriter with the query; empty query → no act", () => {
    const { call, done } = step("workspace.search", { query: "  紧集 开覆盖 " }, { status: "ok", query: "紧集 开覆盖", total: 0, groups: [] });
    expect(compileSceneAct(call)).toEqual({ kind: "typewriter", text: "紧集 开覆盖" });
    expect(compileToolScene(call, done)!.act).toEqual({ kind: "typewriter", text: "紧集 开覆盖" });
    const { call: blank } = step("workspace.search", { query: "   " });
    expect(compileSceneAct(blank)).toBeUndefined();
    expect(compileToolScene(blank)!.act).toBeUndefined();
  });

  it("pdf.outline: rows with 1-based page, only once the result is in", () => {
    const { call, done } = step("pdf.outline", { artifact_id: ART }, {
      status: "ok", artifact_id: ART, total_pages: 9,
      entries: [{ title: "第一章", level: 1, pdf_page_index: 0 }, { title: "1.1 紧集", level: 2, pdf_page_index: 4 }, { title: "无页", level: 2 }],
    });
    expect(compileSceneAct(call)).toBeUndefined();
    expect(compileSceneAct(call, done)).toEqual({
      kind: "outline", total: 3, truncated: false, totalPages: 9,
      rows: [{ title: "第一章", level: 1, page: 1 }, { title: "1.1 紧集", level: 2, page: 5 }, { title: "无页", level: 2 }],
    });
  });

  it("pdf.outline: empty and unreadable empty-states; truncated is still ok", () => {
    const none = step("pdf.outline", { artifact_id: ART }, { status: "ok", artifact_id: ART, entries: [] });
    expect(compileSceneAct(none.call, none.done)).toMatchObject({ kind: "outline", rows: [], empty: "none" });
    const bad = step("pdf.outline", { artifact_id: ART }, { status: "ok", artifact_id: ART, entries: [], diagnostics: [{ code: "outline_unreadable" }] });
    expect(compileSceneAct(bad.call, bad.done)).toMatchObject({ empty: "unreadable" });
    const entries = Array.from({ length: OUTLINE_ROWS_CAP + 5 }, (_, i) => ({ title: `t${i}`, level: 1, pdf_page_index: i }));
    const big = step("pdf.outline", { artifact_id: ART }, { status: "truncated", artifact_id: ART, entries, truncated: true, total_entries: 500 });
    const act = compileSceneAct(big.call, big.done);
    expect(act).toMatchObject({ kind: "outline", total: 500, truncated: true });
    expect(act && act.kind === "outline" ? act.rows.length : 0).toBe(OUTLINE_ROWS_CAP);
    expect(compileToolScene(big.call, big.done)!.status).toBe("ok");
    const missing = step("pdf.outline", { artifact_id: ART }, { status: "not_found", artifact_id: ART });
    expect(compileSceneAct(missing.call, missing.done)).toBeUndefined();
  });

  it("ink.page_image: PNG data URL + note title + page; degrades to meta without an image", () => {
    const { call, done } = step("ink.page_image", { node_id: "ink-node", page_index: 1 }, INK_META, { text: true, image: { data: PNG } });
    expect(resultImage(done)).toBe(`data:image/png;base64,${PNG}`);
    expect(compileSceneAct(call, done)).toEqual({
      kind: "page_image", src: `data:image/png;base64,${PNG}`, noteTitle: "笔记 A", pageLabel: "第 2/3 页", width: 800, height: 1000,
    });
    expect(compileToolScene(call, done)!.summary).toContain("已取得页图");
    const bare = step("ink.page_image", { node_id: "ink-node", page_index: 1 }, INK_META, { text: true, image: null });
    expect(resultImage(bare.done)).toBeUndefined();
    expect(compileSceneAct(bare.call, bare.done)).toMatchObject({ kind: "page_image", noteTitle: "笔记 A", pageLabel: "第 2/3 页" });
    expect(compileSceneAct(bare.call, bare.done)).not.toHaveProperty("src");
    expect(compileToolScene(bare.call, bare.done)!.summary).toContain("仅元数据");
  });

  it("pdf.page_image: 1-based page, artifact seed, PNG act; meta-only when no image", () => {
    const PDF_META = {
      status: "ok", artifact_id: ART, page: 2, page_count: 5,
      page_size: { width: 612, height: 792, unit: "pt", rotation: 0 },
      image: { format: "png", width: 800, height: 1000, scale: 1, bytes: 12, sha256: "abc" },
      renderer: "hayro-0.8",
    };
    const { call, done } = step("pdf.page_image", { artifact_id: ART, page: 2 }, PDF_META, { text: true, image: { data: PNG } });
    const draft = compileToolScene(call, done)!;
    expect(draft.verb).toBe("PDF 页图");
    expect(draft.detail).toBe("第 2/5 页");
    expect(draft.seeds).toEqual([ART]);
    expect(draft.summary).toContain("已取得页图");
    expect(compileSceneAct(call, done)).toEqual({
      kind: "page_image", src: `data:image/png;base64,${PNG}`, noteTitle: "PDF 页图", pageLabel: "第 2/5 页", width: 800, height: 1000,
    });
    const scene = resolveScene(draft, resolver());
    expect(scene.title).toBe(`PDF 页图 · L:${ART.slice(-4)} · 第 2/5 页`);
    expect(scene.seeds).toEqual([`n:${ART}`]);
    expect(scene.anchor).toBe(`n:${ART}`);

    const bare = step("pdf.page_image", { artifact_id: ART, page: 2 }, PDF_META, { text: true, image: null });
    expect(compileSceneAct(bare.call, bare.done)).toMatchObject({ kind: "page_image", noteTitle: "PDF 页图", pageLabel: "第 2/5 页" });
    expect(compileSceneAct(bare.call, bare.done)).not.toHaveProperty("src");
    expect(compileToolScene(bare.call, bare.done)!.summary).toContain("仅元数据");

    const cropped = step("pdf.page_image", { artifact_id: ART, page: 2, crop: { left: 0.1, top: 0.2, right: 0.9, bottom: 0.8 } }, {
      ...PDF_META, crop: { left: 0.1, top: 0.2, right: 0.9, bottom: 0.8 },
    }, { text: true, image: { data: PNG } });
    expect(compileToolScene(cropped.call, cropped.done)!.detail).toBe("第 2/5 页 · 裁剪");
  });

  it("resultImage rejects non-image mime types and non-base64 data", () => {
    expect(resultImage(step("ink.page_image", {}, INK_META, { text: true, image: { data: PNG, mimeType: "text/html" } }).done)).toBeUndefined();
    expect(resultImage(step("ink.page_image", {}, INK_META, { text: true, image: { data: "<svg onload=x>" } }).done)).toBeUndefined();
    expect(resultImage(step("ink.page_image", {}, INK_META, { text: true, image: { data: PNG, mimeType: undefined, mime_type: "image/jpeg" } }).done))
      .toBe(`data:image/jpeg;base64,${PNG}`);
  });

  it("document_id is never registered as a map node; act + anchor survive resolveScene", () => {
    const { call, done } = step("ink.page_image", { node_id: "ink-node", page_index: 1 }, INK_META, { text: true, image: { data: PNG } });
    const draft = compileToolScene(call, done)!;
    expect(JSON.stringify([draft.seeds, draft.layers])).not.toContain("doc-1");
    const offMap = resolveScene(draft, resolver(["ink-node"]));
    expect(offMap.anchor).toBeUndefined();
    expect(offMap.act).toEqual(draft.act);
    const outline = step("pdf.outline", { artifact_id: ART }, { status: "ok", artifact_id: ART, entries: [] });
    const scene = resolveScene(compileToolScene(outline.call, outline.done)!, resolver());
    expect(scene.anchor).toBe(`n:${ART}`);
    expect(scene.act).toMatchObject({ kind: "outline", empty: "none" });
  });

  it("other tools carry no act", () => {
    const { call, done } = step("graph.get", { node_id: UNIT }, { status: "ok", node: { id: UNIT, label: "x" } });
    expect(compileSceneAct(call, done)).toBeUndefined();
  });

  it("material.read after a sync ensure: parsing act done + 解析完成 summary", () => {
    const { call, done } = step("material.read", { artifact_id: ART, locator: { pdf_pages: { from: 3, to: 4 } } }, {
      status: "ok", artifact_id: ART, content_type: "markdown", total_pages: 12,
      details: { ensure: { status: "expanded" }, map_signal: { kind: "parsing", label: "解析中", node_id: ART, pages: { from: 3, to: 4 }, phase: "done", waited_ms: 0 } },
    });
    expect(compileSceneAct(call, done)).toMatchObject({ kind: "parsing", label: "解析中", phase: "done", pages: { from: 3, to: 4 } });
    const scene = resolveScene(compileToolScene(call, done)!, resolver());
    expect(scene.summary).toMatch(/^解析完成 第3–4页/);
  });

  it("material.read whose ensure failed: parsing act failed + 解析失败 reason", () => {
    const { call, done } = step("material.read", { artifact_id: ART, locator: { pdf_pages: { from: 3, to: 4 } } }, {
      status: "capability_unavailable", artifact_id: ART,
      details: { ensure: { status: "failed", message: "Marker 超时", timed_out: true }, map_signal: { kind: "parsing", label: "解析中", node_id: ART, pages: { from: 3, to: 4 }, phase: "failed", waited_ms: 0 } },
    });
    expect(compileSceneAct(call, done)).toMatchObject({ kind: "parsing", phase: "failed" });
    const scene = resolveScene(compileToolScene(call, done)!, resolver());
    expect(scene.summary).toContain("解析失败 第3–4页");
    expect(scene.summary).toContain("Marker 超时");
  });

  it("material.read without map_signal (covered page / text PDF) carries no act", () => {
    const { call, done } = step("material.read", { artifact_id: ART, locator: { pdf_pages: { from: 1, to: 1 } } }, { status: "ok", artifact_id: ART, total_pages: 2 });
    expect(compileSceneAct(call, done)).toBeUndefined();
  });

  it("an in-flight read becomes a running parsing block when the map says 解析中", () => {
    const { call } = step("material.read", { artifact_id: ART, locator: { pdf_pages: { from: 5 } } });
    expect(readPdfPages(call)).toEqual({ from: 5, to: 5 });
    expect(readPdfPages(step("material.grep", { artifact_ids: [ART], pattern: "x" }).call)).toBeUndefined();
    expect(isParsingSummary("PDF · 解析中 第5–5页")).toBe(true);
    expect(isParsingSummary("PDF · 12 页")).toBe(false);
    expect(isParsingSummary(undefined)).toBe(false);
    const draft = parsingScene(compileToolScene(call)!, { from: 5, to: 6 });
    expect(draft.summary).toBe("解析中 第5–6页 · 等待 Marker 完成…");
    expect(draft.act).toEqual({ kind: "parsing", label: "解析中", phase: "running", pages: { from: 5, to: 6 } });
  });
});
