/**
 * Pure geometry, signal spotlighting, and 1-hop neighborhood briefing model for
 * the In-Map Study Posture (`研读` / `学习视图`).
 *
 * Keeps zero DOM/Cytoscape dependency so coordinates, tree indentation,
 * intra-spine Bezier arc offsets, right-orbit barycenter relaxation, and
 * spotlight signals are deterministic and unit-testable.
 */
import type {
  LearningMaterial,
  LearningView,
  StudyOutlineTarget,
  StudyOutlineUnit,
  StudyStatus,
} from "../semantic/api";
import { findSourceArtifactNode } from "../reader/openable";
import { relationLabel } from "../semantic/labels";
import type { MapPoint } from "./layout";
import type { IncidenceEdge, IncidenceGraph, IncidenceNode, RelationKind } from "./projection";

export type StudySignalMode = "all" | "novel_and_diff" | "cross_book" | "unaligned" | "debts";

export interface CameraFramingBox {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
}

export type CameraTarget =
  | {
      kind: "fit_elements";
      elementIds?: ReadonlySet<string>;
      padding?: number;
      maxZoom?: number;
    }
  | {
      kind: "framing_box";
      box: CameraFramingBox;
      minZoom?: number;
      maxZoom?: number;
      padding?: number;
    }
  | { kind: "preserve" };

export interface SpineArcSpec {
  magnitude: number;
  side: "left" | "right";
}

export interface MapPostureSpec {
  id: string;
  positions: Map<string, MapPoint>;
  inScopeNodeIds: Set<string> | null;
  inScopeRelationIds: Set<string> | null;
  /** Relation keys (`domain_id ?? id`) rendered as unbundled-bezier arcs around vertical tracks. */
  spineArcOffsets: Map<string, SpineArcSpec>;
  /** `has_part` relation keys inside the spine whose redundant "包含" label is suppressed in favor of tree indentation. */
  spineTreeRelationKeys: Set<string>;
  /** Extra CSS classes per projection entity node id. */
  nodeClasses: Map<string, string[]>;
  /** Relation keys muted (`opacity: 0.22`) under a non-`all` study signal. */
  mutedRelationKeys: Set<string>;
  /** Relation keys highlighted under a non-`all` study signal. */
  hitRelationKeys: Set<string>;
  camera: CameraTarget;
}

export interface NodeBriefingNeighbor {
  nodeId: string;
  domainId: string | null;
  label: string;
  relationLabel: string;
  direction: "incoming" | "outgoing" | "undirected";
}

export interface StudyNodeBriefing {
  nodeId: string;
  domainId: string | null;
  domainKind: string | null;
  title: string;
  unitKind: string | null;
  studyStatus: StudyStatus | null;
  alignmentBadge: string | null;
  alignmentTone: "aligned" | "diff" | "cross" | "unaligned" | "semantic" | "debt";
  targets: Array<{
    entityId: string;
    entityLabel: string;
    relationText: string;
    diffs: string[];
    seenInNames: string[];
    masteredElsewhere: boolean;
    staged: boolean;
  }>;
  intraBookNeighbors: NodeBriefingNeighbor[];
  openDebts: Array<{ debtId: string; obligation: string; status: string }>;
  conventions: Array<{ scopeLabel: string; text: string; isBaselineMatch: boolean; diffNote?: string | null }>;
}

const SPINE_X = 0;
const TOP_Y = 0;
const INDENT_STEP_X = 66;
const MAX_INDENT_DEPTH = 4;
const ROW_STEP_Y = 74;
const SECTION_EXTRA_GAP_Y = 20;
const DIALECT_MARGIN_X = -230;
const REPRESENTATION_ORBIT_X = -460;
const SEMANTIC_ORBIT_X = 480;
const OUTER_ORBIT_X = 740;

const INTRA_SPINE_REASONING_KINDS: ReadonlySet<RelationKind> = new Set([
  "states",
  "proves",
  "poses",
  "hints",
  "solves",
  "explains",
  "applies_method",
  "cites",
]);

interface IndexedRel {
  node: IncidenceNode;
  relKey: string;
  into: string[];
  out: string[];
  all: string[];
}

function indexGraphRelations(graph: IncidenceGraph): {
  entitiesById: Map<string, IncidenceNode>;
  entityByDomainId: Map<string, IncidenceNode>;
  relations: IndexedRel[];
} {
  const entitiesById = new Map<string, IncidenceNode>();
  const entityByDomainId = new Map<string, IncidenceNode>();
  const relNodes = new Map<string, IncidenceNode>();
  for (const node of graph.nodes) {
    if (node.element_kind === "entity") {
      entitiesById.set(node.id, node);
      if (node.domain_id) entityByDomainId.set(node.domain_id, node);
    } else if (node.element_kind === "relation") {
      relNodes.set(node.id, node);
    }
  }
  const edgesByRelId = new Map<string, IncidenceEdge[]>();
  for (const edge of graph.edges) {
    const list = edgesByRelId.get(edge.relation_id) ?? [];
    list.push(edge);
    edgesByRelId.set(edge.relation_id, list);
  }
  const relations: IndexedRel[] = [];
  for (const [, rNode] of relNodes) {
    const relKey = rNode.domain_id ?? rNode.id;
    const rawEdges = edgesByRelId.get(relKey) ?? edgesByRelId.get(rNode.id) ?? [];
    const into: string[] = [];
    const out: string[] = [];
    const all: string[] = [];
    for (const e of rawEdges) {
      const other = e.source === rNode.id ? e.target : e.target === rNode.id ? e.source : null;
      if (!other || !entitiesById.has(other)) continue;
      if (!all.includes(other)) all.push(other);
      if (e.direction === "into_relation" && !into.includes(other)) into.push(other);
      else if (e.direction === "out_of_relation" && !out.includes(other)) out.push(other);
    }
    if (all.length >= 2) {
      relations.push({ node: rNode, relKey, into, out, all });
    }
  }
  return { entitiesById, entityByDomainId, relations };
}

function isTheoryNode(node: IncidenceNode | undefined): boolean {
  return (
    node?.element_kind === "entity" &&
    node.domain_kind === "SemanticEntity" &&
    Boolean(node.summary?.startsWith("理论") || node.label.startsWith("理论："))
  );
}

/**
 * 1D order-preserving minimum-gap relaxation around each item's ideal `y`.
 * Keeps barycenter vertical alignment while guaranteeing `y[i+1] - y[i] >= minGap`.
 */
export function relaxVerticalOrbit(
  items: ReadonlyArray<{ id: string; idealY: number }>,
  minGap: number,
): Map<string, number> {
  const result = new Map<string, number>();
  if (items.length === 0) return result;
  const sorted = [...items].sort(
    (a, b) => a.idealY - b.idealY || a.id.localeCompare(b.id),
  );
  const ys = sorted.map((item) => item.idealY);
  if (ys.length === 1) {
    result.set(sorted[0].id, Math.round(ys[0]));
    return result;
  }

  for (let round = 0; round < 24; round += 1) {
    let changed = false;
    for (let i = 0; i < ys.length - 1; i += 1) {
      const diff = ys[i + 1] - ys[i];
      if (diff < minGap) {
        const push = (minGap - diff) / 2;
        ys[i] -= push;
        ys[i + 1] += push;
        changed = true;
      }
    }
    if (!changed) break;
  }
  // Re-center group mean toward ideal mean so pushing apart doesn't drift downward/upward.
  const idealMean = sorted.reduce((acc, item) => acc + item.idealY, 0) / sorted.length;
  const relaxedMean = ys.reduce((acc, y) => acc + y, 0) / ys.length;
  const shift = idealMean - relaxedMean;
  sorted.forEach((item, index) => {
    result.set(item.id, Math.round(ys[index] + shift));
  });
  return result;
}

/**
 * Computes tree depth within the material's `outline_units` (walking `parent_unit`
 * among units present in this spine, with cycle protection).
 */
export function computeSpineDepths(units: readonly StudyOutlineUnit[]): Map<string, number> {
  const byId = new Map(units.map((u) => [u.unit_id, u]));
  const depths = new Map<string, number>();
  const resolve = (u: StudyOutlineUnit, visiting: Set<string>): number => {
    const cached = depths.get(u.unit_id);
    if (cached !== undefined) return cached;
    if (!u.parent_unit || !byId.has(u.parent_unit) || visiting.has(u.unit_id)) {
      depths.set(u.unit_id, 0);
      return 0;
    }
    visiting.add(u.unit_id);
    const d = Math.min(MAX_INDENT_DEPTH, resolve(byId.get(u.parent_unit)!, visiting) + 1);
    visiting.delete(u.unit_id);
    depths.set(u.unit_id, d);
    return d;
  };
  for (const u of units) resolve(u, new Set());
  return depths;
}

function appendRelationCentroids(
  graph: IncidenceGraph,
  positions: Map<string, MapPoint>,
): void {
  const byRel = new Map<string, IncidenceEdge[]>();
  for (const edge of graph.edges) {
    const list = byRel.get(edge.relation_id) ?? [];
    list.push(edge);
    byRel.set(edge.relation_id, list);
  }
  for (const node of graph.nodes) {
    if (node.element_kind !== "relation") continue;
    const relKey = node.domain_id ?? node.id;
    const relEdges = byRel.get(relKey) ?? byRel.get(node.id) ?? [];
    const pts = relEdges
      .map((e) => positions.get(e.source === node.id ? e.target : e.source))
      .filter((p): p is MapPoint => p !== undefined);
    if (pts.length === 0) continue;
    const cx = Math.round(pts.reduce((a, p) => a + p.x, 0) / pts.length);
    const cy = Math.round(pts.reduce((a, p) => a + p.y, 0) / pts.length);
    positions.set(node.id, { x: cx, y: cy });
    positions.set(`junc~${relKey}`, { x: cx, y: cy });
  }
}

/**
 * Default Organic Landscape Posture: preserves deterministic `basePositions`.
 */
export function buildOrganicPosture(
  graph: IncidenceGraph,
  basePositions: ReadonlyMap<string, MapPoint>,
  inScopeNodeIds: Set<string> | null = null,
  inScopeRelationIds: Set<string> | null = null,
): MapPostureSpec {
  const positions = new Map(basePositions);
  appendRelationCentroids(graph, positions);
  return {
    id: "organic",
    positions,
    inScopeNodeIds,
    inScopeRelationIds,
    spineArcOffsets: new Map(),
    spineTreeRelationKeys: new Set(),
    nodeClasses: new Map(),
    mutedRelationKeys: new Set(),
    hitRelationKeys: new Set(),
    camera: { kind: "preserve" },
  };
}

/**
 * Study Overview Posture (before picking a specific material): highlights all
 * `SourceSemanticUnit` + `SemanticEntity / DialectScope / Debt` nodes across the map
 * while receding file plumbing.
 */
export function buildStudyOverviewPosture(
  graph: IncidenceGraph,
  basePositions: ReadonlyMap<string, MapPoint>,
  learningView?: LearningView | null,
): MapPostureSpec {
  const positions = new Map(basePositions);
  appendRelationCentroids(graph, positions);
  const inScopeNodeIds = new Set<string>();
  const nodeClasses = new Map<string, string[]>();

  const outlineByDomainId = new Map<string, StudyOutlineUnit>();
  for (const mat of learningView?.materials ?? []) {
    for (const u of mat.outline_units ?? []) {
      outlineByDomainId.set(u.unit_id, u);
    }
  }

  for (const node of graph.nodes) {
    if (node.element_kind !== "entity") continue;
    if (
      node.domain_kind === "SourceSemanticUnit" ||
      node.domain_kind === "SemanticEntity" ||
      node.domain_kind === "RepresentationCore" ||
      node.domain_kind === "DialectScope" ||
      node.domain_kind === "Debt"
    ) {
      inScopeNodeIds.add(node.id);
      if (node.domain_kind === "SourceSemanticUnit" && node.domain_id) {
        const outline = outlineByDomainId.get(node.domain_id);
        if (outline) {
          nodeClasses.set(node.id, unitPermanentClasses(outline, false));
        }
      }
    }
  }

  return {
    id: "study:all",
    positions,
    inScopeNodeIds: inScopeNodeIds.size > 0 ? inScopeNodeIds : null,
    inScopeRelationIds: null,
    spineArcOffsets: new Map(),
    spineTreeRelationKeys: new Set(),
    nodeClasses,
    mutedRelationKeys: new Set(),
    hitRelationKeys: new Set(),
    camera: {
      kind: "fit_elements",
      elementIds: inScopeNodeIds.size > 0 ? inScopeNodeIds : undefined,
      padding: 52,
      maxZoom: 1.2,
    },
  };
}

function unitPermanentClasses(u: StudyOutlineUnit, onSpine: boolean): string[] {
  const cls: string[] = [];
  if (onSpine) cls.push("spine-unit-node");
  if (u.study_status === "mastered") cls.push("study-status-mastered");
  else if (u.study_status === "studying") cls.push("study-status-studying");

  if (u.category === "unaligned") cls.push("study-cat-unaligned");
  else if (u.category === "seen_elsewhere" || u.category === "mastered_elsewhere") {
    cls.push("study-cat-cross");
  } else {
    cls.push("study-cat-aligned");
  }
  if (u.targets.some((t) => t.only_differs_by.length > 0)) {
    cls.push("study-cat-diff");
  }
  return cls;
}

function matchesSignal(u: StudyOutlineUnit, signal: StudySignalMode, unitHasDebt: boolean): boolean {
  switch (signal) {
    case "all":
      return true;
    case "novel_and_diff":
      return (
        u.category === "fresh" ||
        u.targets.some((t) => t.only_differs_by.length > 0)
      );
    case "cross_book":
      return u.category === "seen_elsewhere" || u.category === "mastered_elsewhere";
    case "unaligned":
      return u.category === "unaligned";
    case "debts":
      return unitHasDebt;
  }
}

/**
 * Study Spine-and-Orbit Posture for one selected `LearningMaterial`:
 * - Left Spine: ordered `outline_units` with tree indentation `x = SPINE_X + depth * INDENT_STEP_X`
 *   and left-bowing Bezier arcs for non-adjacent intra-book reasoning edges.
 * - Left Margin: `DialectScope` convention cards anchored beside their section.
 * - Right Orbit: aligned `SemanticEntity` stars relaxed around the vertical barycenter of
 *   their connected spine units.
 * - Outer Right Orbit: `Theory` shelves and `Debt` tags relaxed without crossing bridges.
 */
export function buildStudySpinePosture(
  graph: IncidenceGraph,
  basePositions: ReadonlyMap<string, MapPoint>,
  material: LearningMaterial,
  signalMode: StudySignalMode = "all",
): MapPostureSpec {
  const { entitiesById, entityByDomainId, relations } = indexGraphRelations(graph);
  const positions = new Map(basePositions);
  const inScopeNodeIds = new Set<string>();
  const inScopeRelationIds = new Set<string>();
  const spineArcOffsets = new Map<string, SpineArcSpec>();
  const pendingSpineArcs: Array<{ relKey: string; minRow: number; maxRow: number; baseMag: number }> = [];
  const spineTreeRelationKeys = new Set<string>();
  const nodeClasses = new Map<string, string[]>();

  const outline = material.outline_units ?? [];
  const depths = computeSpineDepths(outline);

  // 1. Place optional SourceArtifact header node above the top of the spine
  const entityList = [...entitiesById.values()];
  const headerArtifact =
    findSourceArtifactNode(entityList, material.artifact_id) ??
    (material.aliased_artifact_ids ?? [])
      .map((id) => findSourceArtifactNode(entityList, id))
      .find((n): n is IncidenceNode => n !== undefined);

  if (headerArtifact) {
    positions.set(headerArtifact.id, { x: SPINE_X, y: TOP_Y - 92 });
    inScopeNodeIds.add(headerArtifact.id);
    nodeClasses.set(headerArtifact.id, ["spine-header-node"]);
  }

  // 2. Place Left Spine units in page/byte reading order
  const spineRowByNodeId = new Map<string, number>();
  const outlineByNodeId = new Map<string, StudyOutlineUnit>();
  let cursorY = TOP_Y;
  let placedCount = 0;

  for (const item of outline) {
    const node = entityByDomainId.get(item.unit_id);
    if (!node) continue;
    const depth = depths.get(item.unit_id) ?? 0;
    if (placedCount > 0 && depth === 0 && item.unit_kind === "section") {
      cursorY += SECTION_EXTRA_GAP_Y;
    }
    const x = SPINE_X + depth * INDENT_STEP_X;
    const y = cursorY;
    positions.set(node.id, { x, y });
    inScopeNodeIds.add(node.id);
    spineRowByNodeId.set(node.id, placedCount);
    outlineByNodeId.set(node.id, item);
    nodeClasses.set(node.id, unitPermanentClasses(item, true));
    cursorY += ROW_STEP_Y;
    placedCount += 1;
  }

  // 3. Collect intra-spine relations, aligned SemanticEntities, DialectScopes, and Debts
  const semanticSamples = new Map<string, number[]>(); // semantic nodeId -> connected spine ys
  const scopeSamples = new Map<string, number[]>(); // dialect scope nodeId -> anchor ys
  const alignmentRelNodes = new Map<string, { rel: IndexedRel; unitNodeId: string; semNodeId: string }>();

  for (const rel of relations) {
    const kind = rel.node.relation_kind;
    const allSpine = rel.all.length >= 2 && rel.all.every((id) => spineRowByNodeId.has(id));
    if (allSpine) {
      inScopeRelationIds.add(rel.node.id);
      inScopeRelationIds.add(rel.relKey);
      const rows = rel.all.map((id) => spineRowByNodeId.get(id)!);
      const minRow = Math.min(...rows);
      const maxRow = Math.max(...rows);
      const rowSpan = maxRow - minRow;
      if (kind === "has_part") {
        spineTreeRelationKeys.add(rel.relKey);
        if (rowSpan > 1) {
          const parentDepth = rel.into[0] ? (depths.get(entitiesById.get(rel.into[0])?.domain_id ?? "") ?? 0) : 0;
          const childDepth = rel.out[0] ? (depths.get(entitiesById.get(rel.out[0])?.domain_id ?? "") ?? 0) : 0;
          if (parentDepth === childDepth) {
            pendingSpineArcs.push({
              relKey: rel.relKey,
              minRow,
              maxRow,
              baseMag: 46 + Math.min(rowSpan, 14) * 11,
            });
          }
        }
      } else if (kind !== null && INTRA_SPINE_REASONING_KINDS.has(kind) && rowSpan >= 1) {
        pendingSpineArcs.push({
          relKey: rel.relKey,
          minRow,
          maxRow,
          baseMag: 50 + Math.min(rowSpan, 14) * 13,
        });
      }
      continue;
    }

    if (kind === "aligns") {
      const unitEndpoint = rel.all.find((id) => spineRowByNodeId.has(id));
      const semEndpoint = rel.all.find((id) => {
        const n = entitiesById.get(id);
        return n?.domain_kind === "SemanticEntity";
      });
      if (unitEndpoint && semEndpoint) {
        inScopeNodeIds.add(semEndpoint);
        inScopeRelationIds.add(rel.node.id);
        inScopeRelationIds.add(rel.relKey);
        const unitY = positions.get(unitEndpoint)?.y ?? TOP_Y;
        const list = semanticSamples.get(semEndpoint) ?? [];
        list.push(unitY);
        semanticSamples.set(semEndpoint, list);
        alignmentRelNodes.set(rel.node.id, { rel, unitNodeId: unitEndpoint, semNodeId: semEndpoint });
        if (rel.node.domain_id) {
          alignmentRelNodes.set(rel.node.domain_id, { rel, unitNodeId: unitEndpoint, semNodeId: semEndpoint });
        }
      }
      continue;
    }

    if (kind === "scope_of") {
      const scopeEndpoint = rel.all.find((id) => entitiesById.get(id)?.domain_kind === "DialectScope");
      const unitEndpoint = rel.all.find((id) => spineRowByNodeId.has(id));
      if (scopeEndpoint && unitEndpoint) {
        inScopeNodeIds.add(scopeEndpoint);
        inScopeRelationIds.add(rel.node.id);
        inScopeRelationIds.add(rel.relKey);
        const unitY = positions.get(unitEndpoint)?.y ?? TOP_Y;
        const list = scopeSamples.get(scopeEndpoint) ?? [];
        list.push(unitY);
        scopeSamples.set(scopeEndpoint, list);
      }
    }
  }

  // Assign staggered lanes to overlapping left-spine arcs so parallel labels never collide
  pendingSpineArcs.sort(
    (a, b) => (a.maxRow - a.minRow) - (b.maxRow - b.minRow) || a.minRow - b.minRow || a.relKey.localeCompare(b.relKey),
  );
  const assignedLanes: Array<{ minRow: number; maxRow: number; mag: number }> = [];
  for (const arc of pendingSpineArcs) {
    let mag = arc.baseMag;
    for (let guard = 0; guard < 8; guard += 1) {
      const collides = assignedLanes.some(
        (other) =>
          arc.minRow < other.maxRow &&
          other.minRow < arc.maxRow &&
          Math.abs(other.mag - mag) < 34,
      );
      if (!collides) break;
      mag += 36;
    }
    assignedLanes.push({ minRow: arc.minRow, maxRow: arc.maxRow, mag });
    spineArcOffsets.set(arc.relKey, { magnitude: mag, side: "left" });
  }

  // Place DialectScopes on left margin next to their anchored section
  const scopeOrbit = relaxVerticalOrbit(
    [...scopeSamples.entries()].map(([id, ys]) => ({
      id,
      idealY: ys.reduce((a, b) => a + b, 0) / ys.length,
    })),
    88,
  );
  for (const [scopeId, y] of scopeOrbit) {
    positions.set(scopeId, { x: DIALECT_MARGIN_X, y });
    nodeClasses.set(scopeId, ["spine-scope-node"]);
  }

  // Place RepresentationCores bound to in-scope DialectScopes on left representation orbit
  const repSamples = new Map<string, number[]>();
  for (const rel of relations) {
    if (rel.node.relation_kind !== "binds_representation") continue;
    const scopeEndpoint = rel.all.find((id) => scopeOrbit.has(id));
    const repEndpoint = rel.all.find((id) => entitiesById.get(id)?.domain_kind === "RepresentationCore");
    if (scopeEndpoint && repEndpoint) {
      inScopeNodeIds.add(repEndpoint);
      inScopeRelationIds.add(rel.node.id);
      inScopeRelationIds.add(rel.relKey);
      const scopeY = positions.get(scopeEndpoint)?.y ?? TOP_Y;
      const list = repSamples.get(repEndpoint) ?? [];
      list.push(scopeY);
      repSamples.set(repEndpoint, list);
    }
  }
  const repOrbit = relaxVerticalOrbit(
    [...repSamples.entries()].map(([id, ys]) => ({
      id,
      idealY: ys.reduce((a, b) => a + b, 0) / ys.length,
    })),
    92,
  );
  for (const [repId, y] of repOrbit) {
    positions.set(repId, { x: REPRESENTATION_ORBIT_X, y });
    nodeClasses.set(repId, ["orbit-semantic-node"]);
  }

  // Place primary SemanticEntities on Right Orbit (`SEMANTIC_ORBIT_X`)
  const primarySemOrbit = relaxVerticalOrbit(
    [...semanticSamples.entries()].map(([id, ys]) => ({
      id,
      idealY: ys.reduce((a, b) => a + b, 0) / ys.length,
    })),
    96,
  );
  for (const [semId, y] of primarySemOrbit) {
    positions.set(semId, { x: SEMANTIC_ORBIT_X, y });
    nodeClasses.set(semId, ["orbit-semantic-node"]);
  }

  // 4. Collect 1-hop semantic context of those SemanticEntities: Theory shelves, referenced entities, and Debts
  const outerItems = new Map<string, number[]>(); // nodeId -> anchor ys on OUTER_ORBIT_X
  const unitIdsWithDebt = new Set<string>();
  const debtNodeIds = new Set<string>();
  const debtEdgesKeys = new Set<string>();

  for (const rel of relations) {
    const kind = rel.node.relation_kind;
    if (kind === "in_theory" || kind === "uses_entity" || kind === "of_concept") {
      const inOrbit = rel.all.filter((id) => primarySemOrbit.has(id));
      if (inOrbit.length === 0) continue;
      const others = rel.all.filter((id) => !primarySemOrbit.has(id));
      if (others.length === 0) {
        // Relation among two already-placed orbit stars: curve gently right if spanning across intermediate stars
        inScopeRelationIds.add(rel.node.id);
        inScopeRelationIds.add(rel.relKey);
        if (inOrbit.length === 2) {
          const yA = positions.get(inOrbit[0])?.y ?? 0;
          const yB = positions.get(inOrbit[1])?.y ?? 0;
          if (Math.abs(yA - yB) > 115) {
            const existingArc = spineArcOffsets.has(rel.relKey);
            spineArcOffsets.set(rel.relKey, {
              magnitude: existingArc ? 76 : 48,
              side: "right",
            });
          }
        }
        continue;
      }
      for (const otherId of others) {
        const otherNode = entitiesById.get(otherId);
        if (!otherNode || otherNode.domain_kind !== "SemanticEntity") continue;
        // Only pull in Theory shelves or directly referenced concepts/declarations (1 hop, no shelf sibling explosion)
        if (kind === "in_theory" && !isTheoryNode(otherNode)) continue;
        inScopeNodeIds.add(otherId);
        inScopeRelationIds.add(rel.node.id);
        inScopeRelationIds.add(rel.relKey);
        const anchorYs = inOrbit.map((id) => positions.get(id)?.y ?? TOP_Y);
        const list = outerItems.get(otherId) ?? [];
        list.push(...anchorYs);
        outerItems.set(otherId, list);
      }
      continue;
    }

    if (kind === "debt_on") {
      const debtEndpoint = rel.all.find((id) => entitiesById.get(id)?.domain_kind === "Debt");
      if (!debtEndpoint) continue;
      // Debt can attach to an in-scope entity (unit / SemanticEntity) OR to an Alignment relation
      const entityHost = rel.all.find((id) => id !== debtEndpoint && inScopeNodeIds.has(id));
      let anchorY: number | undefined;
      if (entityHost) {
        anchorY = positions.get(entityHost)?.y;
        if (spineRowByNodeId.has(entityHost)) unitIdsWithDebt.add(entityHost);
        for (const [, al] of alignmentRelNodes) {
          if (al.semNodeId === entityHost) unitIdsWithDebt.add(al.unitNodeId);
        }
      }
      if (anchorY !== undefined) {
        inScopeNodeIds.add(debtEndpoint);
        inScopeRelationIds.add(rel.node.id);
        inScopeRelationIds.add(rel.relKey);
        debtNodeIds.add(debtEndpoint);
        debtEdgesKeys.add(rel.relKey);
        const list = outerItems.get(debtEndpoint) ?? [];
        list.push(anchorY);
        outerItems.set(debtEndpoint, list);
      }
    }
  }

  // Also check Debts whose `debt_on` endpoint is an Alignment relation in hyperedge/projection form
  for (const node of graph.nodes) {
    if (node.element_kind !== "entity" || node.domain_kind !== "Debt" || inScopeNodeIds.has(node.id)) continue;
    const matHasDebt = material.open_debts.some((d) => d.debt_id === node.domain_id);
    if (!matHasDebt) continue;
    inScopeNodeIds.add(node.id);
    debtNodeIds.add(node.id);
    // Find any edge touching this debt node
    const debtEdges = graph.edges.filter((e) => e.source === node.id || e.target === node.id);
    let anchorY = TOP_Y + Math.max(0, (placedCount - 1) * ROW_STEP_Y) / 2;
    for (const e of debtEdges) {
      inScopeRelationIds.add(e.relation_id);
      debtEdgesKeys.add(e.relation_id);
      const al = alignmentRelNodes.get(e.relation_id);
      if (al) {
        unitIdsWithDebt.add(al.unitNodeId);
        anchorY = positions.get(al.semNodeId)?.y ?? positions.get(al.unitNodeId)?.y ?? anchorY;
      }
    }
    outerItems.set(node.id, [anchorY]);
  }

  const outerOrbit = relaxVerticalOrbit(
    [...outerItems.entries()].map(([id, ys]) => ({
      id,
      idealY: ys.reduce((a, b) => a + b, 0) / ys.length,
    })),
    92,
  );
  for (const [outerId, y] of outerOrbit) {
    positions.set(outerId, { x: OUTER_ORBIT_X, y });
    nodeClasses.set(outerId, ["orbit-outer-node"]);
  }

  appendRelationCentroids(graph, positions);

  // 5. Apply non-destructive Spotlight Signal classes (`all` keeps everything vivid)
  const mutedRelationKeys = new Set<string>();
  const hitRelationKeys = new Set<string>();
  if (signalMode !== "all") {
    const hitUnitNodeIds = new Set<string>();
    for (const [nodeId, item] of outlineByNodeId) {
      if (matchesSignal(item, signalMode, unitIdsWithDebt.has(nodeId))) {
        hitUnitNodeIds.add(nodeId);
      }
    }
    const hitSemanticIds = new Set<string>();
    for (const rel of relations) {
      if (!inScopeRelationIds.has(rel.relKey) && !inScopeRelationIds.has(rel.node.id)) continue;
      if (rel.node.relation_kind === "aligns") {
        const uId = rel.all.find((id) => spineRowByNodeId.has(id));
        const sId = rel.all.find((id) => primarySemOrbit.has(id));
        if (uId && hitUnitNodeIds.has(uId)) {
          hitRelationKeys.add(rel.relKey);
          if (sId) hitSemanticIds.add(sId);
        } else {
          mutedRelationKeys.add(rel.relKey);
        }
      } else if (rel.node.relation_kind === "debt_on") {
        if (signalMode === "debts") hitRelationKeys.add(rel.relKey);
        else mutedRelationKeys.add(rel.relKey);
      } else {
        const anyHit = rel.all.some((id) => hitUnitNodeIds.has(id) || hitSemanticIds.has(id));
        if (anyHit) hitRelationKeys.add(rel.relKey);
        else mutedRelationKeys.add(rel.relKey);
      }
    }

    for (const id of inScopeNodeIds) {
      if (id === headerArtifact?.id) continue;
      const existing = nodeClasses.get(id) ?? [];
      const isHit =
        hitUnitNodeIds.has(id) ||
        hitSemanticIds.has(id) ||
        (signalMode === "debts" && debtNodeIds.has(id));
      nodeClasses.set(id, [...existing, isHit ? "is-study-signal-hit" : "is-study-signal-muted"]);
    }
  }

  // 6. Compute readable Top-of-Spine camera framing box (starts at first non-mastered unit, ~7 rows tall)
  let focusStartRow = 0;
  for (const [nodeId, rowIdx] of spineRowByNodeId) {
    const u = outlineByNodeId.get(nodeId);
    if (u && u.study_status !== "mastered") {
      focusStartRow = Math.max(0, rowIdx - 1);
      break;
    }
  }
  const startNodeId = [...spineRowByNodeId.entries()].find(([, idx]) => idx === focusStartRow)?.[0];
  const windowTopY =
    focusStartRow === 0 && headerArtifact
      ? TOP_Y - 130
      : (startNodeId ? positions.get(startNodeId)?.y ?? TOP_Y : TOP_Y) - 60;
  const windowBottomY = windowTopY + 620;

  const minX = scopeOrbit.size > 0 ? DIALECT_MARGIN_X - 95 : SPINE_X - 90;
  const maxX =
    outerOrbit.size > 0
      ? OUTER_ORBIT_X + 160
      : primarySemOrbit.size > 0
        ? SEMANTIC_ORBIT_X + 210
        : SPINE_X + MAX_INDENT_DEPTH * INDENT_STEP_X + 260;

  return {
    id: `study:material:${material.artifact_id}`,
    positions,
    inScopeNodeIds,
    inScopeRelationIds,
    spineArcOffsets,
    spineTreeRelationKeys,
    nodeClasses,
    mutedRelationKeys,
    hitRelationKeys,
    camera: {
      kind: "framing_box",
      box: { x1: minX, y1: windowTopY, x2: maxX, y2: windowBottomY },
      minZoom: 0.78,
      maxZoom: 1.04,
      padding: 40,
    },
  };
}

/**
 * Extracts a rich, human-friendly briefing card model for a hovered node in
 * Study Mode (including alignment differences, 1-hop intra-book neighbors, and
 * attached open debts).
 */
export function buildStudyNodeBriefing(
  nodeId: string,
  graph: IncidenceGraph,
  learningView: LearningView | null | undefined,
  materialName: (artifactId: string) => string,
): StudyNodeBriefing | null {
  const { entitiesById, relations } = indexGraphRelations(graph);
  const node = entitiesById.get(nodeId);
  if (!node) return null;

  let matchedUnit: StudyOutlineUnit | undefined;
  let matchedMaterial: LearningMaterial | undefined;
  for (const mat of learningView?.materials ?? []) {
    const found = (mat.outline_units ?? []).find((u) => u.unit_id === node.domain_id);
    if (found) {
      matchedUnit = found;
      matchedMaterial = mat;
      break;
    }
  }

  const intraBookNeighbors: NodeBriefingNeighbor[] = [];
  for (const rel of relations) {
    const kind = rel.node.relation_kind;
    if (!kind || (!INTRA_SPINE_REASONING_KINDS.has(kind) && kind !== "has_part")) continue;
    if (!rel.all.includes(node.id)) continue;
    for (const otherId of rel.all) {
      if (otherId === node.id) continue;
      const other = entitiesById.get(otherId);
      if (!other) continue;
      const direction: NodeBriefingNeighbor["direction"] =
        rel.into.includes(node.id) && rel.out.includes(otherId)
          ? "outgoing"
          : rel.into.includes(otherId) && rel.out.includes(node.id)
            ? "incoming"
            : "undirected";
      const relLbl =
        kind === "has_part"
          ? direction === "incoming"
            ? "所属章节"
            : "下属单元"
          : rel.node.label;
      intraBookNeighbors.push({
        nodeId: other.id,
        domainId: other.domain_id,
        label: other.label,
        relationLabel: relLbl,
        direction,
      });
    }
  }

  if (matchedUnit) {
    const hasDiff = matchedUnit.targets.some((t) => t.only_differs_by.length > 0);
    const isCross =
      matchedUnit.category === "seen_elsewhere" || matchedUnit.category === "mastered_elsewhere";
    let alignmentBadge: string;
    let alignmentTone: StudyNodeBriefing["alignmentTone"];
    if (matchedUnit.category === "unaligned") {
      alignmentBadge = "尚未对齐语义对象";
      alignmentTone = "unaligned";
    } else if (matchedUnit.category === "mastered_elsewhere") {
      alignmentBadge = "跨书共指 · 他处已掌握";
      alignmentTone = "cross";
    } else if (isCross) {
      alignmentBadge = "跨书共指 · 别的材料里见过";
      alignmentTone = "cross";
    } else if (hasDiff) {
      alignmentBadge = "已对齐 · 含局部条件差异";
      alignmentTone = "diff";
    } else {
      alignmentBadge = "已对齐共享语义";
      alignmentTone = "aligned";
    }

    const targets = matchedUnit.targets.map((t: StudyOutlineTarget) => ({
      entityId: t.entity_id,
      entityLabel: t.entity_label,
      relationText: relationLabel(t.relation),
      diffs: t.only_differs_by,
      seenInNames: t.seen_in.map(materialName),
      masteredElsewhere: Boolean(t.mastered_in && t.mastered_in.length > 0),
      staged: Boolean(t.staged),
    }));

    const targetEntityIds = new Set(matchedUnit.targets.map((t) => t.entity_id));
    const targetAlignmentIds = new Set(matchedUnit.targets.map((t) => t.alignment_id));
    const openDebts = (matchedMaterial?.open_debts ?? [])
      .filter((d) => {
        // Check if debt node connects to this unit, its alignments, or its target entities
        const debtNode = [...entitiesById.values()].find((n) => n.domain_id === d.debt_id);
        if (!debtNode) return false;
        return graph.edges.some((e) => {
          if (e.source !== debtNode.id && e.target !== debtNode.id) return false;
          if (targetAlignmentIds.has(e.relation_id)) return true;
          return relTouchesTargets(relations, entitiesById, debtNode.id, node.id, targetEntityIds);
        });
      })
      .map((d) => ({ debtId: d.debt_id, obligation: d.obligation, status: d.status }));

    return {
      nodeId: node.id,
      domainId: node.domain_id,
      domainKind: node.domain_kind,
      title: matchedUnit.unit_title || node.label,
      unitKind: matchedUnit.unit_kind,
      studyStatus:
        matchedUnit.study_status === "mastered" || matchedUnit.study_status === "studying"
          ? matchedUnit.study_status
          : "unread",
      alignmentBadge,
      alignmentTone,
      targets,
      intraBookNeighbors: intraBookNeighbors.slice(0, 6),
      openDebts,
      conventions: [],
    };
  }

  // Fallback briefing for SemanticEntity / Debt / DialectScope nodes
  if (
    node.domain_kind === "SemanticEntity" ||
    node.domain_kind === "RepresentationCore" ||
    node.domain_kind === "Debt" ||
    node.domain_kind === "DialectScope"
  ) {
    const alignedUnits: NodeBriefingNeighbor[] = [];
    for (const rel of relations) {
      if (!rel.all.includes(node.id)) continue;
      for (const otherId of rel.all) {
        if (otherId === node.id) continue;
        const other = entitiesById.get(otherId);
        if (!other) continue;
        alignedUnits.push({
          nodeId: other.id,
          domainId: other.domain_id,
          label: other.label,
          relationLabel: rel.node.label,
          direction: "incoming",
        });
      }
    }
    return {
      nodeId: node.id,
      domainId: node.domain_id,
      domainKind: node.domain_kind,
      title: node.label,
      unitKind: node.summary,
      studyStatus: null,
      alignmentBadge:
        node.domain_kind === "Debt"
          ? "未还清的数学账"
          : node.domain_kind === "DialectScope"
            ? "局部记号与编号约定"
            : node.summary ?? "共享数学语义对象",
      alignmentTone: node.domain_kind === "Debt" ? "debt" : "semantic",
      targets: [],
      intraBookNeighbors: alignedUnits.slice(0, 8),
      openDebts: [],
      conventions: [],
    };
  }

  return null;
}

function relTouchesTargets(
  relations: readonly IndexedRel[],
  entitiesById: ReadonlyMap<string, IncidenceNode>,
  debtNodeId: string,
  unitNodeId: string,
  targetEntityDomainIds: ReadonlySet<string>,
): boolean {
  for (const rel of relations) {
    if (rel.node.relation_kind !== "debt_on" || !rel.all.includes(debtNodeId)) continue;
    if (rel.all.includes(unitNodeId)) return true;
    for (const ep of rel.all) {
      const domId = entitiesById.get(ep)?.domain_id;
      if (domId && targetEntityDomainIds.has(domId)) return true;
    }
  }
  return false;
}
