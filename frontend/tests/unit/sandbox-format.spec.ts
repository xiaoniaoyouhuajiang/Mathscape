import { describe, expect, it } from "vitest";

import {
  agentStatusLine,
  modelOptionLabel,
  enterConfirmMessage,
  eventChip,
  fullTimestamp,
  latestRoundEvents,
  returnConfirmMessage,
  sessionOpenByDefault,
  shortTimestamp,
} from "../../src/sandbox/format";

describe("sandbox panel format helpers", () => {
  it("shortens RFC 3339 timestamps for the summary row", () => {
    expect(shortTimestamp("2026-10-30T22:56:46.409+08:00")).toBe("10-30 22:56");
    expect(fullTimestamp("2026-10-30T22:56:46.409+08:00")).toBe("2026-10-30 22:56:46");
  });

  it("returns unparseable values unchanged", () => {
    expect(shortTimestamp("unknown")).toBe("unknown");
    expect(fullTimestamp("")).toBe("");
  });

  it("expands live sessions and collapses discarded ones by default", () => {
    expect(sessionOpenByDefault("active")).toBe(true);
    expect(sessionOpenByDefault("exported")).toBe(true);
    expect(sessionOpenByDefault("discarded")).toBe(false);
  });
});


describe("S1 event strip and switch prompts", () => {
  it("maps the seven agent_events v0 types to chips", () => {
    expect(eventChip({ type: "turn_start", seq: 0 }).label).toBe("开始");
    expect(eventChip({ type: "tool_call", payload: { tool: "workspace.search", args_summary: "query=紧集" } })).toMatchObject({
      label: "→ workspace.search",
      tone: "tool",
    });
    expect(eventChip({ type: "tool_result", payload: { status: "ok" } }).tone).toBe("ok");
    expect(eventChip({ type: "tool_result", payload: { status: "not_found" } }).tone).toBe("warn");
    expect(eventChip({ type: "usage", payload: { input_tokens: 12, output_tokens: 3 } }).label).toBe("用量 12/3");
    expect(eventChip({ type: "cancel", payload: { ack: true } }).label).toBe("取消已确认");
    expect(eventChip({ type: "error", payload: { code: "auth", message: "not logged in" } })).toMatchObject({
      label: "错误 auth",
      tone: "error",
    });
    expect(eventChip({ type: "turn_end", payload: { outcome: "completed" } })).toMatchObject({ label: "完成", tone: "ok" });
    expect(eventChip({ type: "turn_end", payload: { outcome: "cancelled" } }).tone).toBe("warn");
    expect(eventChip({ type: "turn_end", payload: { outcome: "failed" } }).tone).toBe("error");
    expect(eventChip({ type: "mystery" })).toMatchObject({ label: "mystery", tone: "muted" });
  });

  it("keeps only the latest round for the strip", () => {
    const events = [
      { type: "turn_start", round_id: "r1" },
      { type: "turn_end", round_id: "r1" },
      { type: "turn_start", round_id: "r2" },
    ];
    expect(latestRoundEvents(events)).toEqual([{ type: "turn_start", round_id: "r2" }]);
    expect(latestRoundEvents([])).toEqual([]);
  });

  it("names the target workspace in both switch prompts", () => {
    const enter = enterConfirmMessage("sbx_1", "/data/sandbox/sessions/sbx_1/workspace");
    expect(enter).toContain("sbx_1");
    expect(enter).toContain("/data/sandbox/sessions/sbx_1/workspace");
    expect(enter).toContain("检查点和母工作区不会被修改");
    expect(returnConfirmMessage("/data/workspace")).toContain("/data/workspace");
  });
});


describe("S1 agent status line", () => {
  const base = { backend: "codex", codexProgram: "codex", tools: ["a", "b", "c", "d", "e", "f", "g"] };
  it("shows version, model, and tool count when Codex is available without a picker", () => {
    expect(
      agentStatusLine({
        ...base,
        available: true,
        model: "gpt-6.1-sol",
        probes: [{ capability: "codex", state: "available", found_version: "0.124.0" }],
      }),
    ).toBe("本机 Codex 0.124.0 · gpt-6.1-sol · 7 个 Mathscape 工具");
  });
  it("omits the model segment when the dock shows a model picker", () => {
    expect(
      agentStatusLine({
        ...base,
        available: true,
        model: "gpt-6.1-sol",
        modelSelection: true,
        models: [{ id: "gpt-6.1-sol" }],
        probes: [{ capability: "codex", state: "available", found_version: "0.124.0" }],
      }),
    ).toBe("本机 Codex 0.124.0 · 7 个 Mathscape 工具");
  });
  it("omits the model segment when Codex has no default model recorded", () => {
    expect(
      agentStatusLine({ ...base, available: true, probes: [{ capability: "codex", state: "available", found_version: "0.124.0" }] }),
    ).toBe("本机 Codex 0.124.0 · 7 个 Mathscape 工具");
  });
  it("names the program and the install hint when it is missing", () => {
    expect(
      agentStatusLine({ ...base, available: false, probes: [{ capability: "codex", state: "missing", install_hint: "npm i -g @openai/codex" }] }),
    ).toBe("未检测到可用的本机 Codex：codex（npm i -g @openai/codex）");
  });
  it("formats model option labels", () => {
    expect(modelOptionLabel({ id: "gpt-a", displayName: "A" })).toBe("A (gpt-a)");
    expect(modelOptionLabel({ id: "gpt-a", displayName: "gpt-a" })).toBe("gpt-a");
    expect(modelOptionLabel({ id: "gpt-a" })).toBe("gpt-a");
  });
});
