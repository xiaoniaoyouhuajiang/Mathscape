/**
 * Cite chips for the sandbox dialogue (A-PR3).
 *
 * Agent answers, user messages and tool args/results mention material ids:
 * canonical `mathscape:<kind>/<hash>` ids or bare hex (32-hex note
 * document_id, 64-hex hashes). This module finds them, resolves them against
 *
 * 1. the static index built from the landscape snapshot (projection nodes
 *    on the map, keyed by domain_id and its bare hash), and
 * 2. a per-round index built from that round's tool calls/results (the ids
 *    the agent actually looked at, with labels/titles), which wins when the
 *    static index alone is ambiguous,
 *
 * and renders them as chips. Pure: no DOM, no Tauri; the dock owns clicks.
 *
 * Resolve order: exact domain_id → prefix variant (same hash, other
 * `mathscape:<kind>/`) → round index (its ids/aliases looked up again on the
 * map) → label-only. A unique map node gives a jump chip; several give a
 * 「歧义」 chip that prefers the round's context; none gives a non-jump chip
 * (labelled when the round knows the title, grey short id otherwise). Ids are
 * never shortened without the full id on hover.
 */
import type MarkdownIt from "markdown-it";
import type { IncidenceNode } from "../graph/projection";
import { domainKindLabel } from "../graph/taxonomy";
import { parseMathscapeId, shortStableId } from "../core/stable-id";
import type { AgentEventLike } from "./format";

/** One map node a cite can jump to. */
export interface CiteTarget {
  /** Projection (render) node id, what `selectNodeById` takes. */
  nodeId: string;
  domainId: string;
  domainKind: string | null;
  label: string;
}

export type CiteIndex = Map<string, CiteTarget[]>;

/** One id the agent saw this round, with whatever label the tool gave it. */
export interface RoundCiteEntry {
  id: string;
  label?: string;
  kind?: string;
  /** Other ids for the same object (snapshot_id ↔ snapshot artifact id). */
  aliases: string[];
}

export type RoundCiteIndex = Map<string, RoundCiteEntry[]>;

export type CiteStatus = "map" | "ambiguous" | "label" | "unknown";

export interface CiteResolution {
  raw: string;
  status: CiteStatus;
  /** Jump target for `map` and (preferred candidate) `ambiguous`. */
  target?: CiteTarget;
  candidates: CiteTarget[];
  label?: string;
  kind?: string;
}

export type CiteResolver = (raw: string) => CiteResolution;

// ---------------------------------------------------------------------------
// Scan

const ID_PATTERN = /mathscape:[a-z0-9-]+\/[A-Za-z0-9._/-]+|(?<![A-Za-z0-9_])(?:[0-9a-fA-F]{64}|[0-9a-fA-F]{32})(?![A-Za-z0-9_])/g;

export interface IdMatch {
  start: number;
  end: number;
  raw: string;
}

/** Ids in `text`, left to right. Trailing `.`/`/`/`-` (sentence punctuation)
 * is not part of a canonical id. */
export function scanIds(text: string): IdMatch[] {
  const out: IdMatch[] = [];
  for (const match of text.matchAll(ID_PATTERN)) {
    let raw = match[0];
    raw = raw.replace(/[._/-]+$/, "");
    if (raw.startsWith("mathscape:") && !parseMathscapeId(raw)) continue;
    const start = match.index ?? 0;
    out.push({ start, end: start + raw.length, raw });
  }
  return out;
}

/** Lookup keys: the id itself and its bare hash (prefix variants). */
export function citeKeys(id: string): string[] {
  const full = id.trim().toLowerCase();
  const parsed = parseMathscapeId(full);
  return parsed && parsed.hash !== full ? [full, parsed.hash] : [full];
}

// ---------------------------------------------------------------------------
// Static index (landscape snapshot)

function push<T>(index: Map<string, T[]>, key: string, value: T, same: (a: T, b: T) => boolean): void {
  const list = index.get(key);
  if (!list) index.set(key, [value]);
  else if (!list.some((item) => same(item, value))) list.push(value);
}

const sameTarget = (a: CiteTarget, b: CiteTarget) => a.domainId === b.domainId;

/** domain_id / bare hash → map node. One target per domain id: the entity
 * (non-boundary) node wins over relation or boundary copies. */
export function buildCiteIndex(nodes: readonly IncidenceNode[]): CiteIndex {
  const best = new Map<string, IncidenceNode>();
  const rank = (node: IncidenceNode) => (node.element_kind === "entity" ? 0 : 2) + (node.boundary ? 1 : 0);
  for (const node of nodes) {
    if (!node.domain_id) continue;
    const seen = best.get(node.domain_id);
    if (!seen || rank(node) < rank(seen)) best.set(node.domain_id, node);
  }
  const index: CiteIndex = new Map();
  for (const node of best.values()) {
    const target: CiteTarget = { nodeId: node.id, domainId: node.domain_id!, domainKind: node.domain_kind, label: node.label };
    for (const key of citeKeys(node.domain_id!)) push(index, key, target, sameTarget);
  }
  return index;
}

// ---------------------------------------------------------------------------
// Round index (tool_call / tool_result payloads)

/** Fields that hold an object id in MCP arguments and results. */
const ID_FIELDS = new Set([
  "id", "canonical_id", "node_id", "object_id", "artifact_id", "document_id", "snapshot_id",
  "parent_artifact_id", "target_id", "seed_ids", "node_ids", "ids",
]);
/** Fields whose id is the object itself (so it takes the object's label). */
const SELF_FIELDS = ["canonical_id", "id"];
const LABEL_FIELDS = ["display_title", "label", "title", "filename"];
/** Child objects describing the same entity as their parent (search hit
 * targets); `content_ref` points at the parent material instead. */
const SAME_ENTITY_CHILD = new Set(["target"]);

function str(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value : undefined;
}

function looksLikeId(value: string): boolean {
  const hits = scanIds(value);
  return hits.length === 1 && hits[0].start === 0 && hits[0].end === value.length;
}

function parseJsonText(value: string): unknown {
  const trimmed = value.trim();
  if (!(trimmed.startsWith("{") || trimmed.startsWith("["))) return undefined;
  try {
    return JSON.parse(trimmed);
  } catch {
    return undefined;
  }
}

/** Every id an MCP argument/result value mentions, with labels where the
 * object carries one. JSON inside strings (MCP `content[].text`) is parsed. */
export function collectCiteEntries(value: unknown): RoundCiteEntry[] {
  const out: RoundCiteEntry[] = [];
  const add = (id: string, label?: string, kind?: string, aliases: string[] = []) => {
    if (!looksLikeId(id)) return;
    out.push({ id, label, kind, aliases: aliases.filter((alias) => alias !== id && looksLikeId(alias)) });
  };
  const walk = (node: unknown, inherited: { label?: string; kind?: string } | null, depth: number) => {
    if (depth > 24 || node === null || node === undefined) return;
    if (typeof node === "string") {
      const parsed = parseJsonText(node);
      if (parsed !== undefined) walk(parsed, null, depth + 1);
      else for (const hit of scanIds(node)) add(hit.raw);
      return;
    }
    if (Array.isArray(node)) {
      for (const item of node) walk(item, null, depth + 1);
      return;
    }
    if (typeof node !== "object") return;
    const record = node as Record<string, unknown>;
    const ownLabel = LABEL_FIELDS.map((field) => str(record[field])).find(Boolean) ?? inherited?.label;
    const ownKind = str(record.kind) ?? str(record.domain_kind) ?? inherited?.kind;
    const selfId = SELF_FIELDS.map((field) => str(record[field])).find((id) => id && looksLikeId(id));
    const selfAliases = [str(record.snapshot_id), str(record.canonical_id), str(record.id)].filter((id): id is string => !!id);
    for (const [key, value] of Object.entries(record)) {
      if (ID_FIELDS.has(key)) {
        const ids = Array.isArray(value) ? value.map(str).filter((id): id is string => !!id) : [str(value)].filter((id): id is string => !!id);
        for (const id of ids) {
          const isSelf = id === selfId || (key === "snapshot_id" && !!selfId);
          if (isSelf || inherited) add(id, ownLabel, ownKind, selfAliases);
          else add(id);
        }
        continue;
      }
      if (value && typeof value === "object") {
        walk(value, SAME_ENTITY_CHILD.has(key) && ownLabel ? { label: ownLabel, kind: ownKind } : null, depth + 1);
      } else if (typeof value === "string" && !LABEL_FIELDS.includes(key)) {
        walk(value, null, depth + 1);
      }
    }
  };
  walk(value, null, 0);
  return out;
}

function eventSources(event: AgentEventLike): unknown[] {
  const payload = event.payload ?? {};
  const raw = payload.raw && typeof payload.raw === "object" ? (payload.raw as Record<string, unknown>) : undefined;
  if (event.type === "tool_call") return [raw?.arguments ?? payload.args_summary];
  if (event.type === "tool_result") return [raw?.result, raw?.error];
  return [];
}

/** Ids the tool call names as its input (graph seeds, read targets). */
export function toolCallIds(event: AgentEventLike): string[] {
  return event.type === "tool_call" ? uniqueIds(collectCiteEntries(eventSources(event)[0])) : [];
}

/** Ids a tool result returns (graph nodes, search hits, read targets). */
export function toolResultIds(event: AgentEventLike): string[] {
  return event.type === "tool_result" ? uniqueIds(eventSources(event).flatMap((value) => collectCiteEntries(value))) : [];
}

function uniqueIds(entries: RoundCiteEntry[]): string[] {
  return [...new Set(entries.flatMap((entry) => [entry.id, ...entry.aliases]))];
}

/** Index of every id the round's tool calls and results mention. */
export function buildRoundCiteIndex(events: readonly AgentEventLike[]): RoundCiteIndex {
  const index: RoundCiteIndex = new Map();
  const merge = (entry: RoundCiteEntry) => {
    for (const id of [entry.id, ...entry.aliases]) {
      for (const key of citeKeys(id)) {
        const list = index.get(key);
        const same = list?.find((item) => item.id === entry.id);
        if (same) {
          same.label ??= entry.label;
          same.kind ??= entry.kind;
          for (const alias of entry.aliases) if (!same.aliases.includes(alias)) same.aliases.push(alias);
        } else if (list) {
          list.push({ ...entry, aliases: [...entry.aliases] });
        } else {
          index.set(key, [{ ...entry, aliases: [...entry.aliases] }]);
        }
      }
    }
  };
  for (const event of events) for (const source of eventSources(event)) for (const entry of collectCiteEntries(source)) merge(entry);
  return index;
}

// ---------------------------------------------------------------------------
// Resolve

function lookupKeys(index: Map<string, CiteTarget[]>, keys: string[]): CiteTarget[] {
  const out: CiteTarget[] = [];
  for (const key of keys) for (const target of index.get(key) ?? []) if (!out.some((item) => sameTarget(item, target))) out.push(target);
  return out;
}

export function resolveCite(raw: string, round: RoundCiteIndex | undefined, map: CiteIndex): CiteResolution {
  const [full, hash] = citeKeys(raw);
  const exact = map.get(full) ?? [];
  const variants = hash ? map.get(hash) ?? [] : [];
  const roundEntries: RoundCiteEntry[] = [];
  for (const key of [full, hash].filter((key): key is string => !!key)) {
    for (const entry of round?.get(key) ?? []) if (!roundEntries.includes(entry)) roundEntries.push(entry);
  }
  // A bare hex id is looked up by hash only: the map keys both forms.
  let candidates = exact.length ? exact : variants;
  const roundTargets = lookupKeys(map, roundEntries.flatMap((entry) => [entry.id, ...entry.aliases].map((id) => id.toLowerCase())));
  if (!candidates.length) candidates = roundTargets;
  if (candidates.length > 1 && roundTargets.length) {
    const narrowed = candidates.filter((target) => roundTargets.some((item) => sameTarget(item, target)));
    if (narrowed.length === 1) candidates = narrowed;
    else if (narrowed.length > 1) candidates = [...narrowed, ...candidates.filter((target) => !narrowed.includes(target))];
  }
  const roundLabel = roundEntries.map((entry) => entry.label).find(Boolean);
  const roundKind = roundEntries.map((entry) => entry.kind).find(Boolean);
  if (candidates.length === 1) {
    const target = candidates[0];
    return { raw, status: "map", target, candidates, label: target.label || roundLabel, kind: target.domainKind ?? roundKind };
  }
  if (candidates.length > 1) {
    const target = candidates[0];
    return { raw, status: "ambiguous", target, candidates, label: target.label || roundLabel, kind: target.domainKind ?? roundKind };
  }
  if (roundLabel) return { raw, status: "label", candidates: [], label: roundLabel, kind: roundKind };
  return { raw, status: "unknown", candidates: [], kind: roundKind };
}

export function citeResolver(map: CiteIndex, round?: RoundCiteIndex): CiteResolver {
  const cache = new Map<string, CiteResolution>();
  return (raw) => {
    let hit = cache.get(raw);
    if (!hit) {
      hit = resolveCite(raw, round, map);
      cache.set(raw, hit);
    }
    return hit;
  };
}

/** Map node ids for `ids`, unique, in order, capped (echo). Ambiguous ids
 * are skipped: no flash is better than a wrong one. */
export function mapNodeIds(ids: readonly string[], map: CiteIndex, round?: RoundCiteIndex, cap = 40): string[] {
  const out: string[] = [];
  for (const id of ids) {
    const hit = resolveCite(id, round, map);
    if (hit.status === "map" && hit.target && !out.includes(hit.target.nodeId)) out.push(hit.target.nodeId);
    if (out.length >= cap) break;
  }
  return out;
}

// ---------------------------------------------------------------------------
// Render (HTML strings; every text piece is escaped)

export function escapeHtml(value: string): string {
  return value.replace(/[&<>"']/g, (ch) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[ch]!);
}

const CHIP_LABEL_MAX = 40;

function chipLabel(label: string): string {
  return label.length > CHIP_LABEL_MAX ? `${label.slice(0, CHIP_LABEL_MAX - 1)}…` : label;
}

/** Hover text: kind · short id (full id kept in data-cite-id). */
function shortCite(id: string): string {
  const parsed = parseMathscapeId(id);
  return parsed ? `${parsed.kind}/${shortStableId(parsed.hash, { max: 20, head: 8, tail: 6 })}` : shortStableId(id, { max: 20, head: 8, tail: 6 });
}

export function citeChipHtml(cite: CiteResolution): string {
  const kind = cite.kind ? domainKindLabel(cite.kind) : "";
  const idText = shortCite(cite.target?.domainId ?? cite.raw);
  const attrs = (cls: string, title: string) =>
    `class="sandbox-cite ${cls}" data-cite-id="${escapeHtml(cite.raw)}" title="${escapeHtml(title)}"`;
  if ((cite.status === "map" || cite.status === "ambiguous") && cite.target) {
    const head = [kind, idText].filter(Boolean).join(" · ");
    const more = cite.status === "ambiguous"
      ? `\n歧义：${cite.candidates.length} 个候选，点击定位第一个\n${cite.candidates.map((item) => `· ${item.label || shortCite(item.domainId)}`).join("\n")}`
      : "\n点击在地图上定位";
    const flag = cite.status === "ambiguous" ? `<span class="sandbox-cite-flag">歧义</span>` : "";
    return `<button type="button" ${attrs(`is-${cite.status}`, head + more)} data-cite-node="${escapeHtml(cite.target.nodeId)}">${escapeHtml(chipLabel(cite.label || idText))}${flag}</button>`;
  }
  if (cite.status === "label" && cite.label) {
    const title = [kind, idText].filter(Boolean).join(" · ") + "\n不在当前地图上";
    return `<span ${attrs("is-label", title)}>${escapeHtml(chipLabel(cite.label))}</span>`;
  }
  return `<span ${attrs("is-unknown", `${cite.raw}\n未在地图或本轮工具结果中找到`)}>${escapeHtml(idText)}</span>`;
}

/** Plain text → escaped HTML with every id replaced by a chip. */
export function citeTextHtml(text: string, resolve: CiteResolver | undefined): string {
  if (!resolve) return escapeHtml(text);
  let html = "";
  let at = 0;
  for (const match of scanIds(text)) {
    html += escapeHtml(text.slice(at, match.start)) + citeChipHtml(resolve(match.raw));
    at = match.end;
  }
  return html + escapeHtml(text.slice(at));
}

interface CiteEnv {
  resolve?: CiteResolver;
}

/** Hook chip rendering into a markdown-it instance: text, inline code and
 * code blocks get chips; math tokens render through KaTeX untouched. */
export function withCiteChips(md: MarkdownIt): MarkdownIt {
  md.renderer.rules.text = (tokens, idx, _options, env: CiteEnv) => citeTextHtml(tokens[idx].content, env?.resolve);
  md.renderer.rules.code_inline = (tokens, idx, _options, env: CiteEnv) =>
    `<code>${citeTextHtml(tokens[idx].content, env?.resolve)}</code>`;
  const block = (tokens: { content: string }[], idx: number, _options: unknown, env: CiteEnv) =>
    `<pre><code>${citeTextHtml(tokens[idx].content, env?.resolve)}</code></pre>\n`;
  md.renderer.rules.code_block = block;
  md.renderer.rules.fence = block;
  return md;
}
