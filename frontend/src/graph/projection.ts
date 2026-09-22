export const GRAPH_PROJECTION_SCHEMA_VERSION =
  "mathscape.graph-projection.v1" as const;

export type ViewLayer =
  | "source"
  | "local_semantic"
  | "global_semantic"
  | "alignment"
  | "rdtx"
  | "formalization";

export type ProjectionLens =
  | "source"
  | "local_semantic"
  | "global_semantic"
  | "alignment"
  | "rdtx"
  | "formalization"
  | "provenance"
  | "landscape";

export type IncidenceNodeKind = "entity" | "relation" | "group";

/**
 * Stable relation discriminant emitted by the projection layer. The Rust enum
 * is `#[non_exhaustive]`, so consumers must treat any value outside this union
 * the same way they treat `unknown`.
 */
export type RelationKind =
  | "conversion"
  | "located_in"
  | "contains_span"
  | "states"
  | "proves"
  | "has_part"
  | "poses"
  | "hints"
  | "solves"
  | "explains"
  | "applies_method"
  | "cites"
  | "unknown";

export type IncidenceDirection =
  | "into_relation"
  | "out_of_relation"
  | "undirected";

export interface IncidenceNode {
  id: string;
  element_kind: IncidenceNodeKind;
  domain_id: string | null;
  domain_kind: string | null;
  /** Non-null only on relation nodes. */
  relation_kind: RelationKind | null;
  layer: ViewLayer;
  label: string;
  summary: string | null;
  lifecycle: string | null;
  parent: string | null;
  boundary: boolean;
}

export interface IncidenceEdge {
  id: string;
  source: string;
  target: string;
  relation_id: string;
  role: string;
  ordinal: number | null;
  direction: IncidenceDirection;
  directed: boolean;
}

export interface IncidenceGraph {
  schema_version: typeof GRAPH_PROJECTION_SCHEMA_VERSION;
  lens: ProjectionLens;
  nodes: IncidenceNode[];
  edges: IncidenceEdge[];
}
