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
import { t } from "../../i18n";
import { relationLabel, unitKindLabel } from "../../review/labels";
import type { AgentEventLike } from "../format";
import { MCP_STARTUP_TOOL } from "../steps";
import {
  LEAN_CODE_LINES,
  LEAN_DIAGNOSTICS_CAP,
  MATHLIB_ROWS_CAP,
  MATHLIB_TYPE_MAX,
  OUTLINE_ROWS_CAP,
  SCENE_FOCUS_CAP,
  SCENE_TOOLS,
  PARSING_LABEL,
  type LeanAct,
  type LeanDiagnostic,
  type LocateAct,
  type LocateDiagnostic,
  type MathlibAct,
  type OutlineRow,
  type ParsingAct,
  type ProposeAct,
  type ProposeBlock,
  type ProposeEnd,
  type ProposeItem,
  type ProposeRelation,
  type ProposeRelationKind,
  type ProposeVisualKind,
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

/**
 * Tool statuses that are an answer, not a failure: a blocked proposal is a
 * validated verdict (its diagnostics are the content), and a locate that
 * found several / no matches says so on purpose.
 */
const ANSWER_STATUSES: Partial<Record<SceneToolName, readonly string[]>> = {
  propose_changes: ["staged", "blocked"],
  "material.locate": ["ambiguous", "not_found"],
};

function resultStatus(result: AgentEventLike | undefined, payload: Json | undefined, tool?: SceneToolName | null): SceneStatus {
  if (!result) return "running";
  const status = str(result.payload?.status);
  if (rawOf(result)?.error) return "error";
  // `truncated` is a successful, capped result (contract §2.2), not a failure.
  const ok = ["ok", "truncated", ...((tool && ANSWER_STATUSES[tool]) ?? [])];
  if (status && ![...ok, "completed", "success"].includes(status)) return "error";
  const inner = str(payload?.status);
  if (inner && !ok.includes(inner)) return "error";
  if (isObject(payload?.error)) return "error";
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

/** The tool's own error text: `{error:{message}}`, a capability reason, … */
function payloadError(payload: Json | undefined): string | undefined {
  const error = isObject(payload?.error) ? payload.error : undefined;
  const details = isObject(payload?.details) ? payload.details : undefined;
  const first = list(payload?.diagnostics).find(isObject);
  return (
    str(error?.message) ??
    str(payload?.message) ??
    str(payload?.error) ??
    str(details?.reason) ??
    (first ? str(first.message) : undefined)
  );
}

function errorSummary(result: AgentEventLike | undefined, payload: Json | undefined): string {
  const message = payloadError(payload) ?? str(payload?.status);
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
  propose_changes: (args, payload, status) => {
    const counts = stagedCounts(payload?.staged_ids);
    const semantic = counts.semantic;
    const version = num(payload?.version);
    const supersedes = supersedesOf(args, payload);
    const blocks = proposalBlocks(payload);
    const units = list(args.proposed_objects).filter((item) => isObject(item) && item.kind === "source_unit").length;
    const semanticArgs = list(args.proposed_objects).filter((item) => isObject(item) && typeof item.kind === "string" && ["semantic_entity", "alignment", "debt", "dialect_scope"].includes(item.kind)).length;
    const relations = list(args.proposed_relations).length;
    const stagedTail = semantic > 0 && counts.units === 0 && counts.relations === 0 && counts.spans === 0
      ? `语义项 ${semantic}`
      : `单元 ${counts.units} · 关系 ${counts.relations} · 片段 ${counts.spans}${semantic > 0 ? ` · 语义 ${semantic}` : ""}`;
    const runningSummary = semanticArgs > 0 && units === 0 && relations === 0
      ? `正在校验 ${semanticArgs} 个语义项…`
      : `正在校验 ${units} 个单元 · ${relations} 条关系${semanticArgs > 0 ? ` · ${semanticArgs} 个语义项` : ""}…`;
    const verdict =
      str(payload?.status) === "blocked"
        ? `被阻塞 · ${blocks.length} 个问题${blocks[0] ? `：${blocks[0].code}` : ""}`
        : `已暂存 · ${stagedTail}`;
    return {
      verb: "提案",
      subject: str(args.request_id),
      detail: [version !== undefined ? `v${version}` : "", supersedes ? `取代 v${supersedes.version}` : ""].filter(Boolean).join(" · ") || undefined,
      summary: status === "running" ? runningSummary : verdict,
      seeds: proposalAnchors(args),
      layers: [],
    };
  },
  "mathlib.search": (args, payload, status) => {
    const candidates = list(payload?.candidates).filter(isObject);
    const total = num(payload?.total) ?? candidates.length;
    const mode = backendMode(payload);
    return {
      verb: "Mathlib 检索",
      subject: str(args.query) ?? "（空查询）",
      summary:
        status === "running"
          ? "Loogle 检索中…"
          : candidates.length === 0
            ? "没有匹配的声明"
            : `${total} 个候选${payload?.truncated ? "（已截断）" : ""}${mode ? ` · ${MODE_LABEL[mode]}` : ""}`,
      seeds: [],
      layers: [],
    };
  },
  "lean.check": (args, payload, status) => {
    const check = isObject(payload?.check) ? payload.check : undefined;
    const diagnostics = list(check?.diagnostics).filter(isObject);
    const errors = diagnostics.filter((item) => item.severity === "error").length;
    const verdict = str(check?.status);
    return {
      verb: "Lean 检查",
      subject: leanSubject(str(args.source_text)),
      summary:
        status === "running"
          ? "Lean 编译中…"
          : verdict === "checked" && errors === 0
            ? "✓ 编译通过（编译通过 ≠ 证明忠实）"
            : `未通过 · ${diagnostics.length} 条诊断${errors ? `（${errors} 个错误）` : ""}`,
      seeds: [],
      layers: [],
    };
  },
  "material.locate": (args, payload, status) => {
    const artifact = str(args.artifact_id) ?? str(payload?.artifact_id);
    const page = locatePage(payload);
    const inner = str(payload?.status);
    const count = num(payload?.candidate_count) ?? list(payload?.candidates).length;
    return {
      verb: "定位原文",
      subjectId: artifact,
      detail: page !== undefined ? `第 ${page} 页` : undefined,
      summary:
        status === "running"
          ? "正在原文中定位引文…"
          : inner === "ambiguous"
            ? `${count} 处候选，未唯一定位`
            : inner === "not_found"
              ? `未找到：${payloadError(payload) ?? "引文与原文不一致"}`
              : `${MATCH_LABEL[str(payload?.match) ?? ""] ?? t("scene.locate.located")} · ${page !== undefined ? t("scene.locate.page", { page }) : t("scene.locate.pageUnknown")}`,
      seeds: artifact ? [artifact] : [],
      layers: [],
    };
  },
  list_change_sets: (args, payload, status) => {
    const sets = list(payload?.change_sets).filter(isObject);
    const total = num(payload?.total) ?? sets.length;
    const first = sets[0];
    const title = first ? str(first.title) : undefined;
    return {
      verb: "变更集",
      subject: args.pending_only === true ? "待审" : undefined,
      summary:
        status === "running"
          ? "正在列出变更集…"
          : total === 0
            ? "没有变更集"
            : `${total} 个变更集${title ? `：${clip(title, 40)}` : ""}`,
      seeds: [],
      layers: [],
    };
  },
  get_change_set: (args, payload, status) => {
    const staged = isObject(payload?.staged_ids) ? payload.staged_ids : {};
    const counts = stagedCounts(staged);
    const version = num(payload?.version) ?? num(args.version);
    const state = str(payload?.state);
    return {
      verb: "查看变更集",
      subject: shortChangeSet(str(args.change_set_id) ?? str(payload?.change_set_id)),
      detail: version !== undefined ? `v${version}` : undefined,
      summary:
        status === "running"
          ? "正在读取变更集…"
          : `${STATE_LABEL[state ?? ""] ?? state ?? "已读取"} · 单元 ${counts.units} · 关系 ${counts.relations} · 片段 ${counts.spans}${counts.semantic > 0 ? ` · 语义 ${counts.semantic}` : ""}`,
      seeds: [],
      // Briefly light the change set's staged units and semantic nodes (relations / spans are not nodes).
      layers: [Object.values(staged).map(str).filter((id): id is string => !!id && STAGED_NODE_PREFIXES.some((prefix) => id.startsWith(prefix)))],
    };
  },
};

// ---------------------------------------------------------------------------
// Fourth slice helpers (proposal / Lean / locate)

const UNIT_PREFIX = "mathscape:source-unit/";
const SEMANTIC_ENTITY_PREFIX = "mathscape:semantic-entity/";
const DEBT_PREFIX = "mathscape:debt/";
const DIALECT_SCOPE_PREFIX = "mathscape:dialect-scope/";
const ALIGNMENT_PREFIX = "mathscape:alignment/";
const STAGED_NODE_PREFIXES = [UNIT_PREFIX, SEMANTIC_ENTITY_PREFIX, DEBT_PREFIX, DIALECT_SCOPE_PREFIX];
const RELATION_PREFIX = "mathscape:source-relation/";
const SPAN_PREFIX = "mathscape:source-span/";
const MODE_LABEL = { resident: "常驻", oneshot: "一次性" } as const;
const MATCH_LABEL: Record<string, string> = { exact: "精确匹配", normalized: "规范化后匹配" };
const STATE_LABEL: Record<string, string> = { staged: "待审", blocked: "被阻塞", accepted: "已接受", discarded: "已丢弃", superseded: "已被取代" };

const clip = (text: string, max: number): string => ([...text].length > max ? `${[...text].slice(0, max - 1).join("")}…` : text);

const shortChangeSet = (id: string | undefined): string | undefined =>
  id ? `change-set/${shortStableId(id.replace(/^mathscape:change-set\//, ""), { max: 12, head: 8, tail: 0 })}` : undefined;

const semanticPrefixes = [SEMANTIC_ENTITY_PREFIX, ALIGNMENT_PREFIX, DEBT_PREFIX, DIALECT_SCOPE_PREFIX];

export function stagedCounts(staged: unknown): ProposeAct["counts"] {
  const values = isObject(staged) ? Object.values(staged).map(str) : [];
  const count = (prefix: string) => values.filter((id) => id?.startsWith(prefix)).length;
  return {
    units: count(UNIT_PREFIX),
    relations: count(RELATION_PREFIX),
    spans: count(SPAN_PREFIX),
    semantic: values.filter((id) => !!id && semanticPrefixes.some((prefix) => id.startsWith(prefix))).length,
  };
}

function supersedesOf(args: Json, payload: Json | undefined): ProposeAct["supersedes"] {
  const value = isObject(payload?.supersedes) ? payload.supersedes : isObject(args.supersedes) ? args.supersedes : undefined;
  const changeSetId = str(value?.change_set_id);
  const version = num(value?.version);
  return changeSetId && version !== undefined ? { changeSetId, version } : undefined;
}

/** Error-level diagnostics of a proposal (debts duplicate them). */
function proposalBlocks(payload: Json | undefined): (ProposeBlock & { localRef?: string })[] {
  if (str(payload?.status) !== "blocked") return [];
  return list(payload?.diagnostics)
    .filter(isObject)
    .filter((item) => (str(item.level) ?? "error") === "error")
    .map((item) => ({
      code: str(item.code) ?? "invalid",
      message: str(item.message) ?? "",
      ...(str(item.local_ref) ? { localRef: str(item.local_ref) } : {}),
    }));
}

/** `$new:x` → this proposal's item, `mathscape:…` → an existing object. */
function proposeEnd(value: unknown): ProposeEnd | undefined {
  const ref = str(value);
  if (!ref) return undefined;
  if (ref.startsWith("$new:")) return { local: ref.slice(5) };
  if (ref.startsWith("mathscape:")) return { id: ref };
  return { local: ref };
}

/** Existing objects the proposal touches, including nested semantic references. */
function proposalAnchors(args: Json): string[] {
  const out = new Set<string>();
  const collect = (value: unknown): void => {
    if (typeof value === "string") {
      if (value.startsWith("mathscape:")) out.add(value);
      return;
    }
    if (Array.isArray(value)) {
      value.forEach(collect);
      return;
    }
    if (isObject(value)) Object.values(value).forEach(collect);
  };
  list(args.proposed_objects).filter(isObject).forEach((item) => {
    collect(item.payload);
    collect(item.changes);
    const target = str(item.target_id);
    if (target) out.add(target);
  });
  list(args.proposed_relations).filter(isObject).forEach(collect);
  return [...out];
}

/** Staged ids of the version `supersedes` names, from earlier events. */
function supersededIds(supersedes: ProposeAct["supersedes"], prior: readonly AgentEventLike[]): string[] {
  if (!supersedes) return [];
  for (let at = prior.length - 1; at >= 0; at -= 1) {
    const event = prior[at];
    if (event.type !== "tool_result") continue;
    const payload = resultPayload(event);
    if (!payload || str(payload.change_set_id) !== supersedes.changeSetId || num(payload.version) !== supersedes.version) continue;
    if (!isObject(payload.staged_ids)) continue;
    return Object.values(payload.staged_ids).map(str).filter((id): id is string => !!id);
  }
  return [];
}

function semanticBody(item: Json): Json {
  return isObject(item.payload) ? item.payload : isObject(item.changes) ? item.changes : {};
}

function collectKeyStrings(value: unknown, key: string, out: string[] = []): string[] {
  if (Array.isArray(value)) {
    value.forEach((item) => collectKeyStrings(item, key, out));
    return out;
  }
  if (!isObject(value)) return out;
  for (const [name, child] of Object.entries(value)) {
    if (name === key) {
      if (typeof child === "string") out.push(child);
      else if (Array.isArray(child)) child.forEach((item) => { if (typeof item === "string") out.push(item); });
    } else {
      collectKeyStrings(child, key, out);
    }
  }
  return out;
}

function semanticVisualKind(kind: string, body: Json): ProposeVisualKind {
  if (kind === "debt") return "debt";
  if (kind === "dialect_scope") return "dialect_scope";
  const semanticKind = str(body.semantic_kind);
  if (semanticKind === "theory" || semanticKind === "concept" || semanticKind === "declaration" || semanticKind === "problem_schema") return semanticKind;
  return "concept";
}

function semanticLabel(kind: string, body: Json, local: string): string {
  if (kind === "debt") return str(body.obligation) ?? local;
  if (kind === "dialect_scope") return str(body.label) ?? local;
  const labels = list(body.preferred_labels).filter(isObject);
  const preferred = labels.find((label) => label.lang === "zh") ?? labels[0];
  return str(body.plain_label) ?? str(preferred?.label) ?? unitKindLabel(str(body.semantic_kind) ?? "概念") ?? local;
}

function semanticAnchors(body: Json, spans: Map<string, Json>): string[] {
  const out = new Set<string>();
  for (const raw of collectKeyStrings(body, "evidence_spans")) {
    const end = proposeEnd(raw);
    if (!end) continue;
    if ("id" in end) out.add(end.id);
    else {
      const span = spans.get(end.local);
      const artifact = span && isObject(span.payload) ? str(span.payload.artifact_id) : undefined;
      if (artifact) out.add(artifact);
    }
  }
  for (const key of ["home_theory", "target_context", "anchor_unit", "introduced_by_unit", "attached_to"]) {
    const end = proposeEnd(body[key]);
    if (end && "id" in end) out.add(end.id);
  }
  return [...out];
}

function proposalSelf(local: string, item: Json): ProposeEnd {
  const target = str(item.target_id);
  return item.op !== "create" && target ? { id: target } : { local };
}

function addSemanticRelation(
  relations: ProposeRelation[], item: Json, localRef: string, relationKind: ProposeRelationKind,
  relation: string, source: ProposeEnd | undefined, target: ProposeEnd | undefined,
  stagedId: string | undefined, blocks: (local: string) => ProposeBlock[],
): void {
  if (!source || !target) return;
  relations.push({
    localRef, ...(stagedId ? { stagedId } : {}), relation, relationKind,
    label: relationLabel(relation), source, target,
    blocked: blocks(str(item.local_ref) ?? localRef),
  });
}

function proposeAct(args: Json, payload: Json | undefined, status: SceneStatus, prior: readonly AgentEventLike[]): ProposeAct {
  const staged = isObject(payload?.staged_ids) ? payload.staged_ids : {};
  const stagedOf = (local: string) => str(staged[local]);
  const blocks = proposalBlocks(payload);
  const blocksOf = (local: string) => blocks.filter((block) => block.localRef === local).map(({ code, message }) => ({ code, message }));
  const objects = list(args.proposed_objects).filter(isObject);
  const spans = new Map<string, Json>();
  for (const item of objects) if (item.kind === "source_span" && str(item.local_ref)) spans.set(str(item.local_ref)!, item);
  const owned = new Set<string>();
  const items: ProposeItem[] = [];

  for (const item of objects) {
    const local = str(item.local_ref);
    if (!local || item.kind !== "source_unit") continue;
    const body = semanticBody(item);
    const spanRefs = ids(body.source_spans)
      .map(proposeEnd)
      .filter((end): end is { local: string } => !!end && "local" in end)
      .map((end) => end.local);
    spanRefs.forEach((ref) => owned.add(ref));
    const anchors = new Set<string>();
    for (const ref of spanRefs) {
      const span = spans.get(ref);
      const artifact = span && isObject(span.payload) ? str(span.payload.artifact_id) : undefined;
      if (artifact) anchors.add(artifact);
    }
    const target = str(item.target_id);
    if (target) anchors.add(target);
    items.push({
      localRef: local,
      kind: "unit",
      visualKind: "source_unit",
      ...(stagedOf(local) ? { stagedId: stagedOf(local) } : {}),
      label: str(body.display_title) ?? str(body.title) ?? local,
      anchors: [...anchors],
      blocked: [...blocksOf(local), ...spanRefs.flatMap(blocksOf)],
    });
  }

  for (const item of objects) {
    const local = str(item.local_ref);
    if (!local || !["semantic_entity", "debt", "dialect_scope"].includes(String(item.kind))) continue;
    const body = semanticBody(item);
    const anchors = new Set(semanticAnchors(body, spans));
    const target = str(item.target_id);
    if (target) anchors.add(target);
    items.push({
      localRef: local,
      kind: "semantic",
      visualKind: semanticVisualKind(String(item.kind), body),
      ...(stagedOf(local) ? { stagedId: stagedOf(local) } : {}),
      label: semanticLabel(String(item.kind), body, local),
      anchors: [...anchors],
      blocked: [...blocksOf(local), ...collectKeyStrings(body, "evidence_spans").flatMap(blocksOf)],
    });
  }

  for (const [local, span] of spans) {
    if (owned.has(local) || blocksOf(local).length === 0) continue;
    const artifact = isObject(span.payload) ? str(span.payload.artifact_id) : undefined;
    items.push({ localRef: local, kind: "span", visualKind: "source_span", label: `片段 ${local}`, anchors: artifact ? [artifact] : [], blocked: blocksOf(local) });
  }

  const relations: ProposeRelation[] = [];
  const alignmentTargets = new Map<string, ProposeEnd>();
  for (const item of objects) {
    if (item.kind !== "alignment") continue;
    const local = str(item.local_ref);
    const target = proposeEnd(semanticBody(item).target);
    if (local && target) alignmentTargets.set(local, target);
  }

  for (const relation of list(args.proposed_relations).filter(isObject)) {
    const local = str(relation.local_ref);
    const source = proposeEnd(relation.source);
    const target = proposeEnd(relation.target);
    if (!local || !source || !target) continue;
    const kind = str(relation.relation) ?? "relation";
    relations.push({
      localRef: local,
      ...(stagedOf(local) ? { stagedId: stagedOf(local) } : {}),
      relation: kind,
      relationKind: "source_relation",
      label: relationLabel(kind),
      source,
      target,
      blocked: blocksOf(local),
    });
  }

  for (const item of objects) {
    const local = str(item.local_ref);
    if (!local || !["semantic_entity", "alignment", "debt", "dialect_scope"].includes(String(item.kind))) continue;
    const body = semanticBody(item);
    const self = proposalSelf(local, item);
    const stagedId = stagedOf(local);
    if (item.kind === "alignment") {
      addSemanticRelation(relations, item, local, "alignment", str(body.relation) ?? "expresses", proposeEnd(body.source), proposeEnd(body.target), stagedId, blocksOf);
    } else if (item.kind === "semantic_entity") {
      addSemanticRelation(relations, item, `${local}:in_theory`, "in_theory", "in_theory", self, proposeEnd(body.home_theory), undefined, blocksOf);
      const declaration = isObject(body.declaration) ? body.declaration : undefined;
      const problem = isObject(body.problem_schema) ? body.problem_schema : undefined;
      const uses = [declaration?.concept, ...ids(declaration?.uses_entities), ...ids(problem?.uses_entities)];
      uses.forEach((target, index) => addSemanticRelation(relations, item, `${local}:uses_entity:${index}`, "uses_entity", "uses_entity", self, proposeEnd(target), undefined, blocksOf));
    } else if (item.kind === "debt") {
      const attached = proposeEnd(body.attached_to);
      const holder = attached && "local" in attached ? alignmentTargets.get(attached.local) ?? attached : attached;
      addSemanticRelation(relations, item, `${local}:debt_on`, "debt_on", "debt_on", self, holder, undefined, blocksOf);
    } else {
      addSemanticRelation(relations, item, `${local}:scope_of`, "scope_of", "scope_of", self, proposeEnd(body.anchor_unit), undefined, blocksOf);
    }
  }

  for (const relation of relations) {
    for (const [mine, other] of [[relation.source, relation.target], [relation.target, relation.source]] as const) {
      if (!("local" in mine) || !("id" in other)) continue;
      const item = items.find((entry) => entry.localRef === mine.local);
      if (item && !item.anchors.includes(other.id)) item.anchors.push(other.id);
    }
  }

  const stagedIds = Object.values(staged).map(str).filter((id): id is string => !!id);
  const supersedes = supersedesOf(args, payload);
  const keep = new Set(stagedIds);
  const verdict = str(payload?.status);
  return {
    kind: "propose",
    phase: status === "running" ? "running" : status === "error" ? "error" : verdict === "blocked" ? "blocked" : "staged",
    ...(str(payload?.change_set_id) ? { changeSetId: str(payload?.change_set_id) } : {}),
    ...(num(payload?.version) !== undefined ? { version: num(payload?.version) } : {}),
    items,
    relations,
    counts: stagedCounts(staged),
    stagedIds,
    blocks,
    ...(supersedes ? { supersedes } : {}),
    retired: verdict === "staged" ? supersededIds(supersedes, prior).filter((id) => !keep.has(id)) : [],
  };
}

function backendMode(payload: Json | undefined): MathlibAct["mode"] {
  const mode = str(payload?.backend_mode);
  return mode === "resident" || mode === "oneshot" ? mode : undefined;
}

/** `did you mean: a, b` of a rejected Loogle query (the MCP error text). */
function loogleSuggestions(message: string | undefined): string[] {
  // Names contain dots; the list ends at `. ` / `;` / the end.
  const match = message?.match(/did you mean:\s*(.+?)(?:\.\s|;|\.?$)/i);
  return match ? match[1].split(",").map((item) => item.trim()).filter(Boolean).slice(0, 6) : [];
}

function mathlibAct(args: Json, payload: Json | undefined, status: SceneStatus): MathlibAct {
  const candidates = list(payload?.candidates)
    .filter(isObject)
    .slice(0, MATHLIB_ROWS_CAP)
    .map((item) => {
      const type = str(item.type);
      return {
        name: str(item.name) ?? "（未命名）",
        ...(type ? { type: clip(type, MATHLIB_TYPE_MAX) } : {}),
        ...(str(item.module) ? { module: str(item.module) } : {}),
      };
    });
  const error = status === "error" ? payloadError(payload) : undefined;
  const mode = backendMode(payload);
  const fallback = str(payload?.backend_fallback);
  const total = num(payload?.total);
  return {
    kind: "mathlib",
    phase: status,
    query: str(args.query) ?? "",
    candidates,
    ...(total !== undefined ? { total } : {}),
    truncated: payload?.truncated === true || (total ?? 0) > candidates.length,
    ...(mode ? { mode } : {}),
    ...(fallback ? { fallback } : {}),
    noMatches: status === "ok" && (hasDiagnostic(payload, "no_matches") || list(payload?.candidates).length === 0),
    suggestions: loogleSuggestions(error),
    ...(error ? { error } : {}),
  };
}

/** `theorem foo …` → `foo` (else the profile is not interesting: no subject). */
function leanSubject(source: string | undefined): string | undefined {
  const match = source?.match(/\b(?:theorem|lemma|def|example|instance)\s+([^\s:({[]+)?/);
  if (!match) return undefined;
  return match[1] ?? "example";
}

function leanAct(args: Json, payload: Json | undefined, status: SceneStatus): LeanAct {
  const source = str(args.source_text) ?? "";
  const lines = source.replace(/\s+$/, "").split("\n");
  const check = isObject(payload?.check) ? payload.check : undefined;
  const all = list(check?.diagnostics).filter(isObject);
  const diagnostics: LeanDiagnostic[] = all.slice(0, LEAN_DIAGNOSTICS_CAP).map((item) => ({
    severity: str(item.severity) ?? "error",
    ...(num(item.line) !== undefined ? { line: num(item.line) } : {}),
    ...(num(item.column) !== undefined ? { column: num(item.column) } : {}),
    message: clip(str(item.message)?.split("\n")[0] ?? "", 160),
  }));
  const verdict = str(check?.status);
  const errors = all.some((item) => item.severity === "error");
  const elapsed = num(check?.elapsed_ms);
  return {
    kind: "lean",
    phase: status === "running" ? "running" : status === "error" ? "error" : verdict === "checked" && !errors ? "checked" : "failed",
    code: lines.slice(0, LEAN_CODE_LINES).join("\n"),
    codeLines: source ? lines.length : 0,
    diagnostics,
    diagnosticsTruncated: check?.diagnostics_truncated === true || all.length > diagnostics.length,
    ...(elapsed !== undefined ? { elapsedMs: elapsed } : {}),
    ...(status === "error" ? { error: payloadError(payload) ?? "调用失败" } : {}),
  };
}

/** 1-based page of a locate match (`page.page_number`, else index + 1). */
function locatePage(payload: Json | undefined): number | undefined {
  const page = isObject(payload?.page) ? payload.page : undefined;
  const number = num(page?.page_number);
  if (number !== undefined) return number;
  const index = num(page?.pdf_page_index);
  return index !== undefined ? index + 1 : undefined;
}

/** Localize by diagnostic code/fields, never by parsing the agent-facing English. */
function locateDiagnostics(payload: Json | undefined): LocateDiagnostic[] {
  return list(payload?.diagnostics).filter(isObject).map((item): LocateDiagnostic => {
    const code = str(item.code) ?? "unknown";
    const original = str(item.message) ?? "";
    const base = { code, original };
    switch (code) {
      case "page_markers_absent": {
        const pages = isObject(item.snapshot_pages) ? item.snapshot_pages : undefined;
        const from = num(pages?.from);
        const to = num(pages?.to);
        const known = from !== undefined && to !== undefined && Number.isInteger(from) && Number.isInteger(to) && from >= 1 && to >= from;
        return { ...base, category: "page", message: known
          ? t("scene.locate.note.pagesWithoutMarkers", { pages: from === to ? from : `${from}–${to}` })
          : t("scene.locate.note.noPageMarkers") };
      }
      case "page_unresolved":
        return { ...base, category: "page", message: t("scene.locate.note.pageUnresolved") };
      case "matched_normalized":
        return { ...base, category: "match", message: t("scene.locate.note.matchedNormalized") };
      case "matched_raw_ocr_fallback":
        return { ...base, category: "source", message: t("scene.locate.note.matchedRawOcr") };
      case "candidates_truncated":
        return { ...base, category: "candidates", message: t("scene.locate.note.candidatesTruncated") };
      default:
        return { ...base, category: "other", message: t("scene.locate.note.other") };
    }
  });
}

function locateAct(args: Json, payload: Json | undefined, status: SceneStatus): LocateAct {
  const inner = str(payload?.status);
  const phase: LocateAct["phase"] =
    status === "running" ? "running" : status === "error" ? "error" : inner === "ambiguous" ? "ambiguous" : inner === "not_found" ? "miss" : "ok";
  const page = locatePage(payload);
  const count = num(payload?.candidate_count);
  const reason =
    phase === "ok"
      ? undefined
      : phase === "ambiguous"
        ? `${count ?? list(payload?.candidates).length} 处候选：加长引文或用 within 缩小范围`
        : phase === "running"
          ? undefined
          : payloadError(payload) ?? "引文与原文不一致";
  return {
    kind: "locate",
    phase,
    quote: str(args.quote) ?? "",
    artifactId: str(payload?.artifact_id) ?? str(args.artifact_id),
    diagnostics: locateDiagnostics(payload),
    ...(str(payload?.match) ? { match: MATCH_LABEL[str(payload?.match)!] ?? str(payload?.match) } : {}),
    ...(page !== undefined ? { page } : {}),
    ...(count !== undefined ? { candidates: count } : {}),
    ...(reason ? { reason } : {}),
  };
}

type ActCompiler = (
  args: Json,
  payload: Json | undefined,
  status: SceneStatus,
  result: AgentEventLike | undefined,
  prior: readonly AgentEventLike[],
) => SceneAct | undefined;

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
  propose_changes: (args, payload, status, _result, prior) => proposeAct(args, payload, status, prior),
  "mathlib.search": (args, payload, status) => mathlibAct(args, payload, status),
  "lean.check": (args, payload, status) => leanAct(args, payload, status),
  "material.locate": (args, payload, status) => locateAct(args, payload, status),
};

/** One tool step's Act overlay (exported for tests). */
export function compileSceneAct(
  call: AgentEventLike | undefined,
  result?: AgentEventLike,
  prior: readonly AgentEventLike[] = [],
): SceneAct | undefined {
  const tool = sceneToolName(call?.payload?.tool ?? rawOf(call)?.tool ?? rawOf(result)?.tool);
  const act = tool ? ACTS[tool] : undefined;
  if (!act) return undefined;
  const payload = resultPayload(result);
  return act(callArgs(call ?? result), payload, resultStatus(result, payload, tool), result, prior);
}

/** Epoch ms of an event's `ts` (or the raw item's `startedAtMs`). */
function startedAt(call: AgentEventLike | undefined): number | undefined {
  const started = num(rawOf(call)?.startedAtMs);
  if (started !== undefined) return started;
  const ts = typeof call?.ts === "string" ? Date.parse(call.ts) : NaN;
  return Number.isFinite(ts) ? ts : undefined;
}

/**
 * One tool step (call, plus its result once it arrived) → draft. `prior` =
 * earlier events of the session (a superseding proposal finds the staged
 * ids of the version it replaces there).
 */
export function compileToolScene(
  call: AgentEventLike | undefined,
  result?: AgentEventLike,
  prior: readonly AgentEventLike[] = [],
): SceneDraft | null {
  const event = call ?? result;
  const tool = sceneToolName(call?.payload?.tool ?? rawOf(call)?.tool ?? rawOf(result)?.tool);
  const callId = str(event?.payload?.call_id);
  if (!tool || !callId) return null;
  const args = callArgs(call ?? result);
  const payload = resultPayload(result);
  const status = resultStatus(result, payload, tool);
  const parts = COMPILERS[tool](args, payload, status);
  const act = ACTS[tool]?.(args, payload, status, result, prior);
  const started = startedAt(call);
  const duration = num(result?.payload?.duration_ms) ?? num(rawOf(result)?.durationMs);
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
    ...(started !== undefined ? { startedAt: started } : {}),
    ...(duration !== undefined && result ? { durationMs: duration } : {}),
  };
}

/**
 * Every tool step of a round's events, in call order. `history` = the
 * session's events (defaults to `events`): a step sees only what came before
 * it, so a replay compiles exactly what the live round played.
 */
export function compileRoundScenes(events: readonly AgentEventLike[], history: readonly AgentEventLike[] = events): SceneDraft[] {
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
    .map((id) => {
      const result = results.get(id);
      const until = result ? history.indexOf(result) : -1;
      return compileToolScene(calls.get(id), result, until >= 0 ? history.slice(0, until) : history);
    })
    .filter((draft): draft is SceneDraft => draft !== null);
}

/** A proposal's ids on the map: real staged nodes, anchors, retired nodes. */
function resolvePropose(act: ProposeAct, resolve: SceneResolve): ProposeAct {
  const node = (id: string | undefined) => (id ? resolve(id)?.nodeId : undefined);
  const nodes = (raw: readonly string[]) => [...new Set(raw.map(node).filter((id): id is string => !!id))];
  const end = (value: ProposeEnd): ProposeEnd => ("id" in value ? { id: value.id, ...(node(value.id) ? { node: node(value.id) } : {}) } : value);
  return {
    ...act,
    items: act.items.map((item) => ({
      ...item,
      ...(node(item.stagedId) ? { node: node(item.stagedId) } : {}),
      anchorNodes: nodes(item.anchors),
    })),
    relations: act.relations.map((relation) => ({ ...relation, source: end(relation.source), target: end(relation.target) })),
    retiredNodes: nodes(act.retired),
  };
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
  // A proposal's map count is its staged nodes (the player finds those), not its anchors.
  if (draft.act?.kind !== "propose" && (draft.status !== "running" || shown > 0)) {
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
    ...(draft.act ? { act: draft.act.kind === "propose" ? resolvePropose(draft.act, resolve) : draft.act } : {}),
    ...(seeds[0] ? { anchor: seeds[0] } : {}),
    ...(draft.startedAt !== undefined ? { startedAt: draft.startedAt } : {}),
    ...(draft.durationMs !== undefined ? { durationMs: draft.durationMs } : {}),
  };
}

/** A scene with something to light on the map (else HUD-only). */
export function sceneHasFocus(scene: ToolScene): boolean {
  return scene.seeds.length > 0 || scene.layers.length > 0;
}
