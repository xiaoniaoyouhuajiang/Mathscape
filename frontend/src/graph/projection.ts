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
  | "aligns"
  | "in_theory"
  | "debt_on"
  | "scope_of"
  | "uses_entity"
  | "of_concept"
  | "paid_by"
  | "unknown";

export type HopClass =
  | "plumbing"
  | "context_attach"
  | "hierarchy"
  | "source_reasoning"
  | "semantic_bridge"
  | "container_hub"
  | "forbidden";

export function relationHopClass(kind: RelationKind | null): HopClass {
  switch (kind) {
    case "conversion":
    case "located_in":
    case "contains_span":
      return "plumbing";
    case "debt_on":
    case "scope_of":
      return "context_attach";
    case "has_part":
      return "hierarchy";
    case "states":
    case "proves":
    case "poses":
    case "hints":
    case "solves":
    case "explains":
    case "applies_method":
    case "cites":
      return "source_reasoning";
    case "aligns":
    case "uses_entity":
    case "of_concept":
    case "paid_by":
      return "semantic_bridge";
    case "in_theory":
      return "container_hub";
    default:
      return "forbidden";
  }
}

export type IncidenceDirection =
  | "into_relation"
  | "out_of_relation"
  | "undirected";

export interface IncidenceNode {
  /** Projection-local RenderElementId, distinct from domain_id. */
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
  /** Physical artifact to open when `domain_id` is logical (folded canonical
   * markdown → head snapshot). Omitted when `domain_id` itself opens. */
  openable_artifact_id?: string | null;
  /** Every physical snapshot folded into this node (folded canonical
   * markdown: head and non-head segments). Locate / cite / chip resolve any
   * of them onto this node. Omitted when empty. */
  aliased_artifact_ids?: string[];
}

export interface IncidenceEdge {
  id: string;
  source: string;
  target: string;
  /** Owner's domain StableId; find the relation vertex through source/target.
   * Synthetic focus relations may instead use a projection-local owner id. */
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
