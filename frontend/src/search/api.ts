import { safeInvoke } from "../core/ipc";

/** Mirrors `mathscape_query::search` (N3d). Kinds and targets are open-ended:
 * new providers add values without changing this contract's shape. */
export type SearchKind =
  | "note"
  | "source_unit"
  | "theory"
  | "semantic_entity"
  | "material"
  | "snapshot"
  | (string & {});

export type SearchTarget =
  | { kind: "map_node"; node_id: string }
  | { kind: "source_object"; object_id: string }
  | { kind: "artifact"; artifact_id: string }
  | { kind: "ink_note"; document_id: string }
  | { kind: "ink_snapshot"; snapshot_id: string; artifact_id: string; document_id: string };

export interface SearchHit {
  kind: SearchKind;
  id: string;
  title: string;
  subtitle: string | null;
  matched_alias: string | null;
  score: number;
  /** `[start, end)` UTF-16 offsets into `title`. */
  highlights: [number, number][];
  trashed: boolean;
  target: SearchTarget;
  /** Folded 「规范解析正文」 lineage this hit belongs to (MCP sets it; a
   * collapsed desktop hit already uses the canonical id as `id`/target). */
  canonical_id?: string | null;
  coverage?: unknown;
  partial?: boolean | null;
}

export interface SearchGroup {
  kind: SearchKind;
  label: string;
  total: number;
  hits: SearchHit[];
}

export interface SearchResponse {
  query: string;
  total: number;
  groups: SearchGroup[];
}

export interface SearchQuery {
  text: string;
  include_trashed?: boolean;
  kinds?: SearchKind[];
  limit_per_group?: number;
}

export function searchWorkspace(query: SearchQuery): Promise<SearchResponse> {
  return safeInvoke<SearchResponse>("search_workspace", { query });
}

/** Whether a target has a map node to locate (Alt+Enter / 定位). */
export function isMapTarget(target: SearchTarget): boolean {
  return target.kind === "source_object" || target.kind === "artifact" || target.kind === "map_node";
}
