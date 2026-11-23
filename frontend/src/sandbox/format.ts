/** Pure display helpers for the sandbox panel (kept DOM-free for unit tests). */

export type SessionState = "active" | "exported" | "discarded";

export const STATE_LABEL: Record<SessionState, string> = {
  active: "进行中",
  exported: "已导出",
  discarded: "已丢弃",
};

/**
 * Compact timestamp for one-line summaries: `2026-10-30T22:56:46.409+08:00`
 * → `10-30 22:56`. The backend already emits local-offset RFC 3339, so this
 * only trims; anything unparseable is returned unchanged.
 */
export function shortTimestamp(value: string): string {
  const match = /^\d{4}-(\d{2})-(\d{2})[T ](\d{2}):(\d{2})/.exec(value);
  return match ? `${match[1]}-${match[2]} ${match[3]}:${match[4]}` : value;
}

/** Readable full timestamp: `2026-10-30 22:56:46`. */
export function fullTimestamp(value: string): string {
  const match = /^(\d{4}-\d{2}-\d{2})[T ](\d{2}:\d{2}:\d{2})/.exec(value);
  return match ? `${match[1]} ${match[2]}` : value;
}

/** Details default: live sessions open, discarded ones collapsed. */
export function sessionOpenByDefault(state: SessionState): boolean {
  return state !== "discarded";
}

// ------------------------------------------------------------------ S1

export type TranscriptRole = "user" | "agent" | "system";

export const ROLE_LABEL: Record<TranscriptRole, string> = {
  user: "User",
  agent: "Agent",
  system: "System",
};

/** Minimal `agent_events v0` shape the dialogue reads (extra fields ignored). */
export interface AgentEventLike {
  type: string;
  round_id?: string;
  seq?: number;
  /** RFC 3339 timestamp from the event line, used for step duration. */
  ts?: string;
  payload?: Record<string, unknown>;
}

export type EventTone = "turn" | "tool" | "ok" | "warn" | "error" | "muted";

export interface EventChip {
  label: string;
  tone: EventTone;
  title: string;
}

function str(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function clip(value: string, max = 48): string {
  return value.length > max ? `${value.slice(0, max - 1)}…` : value;
}

/**
 * One chip per event for the S1 event strip (turn / tool_call·result /
 * usage / cancel / error). Only the seven v0 types are known; anything else
 * is shown verbatim as muted rather than guessed at.
 */
export function eventChip(event: AgentEventLike): EventChip {
  const p = event.payload ?? {};
  const seq = typeof event.seq === "number" ? `#${event.seq} ` : "";
  switch (event.type) {
    case "turn_start":
      return { label: "开始", tone: "turn", title: `${seq}turn_start` };
    case "turn_end": {
      const outcome = str(p.outcome);
      const label = outcome === "completed" ? "完成" : outcome === "cancelled" ? "已取消" : "失败";
      const tone: EventTone = outcome === "completed" ? "ok" : outcome === "cancelled" ? "warn" : "error";
      return { label, tone, title: `${seq}turn_end · ${outcome}` };
    }
    case "tool_call": {
      const tool = str(p.tool) || "tool";
      const args = str(p.args_summary);
      return { label: `→ ${tool}`, tone: "tool", title: `${seq}${tool}${args ? ` · ${clip(args, 200)}` : ""}` };
    }
    case "tool_result": {
      const status = str(p.status) || "?";
      const ms = typeof p.duration_ms === "number" ? ` · ${p.duration_ms} ms` : "";
      return {
        label: `← ${status}`,
        tone: status === "ok" ? "ok" : status === "error" ? "error" : "warn",
        title: `${seq}tool_result · ${status}${ms}`,
      };
    }
    case "usage": {
      const input = typeof p.input_tokens === "number" ? p.input_tokens : undefined;
      const output = typeof p.output_tokens === "number" ? p.output_tokens : undefined;
      const tokens = input !== undefined || output !== undefined ? `${input ?? "?"}/${output ?? "?"}` : "";
      return { label: tokens ? `用量 ${tokens}` : "用量", tone: "muted", title: `${seq}usage (input/output tokens)` };
    }
    case "cancel":
      return {
        label: p.ack === true ? "取消已确认" : "请求取消",
        tone: "warn",
        title: `${seq}cancel · ${str(p.requested_by)}`,
      };
    case "error": {
      const message = str(p.message);
      if (message.includes("等待审批已超时")) {
        return { label: "等待审批已超时", tone: "warn", title: `${seq}${message}` };
      }
      if (message.includes("审批")) {
        return { label: "审批被拒绝", tone: "error", title: `${seq}${clip(message, 200)}` };
      }
      return {
        label: `错误 ${str(p.code)}`.trim(),
        tone: "error",
        title: `${seq}error · ${clip(message, 200)}`,
      };
    }
    default:
      return { label: event.type || "?", tone: "muted", title: `${seq}${event.type}` };
  }
}

/** Events of the most recent round only (the strip shows one round at a time). */
export function latestRoundEvents<T extends AgentEventLike>(events: T[]): T[] {
  const last = events.at(-1)?.round_id;
  return last === undefined ? [] : events.filter((event) => event.round_id === last);
}

export function enterConfirmMessage(sessionId: string, workspaceRoot: string): string {
  return [
    `把本窗口切换到沙箱 ${sessionId}？`,
    "",
    `隔离工作区：${workspaceRoot}`,
    "",
    "切换后图谱、阅读器、手写等都读写这份隔离副本；检查点和母工作区不会被修改。",
    "顶栏会一直显示「沙箱」标记，可随时「返回母工作区」。窗口将重新加载。",
  ].join("\n");
}

export function returnConfirmMessage(motherRoot: string): string {
  return [
    "返回母工作区？",
    "",
    `母工作区：${motherRoot}`,
    "",
    "沙箱里的改动留在该沙箱的隔离目录中（可稍后导出或丢弃），不会带回母工作区。窗口将重新加载。",
  ].join("\n");
}

export interface AgentEffortLike {
  id: string;
  description?: string | null;
}

export interface AgentModelLike {
  id: string;
  displayName?: string | null;
  /** Reasoning efforts the model advertises (Codex `supported_reasoning_levels`). */
  reasoningEfforts?: AgentEffortLike[];
  /** Catalog default effort (Codex `default_reasoning_level`). */
  defaultReasoningEffort?: string | null;
}

export interface AgentStatusLike {
  backend: string;
  codexProgram: string;
  program?: string;
  backends?: { backend: string; program: string; available: boolean; probes: AgentStatusLike["probes"] }[];
  backendSelection?: boolean;
  tools: string[];
  available: boolean;
  probes: { capability: string; state: string; found_version?: string | null; install_hint?: string | null }[];
  /** Current model id from the agent adapter. */
  model?: string | null;
  /** Selectable models when the adapter supports selection. */
  models?: AgentModelLike[];
  /** True when the adapter exposes list/set model. */
  modelSelection?: boolean;
  /** Reasoning effort the next round will use. */
  effort?: string | null;
  /** Efforts the current model advertises. */
  efforts?: AgentEffortLike[];
  /** True when the adapter can set effort for the current model. */
  effortSelection?: boolean;
}

/** One line for the dock header: harness version, optional model, tool count. */
export function agentStatusLine(status: AgentStatusLike): string {
  const name = status.backend === "opencode" ? "OpenCode" : "Codex";
  const probe = status.probes.find((item) => item.capability === status.backend);
  const tools = `${status.tools.length} 个 Mathscape 工具`;
  if (status.available) {
    const version = probe?.found_version ? ` ${probe.found_version}` : "";
    // When the dock shows a model <select>, keep the status line free of the id.
    const model =
      status.modelSelection || !status.model?.trim() ? "" : ` · ${status.model.trim()}`;
    return `本机 ${name}${version}${model} · ${tools}`;
  }
  const hint = probe?.install_hint ? `（${probe.install_hint}）` : "";
  return `未检测到可用的本机 ${name}：${status.program ?? status.codexProgram}${hint}`;
}

/** Label for a model option: display name when distinct, else id. */
export function modelOptionLabel(model: AgentModelLike): string {
  const name = model.displayName?.trim();
  if (name && name !== model.id) return `${name} (${model.id})`;
  return model.id;
}

/**
 * Explanation shown under the model picker when Codex exposed no alternatives,
 * so the picker never looks like a dead dropdown. `null` when there is a choice.
 */
export function modelCatalogHint(models: AgentModelLike[], current: string, backend = "codex"): string | null {
  if (models.length >= 2) return null;
  if (backend === "opencode") {
    return models.length === 0
      ? "未读到 OpenCode 模型目录，请检查 OpenCode 配置。"
      : `仅显示当前 OpenCode 模型 ${current || models[0].id}。`;
  }
  const where = "config.toml 的 model_catalog_json、~/.codex/models_cache.json 与 codex app-server model/list";
  if (models.length === 0) return `未读到 Codex 模型目录（${where}），也未设置默认模型。`;
  const only = current || models[0].id;
  return `未读到 Codex 的其他可选模型（${where}），仅显示当前模型 ${only}。在终端运行一次 codex 刷新目录后重开对话。`;
}

const EFFORT_ZH: Record<string, string> = {
  none: "无",
  minimal: "最低",
  low: "低",
  medium: "中",
  high: "高",
  xhigh: "超高",
  max: "最大",
  ultra: "极限",
};

/** Label for a reasoning-effort option: `high · 高`; unknown levels show the raw id. */
export function effortOptionLabel(effort: AgentEffortLike, defaultEffort?: string | null): string {
  const zh = EFFORT_ZH[effort.id.toLowerCase()];
  const base = zh ? `${effort.id} · ${zh}` : effort.id;
  return defaultEffort && defaultEffort === effort.id ? `${base}（默认）` : base;
}

/**
 * Effort options for the dock: the current model's advertised levels, plus the
 * current effort first when the catalog does not list it (never drop the
 * value the next round will actually use).
 */
export function effortOptions(
  efforts: AgentEffortLike[] | undefined,
  current: string | null | undefined,
): AgentEffortLike[] {
  const out = [...(efforts ?? [])];
  const id = current?.trim();
  if (id && !out.some((effort) => effort.id === id)) out.unshift({ id });
  return out;
}
