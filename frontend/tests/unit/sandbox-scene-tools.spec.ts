/**
 * Fourth slice: scenes of propose_changes / mathlib.search / lean.check /
 * material.locate / list_change_sets / get_change_set, compiled from real
 * PR #48 session events (tests/fixtures/scene-tool-events.json).
 */
import { describe, expect, it } from "vitest";
import fixtures from "../fixtures/scene-tool-events.json";
import { compileRoundScenes, compileToolScene, resolveScene, sceneToolName, stagedCounts } from "../../src/sandbox/scene/compile";
import { SCENE_TOOLS, type LeanAct, type LocateAct, type MathlibAct, type ProposeAct, type SceneResolve } from "../../src/sandbox/scene/types";
import type { AgentEventLike } from "../../src/sandbox/format";

type Pair = { call: AgentEventLike; result: AgentEventLike | null };
const fx = fixtures as unknown as Record<string, Pair>;
const scene = (name: string, prior: AgentEventLike[] = []) => compileToolScene(fx[name].call, fx[name].result ?? undefined, prior)!;
const running = (name: string) => compileToolScene(fx[name].call)!;
const none: SceneResolve = () => null;

describe("tool names", () => {
  it("covers the six new tools in every spelling", () => {
    for (const tool of ["propose_changes", "mathlib.search", "lean.check", "material.locate", "list_change_sets", "get_change_set"]) {
      expect(SCENE_TOOLS).toContain(tool);
      expect(sceneToolName(tool)).toBe(tool);
      expect(sceneToolName(`mathscape.${tool}`)).toBe(tool);
      expect(sceneToolName(`mcp__mathscape__${tool.replace(".", "_")}`)).toBe(tool);
    }
    // get_change_set_snapshot is a Desktop command, not an MCP tool.
    expect(sceneToolName("get_change_set_snapshot")).toBeNull();
  });
});

describe("propose_changes", () => {
  it("counts staged_ids by prefix (= review overview totals)", () => {
    const draft = scene("propose_staged_57");
    const act = draft.act as ProposeAct;
    expect(draft.status).toBe("ok");
    expect(act.phase).toBe("staged");
    expect(act.counts).toEqual({ units: 17, relations: 20, spans: 20 });
    expect(act.stagedIds).toHaveLength(57);
    expect(draft.summary).toBe("已暂存 · 单元 17 · 关系 20 · 片段 20");
    expect(act.items.filter((item) => item.kind === "unit")).toHaveLength(17);
    expect(act.items.every((item) => item.stagedId?.startsWith("mathscape:source-unit/"))).toBe(true);
    expect(act.relations).toHaveLength(20);
    const first = act.relations[0];
    expect(first.relation).toBe("explains");
    expect(first.source).toEqual({ local: "def1_unit" });
    expect(first.stagedId?.startsWith("mathscape:source-relation/")).toBe(true);
    expect(act.relations.map((relation) => relation.relation)).toContain("applies_method");
    // Units sit near the PDF their spans cite (an existing map object).
    expect(act.items[0].anchors[0]).toMatch(/^mathscape:artifact\//);
    expect([...draft.seeds].sort()).toEqual([...new Set(act.items.flatMap((item) => item.anchors))].sort());
  });

  it("matches stagedCounts for every staged fixture", () => {
    for (const name of ["propose_staged_page8_v1", "propose_staged_page8_v2", "propose_staged_57"]) {
      const act = scene(name).act as ProposeAct;
      const payload = (fx[name].result!.payload!.raw as { result: { structuredContent: { staged_ids: Record<string, string> } } }).result.structuredContent;
      expect(act.counts).toEqual(stagedCounts(payload.staged_ids));
      expect(act.counts.units + act.counts.relations + act.counts.spans).toBe(Object.keys(payload.staged_ids).length);
    }
  });

  it("a blocked proposal is an answer: red items carry their diagnostic codes", () => {
    const draft = scene("propose_blocked_v1");
    const act = draft.act as ProposeAct;
    expect(draft.status).toBe("ok");
    expect(act.phase).toBe("blocked");
    expect(act.counts).toEqual({ units: 0, relations: 0, spans: 0 });
    expect(act.blocks.length).toBeGreaterThan(0);
    expect(act.blocks[0]).toMatchObject({ code: "quote_mismatch", localRef: "span.ch1" });
    expect(draft.summary).toMatch(/^被阻塞 · \d+ 个问题：quote_mismatch$/);
    // A span error marks the unit that cites it.
    const blocked = act.items.filter((item) => item.blocked.length > 0);
    expect(blocked.length).toBeGreaterThan(0);
    expect(blocked.flatMap((item) => item.blocked.map((block) => block.code))).toContain("invalid_utf8_boundary");
  });

  it("blocked relations proposal: unit-level diagnostics and relation ends", () => {
    const act = scene("propose_blocked_relations").act as ProposeAct;
    expect(act.phase).toBe("blocked");
    expect(act.blocks.map((block) => block.code)).toEqual(["mixed_source_context", "mixed_source_context"]);
    const unit = act.items.find((item) => item.localRef === "example11_unit")!;
    expect(unit.blocked.map((block) => block.code)).toContain("mixed_source_context");
    expect(act.relations.length).toBeGreaterThan(0);
  });

  it("supersedes: finds the replaced version's staged ids in earlier events", () => {
    const v1 = fx.propose_staged_page8_v1;
    const draft = scene("propose_staged_page8_v2", [v1.call, v1.result!]);
    const act = draft.act as ProposeAct;
    expect(act.supersedes).toEqual({ changeSetId: expect.stringContaining("69ce87a4"), version: 1 });
    expect(draft.detail).toBe("v2 · 取代 v1");
    // Same local refs → same stable ids: nothing of v1 is retired.
    expect(act.retired).toEqual([]);
    // A v1 that had an extra node: that node fades.
    const extra = structuredClone(v1);
    const payload = (extra.result!.payload!.raw as { result: { structuredContent: { staged_ids: Record<string, string> } } }).result.structuredContent;
    payload.staged_ids.gone = `mathscape:source-unit/${"9".repeat(64)}`;
    const replaced = scene("propose_staged_page8_v2", [extra.call, extra.result!]).act as ProposeAct;
    expect(replaced.retired).toEqual([`mathscape:source-unit/${"9".repeat(64)}`]);
  });

  it("running: HUD only, counts what it is checking", () => {
    const draft = running("propose_staged_57");
    expect(draft.status).toBe("running");
    expect(draft.summary).toBe("正在校验 17 个单元 · 20 条关系…");
    expect((draft.act as ProposeAct).phase).toBe("running");
  });

  it("resolves real staged nodes, anchors and relation ends on the map", () => {
    const draft = scene("propose_staged_57");
    const act = draft.act as ProposeAct;
    const unit = act.items[0].stagedId!;
    const artifact = act.items[0].anchors[0];
    const resolve: SceneResolve = (id) => (id === unit ? { nodeId: "n-unit", label: "U" } : id === artifact ? { nodeId: "n-pdf", label: "PDF" } : null);
    const resolved = resolveScene(draft, resolve);
    const out = resolved.act as ProposeAct;
    expect(out.items[0].node).toBe("n-unit");
    expect(out.items[0].anchorNodes).toEqual(["n-pdf"]);
    expect(out.items[1].node).toBeUndefined();
    expect(resolved.seeds).toEqual(["n-pdf"]);
    // The map count suffix is not about anchors for proposals.
    expect(resolved.summary).toBe("已暂存 · 单元 17 · 关系 20 · 片段 20");
  });

  it("replay compiles exactly what the live round saw (history up to the result)", () => {
    const v1 = fx.propose_staged_page8_v1;
    const v2 = fx.propose_staged_page8_v2;
    const history = [v1.call, v1.result!, v2.call, v2.result!];
    const drafts = compileRoundScenes([v2.call, v2.result!], history);
    expect(drafts).toHaveLength(1);
    expect(drafts[0]).toEqual(compileToolScene(v2.call, v2.result!, history.slice(0, 3)));
  });
});

describe("mathlib.search", () => {
  it("candidates with truncated type signatures, resident badge", () => {
    const draft = scene("mathlib_resident");
    const act = draft.act as MathlibAct;
    expect(act.phase).toBe("ok");
    expect(act.mode).toBe("resident");
    expect(act.query).toBe('"klDiv"');
    expect(act.candidates[0].name).toBe("InformationTheory.klDiv");
    expect(act.candidates.length).toBeLessThanOrEqual(8);
    expect(act.candidates.every((candidate) => [...(candidate.type ?? "")].length <= 90)).toBe(true);
    expect(act.truncated).toBe(true);
    expect(draft.summary).toMatch(/个候选（已截断） · 常驻$/);
  });

  it("no matches + one-shot fallback flag", () => {
    const act = scene("mathlib_oneshot_fallback").act as MathlibAct;
    expect(act.noMatches).toBe(true);
    expect(act.mode).toBe("oneshot");
    expect(act.fallback).toContain("memory budget");
    expect(scene("mathlib_no_matches").summary).toBe("没有匹配的声明");
  });

  it("rejected query: suggestions from the MCP error text", () => {
    const draft = scene("mathlib_rejected");
    const act = draft.act as MathlibAct;
    expect(draft.status).toBe("error");
    expect(act.suggestions).toEqual(["InformationTheory.klDiv", "InformationTheory.klFun"]);
    expect(draft.summary).toContain("Loogle rejected the query");
  });

  it("capability_unavailable / internal error carry the reason", () => {
    expect(scene("mathlib_capability").summary).toContain('Lean profile "mathlib" was not found');
    expect(scene("mathlib_error").summary).toContain("invalid Loogle JSON");
  });

  it("running: Loogle 检索中 with a running act", () => {
    const draft = running("mathlib_resident");
    expect(draft.summary).toBe("Loogle 检索中…");
    expect((draft.act as MathlibAct).phase).toBe("running");
    expect(draft.startedAt).toBeGreaterThan(1.7e12);
  });
});

describe("lean.check", () => {
  it("checked: ✓ and the faithfulness note", () => {
    const draft = scene("lean_checked");
    const act = draft.act as LeanAct;
    expect(act.phase).toBe("checked");
    expect(act.diagnostics).toEqual([]);
    expect(draft.summary).toBe("✓ 编译通过（编译通过 ≠ 证明忠实）");
    expect(act.code.split("\n").length).toBeLessThanOrEqual(12);
  });

  it("failed: diagnostics with line:column, first message line", () => {
    const draft = scene("lean_failed");
    const act = draft.act as LeanAct;
    expect(act.phase).toBe("failed");
    expect(act.diagnostics.length).toBeGreaterThanOrEqual(2);
    expect(act.diagnostics[0]).toMatchObject({ severity: "error", line: expect.any(Number), column: expect.any(Number) });
    expect(act.diagnostics[0].message).not.toContain("\n");
    expect(draft.summary).toMatch(/^未通过 · \d+ 条诊断（\d+ 个错误）$/);
    expect(act.elapsedMs).toBeGreaterThan(0);
  });

  it("invalid arguments: error act with the message", () => {
    const draft = scene("lean_error");
    expect(draft.status).toBe("error");
    expect((draft.act as LeanAct).error).toContain("missing field `profile_id`");
    expect(draft.summary).toBe("调用失败：invalid arguments: missing field `profile_id`");
  });
});

describe("material.locate", () => {
  it("exact match on one page", () => {
    const draft = scene("locate_ok");
    const act = draft.act as LocateAct;
    expect(act).toMatchObject({ phase: "ok", match: "精确匹配", page: 8 });
    expect(draft.summary).toBe("精确匹配 · 第 8 页");
    expect(draft.seeds[0]).toMatch(/^mathscape:artifact\//);
  });

  it("page unresolved: the diagnostic is the note", () => {
    const act = scene("locate_page_unresolved").act as LocateAct;
    expect(act.phase).toBe("ok");
    expect(act.page).toBeUndefined();
    expect(act.reason).toContain("no single PDF page");
  });

  it("not found / invalid range: mismatch reason", () => {
    const miss = scene("locate_not_found");
    expect(miss.status).toBe("ok");
    expect((miss.act as LocateAct).phase).toBe("miss");
    expect(miss.summary).toBe("未找到：引文与原文不一致");
    const error = scene("locate_error");
    expect(error.status).toBe("error");
    expect((error.act as LocateAct).reason).toContain("within must be a range");
  });
});

describe("change sets", () => {
  it("list_change_sets: light HUD", () => {
    expect(scene("list_change_sets_one").summary).toMatch(/^1 个变更集：将快照中覆盖/);
    expect(scene("list_change_sets_empty").summary).toBe("没有变更集");
    expect(resolveScene(scene("list_change_sets_one"), none).title).toBe("变更集 · 待审");
  });

  it("get_change_set: counts and its staged units as the lit layer", () => {
    const draft = scene("get_change_set");
    expect(draft.summary).toMatch(/^待审 · 单元 \d+ · 关系 \d+ · 片段 \d+$/);
    expect(draft.layers).toHaveLength(1);
    expect(draft.layers[0].every((id) => id.startsWith("mathscape:source-unit/"))).toBe(true);
    const resolved = resolveScene(draft, none);
    expect(resolved.title).toMatch(/^查看变更集 · change-set\//);
  });
});
