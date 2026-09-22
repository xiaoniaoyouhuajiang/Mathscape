import {
  GRAPH_PROJECTION_SCHEMA_VERSION,
  type IncidenceEdge,
  type IncidenceGraph,
  type IncidenceNode,
  type RelationKind,
} from "./projection";

export type FocusDepth = 1 | 2 | "all";

/**
 * Marks the synthetic relations this module creates when a derivation path had
 * to be contracted across nodes the user filtered out.
 */
export const FOCUS_CONTRACTION_KIND = "FocusContraction";

interface DerivationRule {
  /** Endpoint side that sits upstream in the derivation direction. */
  upstream: "into_relation" | "out_of_relation";
  costsDepth: boolean;
}

/**
 * Traversal is defined on `relation_kind` alone, never on labels, and this map
 * is the whole allow-list: anything missing — including `unknown`, undirected
 * endpoints and relations added to the domain later — is not traversable.
 *
 * `conversion` and `located_in` are file plumbing, so they do not consume the
 * user's depth budget; `contains_span` and `has_part` do, because they are the
 * steps that actually multiply into content.
 */
const DERIVATION_RULES: Partial<Record<RelationKind, DerivationRule>> = {
  conversion: { upstream: "into_relation", costsDepth: false },
  located_in: { upstream: "out_of_relation", costsDepth: false },
  contains_span: { upstream: "out_of_relation", costsDepth: true },
  has_part: { upstream: "into_relation", costsDepth: true },
};

export interface FocusOptions {
  depth: FocusDepth;
  /** Applied after traversal; the root stays visible regardless. */
  isEntityVisible: (node: IncidenceNode) => boolean;
}

export interface FocusResult {
  graph: IncidenceGraph;
  /** Entities reached by traversal, before the visibility filter. */
  reachedCount: number;
  /** Reached entities the filter removed and contraction folded away. */
  contractedCount: number;
}

interface RelationEntry {
  node: IncidenceNode;
  rule: DerivationRule | undefined;
  into: string[];
  out: string[];
  edges: IncidenceEdge[];
}

interface DerivationStep {
  to: string;
  relationId: string;
}

interface ContractedLink extends DerivationStep {
  contracted: boolean;
}

/**
 * Builds the derivation subgraph rooted at `rootId`.
 *
 * Traversal runs on the unfiltered projection and treats relation nodes as
 * transparent hyperedges, so entity → relation → entity counts as one step.
 * The visibility filter is applied afterwards and any path crossing a hidden
 * entity is contracted into a single synthetic relation, which is what keeps
 * "focus this PDF" meaningful while spans are hidden.
 */
export function focusSubgraph(
  graph: IncidenceGraph,
  rootId: string,
  options: FocusOptions,
): FocusResult {
  const nodesById = new Map(graph.nodes.map((node) => [node.id, node]));
  const { relations, incident } = indexRelations(graph, nodesById);

  const reached = traverse(rootId, relations, incident, options.depth);
  const steps = collectSteps(rootId, relations, incident, reached, options.depth);

  const visible = new Set<string>();
  for (const entityId of reached.keys()) {
    const node = nodesById.get(entityId);
    if (node === undefined) continue;
    if (entityId === rootId || options.isEntityVisible(node)) visible.add(entityId);
  }

  const keptRelations = new Map<string, RelationEntry>();
  for (const [relationId, entry] of relations) {
    const visibleEndpoints = [...entry.into, ...entry.out].filter((id) => visible.has(id));
    if (visibleEndpoints.length >= 2) keptRelations.set(relationId, entry);
  }

  const nodes: IncidenceNode[] = [];
  const edges: IncidenceEdge[] = [];
  for (const entityId of visible) {
    const node = nodesById.get(entityId);
    if (node !== undefined) nodes.push(node);
  }
  for (const [, entry] of keptRelations) {
    nodes.push(entry.node);
    for (const edge of entry.edges) {
      const entityId = edge.source === entry.node.id ? edge.target : edge.source;
      if (visible.has(entityId)) edges.push(edge);
    }
  }

  for (const from of visible) {
    for (const link of contractedLinks(from, steps, visible)) {
      if (!link.contracted && keptRelations.has(link.relationId)) continue;
      const label = link.contracted
        ? "派生自"
        : (relations.get(link.relationId)?.node.label ?? "派生自");
      appendContraction(nodes, edges, nodesById, from, link.to, label);
    }
  }

  return {
    graph: {
      schema_version: GRAPH_PROJECTION_SCHEMA_VERSION,
      lens: graph.lens,
      nodes,
      edges,
    },
    reachedCount: reached.size,
    contractedCount: reached.size - visible.size,
  };
}

function indexRelations(
  graph: IncidenceGraph,
  nodesById: Map<string, IncidenceNode>,
): { relations: Map<string, RelationEntry>; incident: Map<string, string[]> } {
  const relations = new Map<string, RelationEntry>();
  for (const node of graph.nodes) {
    if (node.element_kind !== "relation") continue;
    relations.set(node.id, {
      node,
      rule:
        node.relation_kind === null ? undefined : DERIVATION_RULES[node.relation_kind],
      into: [],
      out: [],
      edges: [],
    });
  }

  const incident = new Map<string, string[]>();
  for (const edge of graph.edges) {
    const relationId = relations.has(edge.source)
      ? edge.source
      : relations.has(edge.target)
        ? edge.target
        : null;
    if (relationId === null) continue;
    const entityId = relationId === edge.source ? edge.target : edge.source;
    if (!nodesById.has(entityId)) continue;
    const entry = relations.get(relationId)!;
    entry.edges.push(edge);
    // Undirected endpoints carry no derivation direction, so they are recorded
    // for rendering but never used as a traversal side.
    if (edge.direction === "into_relation") entry.into.push(entityId);
    else if (edge.direction === "out_of_relation") entry.out.push(entityId);
    const list = incident.get(entityId);
    if (list === undefined) incident.set(entityId, [relationId]);
    else list.push(relationId);
  }
  return { relations, incident };
}

function downstreamOf(
  entityId: string,
  entry: RelationEntry,
): string[] | null {
  if (entry.rule === undefined) return null;
  const upstream = entry.rule.upstream === "into_relation" ? entry.into : entry.out;
  if (!upstream.includes(entityId)) return null;
  return entry.rule.upstream === "into_relation" ? entry.out : entry.into;
}

function traverse(
  rootId: string,
  relations: Map<string, RelationEntry>,
  incident: Map<string, string[]>,
  depth: FocusDepth,
): Map<string, number> {
  const budget = depth === "all" ? Number.POSITIVE_INFINITY : depth;
  const reached = new Map<string, number>([[rootId, 0]]);
  const queue = [rootId];
  while (queue.length > 0) {
    const current = queue.shift()!;
    const spent = reached.get(current)!;
    for (const relationId of incident.get(current) ?? []) {
      const entry = relations.get(relationId)!;
      const downstream = downstreamOf(current, entry);
      if (downstream === null) continue;
      const next = spent + (entry.rule!.costsDepth ? 1 : 0);
      if (next > budget) continue;
      for (const target of downstream) {
        const known = reached.get(target);
        if (known !== undefined && known <= next) continue;
        reached.set(target, next);
        queue.push(target);
      }
    }
  }
  return reached;
}

function collectSteps(
  rootId: string,
  relations: Map<string, RelationEntry>,
  incident: Map<string, string[]>,
  reached: Map<string, number>,
  depth: FocusDepth,
): Map<string, DerivationStep[]> {
  const budget = depth === "all" ? Number.POSITIVE_INFINITY : depth;
  const steps = new Map<string, DerivationStep[]>();
  for (const [entityId, spent] of reached) {
    for (const relationId of incident.get(entityId) ?? []) {
      const entry = relations.get(relationId)!;
      const downstream = downstreamOf(entityId, entry);
      if (downstream === null) continue;
      if (spent + (entry.rule!.costsDepth ? 1 : 0) > budget) continue;
      for (const target of downstream) {
        if (!reached.has(target) || target === entityId) continue;
        const list = steps.get(entityId);
        const step = { to: target, relationId };
        if (list === undefined) steps.set(entityId, [step]);
        else if (!list.some((item) => item.to === target && item.relationId === relationId)) {
          list.push(step);
        }
      }
    }
  }
  return steps;
}

/**
 * Walks the derivation steps below `from` and stops at the first visible
 * descendant on each branch, so a chain of hidden intermediates collapses into
 * one link.
 */
function contractedLinks(
  from: string,
  steps: Map<string, DerivationStep[]>,
  visible: ReadonlySet<string>,
): ContractedLink[] {
  const links: ContractedLink[] = [];
  const emitted = new Set<string>();
  const walked = new Set<string>([from]);
  const stack: ContractedLink[] = (steps.get(from) ?? []).map((step) => ({
    ...step,
    contracted: false,
  }));
  while (stack.length > 0) {
    const item = stack.pop()!;
    if (visible.has(item.to)) {
      if (!emitted.has(item.to)) {
        emitted.add(item.to);
        links.push(item);
      }
      continue;
    }
    if (walked.has(item.to)) continue;
    walked.add(item.to);
    for (const step of steps.get(item.to) ?? []) {
      stack.push({ ...step, contracted: true });
    }
  }
  return links;
}

function appendContraction(
  nodes: IncidenceNode[],
  edges: IncidenceEdge[],
  nodesById: Map<string, IncidenceNode>,
  from: string,
  to: string,
  label: string,
): void {
  const relationId = `relation:focus-contracted/${from}~${to}`;
  nodes.push({
    id: relationId,
    element_kind: "relation",
    domain_id: null,
    domain_kind: FOCUS_CONTRACTION_KIND,
    relation_kind: null,
    layer: nodesById.get(from)?.layer ?? "source",
    label,
    summary: "已折叠的派生路径",
    lifecycle: null,
    parent: null,
    boundary: false,
  });
  edges.push(
    {
      id: `incidence:focus-contracted/${from}~${to}:from`,
      source: from,
      target: relationId,
      relation_id: relationId,
      role: "derived_from",
      ordinal: null,
      direction: "into_relation",
      directed: true,
    },
    {
      id: `incidence:focus-contracted/${from}~${to}:to`,
      source: relationId,
      target: to,
      relation_id: relationId,
      role: "derives",
      ordinal: null,
      direction: "out_of_relation",
      directed: true,
    },
  );
}
