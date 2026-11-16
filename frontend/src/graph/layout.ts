/**
 * Deterministic, physics-backed organic layout (`basePositions`) for the
 * Mathscape landscape graph.
 *
 * How it combines organic Hooke/Coulomb aesthetics with Mental-Map stability:
 * 1. Deterministic Phyllotaxis Seeding: nodes are sorted by stable ID and
 *    placed on a golden-angle spiral so no two nodes ever share `(0, 0)` —
 *    eliminating the only `Math.random()` symmetry-breaker inside Cytoscape's
 *    `cose` simulation (`cose.mjs:774`).
 * 2. Two-Stage Offscreen `cose`:
 *    - Stage 1 runs headless `cose` on the primary knowledge graph (PDFs,
 *      canonical Markdown / Ink snapshots, Content Units, Shared Semantic
 *      Entities, DialectScopes, Debts, and their real edges — no synthetic
 *      spaghetti bridges, and no JSON/image parse-metadata hairballs).
 *      Shared concepts & theories naturally settle at the gravitational hub
 *      of their aligned units ("众星捧月"), while aspect-aware component
 *      packing keeps canvas density high.
 *    - Stage 2 places `SourceSpan`s and auxiliary parse artifacts (`JSON`
 *      metadata, extracted page images) as compact local satellites around
 *      their owning unit or PDF so turning on `原文证据` or `总览` reveals them
 *      in-place without shifting the settled clusters.
 * 3. Gentle Local De-overlap: nudges only nearly-coincident siblings within a
 *    tight cap (`maxShift = 28px`) so wrapped Chinese labels do not sit dead-on
 *    top of each other while preserving the organic `cose` star-cluster shape.
 */
import cytoscape, { type EdgeSingular, type ElementDefinition, type NodeSingular } from "cytoscape";
import type { IncidenceEdge, IncidenceGraph, IncidenceNode, RelationKind } from "./projection";
import { nodeTypeKey, type NodeTypeKey } from "./taxonomy";
import { graphLabelText } from "./tex-plain";

export interface MapPoint {
  x: number;
  y: number;
}

export interface LayoutViewport {
  width: number;
  height: number;
}

const DEFAULT_VIEWPORT: LayoutViewport = { width: 1120, height: 760 };
const GOLDEN_ANGLE = Math.PI * (3 - Math.sqrt(5));
const LAYOUT_RANDOM_SEED = 0x4d617468;

const AUX_ARTIFACT_TYPES: ReadonlySet<NodeTypeKey> = new Set([
  "parse_metadata",
  "parse_image",
  "attachment",
]);

/**
 * Pipeline by-products (`application/json` parse metadata, extracted images,
 * raw attachments). Hidden in normal presets so PDF + Canonical Markdown stay
 * crisp; shown when the user selects `总览`.
 */
export function isAuxiliaryArtifactNode(node: IncidenceNode): boolean {
  return node.domain_kind === "SourceArtifact" && AUX_ARTIFACT_TYPES.has(nodeTypeKey(node));
}

interface ParsedRelation {
  id: string;
  domainId: string;
  domainKind: string | null;
  relationKind: RelationKind | null;
  label: string;
  into: Array<{ entity: string; role: string }>;
  out: Array<{ entity: string; role: string }>;
  all: Array<{ entity: string; role: string }>;
}

function parseRelations(graph: IncidenceGraph): {
  entities: Map<string, IncidenceNode>;
  relations: ParsedRelation[];
} {
  const entities = new Map<string, IncidenceNode>();
  const relNodes = new Map<string, IncidenceNode>();
  for (const node of graph.nodes) {
    if (node.element_kind === "entity") entities.set(node.id, node);
    else if (node.element_kind === "relation") relNodes.set(node.id, node);
  }
  const byRel = new Map<string, IncidenceEdge[]>();
  for (const edge of graph.edges) {
    const list = byRel.get(edge.relation_id) ?? [];
    list.push(edge);
    byRel.set(edge.relation_id, list);
  }
  const relations: ParsedRelation[] = [];
  for (const [domainId, rawEdges] of [...byRel.entries()].sort(([a], [b]) => a.localeCompare(b))) {
    const edges = [...rawEdges].sort((a, b) => a.id.localeCompare(b.id));
    let rn: IncidenceNode | undefined;
    const into: Array<{ entity: string; role: string }> = [];
    const out: Array<{ entity: string; role: string }> = [];
    const all: Array<{ entity: string; role: string }> = [];
    for (const edge of edges) {
      if (relNodes.has(edge.source)) {
        rn = relNodes.get(edge.source);
        if (!entities.has(edge.target)) continue;
        const ep = { entity: edge.target, role: edge.role };
        all.push(ep);
        if (edge.direction === "into_relation") into.push(ep);
        else if (edge.direction === "out_of_relation") out.push(ep);
      } else if (relNodes.has(edge.target)) {
        rn = relNodes.get(edge.target);
        if (!entities.has(edge.source)) continue;
        const ep = { entity: edge.source, role: edge.role };
        all.push(ep);
        if (edge.direction === "into_relation") into.push(ep);
        else if (edge.direction === "out_of_relation") out.push(ep);
      }
    }
    if (!rn || all.length < 2) continue;
    const cmpEp = (a: { entity: string; role: string }, b: { entity: string; role: string }) =>
      a.entity.localeCompare(b.entity) || a.role.localeCompare(b.role);
    into.sort(cmpEp);
    out.sort(cmpEp);
    all.sort(cmpEp);
    relations.push({
      id: rn.id,
      domainId,
      domainKind: rn.domain_kind,
      relationKind: rn.relation_kind,
      label: rn.label,
      into,
      out,
      all,
    });
  }
  return { entities, relations };
}

function hashId(id: string): number {
  let h = 2166136261;
  for (let i = 0; i < id.length; i++) {
    h ^= id.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

/** `cose` uses Math.random only to break exact node overlaps. Seed that
 * symmetry-breaker for this synchronous, offscreen simulation and restore the
 * host's RNG immediately afterwards. */
function runDeterministicCose(run: () => void): void {
  const originalRandom = Math.random;
  let state = LAYOUT_RANDOM_SEED;
  Math.random = () => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return state / 0x100000000;
  };
  try {
    run();
  } finally {
    Math.random = originalRandom;
  }
}

/**
 * Collapses duplicate paged `ConversionRun` relations that connect the exact
 * same visible `(inputs -> outputs)` pair (e.g. 5 paged runs of one PDF all
 * folded into its single Canonical Markdown node).
 */
export function collapseRedundantConversions(
  nodes: IncidenceNode[],
  edges: IncidenceEdge[],
): { nodes: IncidenceNode[]; edges: IncidenceEdge[] } {
  const relNodes = new Map<string, IncidenceNode>();
  for (const n of nodes) {
    if (n.element_kind === "relation" && n.domain_kind === "ConversionRun") {
      relNodes.set(n.id, n);
    }
  }
  if (relNodes.size === 0) return { nodes, edges };

  const byOwner = new Map<string, IncidenceEdge[]>();
  for (const e of edges) {
    const list = byOwner.get(e.relation_id) ?? [];
    list.push(e);
    byOwner.set(e.relation_id, list);
  }

  const seenPairs = new Set<string>();
  const dropRelNodeIds = new Set<string>();
  const dropOwnerIds = new Set<string>();

  const sortedRelIds = [...relNodes.keys()].sort();
  for (const relNodeId of sortedRelIds) {
    const rn = relNodes.get(relNodeId)!;
    const ownerId = rn.domain_id ?? rn.id;
    const spokes = byOwner.get(ownerId) ?? [];
    const ins = new Set<string>();
    const outs = new Set<string>();
    for (const s of spokes) {
      const entity = s.source === relNodeId ? s.target : s.source;
      if (s.direction === "into_relation") ins.add(entity);
      else if (s.direction === "out_of_relation") outs.add(entity);
    }
    if (ins.size === 0 || outs.size === 0) {
      dropRelNodeIds.add(relNodeId);
      dropOwnerIds.add(ownerId);
      continue;
    }
    const sig = `${[...ins].sort().join(",")}->${[...outs].sort().join(",")}`;
    if (seenPairs.has(sig)) {
      dropRelNodeIds.add(relNodeId);
      dropOwnerIds.add(ownerId);
    } else {
      seenPairs.add(sig);
    }
  }

  if (dropRelNodeIds.size === 0) return { nodes, edges };
  return {
    nodes: nodes.filter((n) => !dropRelNodeIds.has(n.id)),
    edges: edges.filter((e) => !dropOwnerIds.has(e.relation_id)),
  };
}

/**
 * Gently pushes apart only nodes that landed nearly on top of each other after
 * `cose` while keeping every node within `maxShift` of its organic equilibrium.
 */
function gentleLabelRelax(
  ids: readonly string[],
  positions: Map<string, MapPoint>,
  minGapX = 68,
  minGapY = 34,
  maxShift = 28,
  rounds = 6,
): void {
  const origin = new Map<string, MapPoint>();
  for (const id of ids) {
    const p = positions.get(id);
    if (p) origin.set(id, { x: p.x, y: p.y });
  }
  for (let r = 0; r < rounds; r++) {
    let moved = false;
    for (let i = 0; i < ids.length; i++) {
      const a = positions.get(ids[i])!;
      for (let j = i + 1; j < ids.length; j++) {
        const b = positions.get(ids[j])!;
        const dx = b.x - a.x;
        const dy = b.y - a.y;
        if (Math.abs(dx) >= minGapX || Math.abs(dy) >= minGapY) continue;
        const nx = dx / minGapX;
        const ny = dy / minGapY;
        const norm = Math.hypot(nx, ny);
        let ux: number;
        let uy: number;
        if (norm < 1e-5) {
          const angle = ((hashId(ids[i]) ^ hashId(ids[j])) % 16) * (Math.PI / 8);
          ux = Math.cos(angle);
          uy = Math.sin(angle);
        } else {
          ux = dx / Math.hypot(dx, dy);
          uy = dy / Math.hypot(dx, dy);
        }
        const overlap = 1 - Math.min(1, norm);
        const push = overlap * 12;
        a.x -= ux * push;
        a.y -= uy * push;
        b.x += ux * push;
        b.y += uy * push;
        moved = true;
      }
    }
    for (const id of ids) {
      const p = positions.get(id)!;
      const o = origin.get(id)!;
      const dx = p.x - o.x;
      const dy = p.y - o.y;
      const d = Math.hypot(dx, dy);
      if (d > maxShift && d > 0) {
        p.x = o.x + (dx / d) * maxShift;
        p.y = o.y + (dy / d) * maxShift;
      }
    }
    if (!moved) break;
  }
}

/**
 * Computes deterministic, order-invariant model positions for every node in
 * `graph` by running an offscreen, deterministically-seeded `cose` simulation
 * on the primary graph and placing fine-grained spans / metadata as satellites.
 */
export function computeLandscapeLayout(
  graph: IncidenceGraph,
  viewport: LayoutViewport = DEFAULT_VIEWPORT,
): Map<string, MapPoint> {
  const { entities, relations } = parseRelations(graph);
  const out = new Map<string, MapPoint>();
  if (entities.size === 0) return out;

  const vw = Math.max(640, Math.round(viewport.width || DEFAULT_VIEWPORT.width));
  const vh = Math.max(480, Math.round(viewport.height || DEFAULT_VIEWPORT.height));

  // Partition entities into Stage-1 Primary Backbone vs Stage-2 Satellites
  const primaryIds: string[] = [];
  const spanIds: string[] = [];
  const auxIds: string[] = [];

  for (const [id, node] of [...entities.entries()].sort(([a], [b]) => a.localeCompare(b))) {
    if (node.domain_kind === "SourceSpan") {
      spanIds.push(id);
    } else if (isAuxiliaryArtifactNode(node)) {
      auxIds.push(id);
    } else {
      primaryIds.push(id);
    }
  }

  const primarySet = new Set(primaryIds);

  // Build Stage-1 elements for offscreen `cose`:
  // - Deterministic golden-angle spiral initial positions (strictly distinct, sorted by id)
  // - Direct semantic edges between primary nodes (deduplicated conversions, multi-endpoint junctions)
  const simElements: ElementDefinition[] = [];
  primaryIds.forEach((id, idx) => {
    const node = entities.get(id)!;
    const typeKey = nodeTypeKey(node);
    const radius = 26 * Math.sqrt(idx + 1);
    const angle = idx * GOLDEN_ANGLE;
    simElements.push({
      group: "nodes",
      data: {
        id,
        nodeType: typeKey,
        domainKind: node.domain_kind,
        label: graphLabelText(node.label, node.summary),
      },
      position: {
        x: vw / 2 + Math.cos(angle) * radius,
        y: vh / 2 + Math.sin(angle) * radius,
      },
    });
  });

  const seenPairEdges = new Set<string>();
  for (const rel of [...relations].sort((a, b) => a.id.localeCompare(b.id))) {
    const ins = rel.into.map((e) => e.entity).filter((id) => primarySet.has(id));
    const outs = rel.out.map((e) => e.entity).filter((id) => primarySet.has(id));
    const regular = rel.all.map((e) => e.entity).filter((id) => primarySet.has(id));
    const uniqueRegular = [...new Set(regular)];
    if (uniqueRegular.length < 2) continue;

    if (ins.length >= 1 && outs.length >= 1 && uniqueRegular.length === 2) {
      const src = ins[0];
      const tgt = outs[0];
      if (src === tgt) continue;
      // Deduplicate parallel conversion runs on the same PDF -> Markdown pair
      if (rel.relationKind === "conversion") {
        const pairKey = `conv:${src}->${tgt}`;
        if (seenPairEdges.has(pairKey)) continue;
        seenPairEdges.add(pairKey);
      }
      simElements.push({
        group: "edges",
        data: {
          id: `sim~${rel.domainId}`,
          source: src,
          target: tgt,
          relationKind: rel.relationKind ?? "unknown",
          domainKind: rel.domainKind ?? "",
        },
      });
    } else if (uniqueRegular.length === 2) {
      simElements.push({
        group: "edges",
        data: {
          id: `sim~${rel.domainId}`,
          source: uniqueRegular[0],
          target: uniqueRegular[1],
          relationKind: rel.relationKind ?? "unknown",
          domainKind: rel.domainKind ?? "",
        },
      });
    } else {
      const juncId = `junc~${rel.domainId}`;
      const h = hashId(juncId);
      simElements.push({
        group: "nodes",
        data: { id: juncId, nodeType: "junction", domainKind: "junction" },
        position: {
          x: vw / 2 + ((h % 97) - 48) * 1.3,
          y: vh / 2 + (((h >>> 8) % 97) - 48) * 1.3,
        },
      });
      uniqueRegular.forEach((epId, idx) => {
        simElements.push({
          group: "edges",
          data: {
            id: `sim~${rel.domainId}~${idx}`,
            source: epId,
            target: juncId,
            relationKind: rel.relationKind ?? "unknown",
            domainKind: rel.domainKind ?? "",
          },
        });
      });
    }
  }

  if (primaryIds.length > 0) {
    const headless = cytoscape({
      headless: true,
      elements: simElements,
      style: [
        {
          selector: "node",
          style: {
            label: "data(label)",
            width: 36,
            height: 36,
            "text-wrap": "wrap",
            "text-max-width": "152px",
            "font-size": "11px",
          },
        },
        {
          selector: 'node[nodeType = "shared_entity"], node[nodeType = "shared_theory"]',
          style: { width: 44, height: 44 },
        },
        {
          selector: 'node[nodeType = "junction"]',
          style: { width: 12, height: 12 },
        },
      ],
    });
    try {
      // Override headless 0×0 dimensions so `cose` computes gravity & component
      // row wrapping against the real viewport aspect ratio without affine-stretching
      // natural spring lengths to touch the outer boundingBox edges.
      (headless as unknown as { width: () => number; height: () => number }).width = () => vw;
      (headless as unknown as { width: () => number; height: () => number }).height = () => vh;
      runDeterministicCose(() => {
        headless.elements().layout({
          name: "cose",
          animate: false,
          fit: false,
          randomize: false,
          nodeDimensionsIncludeLabels: true,
          componentSpacing: 84,
          nodeOverlap: 12,
          gravity: 1.15,
          numIter: 1000,
          nodeRepulsion: (node: NodeSingular) => {
            const t = String(node.data("nodeType") ?? "");
            if (t === "shared_entity" || t === "shared_theory") return 6200;
            if (t === "pdf" || t === "markdown") return 4800;
            if (t === "debt" || t === "dialect_scope") return 3200;
            return 4400;
          },
          idealEdgeLength: (edge: EdgeSingular) => {
            const rk = String(edge.data("relationKind") ?? "");
            if (rk === "debt_on" || rk === "scope_of") return 44;
            if (rk === "conversion") return 52;
            if (rk === "in_theory") return 62;
            if (rk === "aligns") return 76;
            return 62;
          },
          edgeElasticity: (edge: EdgeSingular) => {
            const rk = String(edge.data("relationKind") ?? "");
            if (rk === "debt_on" || rk === "scope_of") return 32;
            if (rk === "aligns" || rk === "in_theory") return 42;
            return 48;
          },
        }).run();
      });

      for (const id of primaryIds) {
        const p = headless.getElementById(id).position();
        out.set(id, { x: p.x, y: p.y });
      }
    } finally {
      headless.destroy();
    }

    gentleLabelRelax(primaryIds, out, 112, 52, 52, 12);
    for (const id of primaryIds) {
      const p = out.get(id)!;
      out.set(id, { x: Math.round(p.x), y: Math.round(p.y) });
    }
  }

  // Stage 2A: Place `SourceSpan`s as compact satellites around their owning unit (or artifact)
  const spanOwners = new Map<string, string[]>();
  for (const rel of relations) {
    if (rel.relationKind === "contains_span") {
      const unit = rel.into[0]?.entity;
      const span = rel.out[0]?.entity;
      if (unit && span && out.has(unit)) {
        const list = spanOwners.get(span) ?? [];
        if (!list.includes(unit)) list.push(unit);
        spanOwners.set(span, list);
      }
    }
  }
  for (const rel of relations) {
    if (rel.relationKind === "located_in") {
      const span = rel.into[0]?.entity;
      const art = rel.out[0]?.entity;
      if (span && art && !spanOwners.has(span) && out.has(art)) {
        spanOwners.set(span, [art]);
      }
    }
  }
  const slotByOwner = new Map<string, number>();
  for (const spanId of spanIds) {
    const owners = spanOwners.get(spanId) ?? [];
    if (owners.length === 0) {
      const idx = slotByOwner.get("__orphan_span__") ?? 0;
      slotByOwner.set("__orphan_span__", idx + 1);
      out.set(spanId, { x: (idx % 6) * 76, y: -120 - Math.floor(idx / 6) * 52 });
      continue;
    }
    const primaryOwner = owners[0];
    const anchor = out.get(primaryOwner)!;
    const slot = slotByOwner.get(primaryOwner) ?? 0;
    slotByOwner.set(primaryOwner, slot + 1);
    const baseAngle = ((hashId(primaryOwner) % 12) / 12) * Math.PI * 2;
    const angle = baseAngle + slot * GOLDEN_ANGLE;
    const radius = 42 + Math.floor(slot / 6) * 22;
    out.set(spanId, {
      x: Math.round(anchor.x + Math.cos(angle) * radius),
      y: Math.round(anchor.y + Math.sin(angle) * radius),
    });
  }

  // Stage 2B: Place auxiliary parse artifacts (`JSON` metadata, images) around their source PDF
  const auxAnchor = new Map<string, string>();
  for (const rel of relations) {
    if (rel.relationKind === "conversion") {
      const src = rel.into.map((e) => e.entity).find((id) => out.has(id));
      if (!src) continue;
      for (const ep of rel.out) {
        if (!out.has(ep.entity) && !auxAnchor.has(ep.entity)) {
          auxAnchor.set(ep.entity, src);
        }
      }
    }
  }
  for (const auxId of auxIds) {
    const owner = auxAnchor.get(auxId);
    if (!owner) {
      const idx = slotByOwner.get("__orphan_aux__") ?? 0;
      slotByOwner.set("__orphan_aux__", idx + 1);
      out.set(auxId, { x: (idx % 6) * 88, y: -220 - Math.floor(idx / 6) * 58 });
      continue;
    }
    const anchor = out.get(owner)!;
    const slot = slotByOwner.get(owner) ?? 0;
    slotByOwner.set(owner, slot + 1);
    const angle = -Math.PI / 2 + slot * GOLDEN_ANGLE;
    const radius = 58 + Math.floor(slot / 7) * 28;
    out.set(auxId, {
      x: Math.round(anchor.x + Math.cos(angle) * radius),
      y: Math.round(anchor.y + Math.sin(angle) * radius),
    });
  }

  // Stage 3: Place relation vertices & multi-endpoint junctions at the centroid of their endpoints
  for (const rel of relations) {
    const pts = rel.all.map((ep) => out.get(ep.entity)).filter((p): p is MapPoint => !!p);
    if (pts.length === 0) continue;
    const cx = Math.round(pts.reduce((acc, p) => acc + p.x, 0) / pts.length);
    const cy = Math.round(pts.reduce((acc, p) => acc + p.y, 0) / pts.length);
    out.set(rel.id, { x: cx, y: cy });
    out.set(`junc~${rel.domainId}`, { x: cx, y: cy });
  }

  return out;
}

/**
 * Keeps settled and user-pinned nodes motionless across filter toggles and
 * incremental sandbox proposals, while placing newly staged nodes gently near
 * their already-settled neighbors.
 */
export function resolveIncrementalPositions(
  graph: IncidenceGraph,
  settled: ReadonlyMap<string, MapPoint>,
  pinned: ReadonlyMap<string, MapPoint>,
  viewport: LayoutViewport = DEFAULT_VIEWPORT,
): Map<string, MapPoint> {
  if (settled.size === 0) {
    const full = computeLandscapeLayout(graph, viewport);
    for (const [id, p] of pinned) full.set(id, { ...p });
    return full;
  }

  const { entities, relations } = parseRelations(graph);
  const out = new Map<string, MapPoint>();
  const missingEntities: string[] = [];

  for (const id of entities.keys()) {
    const pin = pinned.get(id);
    const known = settled.get(id);
    if (pin) out.set(id, { ...pin });
    else if (known) out.set(id, { ...known });
    else missingEntities.push(id);
  }

  // If over half the map is brand new, run a fresh layout while keeping manual pins
  if (missingEntities.length > 0 && missingEntities.length >= entities.size * 0.5) {
    const full = computeLandscapeLayout(graph, viewport);
    for (const [id, p] of pinned) {
      if (entities.has(id)) full.set(id, { ...p });
    }
    return full;
  }

  if (missingEntities.length > 0) {
    const fullFallback = computeLandscapeLayout(graph, viewport);
    const adj = new Map<string, Set<string>>();
    for (const rel of relations) {
      for (const a of rel.all) {
        for (const b of rel.all) {
          if (a.entity === b.entity) continue;
          let set = adj.get(a.entity);
          if (!set) {
            set = new Set();
            adj.set(a.entity, set);
          }
          set.add(b.entity);
        }
      }
    }

    const occupied = [...out.values()];
    const findFreeSpot = (seed: MapPoint, key: string): MapPoint => {
      const collides = (pt: MapPoint) =>
        occupied.some((o) => Math.abs(o.x - pt.x) < 64 && Math.abs(o.y - pt.y) < 38);
      if (!collides(seed)) return seed;
      const startAngle = ((hashId(key) % 16) / 16) * Math.PI * 2;
      for (let step = 1; step <= 32; step++) {
        const angle = startAngle + step * GOLDEN_ANGLE;
        const radius = 48 + step * 11;
        const candidate = {
          x: Math.round(seed.x + Math.cos(angle) * radius),
          y: Math.round(seed.y + Math.sin(angle) * radius),
        };
        if (!collides(candidate)) return candidate;
      }
      return { x: seed.x + 72, y: seed.y + 44 };
    };

    const remaining = new Set(missingEntities);
    for (let pass = 0; pass < 4 && remaining.size > 0; pass++) {
      for (const id of [...remaining].sort()) {
        const neighbors = [...(adj.get(id) ?? [])]
          .map((nid) => out.get(nid))
          .filter((p): p is MapPoint => !!p);
        if (neighbors.length === 0 && pass < 3) continue;
        const seed =
          neighbors.length > 0
            ? {
                x: Math.round(neighbors.reduce((a, p) => a + p.x, 0) / neighbors.length),
                y: Math.round(neighbors.reduce((a, p) => a + p.y, 0) / neighbors.length),
              }
            : (fullFallback.get(id) ?? { x: 0, y: 0 });
        const spot = findFreeSpot(seed, id);
        out.set(id, spot);
        occupied.push(spot);
        remaining.delete(id);
      }
    }
  }

  for (const rel of relations) {
    const pts = rel.all.map((ep) => out.get(ep.entity)).filter((p): p is MapPoint => !!p);
    if (pts.length === 0) continue;
    const cx = Math.round(pts.reduce((acc, p) => acc + p.x, 0) / pts.length);
    const cy = Math.round(pts.reduce((acc, p) => acc + p.y, 0) / pts.length);
    out.set(rel.id, { x: cx, y: cy });
    out.set(`junc~${rel.domainId}`, { x: cx, y: cy });
  }

  return out;
}
