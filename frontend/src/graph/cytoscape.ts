import { graphPalette, resolveGraphColor, watchGraphTheme } from "./theme";
import cytoscape, {
  type Core,
  type ElementDefinition,
  type NodeSingular,
  type StylesheetJson,
} from "cytoscape";

import {
  GRAPH_PROJECTION_SCHEMA_VERSION,
  type IncidenceDirection,
  type IncidenceEdge,
  type IncidenceGraph,
  type IncidenceNode,
} from "./projection";
import { FOCUS_CONTRACTION_KIND } from "./focus";
import { computeLandscapeLayout, type MapPoint } from "./layout";
import { NODE_TYPES, NODE_TYPE_ORDER, nodeTypeKey } from "./taxonomy";
import { nodeHalo } from "./node-glyph";
import { AUTO_LABEL_HIDDEN, mapLabelScale } from "./label-density";
import {
  bindLabelHover,
  LABEL_HOVER,
  LABEL_RECESS,
  LABEL_HOVER_STYLE,
  LABEL_RECESS_STYLE,
  refreshLabels,
  syncNodeLabel,
  type LabelHoverOptions,
  type LabelSet,
} from "./label-hover";
import { graphLabelText, texToPlain } from "./tex-plain";

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
      classes: nodeClasses(node.boundary, node.lifecycle),
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

  return [...nodes, ...edges].map(plainLabels);
}

/**
 * 画布只能画纯文字：标签里的 TeX 换成可读的 Unicode（`$k\ge 4$` → `k ≥ 4`），
 * 在公式中间截断的标签先从摘要找回原文再截断。没有 `$` 的 `f_k = F_k` 也会转成下标。
 * 节点同时存下全文和短名，悬停时展开全文。
 */
function plainLabels(element: ElementDefinition): ElementDefinition {
  const label = element.data.label;
  if (typeof label !== "string") return element;
  const summary = typeof element.data.summary === "string" ? element.data.summary : null;
  if (element.group === "nodes") {
    const full = graphLabelText(label, summary, Number.POSITIVE_INFINITY);
    const compact = graphLabelText(label, summary);
    return { ...element, data: { ...element.data, label: compact, fullLabel: full, compactLabel: compact } };
  }
  return { ...element, data: { ...element.data, label: texToPlain(label) } };
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
    let relationLifecycle: string | null = null;

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
        relationLifecycle = rn.lifecycle;
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
        relationLifecycle = rn.lifecycle;
      }
    }

    if (endpoints.length < 2) continue;

    // Deduplicate identical endpoints (e.g. a folded canonical markdown node
    // referenced by multiple output spokes of one conversion run).
    const deduped: typeof endpoints = [];
    const seenEndpointKeys = new Set<string>();
    for (const ep of endpoints) {
      const key = `${ep.entity}:${ep.direction}:${ep.evidence ? "1" : "0"}`;
      if (seenEndpointKeys.has(key)) continue;
      seenEndpointKeys.add(key);
      deduped.push(ep);
    }
    if (deduped.length < 2) continue;

    const sources = deduped.filter(
      (e) => e.direction === "into_relation" && !e.evidence,
    );
    const targets = deduped.filter(
      (e) => e.direction === "out_of_relation" && !e.evidence,
    );
    const evidenceEndpoints = deduped.filter((e) => e.evidence);
    const regularEndpoints = deduped.filter((e) => !e.evidence);

    if (sources.length === 1 && targets.length === 1 && regularEndpoints.length === 2) {
      // Keep the mathematical source → target backbone direct even when the
      // relation has one or more evidence spans.
      result.push({
        group: "edges",
        classes: `${edgeClasses(true, true)}${contractionClass(relationDomainKind)}${semanticLayerClass(relationDomainKind)}${retiredClass(relationLifecycle)}`,
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

    if (deduped.length === 2) {
      // Binary relation without evidence: keep explicit direction, or fall
      // back to an undirected edge when the endpoint roles are ambiguous.
      const binarySources = deduped.filter((e) => e.direction === "into_relation");
      const binaryTargets = deduped.filter((e) => e.direction === "out_of_relation");

      let edgeSource: string;
      let edgeTarget: string;
      let directed: boolean;

      if (binarySources.length === 1 && binaryTargets.length === 1) {
        edgeSource = binarySources[0].entity;
        edgeTarget = binaryTargets[0].entity;
        directed = true;
      } else if (binarySources.length === 0 && binaryTargets.length === 0) {
        // Both undirected.
        edgeSource = deduped[0].entity;
        edgeTarget = deduped[1].entity;
        directed = false;
      } else {
        // Mixed or ambiguous directions: default to undirected to avoid fabricating arrows.
        edgeSource = deduped[0].entity;
        edgeTarget = deduped[1].entity;
        directed = false;
      }

      result.push({
        group: "edges",
        classes: `${edgeClasses(directed, true)}${semanticLayerClass(relationDomainKind)}${retiredClass(relationLifecycle)}`,
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

      for (let i = 0; i < deduped.length; i++) {
        const ep = deduped[i];
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

function contractionClass(domainKind: string | null): string {
  return domainKind === FOCUS_CONTRACTION_KIND ? " contracted-edge" : "";
}

/** 语义层的连线样式：对齐线、挂账、理论 / 约定卡连接。 */
export function semanticLayerClass(domainKind: string | null): string {
  switch (domainKind) {
    case "Alignment":
      return " alignment-edge";
    case "Debt":
      return " debt-edge";
    case "SemanticEntity":
    case "DialectScope":
      return " semantic-link-edge";
    default:
      return "";
  }
}

function retiredLifecycle(lifecycle: string | null | undefined): boolean {
  return lifecycle === "deprecated" || lifecycle === "superseded" || lifecycle === "rejected";
}

function nodeClasses(boundary: boolean, lifecycle: string | null): string | undefined {
  const classes: string[] = [];
  if (boundary) classes.push("is-boundary");
  if (retiredLifecycle(lifecycle)) classes.push("is-retired");
  return classes.length > 0 ? classes.join(" ") : undefined;
}

function retiredClass(lifecycle: string | null): string {
  return retiredLifecycle(lifecycle) ? " is-retired" : "";
}

function edgeClasses(directed: boolean, semantic = false): string {
  const directionClass = directed ? "is-directed" : "is-undirected";
  return semantic ? `${directionClass} semantic-edge` : directionClass;
}

// ---------------------------------------------------------------------------
// Cytoscape stylesheet
// ---------------------------------------------------------------------------

function graphStyles(): StylesheetJson {
  const t = graphPalette();
  const nodeTypeStyles: StylesheetJson = NODE_TYPE_ORDER.map((key) => {
    const visual = NODE_TYPES[key];
    return {
      selector: `node[nodeType = "${key}"]`,
      style: {
        shape: visual.shape,
        width: `${visual.width}px`,
        height: `${visual.height}px`,
        "background-color": resolveGraphColor(visual.background),
        "background-opacity": visual.backgroundOpacity,
        "border-color": resolveGraphColor(visual.border),
        "border-style": visual.borderStyle,
        "border-width": 1,
        "font-size": (node: NodeSingular) => visual.fontSize * mapLabelScale(node),
      },
    };
  });

  return [
    {
      selector: "node",
      style: {
        label: "data(label)",
        "text-wrap": "wrap",
        "text-max-width": (node: NodeSingular) => `${150 * mapLabelScale(node)}px`,
        "text-events": "yes",
        "font-size": "11px",
        "background-color": t.accent,
        "border-width": 1,
        "border-color": t.accent,
        color: t.text,
        "text-background-color": t.canvas,
        "text-background-opacity": 0,
        "text-background-padding": "3px",
        "text-outline-color": t.canvas,
        "text-outline-width": 2,
        "text-valign": "bottom",
        "text-margin-y": 7,
      },
    },
    ...nodeTypeStyles,
    {
      selector: 'node[nodeType = "semantic_unit"], node[nodeType = "shared_theory"]',
      style: {
        "background-image": nodeHalo(t.accent),
        "background-width": 38, "background-height": 38,
        "background-clip": "none", "background-image-containment": "over",
        "bounds-expansion": 14,
        "background-opacity": 0.8,
      },
    },
    {
      selector: 'node[nodeType = "shared_entity"]',
      style: {
        "background-image": nodeHalo(t.gold, true),
        "background-width": 48, "background-height": 48,
        "background-clip": "none", "background-image-containment": "over",
        "bounds-expansion": 16,
        "text-halign": "left", "text-valign": "center", "text-margin-x": -12, "text-margin-y": 0,
      },
    },
    {
      selector: 'node[lifecycle = "candidate"]',
      style: { "border-style": "dashed", "border-color": t.candidate, "background-opacity": 0.18, "background-image": nodeHalo(t.candidate) },
    },
    {
      selector: 'node[elementKind = "junction"]',
      style: {
        shape: "ellipse",
        width: "12px",
        height: "12px",
        "background-color": t.gold,
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
        width: 1.2,
        "curve-style": "bezier",
        "line-color": t.edge,
        "target-arrow-color": t.edge,
        "target-arrow-shape": "triangle",
        "font-size": "9px",
        color: t.text,
        // 连线标签水平放，以细描边光晕保持清晰，不遮住其他星轨。
        "text-rotation": "none",
        // At rest, titles take priority. Relation text is available on hover
        // or selection rather than competing with nearby mathematical nodes.
        "text-opacity": 0,
        "text-background-color": t.canvas,
        "text-background-opacity": 0,
        "text-background-shape": "roundrectangle",
        "text-background-padding": "2px",
        "text-outline-color": t.canvas,
        "text-outline-width": 2,
      },
    },
    {
      selector: "edge.semantic-edge",
      style: {
        label: "data(label)",
        "line-color": t.edge,
        "target-arrow-color": t.edge,
        color: t.text,
      },
    },
    {
      selector: "edge.evidence-edge",
      style: {
        label: "data(label)",
        width: 1.2,
        "line-style": "dashed",
        "line-dash-pattern": [3, 4],
        "line-color": t.muted,
        "target-arrow-shape": "none",
        color: t.text,
        "font-size": "8px",
      },
    },
    {
      selector: "edge.is-directed",
      style: {
        width: 1.2,
        "target-arrow-shape": "triangle",
        "target-arrow-color": t.accent,
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
    {
      selector: "edge.alignment-edge",
      style: {
        width: 1.2,
        "line-color": t.primaryEdge,
        "target-arrow-color": t.primaryEdge,
        color: t.text,
      },
    },
    {
      selector: "edge.debt-edge",
      style: {
        width: 1.4,
        "line-style": "dashed",
        "line-color": t.warning,
        "target-arrow-shape": "none",
        color: t.warning,
        "font-size": "8px",
      },
    },
    {
      selector: "edge.semantic-link-edge",
      style: {
        width: 1.2,
        "line-style": "dotted",
        "target-arrow-shape": "none",
        "font-size": "8px",
      },
    },
    // Declared last so a contracted path keeps its own weight and arrow colour
    // instead of inheriting the directed-relation emphasis.
    {
      selector: "edge.contracted-edge",
      style: {
        label: "data(label)",
        width: 1.6,
        "line-style": "dotted",
        "line-color": t.edge,
        "target-arrow-shape": "triangle",
        "target-arrow-color": t.edge,
        "arrow-scale": 1.2,
        color: t.text,
        "font-size": "9px",
      },
    },
    {
      selector: ".is-retired",
      style: {
        opacity: 0.4,
        "border-style": "dashed",
        "line-style": "dashed",
      },
    },
    // Phase2 staged proposals (staged-overlay.ts): dashed, translucent, badged.
    {
      selector: ".is-staged",
      style: {
        opacity: 0.84,
        "background-color": t.candidate,
        "background-opacity": 0.16,
        "background-image": nodeHalo(t.candidate),
        "border-style": "dashed",
        "border-width": 2,
        "border-color": t.candidate,
        "line-style": "dashed",
        "line-color": t.candidate,
      },
    },
    {
      selector: "node.is-staged",
      style: {
        // Pending titles include an explicit status badge: keep that longer
        // label below the node, not in the neighboring node's horizontal slot.
        "text-halign": "center", "text-valign": "bottom", "text-margin-x": 0, "text-margin-y": 7,
      },
    },
    {
      selector: ".is-staged-conflict",
      style: {
        "border-style": "double",
        "border-width": 4,
        "border-color": t.error,
      },
    },
    // N2 "我的手稿" overlay (ink-overlay.ts): synthesized, never projected.
    {
      selector: "node.has-ink-notes",
      style: { "border-color": t.candidate, "border-width": 2.5 },
    },
    // Block-level Markdown errata non-blocking review cue (errata-overlay.ts).
    {
      selector: "node.has-errata-review",
      style: {
        "border-color": t.success,
        "border-style": "dashed",
        "border-width": 2.5,
        "overlay-color": t.success,
        "overlay-opacity": 0.14,
        "overlay-padding": 5,
      },
    },
    {
      selector: "node.ink-note-node",
      style: {
        shape: "round-rectangle",
        width: "26px",
        height: "18px",
        "background-color": t.candidate,
        "border-color": t.candidate,
        "border-width": 1.5,
        "border-style": "solid",
        color: t.text,
        "font-size": "10px",
      },
    },
    {
      selector: "edge.ink-note-edge",
      style: {
        width: 1.2,
        "line-color": t.candidate,
        "line-style": "dashed",
        "target-arrow-shape": "none",
        "curve-style": "straight",
        label: "",
        opacity: 0.8,
      },
    },
    {
      selector: "edge.ink-note-edge.is-primary",
      style: { width: 2 },
    },
    // A-PR3 sandbox echo (graph/sandbox-echo.ts) and the Map Scene Player
    // (graph/scene-player.ts) share these: transient, never persisted.
    // Seeds = what a tool call asked about, hits = what its result returned;
    // `-fade` is the second half of the 2–3s pulse.
    {
      selector: "node.sandbox-echo-seed, node.sandbox-echo-hit",
      style: {
        "overlay-opacity": 0.28,
        "overlay-padding": 7,
        "transition-property": "overlay-opacity",
        "transition-duration": 280,
      },
    },
    { selector: "node.sandbox-echo-seed", style: { "overlay-color": t.gold } },
    { selector: "node.sandbox-echo-hit", style: { "overlay-color": t.accent } },
    { selector: "node.sandbox-echo-fade", style: { "overlay-opacity": 0.1 } },
    // Sync ensure: the PDF a waiting material.read is parsing (「解析中」).
    {
      selector: "node.scene-parsing",
      style: { "overlay-color": t.accent, "overlay-opacity": 0.34, "border-style": "dashed" },
    },
    {
      selector: "edge.sandbox-echo-edge",
      style: { "overlay-color": t.accent, "overlay-opacity": 0.22, "overlay-padding": 3 },
    },
    { selector: "edge.sandbox-echo-edge.sandbox-echo-fade", style: { "overlay-opacity": 0.08 } },
    // propose_changes scene (scene-player.ts): transient ghosts for proposed
    // units not yet on the map (same dashed yellow as the staged overlay),
    // relations drawn one by one, blocked items in red, superseded ones faded.
    {
      selector: "node.scene-ghost",
      style: {
        shape: "round-rectangle",
        width: "30px",
        height: "22px",
        "background-color": t.candidate,
        "background-opacity": 0.55,
        "border-width": 2,
        "border-style": "dashed",
        "border-color": t.candidate,
        color: t.text,
        "font-size": "10px",
        opacity: 0.9,
      },
    },
    { selector: "node.scene-ghost-span", style: { width: "16px", height: "12px", "font-size": "9px" } },
    {
      selector: "node.scene-ghost-semantic",
      style: { "background-opacity": 0.7, "border-width": 2.2, "font-size": "10px", opacity: 0.95 },
    },
    { selector: "node.scene-ghost-theory", style: { shape: "hexagon", "background-color": t.candidate, "border-color": t.candidate, color: t.text } },
    { selector: "node.scene-ghost-concept", style: { shape: "ellipse", "background-color": t.candidate, "border-color": t.candidate, color: t.text } },
    { selector: "node.scene-ghost-declaration", style: { shape: "round-rectangle", "background-color": t.candidate, "border-color": t.candidate, color: t.text } },
    { selector: "node.scene-ghost-problem-schema", style: { shape: "diamond", "background-color": t.candidate, "border-color": t.candidate, color: t.text } },
    { selector: "node.scene-ghost-debt", style: { shape: "diamond", "background-color": t.warning, "border-color": t.warning, color: t.warning } },
    { selector: "node.scene-ghost-dialect-scope", style: { shape: "round-rectangle", "background-color": t.candidate, "border-color": t.candidate, color: t.text } },
    { selector: "edge.scene-ghost-alignment-edge", style: { "line-color": t.candidate, "target-arrow-color": t.candidate, color: t.text } },
    { selector: "edge.scene-ghost-theory-edge, edge.scene-ghost-uses-edge", style: { "line-color": t.candidate, "target-arrow-color": t.candidate, color: t.text } },
    { selector: "edge.scene-ghost-debt-edge", style: { "line-color": t.warning, "target-arrow-color": t.warning, color: t.warning } },
    { selector: "edge.scene-ghost-scope-edge", style: { "line-color": t.candidate, "target-arrow-color": t.candidate, color: t.text } },
    {
      selector: "edge.scene-ghost-edge",
      style: {
        label: "data(label)",
        width: 1.6,
        "line-style": "dashed",
        "line-color": t.candidate,
        "target-arrow-shape": "triangle",
        "target-arrow-color": t.candidate,
        color: t.text,
        "font-size": "9px",
        opacity: 0.85,
      },
    },
    {
      selector: "node.scene-propose-hit",
      style: { "overlay-color": t.candidate, "overlay-opacity": 0.3, "overlay-padding": 7 },
    },
    {
      selector: "edge.scene-propose-edge",
      style: { "overlay-color": t.candidate, "overlay-opacity": 0.3, "overlay-padding": 3 },
    },
    {
      selector: "node.scene-propose-semantic-hit",
      style: { "overlay-opacity": 0.34, "overlay-padding": 8 },
    },
    { selector: "node.scene-propose-theory-hit", style: { "overlay-color": t.candidate } },
    { selector: "node.scene-propose-concept-hit", style: { "overlay-color": t.candidate } },
    { selector: "node.scene-propose-declaration-hit", style: { "overlay-color": t.candidate } },
    { selector: "node.scene-propose-problem-schema-hit", style: { "overlay-color": t.candidate } },
    { selector: "node.scene-propose-debt-hit", style: { "overlay-color": t.warning } },
    { selector: "node.scene-propose-dialect-scope-hit", style: { "overlay-color": t.candidate } },
    { selector: "edge.scene-propose-alignment-edge", style: { "overlay-color": t.candidate, "overlay-opacity": 0.38, "overlay-padding": 4 } },
    { selector: "edge.scene-propose-theory-edge, edge.scene-propose-uses-edge", style: { "overlay-color": t.candidate, "overlay-opacity": 0.32, "overlay-padding": 4 } },
    { selector: "edge.scene-propose-debt-edge", style: { "overlay-color": t.warning, "overlay-opacity": 0.38, "overlay-padding": 4 } },
    { selector: "edge.scene-propose-scope-edge", style: { "overlay-color": t.candidate, "overlay-opacity": 0.32, "overlay-padding": 4 } },
    {
      selector: "node.scene-blocked",
      style: { "border-color": t.error, "border-style": "solid", "border-width": 3, color: t.error },
    },
    {
      selector: "edge.scene-blocked",
      style: { "line-color": t.error, "target-arrow-color": t.error },
    },
    {
      selector: ".scene-blocked-flash",
      style: { "overlay-color": t.error, "overlay-opacity": 0.45, "overlay-padding": 8 },
    },
    {
      selector: ".scene-retired",
      style: { opacity: 0.22, "transition-property": "opacity", "transition-duration": 400 },
    },

    // 地图姿态与增量生长动效（GraphChoreographer & StudySpinePosture）
    {
      selector: ".is-choreo-entering",
      style: { opacity: 0.05 },
    },
    {
      selector: ".is-choreo-entered",
      style: {
        opacity: 1,
        "transition-property": "opacity",
        "transition-duration": 220,
      },
    },
    {
      selector: "node.spine-header-node",
      style: {
        "border-width": 2.5,
        "border-color": t.accent,
        "text-max-width": "260px",
        "font-size": "12px",
        color: t.text,
      },
    },
    {
      selector: "node.spine-unit-node",
      style: {
        "text-valign": "center",
        "text-halign": "right",
        "text-margin-x": 10,
        "text-max-width": "240px",
      },
    },
    {
      selector: "node.orbit-semantic-node",
      style: {
        "text-max-width": "210px",
        "font-size": "12px",
      },
    },
    {
      selector: "node.orbit-outer-node",
      style: {
        "text-max-width": "210px",
      },
    },
    {
      selector: "node.study-status-mastered",
      style: {
        "border-color": t.success,
        "border-width": 3.2,
        "border-style": "solid",
        "underlay-color": t.success,
        "underlay-opacity": 0.16,
        "underlay-padding": 4,
      },
    },
    {
      selector: "node.study-status-studying",
      style: {
        "border-color": t.accent,
        "border-width": 2.8,
        "border-style": "solid",
      },
    },
    {
      selector: "node.study-cat-diff",
      style: {
        "underlay-color": t.gold,
        "underlay-opacity": 0.22,
        "underlay-padding": 5,
      },
    },
    {
      selector: "node.study-cat-cross",
      style: {
        "underlay-color": t.candidate,
        "underlay-opacity": 0.22,
        "underlay-padding": 5,
      },
    },
    {
      selector: "edge.spine-tree-edge",
      style: {
        label: "",
        width: 1.2,
        "line-color": t.accent,
        "target-arrow-color": t.accent,
        "arrow-scale": 1.15,
        opacity: 0.72,
      },
    },
    {
      selector: "edge.spine-arc-edge",
      style: {
        "curve-style": "unbundled-bezier",
        "control-point-distances": "data(spineArcOffset)",
        "control-point-weights": 0.5,
        width: 1.9,
        opacity: 0.88,
      },
    },
    {
      selector: "node.is-study-signal-hit",
      style: {
        "border-width": 3.2,
        "border-color": t.gold,
        "z-index": 26,
        opacity: 1,
      },
    },
    {
      selector: "edge.is-study-signal-hit",
      style: {
        width: 2.8,
        "z-index": 24,
        opacity: 1,
      },
    },
    {
      selector: "node.is-study-signal-muted",
      style: {
        opacity: 0.24,
      },
    },
    {
      selector: "edge.is-study-signal-muted",
      style: {
        opacity: 0.14,
        "text-opacity": 0,
        "text-background-opacity": 0,
        "text-border-opacity": 0,
      },
    },

    // 单击选中高亮（Selection Spotlight）：凸显当前节点及其直接关联边/邻居，退后其余背景且隐藏无关连线标签。
    {
      selector: "node.is-selection-receded",
      style: {
        opacity: 0.2,
      },
    },
    {
      selector: "edge.is-selection-receded",
      style: {
        opacity: 0.12,
        "text-opacity": 0,
        "text-background-opacity": 0,
        "text-border-opacity": 0,
      },
    },
    {
      selector: `node.${AUTO_LABEL_HIDDEN}`,
      style: { "text-opacity": 0, "text-events": "no" },
    },
    {
      selector: "node.is-focus-root",
      style: {
        "text-opacity": 1,
        "text-events": "yes",
        "border-width": 4,
        "border-color": t.gold,
        "underlay-color": t.gold,
        "underlay-opacity": 0.25,
        "underlay-padding": 6,
        "z-index": 32,
        opacity: 1,
      },
    },
    {
      selector: "edge.is-focus-root",
      style: {
        width: 3.4,
        "underlay-color": t.gold,
        "underlay-opacity": 0.25,
        "underlay-padding": 4,
        "z-index": 32,
        opacity: 1,
      },
    },
    {
      selector: "node.is-selected-neighbor",
      style: {
        "border-width": 2.6,
        "border-color": t.accent,
        "underlay-color": t.accent,
        "underlay-opacity": 0.2,
        "underlay-padding": 5,
        "z-index": 36,
        opacity: 1,
        color: t.text,
        "text-background-opacity": 0,
      },
    },
    {
      selector: "node.is-selected-primary, node:selected",
      style: {
        "text-opacity": 1,
        "text-events": "yes",
        "border-width": 3.6,
        "border-color": t.gold,
        "underlay-color": t.gold,
        "underlay-opacity": 0.3,
        "underlay-padding": 7,
        "z-index": 50,
        opacity: 1,
        color: t.gold,
        "text-background-color": t.canvas,
        "text-background-opacity": 0,
        "text-border-color": t.canvas,
        "text-border-width": 0,
        "text-border-opacity": 1,
      },
    },
    {
      selector: "edge.is-selected-edge, edge:selected",
      style: {
        "text-opacity": 1,
        width: 3.2,
        opacity: 1,
        "z-index": 45,
        color: t.gold,
        "text-background-color": t.canvas,
        "text-background-opacity": 0,
        "text-border-color": t.canvas,
        "text-border-width": 0,
        "text-border-opacity": 1,
      },
    },

    {
      selector: "edge.is-theme-hover",
      style: { "line-color": t.accent, "target-arrow-color": t.accent, width: 1.8 },
    },

    // Out-of-focus context: placed after staged / ink / selection rules so out-of-focus
    // elements are always faint, label-free, and non-interactive.
    {
      selector: ".is-out-of-focus",
      style: {
        opacity: 0.08,
        events: "no",
        label: "",
        "underlay-opacity": 0,
      },
    },

    {
      selector: `node.${LABEL_HOVER}`,
      style: { ...LABEL_HOVER_STYLE },
    },
    {
      selector: `edge.${LABEL_HOVER}`,
      style: {
        "text-opacity": 1,
        "text-background-color": t.canvas,
        "text-background-opacity": 0,
        "text-background-shape": "roundrectangle",
        "text-background-padding": "3px",
        "text-border-color": t.canvas,
        "text-border-width": 0,
        "text-border-opacity": 1,
        "z-index": 1000,
        "z-index-compare": "manual",
        "z-compound-depth": "top",
        opacity: 1,
      },
    },
    {
      selector: `.${LABEL_RECESS}`,
      style: { ...LABEL_RECESS_STYLE },
    },
  ];

}

export const OUT_OF_FOCUS_CLASS = "is-out-of-focus";
export const FOCUS_ROOT_CLASS = "is-focus-root";
export const SELECTED_PRIMARY_CLASS = "is-selected-primary";
export const SELECTED_NEIGHBOR_CLASS = "is-selected-neighbor";
export const SELECTED_EDGE_CLASS = "is-selected-edge";
export const SELECTION_RECEDED_CLASS = "is-selection-receded";

// ---------------------------------------------------------------------------
// Graph mounting
// ---------------------------------------------------------------------------

function fitLandscape(core: Core): void {
  core.fit(undefined, 48);
  // Sparse maps should not turn a star into a large flowchart block.
  if (core.zoom() > 1.35) core.zoom({ level: 1.35, renderedPosition: { x: core.width() / 2, y: core.height() / 2 } });
}

export function mountCytoscapeGraph(
  container: HTMLElement,
  graph: IncidenceGraph,
  renderMode: RenderMode = "semantic",
  options?: { labelHover?: LabelHoverOptions; positions?: ReadonlyMap<string, MapPoint> },
): Core {
  const rect = container.getBoundingClientRect();
  const positions =
    options?.positions ??
    computeLandscapeLayout(graph, { width: rect.width, height: rect.height });
  const elements = toCytoscapeElements(graph, renderMode).map((el) => {
    if (el.group !== "nodes") return el;
    const pos = positions.get(String(el.data.id));
    return pos ? { ...el, position: { x: pos.x, y: pos.y } } : el;
  });
  const core = cytoscape({
    container,
    elements,
    style: graphStyles(),
    layout: { name: "preset", fit: false },
    minZoom: 0.08,
    maxZoom: 4,
  });
  if (core.elements().nonempty() && core.width() > 0 && core.height() > 0) {
    fitLandscape(core);
  }
  watchGraphTheme(core, () => {
    core.style(graphStyles());
    // Label texture caches include the old outline colour.
    refreshLabels(core, core.elements() as unknown as LabelSet);
  });
  const linked = (node: import("cytoscape").NodeSingular, active: boolean) => {
    node.connectedEdges().toggleClass("is-theme-hover", active);
  };
  core.on("mouseover", "node", event => linked(event.target, true));
  core.on("mouseout", "node", event => linked(event.target, false));
  bindLabelHover(core, options?.labelHover);
  return core;
}

/**
 * Brings an already-mounted instance in line with `graph` without tearing it
 * down, using deterministic `basePositions` so toggling filter checkboxes in
 * any order places every node back at its exact settled coordinate.
 */
export function syncCytoscapeGraph(
  core: Core,
  graph: IncidenceGraph,
  renderMode: RenderMode = "semantic",
  options?: {
    positions?: ReadonlyMap<string, MapPoint>;
    fit?: boolean;
    updateExistingPositions?: boolean;
  },
): void {
  const positions =
    options?.positions ??
    computeLandscapeLayout(graph, { width: core.width(), height: core.height() });
  const desired = new Map(
    toCytoscapeElements(graph, renderMode).map(
      (element) => [String(element.data.id), element] as const,
    ),
  );

  const wasEmpty = core.nodes().empty();
  const staleIds: string[] = [];
  const additions: ElementDefinition[] = [];
  core.elements().forEach((element) => {
    if (!desired.has(element.id())) staleIds.push(element.id());
  });
  for (const [id, element] of desired) {
    const existing = core.getElementById(id);
    if (existing.empty()) {
      const pos = element.group === "nodes" ? positions.get(id) : undefined;
      additions.push(pos ? { ...element, position: { x: pos.x, y: pos.y } } : element);
      continue;
    }
    // Cytoscape cannot repoint an existing edge, so an edge that kept its ID
    // while changing endpoints has to be replaced rather than updated.
    if (
      existing.isEdge() &&
      (existing.data("source") !== element.data.source ||
        existing.data("target") !== element.data.target)
    ) {
      staleIds.push(id);
      additions.push(element);
      continue;
    }
    existing.data(element.data);
    existing.classes(element.classes ?? "");
    if (
      existing.isNode() &&
      (options?.updateExistingPositions || existing.data("elementKind") === "junction")
    ) {
      const pos = positions.get(id);
      if (pos) existing.position({ x: pos.x, y: pos.y });
    }
  }

  const allPriorRemoved =
    !wasEmpty &&
    staleIds.filter((id) => core.getElementById(id).isNode()).length ===
      core.nodes().length;

  const addedNodes = additions.filter((el) => el.group === "nodes");
  const addedEdges = additions.filter((el) => el.group === "edges");
  core.batch(() => {
    core.remove(
      staleIds.reduce(
        (collection, id) => collection.union(core.getElementById(id)),
        core.collection(),
      ),
    );
    if (addedNodes.length > 0) core.add(addedNodes);
    if (addedEdges.length > 0) core.add(addedEdges);
  });

  const shouldFit =
    options?.fit ?? ((wasEmpty || allPriorRemoved) && core.nodes().nonempty());
  if (shouldFit && core.nodes().nonempty() && core.width() > 0 && core.height() > 0) {
    core.fit(undefined, 28);
  }
}

/**
 * Restricts emphasis to the projection nodes in `inScopeIds`, or clears the
 * restriction when it is `null`. Elements outside the scope stay mounted and
 * keep their coordinates, so the focused region keeps its place in the map.
 *
 * Only projection node IDs are passed in: the synthetic junctions and edges
 * that semantic-edge mode invents have IDs the caller cannot know, so their
 * scope is derived from the endpoints they connect.
 */
export function applyFocusScope(
  core: Core,
  inScopeIds: ReadonlySet<string> | null,
  inScopeRelationIds?: ReadonlySet<string> | null,
  focusRootId?: string | null,
  focusRootDomainId?: string | null,
): void {
  core.batch(() => {
    core.elements().removeClass(FOCUS_ROOT_CLASS);
    if (inScopeIds === null) {
      core.elements().removeClass(OUT_OF_FOCUS_CLASS);
      return;
    }
    const junctions = core.nodes('[elementKind = "junction"]');
    const inkNodes = core.nodes(".ink-note-node");
    core.nodes().difference(junctions).difference(inkNodes).forEach((node) => {
      node.toggleClass(OUT_OF_FOCUS_CLASS, !inScopeIds.has(node.id()));
      if (focusRootId && node.id() === focusRootId) {
        node.addClass(FOCUS_ROOT_CLASS);
      }
    });
    // Synthesized ink-note nodes stay visible iff at least one attached host target is in focus.
    inkNodes.forEach((inkNode) => {
      const hosts = inkNode.connectedEdges(".ink-note-edge").connectedNodes().difference(inkNodes);
      const anyHostInScope = hosts.nonempty() && hosts.not(`.${OUT_OF_FOCUS_CLASS}`).nonempty();
      inkNode.toggleClass(OUT_OF_FOCUS_CLASS, !anyHostInScope);
    });
    // A junction stands in for one relation, so it belongs to the focused
    // region only when every endpoint it gathers does (and its relation is in scope).
    junctions.forEach((junction) => {
      const endpoints = junction.neighborhood().nodes();
      const relId = junction.data("domainId") ?? junction.id();
      const relAllowed =
        !inScopeRelationIds || (typeof relId === "string" && inScopeRelationIds.has(relId));
      const inScope =
        relAllowed &&
        endpoints.nonempty() &&
        endpoints.filter(`.${OUT_OF_FOCUS_CLASS}`).empty();
      junction.toggleClass(OUT_OF_FOCUS_CLASS, !inScope);
    });
    core.edges().forEach((edge) => {
      const relId = edge.data("relationId");
      const relAllowed =
        !inScopeRelationIds ||
        edge.hasClass("ink-note-edge") ||
        (typeof relId === "string" && inScopeRelationIds.has(relId));
      const inScope =
        relAllowed &&
        !edge.source().hasClass(OUT_OF_FOCUS_CLASS) &&
        !edge.target().hasClass(OUT_OF_FOCUS_CLASS);
      edge.toggleClass(OUT_OF_FOCUS_CLASS, !inScope);
      if (inScope && focusRootDomainId && relId === focusRootDomainId) {
        edge.addClass(FOCUS_ROOT_CLASS);
      }
    });
  });
}

/**
 * Highlights a single-clicked node or relation (`selectedId`) together with its
 * 1-hop connected edges and neighbor nodes, gently receding all other visible
 * elements and hiding non-incident edge labels to cut visual clutter.
 */
export function applySelectionSpotlight(
  core: Core,
  selectedId: string | null,
  displayedGraph?: IncidenceGraph,
): void {
  const SPOTLIGHT_CLASSES = `${SELECTED_PRIMARY_CLASS} ${SELECTED_NEIGHBOR_CLASS} ${SELECTED_EDGE_CLASS} ${SELECTION_RECEDED_CLASS}`;
  const previouslyHighlighted = core.nodes(`.${SELECTED_PRIMARY_CLASS}`);
  core.batch(() => {
    core.elements().removeClass(SPOTLIGHT_CLASSES);
    previouslyHighlighted.forEach((node) => {
      syncNodeLabel(node, node.hasClass(LABEL_HOVER) || node.hasClass(FOCUS_ROOT_CLASS));
    });
  });

  if (!selectedId) {
    if (previouslyHighlighted.nonempty()) {
      refreshLabels(core, previouslyHighlighted as unknown as LabelSet);
    }
    return;
  }

  let primaryNodes = core.collection();
  let neighborNodes = core.collection();
  let highlightedEdges = core.collection();

  const directNode = core.getElementById(selectedId);
  if (directNode.nonempty() && directNode.isNode() && !directNode.hasClass(OUT_OF_FOCUS_CLASS)) {
    primaryNodes = directNode;
    let incEdges = directNode.connectedEdges().not(`.${OUT_OF_FOCUS_CLASS}`);
    const adjJunctions = incEdges.connectedNodes('[elementKind = "junction"]').not(`.${OUT_OF_FOCUS_CLASS}`);
    if (adjJunctions.nonempty()) {
      incEdges = incEdges.union(adjJunctions.connectedEdges().not(`.${OUT_OF_FOCUS_CLASS}`));
    }
    highlightedEdges = incEdges;
    neighborNodes = incEdges.connectedNodes().not(`.${OUT_OF_FOCUS_CLASS}`).difference(directNode);
  } else if (displayedGraph) {
    const relModel = displayedGraph.nodes.find(
      (n) => n.id === selectedId && n.element_kind === "relation",
    );
    if (relModel) {
      const relKey = relModel.domain_id ?? relModel.id;
      const matchingEdges = core.edges().filter(
        (e) => e.data("relationId") === relKey && !e.hasClass(OUT_OF_FOCUS_CLASS),
      );
      if (matchingEdges.nonempty()) {
        highlightedEdges = matchingEdges;
        neighborNodes = matchingEdges.connectedNodes().not(`.${OUT_OF_FOCUS_CLASS}`);
      }
    }
  }

  if (primaryNodes.empty() && highlightedEdges.empty()) {
    if (previouslyHighlighted.nonempty()) {
      refreshLabels(core, previouslyHighlighted as unknown as LabelSet);
    }
    return;
  }

  const inSpotlight = primaryNodes.union(neighborNodes).union(highlightedEdges);
  core.batch(() => {
    primaryNodes.nodes().addClass(SELECTED_PRIMARY_CLASS);
    neighborNodes.nodes().addClass(SELECTED_NEIGHBOR_CLASS);
    highlightedEdges.edges().addClass(SELECTED_EDGE_CLASS);
    core.elements().not(`.${OUT_OF_FOCUS_CLASS}`).difference(inSpotlight).addClass(SELECTION_RECEDED_CLASS);
    primaryNodes.nodes().forEach((node) => syncNodeLabel(node, true));
  });
  refreshLabels(core, primaryNodes.nodes().union(previouslyHighlighted) as unknown as LabelSet);
}
