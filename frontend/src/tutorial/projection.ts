/**
 * 只读教程投影（fixtures/emir/fibonacci-two-books/tutorial-projection.json）。
 *
 * 这个文件由 Desktop 测试把夹具走一遍真实的暂存 → 接受 → 地图投影后写出；
 * 教程只读它：不调用任何 Tauri 命令，不写用户的工作区。
 */
import type { IncidenceEdge, IncidenceGraph, IncidenceNode } from "../graph/projection";
import type { LearningView } from "../semantic/api";

export const TUTORIAL_PROJECTION_SCHEMA = "mathscape.tutorial-projection.v1";

export type TutorialEpisodeId = "ep1" | "ep2";

export interface TutorialObject {
  id: string;
  kind: string | null;
  payload: Record<string, unknown>;
  /** 原文片段：提案里的原文（地图只存字节范围）。 */
  quote?: string;
}

export interface TutorialProjection {
  schema: string;
  label: string;
  note: string;
  fixture_version: string;
  artifacts: Record<string, string>;
  refs: Record<string, string>;
  objects: Record<string, TutorialObject>;
  graph: IncidenceGraph;
  learning: LearningView;
  /** 夹具的修订记录（v1 → v2），教程“修订”一镜用。 */
  revisions?: Array<Record<string, unknown>>;
}

/** 按需加载指定剧集的只读投影（单独分包，打开对应教程时才读）。 */
export async function loadEpisodeProjection(episode: TutorialEpisodeId): Promise<TutorialProjection> {
  const module = episode === "ep2"
    ? await import("../../../fixtures/emir/generating-functions-two-books/tutorial-projection.json")
    : await import("../../../fixtures/emir/fibonacci-two-books/tutorial-projection.json");
  const projection = (module.default ?? module) as unknown as TutorialProjection;
  if (projection.schema !== TUTORIAL_PROJECTION_SCHEMA) {
    throw new Error(`unsupported tutorial projection: ${projection.schema}`);
  }
  return projection;
}

/** 默认加载第 1 集投影（保持现有调用兼容）。 */
export async function loadTutorialProjection(): Promise<TutorialProjection> {
  return loadEpisodeProjection("ep1");
}

const DROPPED_KINDS = new Set(["SourceSpan"]);

/**
 * 教程地图：去掉原文片段及只连着片段的关系（证据、包含片段），其余原样保留。
 * 原文片段的内容在卡片上显示。
 */
export function tutorialGraph(projection: TutorialProjection): IncidenceGraph {
  const graph = projection.graph;
  const dropped = new Set(
    graph.nodes
      .filter((node) => node.element_kind === "entity" && DROPPED_KINDS.has(node.domain_kind ?? ""))
      .map((node) => node.id),
  );
  const touchesDropped = new Set<string>();
  for (const edge of graph.edges) {
    if (dropped.has(edge.source)) touchesDropped.add(edge.target);
    if (dropped.has(edge.target)) touchesDropped.add(edge.source);
  }
  const keepNode = (node: IncidenceNode) =>
    !dropped.has(node.id) && !(node.element_kind === "relation" && touchesDropped.has(node.id));
  const nodes = graph.nodes.filter(keepNode);
  const kept = new Set(nodes.map((node) => node.id));
  const edges = graph.edges.filter((edge: IncidenceEdge) => kept.has(edge.source) && kept.has(edge.target));
  return { ...graph, nodes, edges };
}

export function objectOf(projection: TutorialProjection, local: string): TutorialObject {
  const object = projection.objects[local];
  if (!object) throw new Error(`tutorial projection has no ${local}`);
  return object;
}

export function payloadOf(projection: TutorialProjection, local: string): Record<string, unknown> {
  return objectOf(projection, local).payload;
}

/** 地图上显示的标签（以投影为准，和真实地图同一份白话模板）。 */
export function graphLabel(projection: TutorialProjection, local: string): string {
  const id = projection.refs[local];
  const node = projection.graph.nodes.find((item) => item.domain_id === id && item.element_kind !== "group");
  return node?.label ?? local;
}
