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

/** The seven read-only Mathscape MCP tools (bare names). */
export const SCENE_TOOLS = [
  "workspace.search",
  "material.read",
  "material.grep",
  "pdf.outline",
  "graph.get",
  "graph.neighborhood",
  "ink.page_image",
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

/** `ink.page_image`: a temporary page-image mini-window. */
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

export type SceneAct = TypewriterAct | OutlineAct | PageImageAct;

/** At most this many outline rows are drawn in the mini-window. */
export const OUTLINE_ROWS_CAP = 60;
