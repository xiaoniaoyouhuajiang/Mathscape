/**
 * Tool events → `SceneDraft` → `ToolScene` (Map Scene Player, first slice).
 *
 * Pure: no DOM, no Cytoscape. Each of the eight tools is compiled from its
 * own argument/result fields (not a blind id walk), so seeds and result
 * layers mean what the tool meant: a `graph.neighborhood` result lights
 * depth by depth, a grep lights only the materials that matched, and so on.
 *
 * Second slice: four tools also carry an Act overlay (`SceneDraft.act`):
 * `workspace.search` types its query in the HUD, `pdf.outline` opens a 目录
 * mini-window, `ink.page_image` / `pdf.page_image` open a page-image mini-window.
 *
 * `material.read` on scanned pages blocks while Marker parses them: a live
 * call gets a `parsing` act via `parsingScene` (the map said 「解析中」), and
 * a result carrying `details.map_signal` records the parse (done / failed).
 */
import { shortStableId } from "../../core/stable-id";
import type { AgentEventLike } from "../format";
import { MCP_STARTUP_TOOL } from "../steps";
import {
  OUTLINE_ROWS_CAP,
  SCENE_FOCUS_CAP,
  SCENE_TOOLS,
  PARSING_LABEL,
  type OutlineRow,
  type ParsingAct,
  type SceneAct,
  type SceneDraft,
  type SceneResolve,
  type SceneStatus,
  type SceneToolName,
  type ToolScene,
} from "./types";

type Json = Record<string, unknown>;

const isObject = (value: unknown): value is Json =>
  typeof value === "object" && value !== null && !Array.isArray(value);
const str = (value: unknown): string | undefined =>
  typeof value === "string" && value.trim() ? value.trim() : undefined;
const num = (value: unknown): number | undefined =>
  typeof value === "number" && Number.isFinite(value) ? value : undefined;
const list = (value: unknown): unknown[] => (Array.isArray(value) ? value : []);
const ids = (value: unknown): string[] => list(value).map(str).filter((id): id is string => !!id);

function parseJson(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return undefined;
  }
}

/** `mathscape.graph.get`, `mcp__mathscape__graph_get`, … → `graph.get`. */
export function sceneToolName(tool: unknown): SceneToolName | null {
  const name = str(tool);
  if (!name || name === MCP_STARTUP_TOOL) return null;
  for (const bare of SCENE_TOOLS) {
    if (name === bare || name.endsWith(`.${bare}`) || name.endsWith(`/${bare}`)) return bare;
    if (name.endsWith(`__${bare.replace(".", "_")}`)) return bare;
  }
  return null;
}

function rawOf(event: AgentEventLike | undefined): Json | undefined {
  const raw = event?.payload?.raw;
  return isObject(raw) ? raw : undefined;
}

/** `k=v k2=v2` (adapter `args_summary`, keys sorted) → object; best effort. */
export function parseArgsSummary(summary: string): Json {
  const out: Json = {};
  for (const part of summary.split(/\s+(?=[a-z_]+=)/)) {
    const at = part.indexOf("=");
    if (at <= 0) continue;
    const key = part.slice(0, at);
    const value = part.slice(at + 1);
    const parsed = /^[[{]|^-?\d/.test(value) ? parseJson(value) : undefined;
    out[key] = parsed === undefined ? value : parsed;
  }
  return out;
}

/** Tool arguments: the raw Codex item's `arguments`, else `args_summary`. */
export function callArgs(call: AgentEventLike | undefined): Json {
  const raw = rawOf(call)?.arguments;
  if (isObject(raw)) return raw;
  if (typeof raw === "string") {
    const parsed = parseJson(raw);
    if (isObject(parsed)) return parsed;
  }
  const summary = call?.payload?.args_summary;
  return typeof summary === "string" ? parseArgsSummary(summary) : {};
}

/** Tool payload: `structuredContent`, else the JSON text block of `content`. */
export function resultPayload(result: AgentEventLike | undefined): Json | undefined {
  const raw = rawOf(result);
  let value: unknown = raw?.result;
  if (typeof value === "string") value = parseJson(value);
  if (!isObject(value)) return undefined;
  if (isObject(value.structuredContent)) return value.structuredContent;
  for (const block of list(value.content)) {
    if (isObject(block) && block.type === "text" && typeof block.text === "string") {
      const parsed = parseJson(block.text);
      if (isObject(parsed)) return parsed;
    }
  }
  return "status" in value ? value : undefined;
}

const IMAGE_MIME = /^image\/(png|jpeg|webp|gif)$/;
const BASE64 = /^[A-Za-z0-9+/=\s]+$/;

/** The result's image block as a `data:` URL (ink/pdf.page_image), if any. */
export function resultImage(result: AgentEventLike | undefined): string | undefined {
  let value: unknown = rawOf(result)?.result;
  if (typeof value === "string") value = parseJson(value);
  if (!isObject(value)) return undefined;
  for (const block of list(value.content)) {
    if (!isObject(block) || block.type !== "image") continue;
    const data = str(block.data);
    const mime = str(block.mimeType) ?? str(block.mime_type) ?? "image/png";
    if (data && IMAGE_MIME.test(mime) && BASE64.test(data)) return `data:${mime};base64,${data.replace(/\s+/g, "")}`;
  }
  return undefined;
}

function resultStatus(result: AgentEventLike | undefined, payload: Json | undefined): SceneStatus {
  if (!result) return "running";
  const status = str(result.payload?.status);
  if (rawOf(result)?.error) return "error";
  // `truncated` is a successful, capped result (contract §2.2), not a failure.
  if (status && !["ok", "completed", "success", "truncated"].includes(status)) return "error";
  const inner = str(payload?.status);
  if (inner && inner !== "ok" && inner !== "truncated") return "error";
  return "ok";
}

const DIRECTION: Record<string, string> = { both: "双向", in: "入边", out: "出边" };

function locatorLabel(locator: unknown): string | undefined {
  if (!isObject(locator)) return undefined;
  const pages = locator.pdf_pages;
  if (isObject(pages)) {
    const from = num(pages.from);
    const to = num(pages.to);
    if (from !== undefined) return to !== undefined && to !== from ? `第 ${from}–${to} 页` : `第 ${from} 页`;
  }
  const range = locator.md_byte_range;
  if (isObject(range)) {
    const start = num(range.start);
    const end = num(range.end);
    if (start !== undefined && end !== undefined) return `字节 ${start}–${end}`;
  }
  return undefined;
}

function errorSummary(result: AgentEventLike | undefined, payload: Json | undefined): string {
  const message = str(payload?.message) ?? str(payload?.error) ?? str(payload?.status);
  const raw = rawOf(result)?.error;
  const rawMessage = isObject(raw) ? str(raw.message) : str(raw);
  return `调用失败${rawMessage ?? message ? `：${rawMessage ?? message}` : ""}`;
}

interface Parts {
  verb: string;
  subject?: string;
  subjectId?: string;
  detail?: string;
  summary: string;
  seeds: string[];
  layers: string[][];
}

type Compiler = (args: Json, payload: Json | undefined, status: SceneStatus) => Parts;

/** What a search hit points at on the map. */
function searchTarget(hit: Json): string | undefined {
  const target = isObject(hit.target) ? hit.target : undefined;
  if (!target) return str(hit.id);
  return (
    str(target.node_id) ??
    str(target.object_id) ??
    str(target.artifact_id) ??
    str(target.snapshot_id) ??
    str(target.document_id) ??
    str(hit.id)
  );
}

const COMPILERS: Record<SceneToolName, Compiler> = {
  "workspace.search": (args, payload, status) => {
    const hits = list(payload?.groups).flatMap((group) => (isObject(group) ? list(group.hits) : [])).filter(isObject);
    const total = num(payload?.total) ?? hits.length;
    return {
      verb: "搜索",
      subject: str(args.query) ?? str(payload?.query) ?? "（空查询）",
      summary: status === "running" ? "正在搜索工作区…" : `命中 ${total} 条`,
      seeds: [],
      layers: [hits.map(searchTarget).filter((id): id is string => !!id)],
    };
  },
  "material.read": (args, payload, status) => {
    const artifact = str(args.artifact_id) ?? str(payload?.artifact_id);
    const where = locatorLabel(payload?.locator) ?? locatorLabel(args.locator);
    const pages = num(payload?.total_pages);
    const bytes = num(payload?.total_bytes);
    const size = pages !== undefined ? `共 ${pages} 页` : bytes !== undefined ? `共 ${bytes} 字节` : "";
    const parsed = parsingSignal(payload);
    const done = [
      parsed?.phase === "done" ? `解析完成${pagesLabel(parsed.pages)}` : "",
      str(payload?.content_type),
      size,
      payload?.truncated ? "已截断" : "",
    ]
      .filter(Boolean)
      .join(" · ");
    return {
      verb: "阅读材料",
      subjectId: artifact,
      detail: where,
      summary: status === "running" ? "正在读取原文…" : done || "已读取",
      seeds: artifact ? [artifact] : [],
      layers: [],
    };
  },
  "material.grep": (args, payload, status) => {
    const hits = list(payload?.hits).filter(isObject);
    const matched = [...new Set(hits.map((hit) => str(hit.artifact_id)).filter((id): id is string => !!id))];
    const seeds = ids(args.artifact_ids);
    return {
      verb: "材料检索",
      subject: str(args.pattern) ?? "（空模式）",
      summary:
        status === "running"
          ? `正在 ${seeds.length || "全部"} 份材料中检索…`
          : `${matched.length} 份材料命中 ${hits.length} 处${payload?.truncated ? "（已截断）" : ""}`,
      seeds,
      layers: [matched],
    };
  },
  "pdf.outline": (args, payload, status) => {
    const artifact = str(args.artifact_id) ?? str(payload?.artifact_id);
    const entries = num(payload?.total_entries) ?? list(payload?.entries).length;
    const pages = num(payload?.total_pages);
    return {
      verb: "PDF 目录",
      subjectId: artifact,
      summary: status === "running" ? "正在读取目录…" : `${entries} 条目录${pages !== undefined ? ` · ${pages} 页` : ""}`,
      seeds: artifact ? [artifact] : [],
      layers: [],
    };
  },
  "graph.get": (args, payload, status) => {
    const node = isObject(payload?.node) ? payload.node : undefined;
    const asked = str(args.node_id);
    const found = node ? str(node.canonical_id) ?? str(node.id) : undefined;
    const kind = node ? str(node.entity_kind) ?? str(node.kind) : undefined;
    return {
      verb: "查看节点",
      subject: node ? str(node.label) : undefined,
      subjectId: asked ?? found,
      summary:
        status === "running" ? "正在查看节点…" : node ? [kind, str(node.label)].filter(Boolean).join(" · ") || "已找到" : "节点不存在",
      seeds: asked ? [asked] : [],
      layers: found ? [[found, ...(node && str(node.id) !== found ? [str(node.id)!] : [])]] : [],
    };
  },
  "graph.neighborhood": (args, payload, status) => {
    const applied = isObject(payload?.applied) ? payload.applied : {};
    const depth = num(applied.depth) ?? num(args.depth) ?? 1;
    const direction = str(applied.direction) ?? str(args.direction) ?? "both";
    const seeds = [
      ...new Set([...ids(payload?.seed_ids), ...ids(args.seed_ids), ...(str(args.node_id) ? [str(args.node_id)!] : [])]),
    ];
    const nodes = list(payload?.nodes).filter(isObject);
    const byDepth = new Map<number, string[]>();
    for (const node of nodes) {
      const id = str(node.id);
      if (!id) continue;
      const level = num(node.depth) ?? 1;
      if (level <= 0) {
        if (!seeds.includes(id)) seeds.push(id);
        continue;
      }
      const layer = byDepth.get(level) ?? [];
      layer.push(id);
      byDepth.set(level, layer);
    }
    const layers = [...byDepth.entries()].sort(([a], [b]) => a - b).map(([, layer]) => layer);
    const edges = list(payload?.edges).length;
    return {
      verb: "邻域",
      subjectId: seeds.length === 1 ? seeds[0] : undefined,
      subject: seeds.length > 1 ? `${seeds.length} 个种子` : undefined,
      detail: `深度 ${depth} · ${DIRECTION[direction] ?? direction}`,
      summary: status === "running" ? "正在展开邻域…" : `${nodes.length} 个节点 · ${edges} 条边 · ${layers.length} 层`,
      seeds,
      layers,
    };
  },
  "ink.page_image": (args, payload, status) => {
    const source = isObject(payload?.source) ? payload.source : {};
    const page = isObject(payload?.page) ? payload.page : {};
    const index = num(page.index) ?? num(args.page_index);
    const count = num(payload?.page_count);
    const focus = [str(source.artifact_id), str(source.snapshot_id), str(source.node_id), str(args.node_id)].filter(
      (id): id is string => !!id,
    );
    return {
      verb: "手稿页图",
      subject: str(source.note_title) ?? str(source.label),
      subjectId: str(args.node_id) ?? str(source.node_id),
      detail: index !== undefined ? `第 ${index + 1}${count !== undefined ? `/${count}` : ""} 页` : undefined,
      summary: status === "running" ? "正在渲染手稿页…" : "已取得页图",
      seeds: [...new Set(focus)],
      layers: [],
    };
  },
  "pdf.page_image": (args, payload, status) => {
    const artifactId = str(args.artifact_id) ?? str(payload?.artifact_id);
    const page = num(payload?.page) ?? num(args.page);
    const count = num(payload?.page_count);
    const crop = isObject(args.crop) ? args.crop : isObject(payload?.crop) ? payload.crop : undefined;
    const cropDetail =
      crop &&
      ["left", "top", "right", "bottom"].every((key) => typeof crop[key] === "number")
        ? ` · 裁剪`
        : "";
    return {
      verb: "PDF 页图",
      subjectId: artifactId,
      detail:
        page !== undefined
          ? `第 ${page}${count !== undefined ? `/${count}` : ""} 页${cropDetail}`
          : cropDetail || undefined,
      summary: status === "running" ? "正在渲染 PDF 页…" : "已取得页图",
      seeds: artifactId ? [artifactId] : [],
      layers: [],
    };
  },
};

type ActCompiler = (args: Json, payload: Json | undefined, status: SceneStatus, result: AgentEventLike | undefined) => SceneAct | undefined;

function outlineRows(entries: unknown[]): OutlineRow[] {
  const rows: OutlineRow[] = [];
  for (const entry of entries) {
    if (!isObject(entry)) continue;
    const title = str(entry.title);
    if (!title) continue;
    const index = num(entry.pdf_page_index);
    rows.push({
      title,
      level: Math.max(1, Math.min(8, Math.round(num(entry.level) ?? 1))),
      ...(index !== undefined && index >= 0 ? { page: index + 1 } : {}),
    });
    if (rows.length >= OUTLINE_ROWS_CAP) break;
  }
  return rows;
}

const hasDiagnostic = (payload: Json | undefined, code: string): boolean =>
  list(payload?.diagnostics).some((item) => isObject(item) && item.code === code);

const pagesLabel = (pages: ParsingAct["pages"]): string =>
  pages ? (pages.from === pages.to ? ` 第${pages.from}页` : ` 第${pages.from}–${pages.to}页`) : "";

/** `material.read` result `details.map_signal` (sync ensure record), if any. */
function parsingSignal(payload: Json | undefined): ParsingAct | undefined {
  const details = isObject(payload?.details) ? payload.details : undefined;
  const signal = isObject(details?.map_signal) ? details.map_signal : undefined;
  if (!signal || signal.kind !== "parsing") return undefined;
  const range = isObject(signal.pages) ? signal.pages : undefined;
  const from = num(range?.from);
  const to = num(range?.to) ?? from;
  return {
    kind: "parsing",
    label: PARSING_LABEL,
    phase: signal.phase === "failed" ? "failed" : "done",
    ...(from !== undefined && to !== undefined ? { pages: { from, to } } : {}),
  };
}

/** `解析失败 第a–b页：{ensure message}` (falls back to the generic error). */
function parseFailure(act: ParsingAct, payload: Json | undefined, fallback: string): string {
  const details = isObject(payload?.details) ? payload.details : undefined;
  const ensure = isObject(details?.ensure) ? details.ensure : undefined;
  const reason = str(ensure?.message) ?? fallback.replace(/^调用失败：?/, "");
  return `解析失败${pagesLabel(act.pages)}${reason ? `：${reason}` : ""}`;
}

/** Requested `pdf_pages` of a `material.read` call (1-based), if any. */
export function readPdfPages(call: AgentEventLike | undefined): { from: number; to: number } | undefined {
  if (sceneToolName(call?.payload?.tool ?? rawOf(call)?.tool) !== "material.read") return undefined;
  const locator = callArgs(call).locator;
  const pages = isObject(locator) && isObject(locator.pdf_pages) ? locator.pdf_pages : undefined;
  const from = num(pages?.from);
  if (from === undefined) return undefined;
  return { from, to: num(pages?.to) ?? from };
}

/** A map summary that says the PDF is being parsed right now. */
export const isParsingSummary = (summary: string | undefined): boolean => !!summary?.includes(PARSING_LABEL);

/**
 * A still-running `material.read` draft as a parsing block: the map showed
 * 「解析中」 for its PDF, so the call is waiting on Marker (sync ensure).
 */
export function parsingScene(draft: SceneDraft, pages?: { from: number; to: number }): SceneDraft {
  return {
    ...draft,
    summary: `${PARSING_LABEL}${pagesLabel(pages)} · 等待 Marker 完成…`,
    act: { kind: "parsing", label: PARSING_LABEL, phase: "running", ...(pages ? { pages } : {}) },
  };
}

/** Act overlays: only these tools have one (others stay unchanged). */
const ACTS: Partial<Record<SceneToolName, ActCompiler>> = {
  "material.read": (_args, payload, status) => (status === "running" ? undefined : parsingSignal(payload)),
  "workspace.search": (args, payload) => {
    const text = str(args.query) ?? str(payload?.query);
    return text ? { kind: "typewriter", text } : undefined;
  },
  "pdf.outline": (_args, payload, status) => {
    if (status !== "ok") return undefined;
    const entries = list(payload?.entries);
    const rows = outlineRows(entries);
    const total = num(payload?.total_entries) ?? entries.length;
    const truncated = payload?.truncated === true || str(payload?.status) === "truncated" || total > rows.length;
    return {
      kind: "outline",
      rows,
      total,
      truncated: rows.length > 0 && truncated,
      ...(rows.length === 0 ? { empty: hasDiagnostic(payload, "outline_unreadable") ? "unreadable" as const : "none" as const } : {}),
      ...(num(payload?.total_pages) !== undefined ? { totalPages: num(payload?.total_pages) } : {}),
    };
  },
  "ink.page_image": (args, payload, status, result) => {
    if (status !== "ok") return undefined;
    const source = isObject(payload?.source) ? payload.source : {};
    const page = isObject(payload?.page) ? payload.page : {};
    const image = isObject(payload?.image) ? payload.image : {};
    const index = num(page.index) ?? num(args.page_index);
    const count = num(payload?.page_count);
    const src = resultImage(result);
    return {
      kind: "page_image",
      ...(src ? { src } : {}),
      noteTitle: str(source.note_title) ?? str(source.label),
      pageLabel: index !== undefined ? `第 ${index + 1}${count !== undefined ? `/${count}` : ""} 页` : undefined,
      width: num(image.width),
      height: num(image.height),
    };
  },
  "pdf.page_image": (args, payload, status, result) => {
    if (status !== "ok") return undefined;
    const image = isObject(payload?.image) ? payload.image : {};
    const page = num(payload?.page) ?? num(args.page);
    const count = num(payload?.page_count);
    const src = resultImage(result);
    return {
      kind: "page_image",
      ...(src ? { src } : {}),
      noteTitle: "PDF 页图",
      pageLabel: page !== undefined ? `第 ${page}${count !== undefined ? `/${count}` : ""} 页` : undefined,
      width: num(image.width),
      height: num(image.height),
    };
  },
};

/** One tool step's Act overlay (exported for tests). */
export function compileSceneAct(call: AgentEventLike | undefined, result?: AgentEventLike): SceneAct | undefined {
  const tool = sceneToolName(call?.payload?.tool ?? rawOf(call)?.tool ?? rawOf(result)?.tool);
  const act = tool ? ACTS[tool] : undefined;
  if (!act) return undefined;
  const payload = resultPayload(result);
  return act(callArgs(call ?? result), payload, resultStatus(result, payload), result);
}

/** One tool step (call, plus its result once it arrived) → draft. */
export function compileToolScene(call: AgentEventLike | undefined, result?: AgentEventLike): SceneDraft | null {
  const event = call ?? result;
  const tool = sceneToolName(call?.payload?.tool ?? rawOf(call)?.tool ?? rawOf(result)?.tool);
  const callId = str(event?.payload?.call_id);
  if (!tool || !callId) return null;
  const args = callArgs(call ?? result);
  const payload = resultPayload(result);
  const status = resultStatus(result, payload);
  const parts = COMPILERS[tool](args, payload, status);
  const act = ACTS[tool]?.(args, payload, status, result);
  if ((tool === "ink.page_image" || tool === "pdf.page_image") && act?.kind === "page_image" && !act.src && status === "ok") {
    parts.summary = "仅元数据（无页图）";
  }
  return {
    callId,
    roundId: event?.round_id,
    tool,
    status,
    ...parts,
    summary:
      status === "error"
        ? act?.kind === "parsing" && act.phase === "failed"
          ? parseFailure(act, payload, errorSummary(result, payload))
          : errorSummary(result, payload)
        : parts.summary,
    ...(act ? { act } : {}),
  };
}

/** Every tool step of a round's events, in call order. */
export function compileRoundScenes(events: readonly AgentEventLike[]): SceneDraft[] {
  const calls = new Map<string, AgentEventLike>();
  const results = new Map<string, AgentEventLike>();
  const order: string[] = [];
  for (const event of events) {
    const id = str(event.payload?.call_id);
    if (!id) continue;
    if (event.type === "tool_call" && !calls.has(id)) {
      calls.set(id, event);
      order.push(id);
    } else if (event.type === "tool_result") {
      results.set(id, event);
    }
  }
  return order
    .map((id) => compileToolScene(calls.get(id), results.get(id)))
    .filter((draft): draft is SceneDraft => draft !== null);
}

/** Resolve a draft's ids on the map: unique, seeds first, capped. */
export function resolveScene(draft: SceneDraft, resolve: SceneResolve, cap = SCENE_FOCUS_CAP): ToolScene {
  // `focus` caps the scene; `lit` keeps a node in one layer only. A seed may
  // reappear in a layer: the result confirmed it (seed → hit).
  const focus = new Set<string>();
  const lit = new Set<string>();
  let offMap = 0;
  let capped = false;
  const take = (raw: readonly string[], once: Set<string>): string[] => {
    const out: string[] = [];
    for (const id of raw) {
      const target = resolve(id);
      if (!target) {
        offMap += 1;
        continue;
      }
      if (once.has(target.nodeId)) continue;
      if (!focus.has(target.nodeId) && focus.size >= cap) {
        capped = true;
        continue;
      }
      focus.add(target.nodeId);
      once.add(target.nodeId);
      out.push(target.nodeId);
    }
    return out;
  };
  const seeds = take(draft.seeds, new Set());
  const layers = draft.layers.map((layer) => take(layer, lit)).filter((layer) => layer.length > 0);
  const subjectTarget = draft.subjectId ? resolve(draft.subjectId) : null;
  const subject =
    draft.subject ?? subjectTarget?.label ?? (draft.subjectId ? shortStableId(draft.subjectId) : undefined);
  const title = [draft.verb, subject, draft.detail].filter(Boolean).join(" · ");
  const named = draft.seeds.length + draft.layers.reduce((sum, layer) => sum + layer.length, 0);
  const shown = focus.size;
  let summary = draft.summary;
  if (draft.status !== "running" || shown > 0) {
    if (shown > 0) summary += ` · 地图上 ${shown} 个${capped ? `（上限 ${cap}）` : ""}`;
    else if (named > 0 && draft.status !== "error") summary += " · 不在当前地图上";
  }
  return {
    callId: draft.callId,
    roundId: draft.roundId,
    tool: draft.tool,
    status: draft.status,
    title,
    summary,
    seeds,
    layers,
    offMap,
    capped,
    ...(draft.act ? { act: draft.act } : {}),
    ...(seeds[0] ? { anchor: seeds[0] } : {}),
  };
}

/** A scene with something to light on the map (else HUD-only). */
export function sceneHasFocus(scene: ToolScene): boolean {
  return scene.seeds.length > 0 || scene.layers.length > 0;
}
