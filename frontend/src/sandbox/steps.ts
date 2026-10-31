/**
 * Turn timeline for the sandbox dialogue.
 *
 * Codex `agent_events v0` stays the wire format. This module folds those
 * events into a small step list (id, kind, title, status, raw input/output)
 * so the DOM does not special-case every app-server item.
 */
import type { AgentEventLike } from "./format";

export type StepKind =
  | "search"
  | "read"
  | "execute"
  | "file"
  | "think"
  | "approval"
  | "error"
  | "usage"
  | "tool";

export type StepStatus =
  | "pending"
  | "running"
  | "completed"
  | "error"
  | "denied"
  | "awaiting-approval";

export interface ToolStep {
  id: string;
  kind: StepKind;
  title: string;
  status: StepStatus;
  rawInput?: string;
  rawOutput?: string;
  error?: string;
  startedAt?: string;
  endedAt?: string;
}

export type HeaderTone = "running" | "ok" | "warn" | "error" | "approval";

export interface TurnView {
  roundId?: string;
  running: boolean;
  outcome?: string;
  header: string;
  headerTone: HeaderTone;
  steps: ToolStep[];
  /** Collapsed label, e.g. `已完成 2 步 · 1.2s`. */
  summary: string;
  expandedByDefault: boolean;
  durationMs: number;
}

export interface TranscriptLike {
  ts: string;
  role: "user" | "agent" | "system";
  text: string;
  round_id?: string;
  outcome?: string;
}

export type DialogueRow =
  | { kind: "line"; entry: TranscriptLike; process: TurnView | null }
  | { kind: "live"; roundId: string; process: TurnView };

export const KIND_ICON: Record<StepKind, string> = {
  search: "搜",
  read: "读",
  execute: "令",
  file: "文",
  think: "思",
  approval: "批",
  error: "误",
  usage: "量",
  tool: "具",
};

export const STATUS_LABEL: Record<StepStatus, string> = {
  pending: "等待",
  running: "进行中",
  completed: "完成",
  error: "错误",
  denied: "已拒绝",
  "awaiting-approval": "等待审批",
};

interface ApprovalRaw {
  phase?: string;
  method?: string;
  requestId?: unknown;
  itemId?: unknown;
  summary?: string;
}

function str(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function payloadOf(event: AgentEventLike): Record<string, unknown> {
  return event.payload ?? {};
}

function clip(value: string, max = 48): string {
  const chars = [...value];
  if (chars.length <= max) return value;
  return `${chars.slice(0, max - 1).join("")}…`;
}

export function formatElapsed(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "";
  if (ms < 1000) return `${Math.round(ms)}ms`;
  if (ms < 10_000) return `${(ms / 1000).toFixed(1)}s`;
  return `${Math.round(ms / 1000)}s`;
}

function pretty(value: unknown): string {
  if (value === undefined || value === null) return "";
  if (typeof value === "string") {
    const trimmed = value.trim();
    if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
      try {
        return JSON.stringify(JSON.parse(trimmed), null, 2);
      } catch {
        return value;
      }
    }
    return value;
  }
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function kindFor(tool: string, rawType: string): StepKind {
  if (rawType === "commandExecution" || tool === "shell") return "execute";
  if (rawType === "fileChange" || tool === "file_change") return "file";
  if (tool.includes("search") || tool.includes("grep")) return "search";
  if (tool.includes("read") || tool.includes("outline") || tool.includes("image") || tool.includes("page")) {
    return "read";
  }
  if (tool.startsWith("graph.")) return "tool";
  return "tool";
}

function titleFor(tool: string, args: string, rawType: string): string {
  if (rawType === "commandExecution" || tool === "shell") return args || "shell";
  if (rawType === "fileChange" || tool === "file_change") return args || "文件变更";
  return tool || "工具";
}

function approvalOf(payload: Record<string, unknown>): ApprovalRaw | null {
  const raw = payload.raw;
  if (!raw || typeof raw !== "object") return null;
  const approval = (raw as Record<string, unknown>).approval;
  if (!approval || typeof approval !== "object") return null;
  return approval as ApprovalRaw;
}

function approvalStepId(approval: ApprovalRaw, seq: number): string {
  if (typeof approval.itemId === "string" && approval.itemId) return approval.itemId;
  if (approval.requestId !== undefined && approval.requestId !== null) {
    return `approval:${JSON.stringify(approval.requestId)}`;
  }
  return `approval:${seq}`;
}

function statusFromApproval(phase: string): StepStatus {
  if (phase === "awaiting") return "awaiting-approval";
  if (phase === "timeout" || phase === "denied") return "denied";
  return "error";
}

function statusFromResult(payload: Record<string, unknown>): StepStatus {
  const raw = payload.raw;
  const rawStatus =
    raw && typeof raw === "object" ? str((raw as Record<string, unknown>).status) : "";
  if (rawStatus === "declined" || rawStatus === "denied") return "denied";
  if (str(payload.status) === "error") return "error";
  return "completed";
}

function resultText(payload: Record<string, unknown>): string {
  const raw = payload.raw;
  if (raw && typeof raw === "object") {
    const record = raw as Record<string, unknown>;
    if (record.result !== undefined && record.result !== null) return pretty(record.result);
    if (record.error !== undefined && record.error !== null) return pretty(record.error);
    if (record.status) return str(record.status);
  }
  const status = str(payload.status);
  const ms = typeof payload.duration_ms === "number" ? `${payload.duration_ms} ms` : "";
  return [status, ms].filter(Boolean).join(" · ");
}

function inputText(payload: Record<string, unknown>): string {
  const raw = payload.raw;
  if (raw && typeof raw === "object") {
    const record = raw as Record<string, unknown>;
    if (record.arguments !== undefined) return pretty(record.arguments);
    if (typeof record.command === "string") return record.command;
  }
  return str(payload.args_summary);
}

function upsert(steps: ToolStep[], step: ToolStep): void {
  const index = steps.findIndex((item) => item.id === step.id);
  if (index < 0) {
    steps.push(step);
    return;
  }
  const prev = steps[index];
  steps[index] = {
    ...prev,
    ...step,
    rawInput: step.rawInput || prev.rawInput,
    rawOutput: step.rawOutput || prev.rawOutput,
    error: step.error || prev.error,
    startedAt: prev.startedAt ?? step.startedAt,
    kind: step.kind === "approval" || step.kind === "error" ? prev.kind : step.kind,
    title: step.title || prev.title,
    status: step.status,
  };
}

function applyApproval(steps: ToolStep[], event: AgentEventLike, approval: ApprovalRaw, message: string): void {
  const phase = str(approval.phase);
  const status = statusFromApproval(phase);
  const id = approvalStepId(approval, event.seq ?? steps.length);
  const title =
    phase === "timeout"
      ? "等待审批已超时"
      : str(approval.summary) || message || "审批";
  const existing = steps.find((step) => step.id === id);
  if (existing && existing.kind !== "approval" && existing.kind !== "error") {
    existing.status = status;
    existing.error = message || existing.error;
    if (phase === "timeout") existing.title = existing.title || title;
    existing.endedAt = existing.endedAt ?? event.ts;
    return;
  }
  upsert(steps, {
    id,
    kind: "approval",
    title,
    status,
    error: message,
    rawInput: str(approval.summary) || str(approval.method),
    startedAt: event.ts,
    endedAt: phase === "awaiting" ? undefined : event.ts,
  });
}

export function buildTurnProcess(
  events: AgentEventLike[],
  options: { running?: boolean; nowMs?: number; outcome?: string; roundId?: string } = {},
): TurnView {
  const steps: ToolStep[] = [];
  let outcome = options.outcome;
  for (const event of events) {
    const payload = payloadOf(event);
    if (event.type === "turn_end") {
      const end = str(payload.outcome);
      if (end) outcome = end;
      continue;
    }
    if (event.type === "tool_call") {
      const tool = str(payload.tool) || "tool";
      const rawType =
        payload.raw && typeof payload.raw === "object"
          ? str((payload.raw as Record<string, unknown>).type)
          : "";
      const args = str(payload.args_summary);
      const id = str(payload.call_id) || `call-${event.seq ?? steps.length}`;
      const existing = steps.find((step) => step.id === id);
      const keep =
        existing?.status === "denied" || existing?.status === "awaiting-approval"
          ? existing.status
          : "running";
      upsert(steps, {
        id,
        kind: kindFor(tool, rawType),
        title: titleFor(tool, args, rawType),
        status: keep,
        rawInput: inputText(payload) || args,
        startedAt: event.ts,
        error: existing?.error,
      });
      continue;
    }
    if (event.type === "tool_result") {
      const id = str(payload.call_id) || `result-${event.seq ?? steps.length}`;
      const status = statusFromResult(payload);
      const existing = steps.find((step) => step.id === id);
      const output = resultText(payload);
      if (existing) {
        existing.status = status;
        existing.rawOutput = output;
        existing.endedAt = event.ts;
        if (status === "error" && !existing.error) existing.error = output;
        if (status === "denied" && !existing.error) existing.error = "已拒绝";
      } else {
        upsert(steps, {
          id,
          kind: "tool",
          title: "工具结果",
          status,
          rawOutput: output,
          error: status === "error" ? output : status === "denied" ? "已拒绝" : undefined,
          endedAt: event.ts,
        });
      }
      continue;
    }
    if (event.type === "error") {
      const message = str(payload.message);
      const approval = approvalOf(payload);
      if (approval) {
        applyApproval(steps, event, approval, message);
      } else {
        steps.push({
          id: `error-${event.seq ?? steps.length}`,
          kind: "error",
          title: message || `错误 ${str(payload.code) || ""}`.trim(),
          status: "error",
          error: message,
          startedAt: event.ts,
          endedAt: event.ts,
        });
      }
      continue;
    }
    if (event.type === "usage") {
      const input = typeof payload.input_tokens === "number" ? payload.input_tokens : undefined;
      const output = typeof payload.output_tokens === "number" ? payload.output_tokens : undefined;
      const tokens =
        input !== undefined || output !== undefined ? `${input ?? "?"}/${output ?? "?"}` : "";
      steps.push({
        id: `usage-${event.seq ?? steps.length}`,
        kind: "usage",
        title: tokens ? `用量 ${tokens}` : "用量",
        status: "completed",
        rawOutput: pretty(payload),
        startedAt: event.ts,
        endedAt: event.ts,
      });
      continue;
    }
    if (event.type === "cancel" && payload.ack !== true) {
      steps.push({
        id: `cancel-${event.seq ?? steps.length}`,
        kind: "think",
        title: "请求取消",
        status: "pending",
        startedAt: event.ts,
      });
    }
  }

  const ended = events.some((event) => event.type === "turn_end");
  const running = options.running === true && !ended;
  const durationMs = elapsedMs(events, running, options.nowMs ?? Date.now());
  const header = headerText(steps, running, outcome);
  return {
    roundId: options.roundId,
    running,
    outcome,
    header,
    headerTone: headerTone(header, running, outcome),
    steps,
    summary: summaryText(steps.length, durationMs, running),
    expandedByDefault: running,
    durationMs,
  };
}

function elapsedMs(events: AgentEventLike[], running: boolean, nowMs: number): number {
  const stamps = events
    .map((event) => (event.ts ? Date.parse(event.ts) : NaN))
    .filter((ms) => Number.isFinite(ms));
  let span = 0;
  if (stamps.length > 0) {
    const start = Math.min(...stamps);
    const end = running ? nowMs : Math.max(...stamps);
    span = Math.max(0, end - start);
  }
  if (span > 0) return span;
  return events.reduce((sum, event) => {
    if (event.type !== "tool_result") return sum;
    const ms = event.payload?.duration_ms;
    return sum + (typeof ms === "number" ? ms : 0);
  }, 0);
}

function headerText(steps: ToolStep[], running: boolean, outcome?: string): string {
  if (running) {
    const awaiting = [...steps].reverse().find((step) => step.status === "awaiting-approval");
    if (awaiting) return "等待审批";
    const active = [...steps].reverse().find((step) => step.status === "running" || step.status === "pending");
    if (active) return clip(`正在调用 ${active.title}`);
    return "处理中";
  }
  if (outcome === "cancelled") return "已取消";
  if (outcome === "failed") {
    const readable = [...steps]
      .reverse()
      .find((step) => (step.error ?? step.title).includes("审批"));
    if (readable) return clip(readable.error || readable.title);
    return "失败";
  }
  return "已完成";
}

function headerTone(header: string, running: boolean, outcome?: string): HeaderTone {
  if (header === "等待审批" || header.includes("等待审批")) return "approval";
  if (running) return "running";
  if (outcome === "failed" || header.startsWith("审批被策略拒绝")) return "error";
  if (outcome === "cancelled") return "warn";
  return "ok";
}

function summaryText(count: number, durationMs: number, running: boolean): string {
  const time = formatElapsed(durationMs);
  const suffix = time ? ` · ${time}` : "";
  if (running) return count > 0 ? `进行中 · ${count} 步${suffix}` : `进行中${suffix}`;
  return count > 0 ? `已完成 ${count} 步${suffix}` : `已完成${suffix}`;
}

export function dialogueRows(
  transcript: TranscriptLike[],
  events: AgentEventLike[],
  activeRoundId: string | null,
  nowMs?: number,
): DialogueRow[] {
  const rows: DialogueRow[] = [];
  const consumed = new Set<string>();
  for (const entry of transcript) {
    const roundId = entry.round_id;
    const show =
      (entry.role === "agent" || entry.role === "system") &&
      typeof roundId === "string" &&
      roundId.length > 0;
    let process: TurnView | null = null;
    if (show && roundId) {
      consumed.add(roundId);
      const roundEvents = events.filter((event) => event.round_id === roundId);
      if (roundEvents.length > 0 || activeRoundId === roundId) {
        process = buildTurnProcess(roundEvents, {
          running: activeRoundId === roundId,
          nowMs,
          outcome: entry.outcome,
          roundId,
        });
      }
    }
    rows.push({ kind: "line", entry, process });
  }
  if (activeRoundId && !consumed.has(activeRoundId)) {
    const roundEvents = events.filter((event) => event.round_id === activeRoundId);
    rows.push({
      kind: "live",
      roundId: activeRoundId,
      process: buildTurnProcess(roundEvents, {
        running: true,
        nowMs,
        roundId: activeRoundId,
      }),
    });
  }
  return rows;
}
