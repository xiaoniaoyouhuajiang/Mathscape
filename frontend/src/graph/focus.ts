import {
  GRAPH_PROJECTION_SCHEMA_VERSION,
  relationHopClass,
  type HopClass,
  type IncidenceDirection,
  type IncidenceEdge,
  type IncidenceGraph,
  type IncidenceNode,
  type RelationKind,
} from "./projection";

export type FocusDepth = 1 | 2 | "all";
export type FocusLens = "heuristic" | "structural" | "semantic_only";

/**
 * Marks the synthetic relations this module creates when a derivation path had
 * to be contracted across nodes the user filtered out.
 */
export const FOCUS_CONTRACTION_KIND = "FocusContraction";

/** Semantic hop cap when `depth === "all"` so cross-book bridges stay bounded. */
const ALL_SEMANTIC_HOP_CAP = 3;

export interface FocusOptions {
  depth: FocusDepth;
  lens?: FocusLens;
  /** Applied after traversal; seed entities stay visible regardless. */
  isEntityVisible: (node: IncidenceNode) => boolean;
}

export interface FocusResult {
  graph: IncidenceGraph;
  /** Entities reached by traversal, before the visibility filter. */
  reachedCount: number;
  /** Reached entities the filter removed and contraction folded away. */
  contractedCount: number;
}

/**
 * Capability state attached to each reached node in the typed-hop automaton
 * (mirrors `crates/mathscape-mcp/src/tools/graph.rs` `NodeMode`).
 */
const enum NodeMode {
  /** Absorbed container/context endpoint (non-seed Theory, terminal Artifact, DialectScope). */
  Sink = 0,
  /** Upward provenance chain (`unit -> span -> markdown -> pdf`): never bounces back down. */
  ProvenanceUp = 1,
  /** Parent section reached upward (`child <-has_part- section`): attaches conventions without sibling fanout. */
  SectionContext = 2,
  /** Downward file plumbing from an Artifact seed (`pdf -> markdown <- span`). */
  PlumbingDown = 3,
  /** Downward `has_part` subtree of a root material/unit under `depth = "all"`. */
  RootSubtree = 4,
  /** Active node eligible for all hops permitted by the active lens. */
  Full = 5,
}

interface EndpointEntry {
  entityId: string;
  direction: IncidenceDirection;
  role: string;
}

interface RelationEntry {
  node: IncidenceNode;
  hopClass: HopClass;
  endpoints: EndpointEntry[];
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

interface VisitState {
  depth: number;
  mode: NodeMode;
}

function isTheoryEntity(node: IncidenceNode | undefined): boolean {
  return (
    node?.element_kind === "entity" &&
    node.domain_kind === "SemanticEntity" &&
    Boolean(node.summary?.startsWith("理论"))
  );
}

function isEvidenceRole(role: string, node: IncidenceNode | undefined): boolean {
  return role === "evidence" || role === "span" || node?.domain_kind === "SourceSpan";
}

/**
 * Builds the typed-hop focus subgraph rooted at `rootId` under `options.lens`
 * (default `"heuristic"`).
 *
 * Traversal runs on the unfiltered projection using the shared `HopClass`
 * transition matrix (`Plumbing`, `ContextAttach`, `Hierarchy`,
 * `SourceReasoning`, `SemanticBridge`, `ContainerHub`), then applies the
 * visibility filter and contracts hidden provenance/hierarchy intermediates
 * (such as filtered `SourceSpan`s) into synthetic `"派生自"` links.
 */
export function focusSubgraph(
  graph: IncidenceGraph,
  rootId: string,
  options: FocusOptions,
): FocusResult {
  const lens: FocusLens = options.lens ?? "heuristic";
  const nodesById = new Map(graph.nodes.map((node) => [node.id, node]));
  const { relations, incident } = indexRelations(graph, nodesById);

  const rootNode = nodesById.get(rootId);
  const seedIds = new Set<string>();
  let forcedRelationId: string | null = null;

  if (rootNode?.element_kind === "relation") {
    forcedRelationId = rootNode.id;
    const rel = relations.get(rootNode.id);
    for (const ep of rel?.endpoints ?? []) {
      const target = nodesById.get(ep.entityId);
      if (target && !isEvidenceRole(ep.role, target)) {
        seedIds.add(ep.entityId);
      }
    }
    if (seedIds.size === 0) {
      for (const ep of rel?.endpoints ?? []) seedIds.add(ep.entityId);
    }
  } else if (rootNode?.element_kind === "entity") {
    seedIds.add(rootId);
  } else {
    seedIds.add(rootId);
  }

  const reached = traverseTypedHops(
    seedIds,
    nodesById,
    relations,
    incident,
    options.depth,
    lens,
  );
  const steps = collectContractionSteps(relations, reached);

  const visible = new Set<string>();
  for (const entityId of reached.keys()) {
    const node = nodesById.get(entityId);
    if (node === undefined) continue;
    if (seedIds.has(entityId) || options.isEntityVisible(node)) {
      visible.add(entityId);
    }
  }

  const keptRelations = new Map<string, RelationEntry>();
  for (const [relationId, entry] of relations) {
    if (lens === "semantic_only" && !isSemanticLensRelation(entry.node.relation_kind)) {
      if (relationId !== forcedRelationId) continue;
    }
    if (lens === "structural" && !isStructuralLensRelation(entry.node.relation_kind)) {
      if (relationId !== forcedRelationId) continue;
    }
    const nonEvidenceVisible = entry.endpoints.filter(
      (ep) => !isEvidenceRole(ep.role, nodesById.get(ep.entityId)) && visible.has(ep.entityId),
    );
    const totalVisible = entry.endpoints.filter((ep) => visible.has(ep.entityId));
    if (
      relationId === forcedRelationId ||
      nonEvidenceVisible.length >= 2 ||
      (entry.endpoints.length === 2 && totalVisible.length === 2)
    ) {
      keptRelations.set(relationId, entry);
    }
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

  const visibleParentsOf = new Map<string, string[]>();
  for (const [, entry] of keptRelations) {
    if (entry.node.relation_kind !== "has_part") continue;
    const parents = entry.into.filter((p) => visible.has(p));
    if (parents.length === 0) continue;
    for (const c of entry.out) {
      if (!visible.has(c)) continue;
      const list = visibleParentsOf.get(c);
      if (list === undefined) visibleParentsOf.set(c, [...parents]);
      else {
        for (const p of parents) {
          if (!list.includes(p)) list.push(p);
        }
      }
    }
  }

  for (const from of visible) {
    const fromNode = nodesById.get(from);
    const isArtifact = fromNode?.domain_kind === "SourceArtifact";
    const isNonSeedArtifact = isArtifact && !seedIds.has(from);
    const candidateLinks = contractedLinks(from, steps, visible).filter(
      (link) => !(!link.contracted && keptRelations.has(link.relationId)),
    );
    const directTargets = isArtifact
      ? new Set(candidateLinks.map((link) => link.to))
      : null;
    const hasLinkedAncestor = (targetId: string): boolean => {
      if (directTargets === null) return false;
      const seen = new Set<string>([targetId]);
      const queue = [...(visibleParentsOf.get(targetId) ?? [])];
      while (queue.length > 0) {
        const anc = queue.pop()!;
        if (seen.has(anc)) continue;
        seen.add(anc);
        if (directTargets.has(anc)) return true;
        for (const grand of visibleParentsOf.get(anc) ?? []) {
          queue.push(grand);
        }
      }
      return false;
    };

    let rawLinks = candidateLinks.filter((link) => !hasLinkedAncestor(link.to));
    if (isNonSeedArtifact && rawLinks.length > 1) {
      const minDepth = Math.min(
        ...rawLinks.map((l) => reached.get(l.to)?.depth ?? Number.POSITIVE_INFINITY),
      );
      rawLinks = rawLinks.filter(
        (l) => (reached.get(l.to)?.depth ?? Number.POSITIVE_INFINITY) <= minDepth,
      );
    }
    for (const link of rawLinks) {
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

function isStructuralLensRelation(kind: RelationKind | null): boolean {
  const hop = relationHopClass(kind);
  return (
    hop === "plumbing" ||
    hop === "hierarchy" ||
    hop === "source_reasoning" ||
    kind === "scope_of"
  );
}

function isSemanticLensRelation(kind: RelationKind | null): boolean {
  const hop = relationHopClass(kind);
  return hop === "semantic_bridge" || hop === "container_hub" || kind === "debt_on";
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
      hopClass: relationHopClass(node.relation_kind),
      endpoints: [],
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
    entry.endpoints.push({
      entityId,
      direction: edge.direction,
      role: edge.role,
    });
    if (edge.direction === "into_relation") entry.into.push(entityId);
    else if (edge.direction === "out_of_relation") entry.out.push(entityId);
    const list = incident.get(entityId);
    if (list === undefined) incident.set(entityId, [relationId]);
    else if (!list.includes(relationId)) list.push(relationId);
  }
  return { relations, incident };
}

function evaluateTypedHop(
  lens: FocusLens,
  depthSetting: FocusDepth,
  mode: NodeMode,
  currentNode: IncidenceNode,
  currentIsSeed: boolean,
  entry: RelationEntry,
  own: EndpointEntry,
  other: EndpointEntry,
  targetNode: IncidenceNode,
): { cost: number; nextMode: NodeMode } | null {
  const relKind = entry.node.relation_kind;
  const hopClass = entry.hopClass;
  if (relKind === null || hopClass === "forbidden") return null;

  // Never walk across evidence-span side-spokes of a content relation unless the relation itself is contains_span / located_in.
  if (
    relKind !== "contains_span" &&
    relKind !== "located_in" &&
    (isEvidenceRole(own.role, currentNode) || isEvidenceRole(other.role, targetNode))
  ) {
    return null;
  }

  const forward =
    own.direction === "into_relation" && other.direction === "out_of_relation";
  const backward =
    own.direction === "out_of_relation" && other.direction === "into_relation";
  const targetIsTheory = isTheoryEntity(targetNode);

  switch (mode) {
    case NodeMode.Sink:
      return null;

    case NodeMode.ProvenanceUp:
      if (relKind === "located_in" && forward) {
        return { cost: 0, nextMode: NodeMode.ProvenanceUp };
      }
      if (relKind === "conversion" && backward) {
        return { cost: 0, nextMode: NodeMode.Sink };
      }
      return null;

    case NodeMode.SectionContext:
      if (relKind === "scope_of" && backward) {
        return { cost: 0, nextMode: NodeMode.Sink };
      }
      if (relKind === "has_part" && backward) {
        return { cost: 0, nextMode: NodeMode.SectionContext };
      }
      if (relKind === "contains_span" && forward) {
        return { cost: 0, nextMode: NodeMode.ProvenanceUp };
      }
      return null;

    case NodeMode.PlumbingDown:
      if (relKind === "conversion" && forward) {
        return { cost: 0, nextMode: NodeMode.PlumbingDown };
      }
      if (relKind === "located_in" && backward) {
        return { cost: 0, nextMode: NodeMode.PlumbingDown };
      }
      if (relKind === "contains_span" && backward) {
        const nextMode = depthSetting === "all" ? NodeMode.RootSubtree : NodeMode.Full;
        return { cost: 1, nextMode };
      }
      return null;

    case NodeMode.RootSubtree:
    case NodeMode.Full: {
      if (lens === "semantic_only") {
        if (hopClass === "semantic_bridge") {
          if (relKind === "aligns" && targetNode.domain_kind === "SourceSemanticUnit") {
            return null;
          }
          if (currentNode.domain_kind === "SourceSemanticUnit" && !currentIsSeed) {
            return null;
          }
          return { cost: 1, nextMode: targetIsTheory ? NodeMode.Sink : NodeMode.Full };
        }
        if (hopClass === "context_attach" && relKind === "debt_on") {
          return { cost: 0, nextMode: targetIsTheory ? NodeMode.Sink : NodeMode.Full };
        }
        if (hopClass === "container_hub") {
          if (forward && targetIsTheory) return { cost: 0, nextMode: NodeMode.Sink };
          if (backward && currentIsSeed) return { cost: 1, nextMode: NodeMode.Full };
        }
        return null;
      }

      // Heuristic & Structural lenses
      switch (hopClass) {
        case "plumbing":
          if (relKind === "conversion" && forward && currentNode.domain_kind === "SourceArtifact") {
            return { cost: 0, nextMode: NodeMode.PlumbingDown };
          }
          if (relKind === "located_in" && backward && currentNode.domain_kind === "SourceArtifact") {
            return { cost: 0, nextMode: NodeMode.PlumbingDown };
          }
          if (relKind === "contains_span" && backward && currentNode.domain_kind === "SourceSpan") {
            const nextMode =
              depthSetting === "all" && currentIsSeed ? NodeMode.RootSubtree : NodeMode.Full;
            return { cost: 1, nextMode };
          }
          // Upward provenance attachment (0 cost, never bounces back down into other units)
          if (relKind === "contains_span" && forward) {
            return { cost: 0, nextMode: NodeMode.ProvenanceUp };
          }
          if (relKind === "located_in" && forward) {
            return { cost: 0, nextMode: NodeMode.ProvenanceUp };
          }
          if (relKind === "conversion" && backward) {
            return { cost: 0, nextMode: NodeMode.Sink };
          }
          return null;

        case "context_attach":
          if (relKind === "scope_of") {
            if (backward) return { cost: 0, nextMode: NodeMode.Sink };
            if (currentIsSeed) return { cost: 1, nextMode: NodeMode.Full };
            return null;
          }
          if (relKind === "debt_on" && lens === "heuristic") {
            const cost = backward ? 0 : 1;
            return { cost, nextMode: targetIsTheory ? NodeMode.Sink : NodeMode.Full };
          }
          return null;

        case "hierarchy":
          if (relKind === "has_part") {
            if (forward) {
              // Under `depth === "all"`, descending the seed's own `has_part` tree costs 0 beyond the first step so the entire book/section unfolds cleanly.
              const inSeedSubtree =
                depthSetting === "all" && (currentIsSeed || mode === NodeMode.RootSubtree);
              return {
                cost: inSeedSubtree && mode === NodeMode.RootSubtree ? 0 : 1,
                nextMode: inSeedSubtree ? NodeMode.RootSubtree : NodeMode.Full,
              };
            }
            if (backward) {
              return { cost: 0, nextMode: NodeMode.SectionContext };
            }
          }
          return null;

        case "source_reasoning":
          return { cost: 1, nextMode: NodeMode.Full };

        case "semantic_bridge":
          if (lens !== "heuristic") return null;
          return { cost: 1, nextMode: targetIsTheory ? NodeMode.Sink : NodeMode.Full };

        case "container_hub":
          if (lens !== "heuristic") return null;
          if (forward && targetIsTheory) {
            return { cost: 0, nextMode: NodeMode.Sink };
          }
          if (backward && currentIsSeed) {
            return { cost: 1, nextMode: NodeMode.Full };
          }
          return null;

        default:
          return null;
      }
    }
  }
}

function traverseTypedHops(
  seedIds: ReadonlySet<string>,
  nodesById: Map<string, IncidenceNode>,
  relations: Map<string, RelationEntry>,
  incident: Map<string, string[]>,
  depthSetting: FocusDepth,
  lens: FocusLens,
): Map<string, VisitState> {
  const budget =
    depthSetting === "all"
      ? lens === "structural"
        ? Number.POSITIVE_INFINITY
        : ALL_SEMANTIC_HOP_CAP
      : depthSetting;

  const reached = new Map<string, VisitState>();
  const deque: Array<{ id: string; depth: number; mode: NodeMode }> = [];

  for (const seedId of seedIds) {
    if (!nodesById.has(seedId)) continue;
    const initialMode =
      depthSetting === "all" &&
      (nodesById.get(seedId)?.domain_kind === "SourceArtifact" ||
        nodesById.get(seedId)?.domain_kind === "SourceSemanticUnit")
        ? NodeMode.RootSubtree
        : NodeMode.Full;
    reached.set(seedId, { depth: 0, mode: initialMode });
    deque.push({ id: seedId, depth: 0, mode: initialMode });
  }

  while (deque.length > 0) {
    const item = deque.shift()!;
    const currentVisit = reached.get(item.id);
    if (
      currentVisit === undefined ||
      item.depth > currentVisit.depth ||
      item.mode < currentVisit.mode ||
      currentVisit.mode === NodeMode.Sink
    ) {
      continue;
    }
    const currentNode = nodesById.get(item.id);
    if (currentNode === undefined) continue;
    const currentIsSeed = seedIds.has(item.id);

    for (const relationId of incident.get(item.id) ?? []) {
      const entry = relations.get(relationId);
      if (entry === undefined) continue;

      for (const own of entry.endpoints) {
        if (own.entityId !== item.id) continue;
        for (const other of entry.endpoints) {
          if (other.entityId === item.id) continue;
          const targetNode = nodesById.get(other.entityId);
          if (targetNode === undefined) continue;

          const step = evaluateTypedHop(
            lens,
            depthSetting,
            currentVisit.mode,
            currentNode,
            currentIsSeed,
            entry,
            own,
            other,
            targetNode,
          );
          if (step === null) continue;

          const nextDepth = currentVisit.depth + step.cost;
          if (nextDepth > budget) continue;

          const known = reached.get(other.entityId);
          if (known !== undefined) {
            const betterDepth = nextDepth < known.depth;
            const betterMode = step.nextMode > known.mode;
            if (!betterDepth && !betterMode) continue;
            const merged: VisitState = {
              depth: Math.min(known.depth, nextDepth),
              mode: Math.max(known.mode, step.nextMode) as NodeMode,
            };
            reached.set(other.entityId, merged);
            if (step.cost === 0) deque.unshift({ id: other.entityId, ...merged });
            else deque.push({ id: other.entityId, ...merged });
          } else {
            const fresh: VisitState = { depth: nextDepth, mode: step.nextMode };
            reached.set(other.entityId, fresh);
            if (step.cost === 0) deque.unshift({ id: other.entityId, ...fresh });
            else deque.push({ id: other.entityId, ...fresh });
          }
        }
      }
    }
  }

  return reached;
}

/**
 * Canonical top-down provenance/hierarchy steps (`PDF -> Markdown -> Span -> Unit -> Child`)
 * among reached nodes so hidden intermediates (like `SourceSpan`) fold cleanly
 * into synthetic `"派生自"` edges regardless of whether traversal walked down or up.
 */
const CONTRACTION_UPSTREAM: Partial<Record<RelationKind, "into_relation" | "out_of_relation">> = {
  conversion: "into_relation",
  located_in: "out_of_relation",
  contains_span: "out_of_relation",
  has_part: "into_relation",
};

function collectContractionSteps(
  relations: Map<string, RelationEntry>,
  reached: ReadonlyMap<string, VisitState>,
): Map<string, DerivationStep[]> {
  const steps = new Map<string, DerivationStep[]>();
  for (const [relationId, entry] of relations) {
    const upstreamSide =
      entry.node.relation_kind === null
        ? undefined
        : CONTRACTION_UPSTREAM[entry.node.relation_kind];
    if (upstreamSide === undefined) continue;
    const upstreams = upstreamSide === "into_relation" ? entry.into : entry.out;
    const downstreams = upstreamSide === "into_relation" ? entry.out : entry.into;
    for (const from of upstreams) {
      if (!reached.has(from)) continue;
      for (const to of downstreams) {
        if (!reached.has(to) || to === from) continue;
        const list = steps.get(from);
        const step = { to, relationId };
        if (list === undefined) steps.set(from, [step]);
        else if (!list.some((item) => item.to === to && item.relationId === relationId)) {
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
