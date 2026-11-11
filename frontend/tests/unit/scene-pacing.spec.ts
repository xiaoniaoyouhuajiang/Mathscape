import { describe, expect, it } from "vitest";
import { SCENE_PACING, capLayers, catchUpLive, elapsedLabel, enqueueLive, runningLabel, type Segment } from "../../src/graph/scene-pacing";
import type { SceneStatus, ToolScene } from "../../src/sandbox/scene/types";

const scene = (callId: string, tool: ToolScene["tool"], status: SceneStatus = "ok", extra: Partial<ToolScene> = {}): ToolScene => ({
  callId,
  tool,
  status,
  title: tool,
  summary: "",
  seeds: [],
  layers: [],
  offMap: 0,
  capped: false,
  ...extra,
});
const seg = (s: ToolScene, part: Segment["part"] = "result", enqueuedAt = 0): Segment => ({ scene: s, part, enqueuedAt, merged: new Set([s.callId]) });
const ids = (queue: Segment[]) => queue.map((item) => `${item.scene.callId}:${item.part}`);

describe("live pacing", () => {
  it("a newer frame of a queued call replaces it in place", () => {
    let queue = enqueueLive([], seg(scene("a", "material.read", "running"), "call"));
    queue = enqueueLive(queue, seg(scene("b", "graph.get", "running"), "call"));
    queue = enqueueLive(queue, seg(scene("a", "material.read"), "result"));
    expect(ids(queue)).toEqual(["a:result", "b:call"]);
  });

  it("coalesces consecutive same-tool segments into the newest", () => {
    let queue: Segment[] = [];
    for (const id of ["g1", "g2", "g3"]) queue = enqueueLive(queue, seg(scene(id, "material.grep")));
    expect(ids(queue)).toEqual(["g3:result"]);
    expect(queue[0].scene.coalesced).toBe(2);
    // A different tool in between stops the merge.
    queue = enqueueLive(queue, seg(scene("r", "graph.get")));
    queue = enqueueLive(queue, seg(scene("g4", "material.grep")));
    expect(ids(queue)).toEqual(["g3:result", "r:result", "g4:result"]);
  });

  it("never coalesces or drops proposals", () => {
    let queue: Segment[] = [];
    queue = enqueueLive(queue, seg(scene("p1", "propose_changes")));
    queue = enqueueLive(queue, seg(scene("p2", "propose_changes")));
    expect(ids(queue)).toEqual(["p1:result", "p2:result"]);
    const tools: ToolScene["tool"][] = ["graph.get", "material.read", "pdf.outline", "graph.neighborhood", "material.grep", "workspace.search", "lean.check"];
    tools.forEach((tool, index) => {
      queue = enqueueLive(queue, seg(scene(`x${index}`, tool)));
    });
    expect(queue.length).toBe(SCENE_PACING.queueCap);
    expect(ids(queue).slice(0, 2)).toEqual(["p1:result", "p2:result"]);
    expect(ids(queue).at(-1)).toBe("x6:result");
  });

  it("parsing frames are not coalesced with other reads", () => {
    const parsing = scene("a", "material.read", "running", { act: { kind: "parsing", label: "解析中", phase: "running" } });
    let queue = enqueueLive([], seg(scene("z", "material.read")));
    queue = enqueueLive(queue, seg(parsing, "call"));
    expect(ids(queue)).toEqual(["z:result", "a:call"]);
  });

  it("skips to the latest when the head is far behind (proposals kept)", () => {
    const queue = [
      seg(scene("a", "graph.get"), "result", 0),
      seg(scene("p", "propose_changes"), "result", 100),
      seg(scene("b", "material.read"), "result", 200),
      seg(scene("c", "pdf.outline"), "result", 300),
    ];
    expect(ids(catchUpLive(queue, SCENE_PACING.lagMs - 1))).toHaveLength(4);
    const caught = catchUpLive(queue, SCENE_PACING.lagMs + 1);
    expect(ids(caught)).toEqual(["p:result", "c:result"]);
    expect(caught.at(-1)!.skipped).toBe(2);
  });

  it("a replay (no enqueuedAt) is never skipped", () => {
    const queue: Segment[] = [{ scene: scene("a", "graph.get"), part: "full" }, { scene: scene("b", "graph.get"), part: "full" }];
    expect(catchUpLive(queue, 1e15)).toHaveLength(2);
  });

  it("caps result layers", () => {
    expect(capLayers([["a"], ["b"], ["c"], ["d"], ["e"], ["f"]], 4)).toEqual([["a"], ["b"], ["c"], ["d", "e", "f"]]);
    expect(capLayers([["a"]], 4)).toEqual([["a"]]);
  });
});

describe("running labels", () => {
  it("names the long-running tools", () => {
    expect(runningLabel({ tool: "mathlib.search", title: "Mathlib 检索 · x" })).toBe("Loogle 启动中…");
    expect(runningLabel({ tool: "lean.check", title: "Lean 检查" })).toBe("Lean 编译中…");
    expect(runningLabel({ tool: "material.read", title: "阅读材料 · a.pdf", act: { kind: "parsing", label: "解析中", phase: "running", pages: { from: 3, to: 4 } } })).toBe(
      "Marker 解析中 第3–4页",
    );
    expect(runningLabel({ tool: "graph.get", title: "查看节点 · 引理" })).toBe("查看节点 运行中…");
  });

  it("formats elapsed time", () => {
    expect(elapsedLabel(2300)).toBe("2.3s");
    expect(elapsedLabel(35_400)).toBe("35s");
    expect(elapsedLabel(78_000)).toBe("1m18s");
  });
});
