import { describe, expect, it } from "vitest";

import { buildTurnProcess, dialogueRows, formatElapsed } from "../../src/sandbox/steps";
import type { AgentEventLike } from "../../src/sandbox/format";

function event(seq: number, type: string, payload: Record<string, unknown>, ts = "2026-10-31T09:01:00+08:00"): AgentEventLike {
  return { type, seq, round_id: "r1", ts, payload };
}

describe("formatElapsed", () => {
  it("uses milliseconds under a second and Xs after", () => {
    expect(formatElapsed(0)).toBe("");
    expect(formatElapsed(12)).toBe("12ms");
    expect(formatElapsed(3200)).toBe("3.2s");
    expect(formatElapsed(12500)).toBe("13s");
  });
});

describe("buildTurnProcess", () => {
  it("shows a running tool above the answer and collapses when the turn ends", () => {
    const running = buildTurnProcess(
      [
        event(0, "turn_start", { input_ref: "x" }),
        event(1, "tool_call", { call_id: "c1", tool: "workspace.search", args_summary: "query=紧集" }),
      ],
      { running: true, nowMs: Date.parse("2026-10-31T09:01:01.200+08:00") },
    );
    expect(running.header).toBe("正在调用 workspace.search");
    expect(running.expandedByDefault).toBe(true);
    expect(running.steps[0]).toMatchObject({ id: "c1", kind: "search", status: "running", title: "workspace.search" });
    expect(running.summary).toContain("进行中");

    const done = buildTurnProcess(
      [
        event(0, "turn_start", { input_ref: "x" }),
        event(1, "tool_call", { call_id: "c1", tool: "workspace.search", args_summary: "query=紧集" }),
        event(2, "tool_result", { call_id: "c1", status: "ok", duration_ms: 12 }),
        event(3, "usage", { input_tokens: 120, output_tokens: 30 }),
        event(4, "turn_end", { outcome: "completed" }),
      ],
      { running: false },
    );
    expect(done.header).toBe("已完成");
    expect(done.expandedByDefault).toBe(false);
    expect(done.summary).toBe("已完成 1 步 · 12ms · 120/30 tokens");
    expect(done.steps.map((step) => step.status)).toEqual(["completed"]);
    expect(done.usage).toMatchObject({ inputTokens: 120, outputTokens: 30 });
    expect(done.steps[0].rawInput).toBe("query=紧集");
  });

  it("folds usage into the turn and hides it when every count is 0", () => {
    const zero = buildTurnProcess(
      [
        event(0, "turn_start", {}),
        event(1, "usage", { input_tokens: 0, output_tokens: 0 }),
        event(2, "usage", { input_tokens: 0, output_tokens: 0 }),
        event(3, "turn_end", { outcome: "completed" }),
      ],
      { running: false },
    );
    expect(zero.steps).toEqual([]);
    expect(zero.usage).toBeUndefined();
    expect(zero.summary).not.toContain("tokens");

    const summed = buildTurnProcess(
      [
        event(1, "usage", { input_tokens: 1200, output_tokens: 100, cached_input_tokens: 800 }),
        event(2, "usage", { input_tokens: 300, output_tokens: 200 }),
        event(3, "turn_end", { outcome: "completed" }),
      ],
      { running: false },
    );
    expect(summed.steps).toEqual([]);
    expect(summed.usage).toMatchObject({ inputTokens: 1500, outputTokens: 300, cachedInputTokens: 800 });
    expect(summed.summary).toContain("1.5k/300 tokens");
    expect(summed.usage?.raw).toContain("1200");
  });

  it("renders MCP startup as one uncounted step with the tool count", () => {
    const mcp = (status: string, raw: Record<string, unknown>) => [
      event(1, "tool_call", { call_id: "mcp-startup", tool: "mcp.startup", server: "codex", args_summary: "mathscape", raw: { mcp: raw } }),
      event(2, "tool_result", { call_id: "mcp-startup", status, raw: { mcp: raw } }),
    ];
    const eight = ["workspace.search", "material.read", "material.grep", "pdf.outline", "graph.get", "graph.neighborhood", "ink.page_image", "pdf.page_image"];
    const ok = buildTurnProcess(
      [
        event(0, "turn_start", {}),
        ...mcp("ok", { server: "mathscape", status: "ready", tools: eight, expected: eight, missing: [] }),
        event(3, "tool_call", { call_id: "c1", tool: "workspace.search" }),
        event(4, "tool_result", { call_id: "c1", status: "ok", duration_ms: 3 }),
        event(5, "turn_end", { outcome: "completed" }),
      ],
      { running: false },
    );
    expect(ok.steps[0]).toMatchObject({ kind: "mcp", title: "MCP 工具 8/8", status: "completed" });
    expect(ok.summary).toMatch(/^已完成 1 步/);
    expect(ok.mcpAlert).toBeUndefined();
    expect(ok.headerTone).toBe("ok");

    const partial = buildTurnProcess(
      [
        ...mcp("error", { server: "mathscape", status: "ready", tools: eight.slice(0, 4), expected: eight, missing: eight.slice(4) }),
        event(3, "turn_end", { outcome: "completed" }),
      ],
      { running: false },
    );
    expect(partial.steps[0]).toMatchObject({ kind: "mcp", title: "MCP 工具 4/8", status: "error" });
    expect(partial.steps[0].rawOutput).toContain("缺少：graph.get、graph.neighborhood、ink.page_image、pdf.page_image");
    expect(partial.summary).toBe("已完成 · MCP 工具 4/8");
    expect(partial.headerTone).toBe("warn");

    const failed = buildTurnProcess(
      [...mcp("error", {
        server: "mathscape",
        status: "failed",
        error: "MCP client for `mathscape` failed to start: MCP startup failed: No such file or directory",
      })],
      { running: true },
    );
    expect(failed.steps[0].title).toBe("MCP 启动失败：No such file or directory");
    expect(failed.header).toBe("处理中");
    expect(failed.steps[0].rawOutput).toContain("MCP client for `mathscape` failed to start");

    const unknown = buildTurnProcess(mcp("ok", { server: "mathscape", status: "ready", tools: null }), {
      running: false,
    });
    expect(unknown.steps[0].title).toBe("MCP 已就绪");
  });

  it("names an approval-policy failure instead of a bare internal code", () => {
    const view = buildTurnProcess(
      [
        event(0, "turn_start", {}),
        event(1, "error", {
          code: "protocol",
          message: "审批被策略拒绝：MCP tool call requires approval, but approval policy is never",
          fatal: true,
        }),
        event(2, "turn_end", { outcome: "failed" }),
      ],
      { outcome: "failed" },
    );
    expect(view.header).toContain("审批被策略拒绝");
    expect(view.header).not.toBe("internal");
    expect(view.steps[0]).toMatchObject({ kind: "error", status: "error" });
    expect(view.steps[0].title).toContain("审批被策略拒绝");
  });

  it("maps awaiting approval and the timeout phrase onto a denied card", () => {
    const waiting = buildTurnProcess(
      [
        event(1, "error", {
          message: "等待审批：rm -rf",
          raw: { approval: { phase: "awaiting", requestId: 9, summary: "rm -rf" } },
        }),
      ],
      { running: true },
    );
    expect(waiting.header).toBe("等待审批");
    expect(waiting.steps[0].status).toBe("awaiting-approval");

    const timed = buildTurnProcess(
      [
        event(1, "error", {
          message: "等待审批：rm -rf",
          raw: { approval: { phase: "awaiting", requestId: 9, summary: "rm -rf" } },
        }),
        event(2, "error", {
          message: "等待审批已超时",
          raw: { approval: { phase: "timeout", requestId: 9, summary: "rm -rf" } },
        }),
      ],
      { running: false },
    );
    expect(timed.steps).toHaveLength(1);
    expect(timed.steps[0]).toMatchObject({ status: "denied", title: "等待审批已超时", error: "等待审批已超时" });
  });

  it("marks a declined shell item as denied and keeps the command as the title", () => {
    const view = buildTurnProcess([
      event(1, "tool_call", {
        call_id: "cmd_1",
        tool: "shell",
        server: "codex",
        args_summary: "rm -rf /tmp",
        raw: { type: "commandExecution", command: "rm -rf /tmp" },
      }),
      event(2, "error", {
        message: "审批策略拒绝了 shell 命令（沙箱保持只读）",
        raw: { approval: { phase: "denied", itemId: "cmd_1", summary: "rm -rf /tmp" } },
      }),
      event(3, "tool_result", {
        call_id: "cmd_1",
        status: "error",
        raw: { type: "commandExecution", status: "declined" },
      }),
    ]);
    expect(view.steps).toHaveLength(1);
    expect(view.steps[0]).toMatchObject({
      id: "cmd_1",
      kind: "execute",
      title: "rm -rf /tmp",
      status: "denied",
    });
    expect(view.steps[0].error).toContain("审批策略拒绝了 shell");
  });
});

describe("dialogueRows", () => {
  it("keeps the live process ahead of the answer and attaches history to the agent line", () => {
    const events = [
      event(0, "turn_start", {}),
      event(1, "tool_call", { call_id: "c1", tool: "material.read", args_summary: "artifact_id=a1" }),
    ];
    const live = dialogueRows(
      [{ ts: "t", role: "user", text: "hi", round_id: "r1" }],
      events,
      "r1",
    );
    expect(live.map((row) => row.kind)).toEqual(["line", "live"]);
    expect(live[1]).toMatchObject({ kind: "live", process: { header: "正在调用 material.read", running: true } });

    const settled = dialogueRows(
      [
        { ts: "t", role: "user", text: "hi" },
        { ts: "t2", role: "agent", text: "done", round_id: "r1", outcome: "completed" },
      ],
      [...events, event(2, "tool_result", { call_id: "c1", status: "ok", duration_ms: 4 }), event(3, "turn_end", { outcome: "completed" })],
      null,
    );
    expect(settled).toHaveLength(2);
    expect(settled[1]).toMatchObject({
      kind: "line",
      process: { header: "已完成", expandedByDefault: false },
    });
  });
});

describe("parsing (sync ensure)", () => {
  it("marks a running material.read the map shows as 解析中, and drops it once done", () => {
    const events = [
      event(0, "turn_start", { input_ref: "x" }),
      event(1, "tool_call", { call_id: "c1", tool: "material.read", args_summary: "pdf_pages=3-4" }),
    ];
    const parsing = new Set(["c1"]);
    const running = buildTurnProcess(events, { running: true, parsing });
    expect(running.steps[0]).toMatchObject({ id: "c1", status: "running", title: "material.read · 解析中" });
    expect(running.header).toContain("解析中");
    const plain = buildTurnProcess(events, { running: true });
    expect(plain.steps[0].title).toBe("material.read");
    const done = buildTurnProcess([...events, event(2, "tool_result", { call_id: "c1", status: "ok", duration_ms: 9000 })], { running: true, parsing });
    expect(done.steps[0].title).toBe("material.read");
  });
});
