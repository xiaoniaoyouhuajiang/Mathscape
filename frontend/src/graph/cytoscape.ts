import cytoscape, {
  type Core,
  type ElementDefinition,
  type StylesheetJson,
} from "cytoscape";

import {
  GRAPH_PROJECTION_SCHEMA_VERSION,
  type IncidenceDirection,
  type IncidenceEdge,
  type IncidenceGraph,
  type IncidenceNode,
} from "./projection";
import { EDGE_TYPES, NODE_TYPES, NODE_TYPE_ORDER, nodeTypeKey } from "./taxonomy";

export type RenderMode = "semantic" | "hyperedge";

export function toCytoscapeElements(
  graph: IncidenceGraph,
  renderMode: RenderMode = "semantic",
): ElementDefinition[] {
  if (graph.schema_version !== GRAPH_PROJECTION_SCHEMA_VERSION) {
    throw new Error(
      `unsupported graph projection schema: ${graph.schema_version}`,
    );
  }

  const useSemanticEdges = renderMode === "semantic";

  // Build relation-node lookup for semantic edge conversion.
  const relationNodesById = new Map<string, IncidenceNode>();
  const entityNodesById = new Map<string, IncidenceNode>();
  for (const node of graph.nodes) {
    if (node.element_kind === "relation") {
      relationNodesById.set(node.id, node);
    } else if (node.element_kind === "entity") {
      entityNodesById.set(node.id, node);
    }
  }

  // Nodes: always skip groups; also skip relation nodes in semantic mode.
  const nodes: ElementDefinition[] = graph.nodes
    .filter((node) => node.element_kind !== "group")
    .filter((node) => (useSemanticEdges ? node.element_kind !== "relation" : true))
    .map((node) => ({
      group: "nodes",
      classes: node.boundary ? "is-boundary" : undefined,
      data: {
        id: node.id,
        label: node.label,
        elementKind: node.element_kind,
        nodeType: nodeTypeKey(node),
        domainId: node.domain_id,
        domainKind: node.domain_kind,
        layer: node.layer,
        summary: node.summary,
        lifecycle: node.lifecycle,
        boundary: node.boundary,
      },
    }));

  let edges: ElementDefinition[];

  if (useSemanticEdges) {
    edges = buildSemanticEdges(
      graph.edges,
      relationNodesById,
      entityNodesById,
      nodes,
    );
  } else {
    edges = graph.edges.map((edge) => ({
      group: "edges",
      classes: edgeClasses(edge.directed),
      data: {
        id: edge.id,
        source: edge.source,
        target: edge.target,
        relationId: edge.relation_id,
        role: edge.role,
        ordinal: edge.ordinal,
        direction: edge.direction,
        directed: edge.directed,
      },
    }));
  }

  return [...nodes, ...edges];
}

// ---------------------------------------------------------------------------
// Semantic edge builder
// ---------------------------------------------------------------------------

/**
 * Convert relation-mediated incidence edges into direct entity-to-entity edges.
 *
 * Binary relations (exactly 2 entity endpoints) become a single labeled edge.
 * Multi-endpoint relations (≥3) use a tiny hidden junction node with spoke
 * edges, preserving the topology without the prominent diamond node.
 *
 * Junction nodes are pushed into `outNodes` so the caller's spread includes
 * them automatically.
 */
function buildSemanticEdges(
  incidenceEdges: IncidenceEdge[],
  relationNodes: Map<string, IncidenceNode>,
  entityNodes: Map<string, IncidenceNode>,
  outNodes: ElementDefinition[],
): ElementDefinition[] {
  // Group incidence edges by their owning relation.
  const byRelation = new Map<string, IncidenceEdge[]>();
  for (const edge of incidenceEdges) {
    const list = byRelation.get(edge.relation_id) ?? [];
    list.push(edge);
    byRelation.set(edge.relation_id, list);
  }

  const result: ElementDefinition[] = [];

  for (const [relationDomainId, relEdges] of byRelation) {
    // Resolve each incidence edge into an entity endpoint + relation metadata.
    const endpoints: Array<{
      entity: string;
      role: string;
      direction: IncidenceDirection;
      evidence: boolean;
    }> = [];
    let relationLabel = "";
    let relationSummary: string | null = null;
    let relationDomainKind: string | null = null;

    for (const edge of relEdges) {
      if (relationNodes.has(edge.source)) {
        // source is a relation node → target is the entity
        const rn = relationNodes.get(edge.source)!;
        endpoints.push({
          entity: edge.target,
          role: edge.role,
          direction: edge.direction,
          evidence: isEvidenceEndpoint(edge.role, entityNodes.get(edge.target)),
        });
        relationLabel = rn.label;
        relationSummary = rn.summary;
        relationDomainKind = rn.domain_kind;
      } else if (relationNodes.has(edge.target)) {
        // target is a relation node → source is the entity
        const rn = relationNodes.get(edge.target)!;
        endpoints.push({
          entity: edge.source,
          role: edge.role,
          direction: edge.direction,
          evidence: isEvidenceEndpoint(edge.role, entityNodes.get(edge.source)),
        });
        relationLabel = rn.label;
        relationSummary = rn.summary;
        relationDomainKind = rn.domain_kind;
      }
    }

    if (endpoints.length < 2) continue;

    const sources = endpoints.filter(
      (e) => e.direction === "into_relation" && !e.evidence,
    );
    const targets = endpoints.filter(
      (e) => e.direction === "out_of_relation" && !e.evidence,
    );
    const evidenceEndpoints = endpoints.filter((e) => e.evidence);
    const regularEndpoints = endpoints.filter((e) => !e.evidence);

    if (sources.length === 1 && targets.length === 1 && regularEndpoints.length === 2) {
      // Keep the mathematical source → target backbone direct even when the
      // relation has one or more evidence spans.
      result.push({
        group: "edges",
        classes: edgeClasses(true, true),
        data: {
          id: `sem~${relationDomainId}`,
          source: sources[0].entity,
          target: targets[0].entity,
          label: relationLabel,
          relationId: relationDomainId,
          domainKind: relationDomainKind,
          summary: relationSummary,
          directed: true,
          semantic: true,
        },
      });
      evidenceEndpoints.forEach((evidence, index) => {
        addEvidenceEdge(
          result,
          evidence.entity,
          targets[0].entity,
          relationDomainId,
          index,
        );
      });
      continue;
    }

    if (regularEndpoints.length === 1 && evidenceEndpoints.length === 1) {
      // A unit/span provenance pair has no mathematical source/target
      // direction. Show it as an explicit evidence edge.
      addEvidenceEdge(
        result,
        evidenceEndpoints[0].entity,
        regularEndpoints[0].entity,
        relationDomainId,
        0,
        relationLabel,
      );
      continue;
    }

    if (endpoints.length === 2) {
      // Binary relation without evidence: keep explicit direction, or fall
      // back to an undirected edge when the endpoint roles are ambiguous.
      const binarySources = endpoints.filter((e) => e.direction === "into_relation");
      const binaryTargets = endpoints.filter((e) => e.direction === "out_of_relation");

      let edgeSource: string;
      let edgeTarget: string;
      let directed: boolean;

      if (binarySources.length === 1 && binaryTargets.length === 1) {
        edgeSource = binarySources[0].entity;
        edgeTarget = binaryTargets[0].entity;
        directed = true;
      } else if (binarySources.length === 0 && binaryTargets.length === 0) {
        // Both undirected.
        edgeSource = endpoints[0].entity;
        edgeTarget = endpoints[1].entity;
        directed = false;
      } else {
        // Mixed or ambiguous directions: default to undirected to avoid fabricating arrows.
        edgeSource = endpoints[0].entity;
        edgeTarget = endpoints[1].entity;
        directed = false;
      }

      result.push({
        group: "edges",
        classes: edgeClasses(directed, true),
        data: {
          id: `sem~${relationDomainId}`,
          source: edgeSource,
          target: edgeTarget,
          label: relationLabel,
          relationId: relationDomainId,
          domainKind: relationDomainKind,
          summary: relationSummary,
          directed,
          semantic: true,
        },
      });
    } else {
      // Multi-endpoint relation: hidden junction node + spoke edges.
      const junctionId = `junc~${relationDomainId}`;
      outNodes.push({
        group: "nodes",
        data: {
          id: junctionId,
          label: relationLabel,
          elementKind: "junction",
          domainId: relationDomainId,
          domainKind: relationDomainKind,
          summary: relationSummary,
        },
      });

      for (let i = 0; i < endpoints.length; i++) {
        const ep = endpoints[i];
        const isInto = ep.direction === "into_relation";
        result.push({
          group: "edges",
          classes: edgeClasses(ep.direction !== "undirected", true),
          data: {
            id: `sem~${relationDomainId}~${i}`,
            source: isInto ? ep.entity : junctionId,
            target: isInto ? junctionId : ep.entity,
            label: ep.role,
            relationId: relationDomainId,
            directed: ep.direction !== "undirected",
            semantic: true,
          },
        });
      }
    }
  }

  return result;
}

function isEvidenceEndpoint(
  role: string,
  node: IncidenceNode | undefined,
): boolean {
  return role === "evidence" || role === "span" || node?.domain_kind === "SourceSpan";
}

function addEvidenceEdge(
  result: ElementDefinition[],
  evidence: string,
  anchor: string,
  relationDomainId: string,
  ordinal: number,
  label = "证据",
): void {
  result.push({
    group: "edges",
    classes: `${edgeClasses(false, true)} evidence-edge`,
    data: {
      id: `evidence~${relationDomainId}~${ordinal}`,
      source: evidence,
      target: anchor,
      label,
      relationId: relationDomainId,
      evidence: true,
      directed: false,
      semantic: true,
    },
  });
}

function edgeClasses(directed: boolean, semantic = false): string {
  const directionClass = directed ? "is-directed" : "is-undirected";
  return semantic ? `${directionClass} semantic-edge` : directionClass;
}

// ---------------------------------------------------------------------------
// Cytoscape stylesheet
// ---------------------------------------------------------------------------

const nodeTypeStyles: StylesheetJson = NODE_TYPE_ORDER.map((key) => {
  const visual = NODE_TYPES[key];
  return {
    selector: `node[nodeType = "${key}"]`,
    style: {
      shape: visual.shape,
      width: `${visual.width}px`,
      height: `${visual.height}px`,
      "background-color": visual.background,
      "background-opacity": visual.backgroundOpacity,
      "border-color": visual.border,
      "border-style": visual.borderStyle,
      "border-width": 1.5,
      "font-size": `${visual.fontSize}px`,
    },
  };
});

const styles: StylesheetJson = [
  {
    selector: "node",
    style: {
      label: "data(label)",
      "text-wrap": "wrap",
      "text-max-width": "150px",
      "font-size": "11px",
      "background-color": "#59918b",
      "border-width": 1,
      "border-color": "#8bbab4",
      color: "#dbe7e9",
      "text-background-color": "#0c131a",
      "text-background-opacity": 0.82,
      "text-background-padding": "3px",
    },
  },
  ...nodeTypeStyles,
  {
    selector: 'node[elementKind = "junction"]',
    style: {
      shape: "ellipse",
      width: "12px",
      height: "12px",
      "background-color": "#b47b49",
      "background-opacity": 0.6,
      "border-width": 0,
      label: "",
    },
  },
  {
    selector: "node.is-boundary",
    style: {
      "background-opacity": 0.45,
      "border-style": "dashed",
      "border-width": 2,
    },
  },
  {
    selector: "edge",
    style: {
      label: "data(role)",
      width: 1.5,
      "curve-style": "bezier",
      "line-color": EDGE_TYPES.incidence.color,
      "target-arrow-color": EDGE_TYPES.incidence.color,
      "target-arrow-shape": "triangle",
      "font-size": "9px",
      color: "#9aa9b4",
      "text-rotation": "autorotate",
    },
  },
  {
    selector: "edge.semantic-edge",
    style: {
      label: "data(label)",
      "line-color": EDGE_TYPES.relation.color,
      "target-arrow-color": EDGE_TYPES.relation.color,
      color: "#c9a87a",
    },
  },
  {
    selector: "edge.evidence-edge",
    style: {
      label: "data(label)",
      width: 1.2,
      "line-style": "dashed",
      "line-color": EDGE_TYPES.evidence.color,
      "target-arrow-shape": "none",
      color: "#9db2aa",
      "font-size": "8px",
    },
  },
  {
    selector: "edge.is-directed",
    style: {
      width: 2.2,
      "target-arrow-shape": "triangle",
      "target-arrow-color": "#f0c27d",
      "arrow-scale": 1.55,
    },
  },
  {
    selector: "edge.is-undirected",
    style: {
      "target-arrow-shape": "none",
      "line-style": "dashed",
    },
  },
];

// ---------------------------------------------------------------------------
// Graph mounting
// ---------------------------------------------------------------------------

export function mountCytoscapeGraph(
  container: HTMLElement,
  graph: IncidenceGraph,
  renderMode: RenderMode = "semantic",
): Core {
  return cytoscape({
    container,
    elements: toCytoscapeElements(graph, renderMode),
    style: styles,
    layout: {
      name: "cose",
      animate: false,
      fit: true,
      padding: 24,
    },
    minZoom: 0.08,
    maxZoom: 4,
  });
}
