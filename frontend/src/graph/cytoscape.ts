import cytoscape, {
  type Core,
  type ElementDefinition,
  type StylesheetJson,
} from "cytoscape";

import {
  GRAPH_PROJECTION_SCHEMA_VERSION,
  type IncidenceGraph,
} from "./projection";

export function toCytoscapeElements(
  graph: IncidenceGraph,
): ElementDefinition[] {
  if (graph.schema_version !== GRAPH_PROJECTION_SCHEMA_VERSION) {
    throw new Error(
      `unsupported graph projection schema: ${graph.schema_version}`,
    );
  }

  const nodes: ElementDefinition[] = graph.nodes.map((node) => ({
    group: "nodes",
    data: {
      id: node.id,
      label: node.label,
      elementKind: node.element_kind,
      domainId: node.domain_id,
      domainKind: node.domain_kind,
      layer: node.layer,
      summary: node.summary,
      lifecycle: node.lifecycle,
      boundary: node.boundary,
      ...(node.parent === null ? {} : { parent: node.parent }),
    },
  }));

  const edges: ElementDefinition[] = graph.edges.map((edge) => ({
    group: "edges",
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

  return [...nodes, ...edges];
}

const styles: StylesheetJson = [
  {
    selector: "node",
    style: {
      label: "data(label)",
      "text-wrap": "wrap",
      "text-max-width": "150px",
      "font-size": "11px",
      "background-color": "#4263a8",
      color: "#182033",
    },
  },
  {
    selector: 'node[elementKind = "relation"]',
    style: {
      shape: "diamond",
      width: "34px",
      height: "34px",
      "background-color": "#b36b35",
    },
  },
  {
    selector: 'node[elementKind = "group"]',
    style: {
      shape: "round-rectangle",
      "background-opacity": 0.08,
      "border-width": 1,
      "border-color": "#8c96a8",
      "text-valign": "top",
      "text-halign": "center",
      padding: "20px",
    },
  },
  {
    selector: "node[boundary = true]",
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
      "line-color": "#8791a3",
      "target-arrow-color": "#8791a3",
      "target-arrow-shape": "triangle",
      "font-size": "9px",
      "text-rotation": "autorotate",
    },
  },
  {
    selector: "edge[directed = false]",
    style: {
      "target-arrow-shape": "none",
      "line-style": "dashed",
    },
  },
];

export function mountCytoscapeGraph(
  container: HTMLElement,
  graph: IncidenceGraph,
): Core {
  return cytoscape({
    container,
    elements: toCytoscapeElements(graph),
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
