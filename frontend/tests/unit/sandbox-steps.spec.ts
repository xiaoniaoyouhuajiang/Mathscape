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
    expect(done.summary).toBe("已完成 2 步 · 12ms");
    expect(done.steps.map((step) => step.status)).toEqual(["completed", "completed"]);
    expect(done.steps[0].rawInput).toBe("query=紧集");
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
