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
