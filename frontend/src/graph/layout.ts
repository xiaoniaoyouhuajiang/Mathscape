/**
 * Deterministic organic layout for the landscape graph.
 *
 * The visible graph is a projection of a larger incidence graph.  Layout is
 * therefore computed from the full graph once, then filtered views reuse the
 * same model coordinates.  A headless Cytoscape `cose` run supplies the
 * organic force-directed shape; stable ordering and stable seeds remove its
 * dependence on projection insertion order.
 */
import cytoscape from "cytoscape";
import type { IncidenceEdge, IncidenceGraph, IncidenceNode, RelationKind } from "./projection";
import { nodeTypeKey, type NodeTypeKey } from "./taxonomy";

export interface MapPoint {
  x: number;
  y: number;
}

interface Endpoint {
  entity: string;
  role: string;
  direction: "into_relation" | "out_of_relation" | "undirected";
}

interface ParsedRelation {
  domainId: string;
  relationNodeId: string;
  domainKind: string | null;
  relationKind: RelationKind | null;
  endpoints: Endpoint[];
}

interface LayoutNodeData {
  id: string;
  label: string;
  domainKind: string | null;
  nodeType: string;
  junction?: boolean;
}

interface LayoutEdgeData {
  id: string;
  source: string;
  target: string;
  relationKind: RelationKind | null;
  domainKind: string | null;
}

const AUXILIARY_ARTIFACT_TYPES: ReadonlySet<NodeTypeKey> = new Set([
  "parse_metadata",
  "parse_image",
  "attachment",
]);

const SATELLITE_MIN_DX = 74;
const SATELLITE_MIN_DY = 46;
const GOLDEN_ANGLE = Math.PI * (3 - Math.sqrt(5));
const TAU = Math.PI * 2;

export function isAuxiliaryArtifactNode(node: IncidenceNode): boolean {
  return node.domain_kind === "SourceArtifact" && AUXILIARY_ARTIFACT_TYPES.has(nodeTypeKey(node));
}

export function isSatelliteNode(node: IncidenceNode): boolean {
  return node.domain_kind === "SourceSpan" || isAuxiliaryArtifactNode(node);
}

function compareIds(a: string, b: string): number {
  return a.localeCompare(b, "zh-Hans-CN", { numeric: true }) || a.localeCompare(b);
}

function stableHash(value: string): number {
  let hash = 2166136261;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
}

function parseRelations(graph: IncidenceGraph): {
  entities: Map<string, IncidenceNode>;
  relations: ParsedRelation[];
} {
  const entities = new Map<string, IncidenceNode>();
  const relationNodes = new Map<string, IncidenceNode>();
  for (const node of graph.nodes) {
    if (node.element_kind === "entity") entities.set(node.id, node);
    if (node.element_kind === "relation") relationNodes.set(node.id, node);
  }

  const byRelation = new Map<string, IncidenceEdge[]>();
  for (const edge of graph.edges) {
    const list = byRelation.get(edge.relation_id) ?? [];
    list.push(edge);
    byRelation.set(edge.relation_id, list);
  }

  const relations: ParsedRelation[] = [];
  for (const [domainId, edges] of byRelation) {
    const relationNode = edges
      .map((edge) => relationNodes.get(edge.source) ?? relationNodes.get(edge.target))
      .find((node): node is IncidenceNode => node !== undefined);
    if (relationNode === undefined) continue;

    const endpoints: Endpoint[] = [];
    const seen = new Set<string>();
    for (const edge of edges) {
      let entity: string | undefined;
      if (edge.source === relationNode.id) entity = edge.target;
      else if (edge.target === relationNode.id) entity = edge.source;
      if (entity === undefined || !entities.has(entity)) continue;
      const key = `${entity}:${edge.direction}:${edge.role}`;
      if (seen.has(key)) continue;
      seen.add(key);
      endpoints.push({ entity, role: edge.role, direction: edge.direction });
    }
    if (endpoints.length < 2) continue;

    relations.push({
      domainId,
      relationNodeId: relationNode.id,
      domainKind: relationNode.domain_kind,
      relationKind: relationNode.relation_kind,
      endpoints,
    });
  }

  relations.sort((a, b) => compareIds(a.domainId, b.domainId));
  return { entities, relations };
}

function uniqueEntities(endpoints: readonly Endpoint[]): string[] {
  return [...new Set(endpoints.map((endpoint) => endpoint.entity))].sort(compareIds);
}

function isMainNode(node: IncidenceNode): boolean {
  return node.element_kind === "entity" && !isSatelliteNode(node);
}

function relationIdealLength(relationKind: RelationKind | null): number {
  switch (relationKind) {
    case "has_part":
      return 92;
    case "conversion":
      return 112;
    case "aligns":
      return 168;
    case "in_theory":
      return 116;
    case "debt_on":
    case "scope_of":
      return 104;
    case "proves":
    case "solves":
    case "hints":
    case "explains":
      return 132;
    case "cites":
      return 188;
    default:
      return 128;
  }
}

function nodeRepulsion(nodeType: string): number {
  switch (nodeType) {
    case "shared_theory":
      return 5200;
    case "shared_entity":
      return 3600;
    case "debt":
    case "dialect_scope":
      return 2600;
    case "pdf":
    case "markdown":
    case "ink_snapshot":
      return 3200;
    case "semantic_unit":
      return 2300;
    default:
      return 2500;
  }
}

function seedRadius(nodeType: string): number {
  switch (nodeType) {
    case "shared_theory":
      return 120;
    case "shared_entity":
      return 210;
    case "debt":
    case "dialect_scope":
      return 260;
    case "pdf":
    case "markdown":
    case "ink_snapshot":
      return 520;
    case "semantic_unit":
      return 680;
    default:
      return 760;
  }
}

function seedPoint(node: LayoutNodeData, index: number): MapPoint {
  const hash = stableHash(node.id);
  const hashAngle = ((hash % 10000) / 10000) * TAU;
  const angle = index * GOLDEN_ANGLE + hashAngle * 0.17;
  const radius = seedRadius(node.nodeType) + Math.sqrt(index + 1) * 24 + (hash % 37);
  const eccentricity = 0.78 + ((hash >>> 8) % 29) / 100;
  return {
    x: Math.cos(angle) * radius,
    y: Math.sin(angle) * radius * eccentricity,
  };
}

function buildLayoutGraph(
  entities: ReadonlyMap<string, IncidenceNode>,
  relations: readonly ParsedRelation[],
): {
  nodes: LayoutNodeData[];
  edges: LayoutEdgeData[];
} {
  const nodes: LayoutNodeData[] = [...entities.values()]
    .filter(isMainNode)
    .sort((a, b) => compareIds(a.id, b.id))
    .map((node) => ({
      id: node.id,
      label: node.label,
      domainKind: node.domain_kind,
      nodeType: nodeTypeKey(node),
    }));
  const nodeIds = new Set(nodes.map((node) => node.id));
  const edges: LayoutEdgeData[] = [];

  for (const relation of relations) {
    const endpoints = uniqueEntities(relation.endpoints).filter((id) => nodeIds.has(id));
    if (endpoints.length < 2) continue;

    if (endpoints.length === 2) {
      edges.push({
        id: `layout-edge~${relation.domainId}`,
        source: endpoints[0],
        target: endpoints[1],
        relationKind: relation.relationKind,
        domainKind: relation.domainKind,
      });
      continue;
    }

    const junctionId = `layout-junc~${relation.domainId}`;
    nodes.push({
      id: junctionId,
      label: "",
      domainKind: relation.domainKind,
      nodeType: "layout_junction",
      junction: true,
    });
    for (const endpoint of endpoints) {
      edges.push({
        id: `layout-edge~${relation.domainId}~${endpoint}`,
        source: endpoint,
        target: junctionId,
        relationKind: relation.relationKind,
        domainKind: relation.domainKind,
      });
    }
  }

  nodes.sort((a, b) => compareIds(a.id, b.id));
  edges.sort((a, b) => compareIds(a.id, b.id));
  return { nodes, edges };
}

function runOrganicLayout(nodes: readonly LayoutNodeData[], edges: readonly LayoutEdgeData[]): Map<string, MapPoint> {
  if (nodes.length === 0) return new Map();

  const seedPositions = new Map<string, MapPoint>();
  nodes.forEach((node, index) => seedPositions.set(node.id, seedPoint(node, index)));

  const elements = [
    ...nodes.map((node) => ({
      group: "nodes" as const,
      data: {
        id: node.id,
        label: node.label,
        nodeType: node.nodeType,
        domainKind: node.domainKind,
        junction: node.junction === true,
      },
      position: seedPositions.get(node.id),
    })),
    ...edges.map((edge) => ({
      group: "edges" as const,
      data: {
        id: edge.id,
        source: edge.source,
        target: edge.target,
        relationKind: edge.relationKind,
        domainKind: edge.domainKind,
      },
    })),
  ];

  const cy = cytoscape({
    headless: true,
    elements,
    layout: { name: "preset", fit: false },
  });

  try {
    cy
      .layout({
        name: "cose",
        animate: false,
        fit: false,
        randomize: false,
        boundingBox: { x1: -2200, y1: -1500, w: 4400, h: 3000 },
        componentSpacing: 92,
        nodeDimensionsIncludeLabels: false,
        nodeRepulsion: (node: { data: (key: string) => unknown }) =>
          node.data("junction") === true ? 900 : nodeRepulsion(String(node.data("nodeType") ?? "")),
        idealEdgeLength: (edge: { data: (key: string) => unknown }) =>
          relationIdealLength((edge.data("relationKind") as RelationKind | null) ?? null),
        edgeElasticity: (edge: { data: (key: string) => unknown }) =>
          edge.data("relationKind") === "aligns" ? 24 : 34,
        gravity: 0.75,
        numIter: 620,
        initialTemp: 850,
        coolingFactor: 0.985,
        minTemp: 1,
      })
      .run();

    const positions = new Map<string, MapPoint>();
    cy.nodes().forEach((node) => {
      const position = node.position();
      if (Number.isFinite(position.x) && Number.isFinite(position.y)) {
        positions.set(node.id(), { x: position.x, y: position.y });
      }
    });
    if (positions.size === nodes.length) return normalizePositions(compactComponents(positions, nodes, edges));
  } finally {
    cy.destroy();
  }

  return normalizePositions(compactComponents(seedPositions, nodes, edges));
}

function compactComponents(
  positions: Map<string, MapPoint>,
  nodes: readonly LayoutNodeData[],
  edges: readonly LayoutEdgeData[],
): Map<string, MapPoint> {
  const adjacency = new Map<string, Set<string>>();
  for (const node of nodes) adjacency.set(node.id, new Set());
  for (const edge of edges) {
    adjacency.get(edge.source)?.add(edge.target);
    adjacency.get(edge.target)?.add(edge.source);
  }

  const components: string[][] = [];
  const visited = new Set<string>();
  for (const node of nodes) {
    if (visited.has(node.id)) continue;
    const component: string[] = [];
    const queue = [node.id];
    visited.add(node.id);
    while (queue.length > 0) {
      const id = queue.shift()!;
      component.push(id);
      for (const neighbor of adjacency.get(id) ?? []) {
        if (!visited.has(neighbor)) {
          visited.add(neighbor);
          queue.push(neighbor);
        }
      }
    }
    components.push(component.sort(compareIds));
  }

  const paddingX = 56;
  const paddingY = 40;
  const boxes = components.map((members) => {
    let x1 = Infinity;
    let x2 = -Infinity;
    let y1 = Infinity;
    let y2 = -Infinity;
    for (const id of members) {
      const point = positions.get(id);
      if (point === undefined) continue;
      x1 = Math.min(x1, point.x - paddingX);
      x2 = Math.max(x2, point.x + paddingX);
      y1 = Math.min(y1, point.y - paddingY);
      y2 = Math.max(y2, point.y + paddingY);
    }
    return { members, x1, x2, y1, y2, w: x2 - x1, h: y2 - y1 };
  });
  boxes.sort((a, b) => b.w * b.h - a.w * a.h || compareIds(a.members[0] ?? "", b.members[0] ?? ""));

  const componentGap = 120;
  const totalArea = boxes.reduce((sum, box) => sum + box.w * box.h, 0);
  const targetWidth = Math.max(1000, Math.sqrt(totalArea) * 1.45);
  let cursorX = 0;
  let cursorY = 0;
  let rowHeight = 0;
  for (const box of boxes) {
    if (cursorX > 0 && cursorX + box.w > targetWidth) {
      cursorX = 0;
      cursorY += rowHeight + componentGap;
      rowHeight = 0;
    }
    const dx = cursorX - box.x1;
    const dy = cursorY - box.y1;
    for (const id of box.members) {
      const point = positions.get(id);
      if (point !== undefined) {
        point.x += dx;
        point.y += dy;
      }
    }
    cursorX += box.w + componentGap;
    rowHeight = Math.max(rowHeight, box.h);
  }
  return positions;
}

function normalizePositions(positions: Map<string, MapPoint>): Map<string, MapPoint> {
  if (positions.size === 0) return positions;
  let minX = Infinity;
  let maxX = -Infinity;
  let minY = Infinity;
  let maxY = -Infinity;
  for (const point of positions.values()) {
    minX = Math.min(minX, point.x);
    maxX = Math.max(maxX, point.x);
    minY = Math.min(minY, point.y);
    maxY = Math.max(maxY, point.y);
  }
  const centerX = (minX + maxX) / 2;
  const centerY = (minY + maxY) / 2;
  for (const point of positions.values()) {
    point.x = Math.round(point.x - centerX);
    point.y = Math.round(point.y - centerY);
  }
  return positions;
}

function placeNearAnchor(
  seed: MapPoint,
  occupied: readonly MapPoint[],
  offset: number,
): MapPoint {
  const collides = (point: MapPoint) =>
    occupied.some(
      (other) =>
        Math.abs(other.x - point.x) < SATELLITE_MIN_DX &&
        Math.abs(other.y - point.y) < SATELLITE_MIN_DY,
    );
  if (!collides(seed)) return seed;

  for (let ring = 1; ring <= 6; ring += 1) {
    const steps = ring * 10;
    for (let step = 0; step < steps; step += 1) {
      const angle = offset + (step / steps) * TAU;
      const candidate = {
        x: Math.round(seed.x + Math.cos(angle) * ring * 82),
        y: Math.round(seed.y + Math.sin(angle) * ring * 58),
      };
      if (!collides(candidate)) return candidate;
    }
  }
  return { x: seed.x + 92, y: seed.y + 58 };
}

function buildSatelliteAnchors(
  entities: ReadonlyMap<string, IncidenceNode>,
  relations: readonly ParsedRelation[],
): Map<string, string> {
  const anchors = new Map<string, string>();
  const spanToArtifact = new Map<string, string>();
  const spanToUnit = new Map<string, string>();
  const artifactToPdf = new Map<string, string>();

  for (const relation of relations) {
    const endpointNodes = relation.endpoints
      .map((endpoint) => entities.get(endpoint.entity))
      .filter((node): node is IncidenceNode => node !== undefined);
    const spans = endpointNodes.filter((node) => node.domain_kind === "SourceSpan");
    const units = endpointNodes.filter((node) => node.domain_kind === "SourceSemanticUnit");
    const artifacts = endpointNodes.filter((node) => node.domain_kind === "SourceArtifact");
    const pdf = artifacts.find((node) => nodeTypeKey(node) === "pdf");

    if (relation.relationKind === "contains_span") {
      const span = spans[0];
      const unit = units[0];
      if (span && unit) spanToUnit.set(span.id, unit.id);
    }
    if (relation.relationKind === "located_in") {
      const span = spans[0];
      const artifact = artifacts[0];
      if (span && artifact) spanToArtifact.set(span.id, artifact.id);
    }
    if (relation.relationKind === "conversion" && pdf) {
      for (const artifact of artifacts) {
        if (artifact.id !== pdf.id) artifactToPdf.set(artifact.id, pdf.id);
      }
    }
  }

  for (const node of entities.values()) {
    if (node.domain_kind === "SourceSpan") {
      const anchor = spanToUnit.get(node.id) ?? spanToArtifact.get(node.id);
      if (anchor) anchors.set(node.id, anchor);
    } else if (isAuxiliaryArtifactNode(node)) {
      const pdf = artifactToPdf.get(node.id);
      if (pdf) anchors.set(node.id, pdf);
    }
  }
  return anchors;
}

function addSatellites(
  positions: Map<string, MapPoint>,
  entities: ReadonlyMap<string, IncidenceNode>,
  relations: readonly ParsedRelation[],
): void {
  const anchors = buildSatelliteAnchors(entities, relations);
  const slots = new Map<string, number>();
  const occupied = [...positions.values()];

  const satellites = [...entities.values()]
    .filter(isSatelliteNode)
    .sort((a, b) => compareIds(a.id, b.id));

  for (const node of satellites) {
    const anchorId = anchors.get(node.id);
    const anchor = anchorId === undefined ? undefined : positions.get(anchorId);
    if (anchorId === undefined || anchor === undefined) continue;
    const slot = slots.get(anchorId) ?? 0;
    slots.set(anchorId, slot + 1);
    const phase = node.domain_kind === "SourceSpan" ? -Math.PI / 2 : Math.PI / 2;
    const angle = phase + slot * 0.72;
    const radius = node.domain_kind === "SourceSpan" ? 68 : 98;
    const point = placeNearAnchor(
      {
        x: Math.round(anchor.x + Math.cos(angle) * radius),
        y: Math.round(anchor.y + Math.sin(angle) * radius),
      },
      occupied,
      angle,
    );
    positions.set(node.id, point);
    occupied.push(point);
  }
}

function addRelationPositions(
  positions: Map<string, MapPoint>,
  relations: readonly ParsedRelation[],
): void {
  for (const relation of relations) {
    const endpointPositions = uniqueEntities(relation.endpoints)
      .map((id) => positions.get(id))
      .filter((point): point is MapPoint => point !== undefined);
    if (endpointPositions.length === 0) continue;
    const center = {
      x: Math.round(endpointPositions.reduce((sum, point) => sum + point.x, 0) / endpointPositions.length),
      y: Math.round(endpointPositions.reduce((sum, point) => sum + point.y, 0) / endpointPositions.length),
    };
    positions.set(relation.relationNodeId, center);
    positions.set(`junc~${relation.domainId}`, center);
    positions.set(`layout-junc~${relation.domainId}`, center);
  }
}

/**
 * Stable key for deciding whether a new snapshot actually needs a new layout.
 * Node and edge insertion order is deliberately ignored.
 */
export function landscapeLayoutKey(graph: IncidenceGraph): string {
  const nodes = graph.nodes
    .filter((node) => node.element_kind !== "group")
    .map((node) =>
      [node.id, node.element_kind, node.domain_kind, node.relation_kind, node.label, node.summary, node.lifecycle].join("\u001f"),
    )
    .sort(compareIds);
  const edges = graph.edges
    .map((edge) =>
      [edge.id, edge.source, edge.target, edge.relation_id, edge.role, edge.direction, edge.directed].join("\u001f"),
    )
    .sort(compareIds);
  return `${nodes.join("\u001e")}\u001d${edges.join("\u001e")}`;
}

/**
 * Computes model coordinates for every entity, relation node and synthetic
 * semantic junction in the full landscape graph.
 */
export function computeLandscapeLayout(graph: IncidenceGraph): Map<string, MapPoint> {
  const { entities, relations } = parseRelations(graph);
  const layoutGraph = buildLayoutGraph(entities, relations);
  const positions = runOrganicLayout(layoutGraph.nodes, layoutGraph.edges);
  addSatellites(positions, entities, relations);
  addRelationPositions(positions, relations);
  return positions;
}

function neighborsByEntity(relations: readonly ParsedRelation[]): Map<string, Set<string>> {
  const adjacency = new Map<string, Set<string>>();
  for (const relation of relations) {
    const ids = uniqueEntities(relation.endpoints);
    for (const source of ids) {
      const neighbors = adjacency.get(source) ?? new Set<string>();
      for (const target of ids) if (source !== target) neighbors.add(target);
      adjacency.set(source, neighbors);
    }
  }
  return adjacency;
}

/**
 * Preserve already visible/user-pinned positions while placing newly introduced
 * nodes near their settled neighbors. `full` is normally computed once per
 * landscape snapshot by GraphController.
 */
export function resolveIncrementalPositions(
  graph: IncidenceGraph,
  settled: ReadonlyMap<string, MapPoint>,
  pinned: ReadonlyMap<string, MapPoint>,
  full: ReadonlyMap<string, MapPoint> = computeLandscapeLayout(graph),
): Map<string, MapPoint> {
  const { entities, relations } = parseRelations(graph);
  const activeIds = new Set<string>([
    ...entities.keys(),
    ...relations.map((relation) => relation.relationNodeId),
    ...relations.map((relation) => `junc~${relation.domainId}`),
  ]);
  const out = new Map<string, MapPoint>();

  for (const id of activeIds) {
    const pinnedPoint = pinned.get(id);
    const settledPoint = settled.get(id);
    const fullPoint = full.get(id);
    if (pinnedPoint) out.set(id, { ...pinnedPoint });
    else if (settledPoint) out.set(id, { ...settledPoint });
    else if (fullPoint) out.set(id, { ...fullPoint });
  }

  const adjacency = neighborsByEntity(relations);
  const occupied = [...out.values()];
  const newEntities = [...entities.values()]
    .filter((node) => !out.has(node.id))
    .sort((a, b) => compareIds(a.id, b.id));
  for (const node of newEntities) {
    const neighbors = [...(adjacency.get(node.id) ?? [])]
      .map((id) => out.get(id))
      .filter((point): point is MapPoint => point !== undefined);
    const fullPoint = full.get(node.id) ?? { x: 0, y: 0 };
    const seed =
      neighbors.length === 0
        ? fullPoint
        : {
            x: Math.round(neighbors.reduce((sum, point) => sum + point.x, 0) / neighbors.length),
            y: Math.round(neighbors.reduce((sum, point) => sum + point.y, 0) / neighbors.length + 92),
          };
    const hash = stableHash(node.id);
    const candidate = placeNearAnchor(seed, occupied, (hash % 1000) / 1000 * TAU);
    out.set(node.id, candidate);
    occupied.push(candidate);
  }

  addRelationPositions(out, relations);
  return out;
}
