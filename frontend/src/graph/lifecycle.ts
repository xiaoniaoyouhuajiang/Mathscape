import type { IncidenceGraph } from "./projection";

export function isRetired(lifecycle: string | null | undefined): boolean {
  return lifecycle === "deprecated" || lifecycle === "superseded" || lifecycle === "rejected";
}

/** Current activity lens, not a historical registration snapshot. */
export function activityGraph(graph: IncidenceGraph, showRetired: boolean): IncidenceGraph {
  if (showRetired) return graph;

  const activeNodes = graph.nodes.filter(node => !isRetired(node.lifecycle));
  const activeIds = new Set(activeNodes.map(node => node.id));
  const relationIds = new Set(
    activeNodes.filter(node => node.element_kind === "relation").map(node => node.id),
  );
  const activeEdges = graph.edges.filter(
    edge => activeIds.has(edge.source) && activeIds.has(edge.target),
  );

  // Endpoints are RenderElementIds. edge.relation_id is the owning domain
  // StableId, NOT the relation vertex id (e.g. "relation:mathscape:...").
  // Use incidence endpoints so backend and synthetic focus relations both work.
  const degree = new Map<string, number>();
  for (const edge of activeEdges) {
    const relation = relationIds.has(edge.source)
      ? edge.source
      : relationIds.has(edge.target) ? edge.target : undefined;
    if (relation !== undefined) degree.set(relation, (degree.get(relation) ?? 0) + 1);
  }

  const nodes = activeNodes.filter(
    node => node.element_kind !== "relation" || (degree.get(node.id) ?? 0) >= 2,
  );
  const keptIds = new Set(nodes.map(node => node.id));
  // Pruning a relation must also remove its spokes, regardless of domain id.
  const edges = activeEdges.filter(
    edge => keptIds.has(edge.source) && keptIds.has(edge.target),
  );
  return { ...graph, nodes, edges };
}
