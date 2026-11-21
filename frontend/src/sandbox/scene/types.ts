/**
 * Map Scene Player data (first slice).
 *
 * Every MCP tool step of a sandbox round compiles to a `ToolScene`: a short
 * declarative script the map player performs — Announce (HUD title +
 * summary), Isolate (dim everything outside the focus with the existing
 * `is-out-of-focus` style), fit the camera to the focus, light the seeds and
 * then the result layers, Settle (restore camera + focus scope).
 *
 * Scenes are animation state only. They never touch the landscape data, and
 * they are not persisted on their own: the round's agent events already are
 * (`sandbox_session_dialogue`), so replay recompiles from those events.
 */

/**
 * Mathscape MCP tools with a scene (bare names): the eight read-only map
 * tools, then (fourth slice) the proposal / Lean / locate tools.
 */
export const SCENE_TOOLS = [
  "workspace.search",
  "material.read",
  "material.grep",
  "pdf.outline",
  "graph.get",
  "graph.neighborhood",
  "ink.page_image",
  "pdf.page_image",
  "propose_changes",
  "mathlib.search",
  "lean.check",
  "material.locate",
  "list_change_sets",
  "get_change_set",
] as const;

export type SceneToolName = (typeof SCENE_TOOLS)[number];

export type SceneStatus = "running" | "ok" | "error";

/** Map-independent compile output: ids are raw domain/stable ids. */
export interface SceneDraft {
  callId: string;
  roundId?: string;
  tool: SceneToolName;
  status: SceneStatus;
  /** HUD title verb, e.g. `搜索`. */
  verb: string;
  /** HUD title subject (query, pattern, label…); falls back to `subjectId`. */
  subject?: string;
  /** Id whose map label (or short form) names the subject when `subject` is unset. */
  subjectId?: string;
  /** Extra title suffix, e.g. `深度 2 · 双向`. */
  detail?: string;
  /** HUD summary line (Chinese). */
  summary: string;
  /** What the call asked about. */
  seeds: string[];
  /** What the result returned, shallow first (neighborhood: one per depth). */
  layers: string[][];
  /** Act overlay (second slice): typewriter / outline window / page image. */
  act?: SceneAct;
  /** Call start (epoch ms, from the event) for the running indicator. */
  startedAt?: number;
  /** Result duration (ms) when the result says so. */
  durationMs?: number;
}

/** A draft resolved against the current map: projection node ids only. */
export interface ToolScene {
  callId: string;
  roundId?: string;
  tool: SceneToolName;
  status: SceneStatus;
  title: string;
  summary: string;
  seeds: string[];
  /** Non-empty result layers, each node in one layer; a seed the result
   * confirms reappears here (seed → hit). */
  layers: string[][];
  /** Ids the draft named that are not (unambiguously) on the map. */
  offMap: number;
  /** Focus was cut to `SCENE_FOCUS_CAP`. */
  capped: boolean;
  /** Act overlay, copied from the draft. */
  act?: SceneAct;
  /** Map node an Act window sits beside (first resolved seed), if any. */
  anchor?: string;
  startedAt?: number;
  durationMs?: number;
  /** Pacing: this many earlier same-tool segments were merged into this one. */
  coalesced?: number;
}

/** Map node a raw id resolves to (unique, unambiguous). */
export interface SceneTarget {
  nodeId: string;
  label: string;
}

export type SceneResolve = (id: string) => SceneTarget | null;

/**
 * At most this many map nodes per scene. The A-PR3 echo used 40; the
 * player raises it to 60 so a default `graph.neighborhood` (max_nodes 40)
 * plus its seeds fits whole. Seeds and shallow layers are kept first.
 */
export const SCENE_FOCUS_CAP = 60;

// ---------------------------------------------------------------------------
// Act overlays (second slice)

/** `workspace.search`: type the query in the HUD before the hits land. */
export interface TypewriterAct {
  kind: "typewriter";
  text: string;
}

export interface OutlineRow {
  title: string;
  /** 1 = top level. */
  level: number;
  /** 1-based page shown to the user (`pdf_page_index + 1`). */
  page?: number;
}

/** `pdf.outline`: a temporary 目录 mini-window beside the artifact. */
export interface OutlineAct {
  kind: "outline";
  rows: OutlineRow[];
  /** Entries the reader produced (may exceed `rows` when truncated). */
  total: number;
  truncated: boolean;
  /** Why `rows` is empty: no bookmarks, or an unreadable bookmark tree. */
  empty?: "none" | "unreadable";
  totalPages?: number;
}

/** `ink.page_image` / `pdf.page_image`: a temporary page-image mini-window. */
export interface PageImageAct {
  kind: "page_image";
  /** `data:image/…;base64,…` of the returned image block, when present. */
  src?: string;
  noteTitle?: string;
  /** e.g. `第 2/3 页`. */
  pageLabel?: string;
  width?: number;
  height?: number;
}

/**
 * `material.read` on scanned PDF pages: the call blocks while Marker parses
 * the pages (synchronous ensure). `running` = still waiting (live, seen on
 * the map summary 「解析中」); `done` / `failed` = the result's post-hoc
 * `details.map_signal`.
 */
export interface ParsingAct {
  kind: "parsing";
  label: "解析中";
  phase: "running" | "done" | "failed";
  /** 1-based inclusive pages being parsed, when known. */
  pages?: { from: number; to: number };
}

// ---------------------------------------------------------------------------
// Fourth slice: proposal / Lean / locate tools

/** A proposal item kind: source evidence, source unit or shared semantic object. */
export type ProposeItemKind = "unit" | "span" | "semantic";

/** Visual distinction used by proposal ghosts / highlights. */
export type ProposeVisualKind =
  | "source_unit"
  | "source_span"
  | "theory"
  | "concept"
  | "declaration"
  | "problem_schema"
  | "debt"
  | "dialect_scope";

export type ProposeRelationKind =
  | "source_relation"
  | "alignment"
  | "in_theory"
  | "uses_entity"
  | "debt_on"
  | "scope_of";

/** A proposed source / semantic item. */
export interface ProposeItem {
  localRef: string;
  kind: ProposeItemKind;
  /** Stable id when the staged object is already projected on the map. */
  stagedId?: string;
  /** Display label for a ghost or HUD. */
  label: string;
  /** Existing raw ids the ghost sits near. */
  anchors: string[];
  /** Blocking diagnostics attributed to this item. */
  blocked: ProposeBlock[];
  /** Semantic kind / visual role for semantic items. */
  visualKind?: ProposeVisualKind;
  /** Resolved (`resolveScene`): the real staged node on the map, if any. */
  node?: string;
  /** Resolved: map nodes of `anchors`. */
  anchorNodes?: string[];
}

export interface ProposeRelation {
  localRef: string;
  stagedId?: string;
  /** Relation kind (`proves`, `in_theory`, `debt_on`, …) and its label. */
  relation: string;
  relationKind?: ProposeRelationKind;
  label: string;
  /** Endpoint: `{ local }` = an item of this proposal, `{ id }` = an existing object. */
  source: ProposeEnd;
  target: ProposeEnd;
  blocked: ProposeBlock[];
}

/** Proposal endpoint: a local item or an existing map object. */
export type ProposeEnd = { local: string } | { id: string; node?: string };

export interface ProposeBlock {
  code: string;
  message: string;
}

/** `propose_changes` on the main map. */
export interface ProposeAct {
  kind: "propose";
  phase: "running" | "staged" | "blocked" | "error";
  changeSetId?: string;
  version?: number;
  items: ProposeItem[];
  relations: ProposeRelation[];
  /** `staged_ids` split by id prefix. */
  counts: { units: number; relations: number; spans: number; semantic: number };
  /** Every staged id (units, relations, spans and semantic objects). */
  stagedIds: string[];
  /** All blocking diagnostics (incl. ones without a local ref). */
  blocks: (ProposeBlock & { localRef?: string })[];
  supersedes?: { changeSetId: string; version: number };
  /** Staged ids of the superseded version that this version no longer has. */
  retired: string[];
  /** Resolved: map nodes of `retired`. */
  retiredNodes?: string[];
}

export interface MathlibCandidate {
  name: string;
  /** Type signature, truncated for the window. */
  type?: string;
  module?: string;
}

/** `mathlib.search`: query → candidates one by one; backend badge. */
export interface MathlibAct {
  kind: "mathlib";
  phase: "running" | "ok" | "error";
  query: string;
  candidates: MathlibCandidate[];
  total?: number;
  truncated: boolean;
  /** `backend_mode`: 常驻 (resident) / 一次性 (oneshot). */
  mode?: "resident" | "oneshot";
  /** `backend_fallback`: why the resident Loogle was not used. */
  fallback?: string;
  noMatches: boolean;
  suggestions: string[];
  error?: string;
}

export interface LeanDiagnostic {
  severity: string;
  line?: number;
  column?: number;
  message: string;
}

/** `lean.check`: the snippet, running, then ✓ or diagnostics. */
export interface LeanAct {
  kind: "lean";
  phase: "running" | "checked" | "failed" | "error";
  /** First lines of `source_text`. */
  code: string;
  codeLines: number;
  diagnostics: LeanDiagnostic[];
  diagnosticsTruncated: boolean;
  elapsedMs?: number;
  error?: string;
}

export interface LocateDiagnostic {
  code: string;
  category: "page" | "match" | "source" | "candidates" | "other";
  message: string;
  /** Unmodified tool diagnostic, shown only in technical details. */
  original: string;
}

/** `material.locate`: the quote, the match and its page, or why not. */
export interface LocateAct {
  kind: "locate";
  /** `ok` one match; `ambiguous` several; `miss` none (`not_found`). */
  phase: "running" | "ok" | "ambiguous" | "miss" | "error";
  quote: string;
  match?: string;
  /** 1-based page number shown to the user. */
  page?: number;
  candidates?: number;
  reason?: string;
  artifactId?: string;
  diagnostics?: LocateDiagnostic[];
}

export type SceneAct =
  | TypewriterAct
  | OutlineAct
  | PageImageAct
  | ParsingAct
  | ProposeAct
  | MathlibAct
  | LeanAct
  | LocateAct;

/** Map PDF node summary while its canonical-markdown lease is running. */
export const PARSING_LABEL = "解析中";

/** At most this many outline rows are drawn in the mini-window. */
export const OUTLINE_ROWS_CAP = 60;

/** At most this many mathlib candidates are listed in the window. */
export const MATHLIB_ROWS_CAP = 8;
/** Type signatures are cut to this many characters. */
export const MATHLIB_TYPE_MAX = 90;
/** Lean snippet lines shown in the window. */
export const LEAN_CODE_LINES = 12;
export const LEAN_DIAGNOSTICS_CAP = 6;
/** Ghost / relation steps a propose scene animates one by one. */
export const PROPOSE_STEP_CAP = 24;
