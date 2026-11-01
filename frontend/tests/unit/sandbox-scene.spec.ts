import { describe, expect, it } from "vitest";
import {
  callArgs,
  compileRoundScenes,
  compileToolScene,
  parseArgsSummary,
  resolveScene,
  resultPayload,
  sceneHasFocus,
  sceneToolName,
} from "../../src/sandbox/scene/compile";
import { SCENE_FOCUS_CAP, SCENE_TOOLS, type SceneResolve } from "../../src/sandbox/scene/types";
import type { AgentEventLike } from "../../src/sandbox/format";

const UNIT = `mathscape:source-unit/${"a".repeat(64)}`;
const ART = `mathscape:artifact/${"b".repeat(64)}`;
const ART2 = `mathscape:artifact/${"c".repeat(64)}`;

/** A tool_call + tool_result pair in the real adapter shape. */
function step(tool: string, args: Record<string, unknown>, result?: unknown, opts: { id?: string; text?: boolean; status?: string } = {}) {
  const id = opts.id ?? "c1";
  const call: AgentEventLike = {
    type: "tool_call",
    round_id: "r1",
    payload: { call_id: id, tool, server: "mathscape", args_summary: "", raw: { tool, server: "mathscape", arguments: args, id } },
  };
  const content = opts.text
    ? { content: [{ type: "text", text: JSON.stringify(result) }, { type: "image", data: "…", mimeType: "image/png" }] }
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
