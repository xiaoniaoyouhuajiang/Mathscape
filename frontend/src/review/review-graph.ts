import { graphPalette, watchGraphTheme } from "../graph/theme";
import cytoscape, { type Core, type ElementDefinition, type StylesheetJson } from "cytoscape";

export interface ReviewGraphUnit {
  ref: string;
  label: string;
  state: "selectable" | "blocked" | "pending" | "registered";
}

export interface ReviewGraphRelation {
  id: string;
  source: string;
  target: string;
  label: string;
  /** Drawn dashed: the relation crosses snapshots. */
  cross?: boolean;
}

export interface ReviewGraphStructure {
  parent: string;
  child: string;
}

export interface ReviewGraphEvidence {
  id: string;
  unit: string;
  label: string;
}

/** A relation end outside the shown units: an existing map object or a unit of another snapshot. */
export interface ReviewGraphExternal {
  ref: string;
  label: string;
  kind: "existing" | "cross";
}

export interface ReviewGraphCallbacks {
  unit(ref: string, hover: boolean): void;
  relation(id: string, hover: boolean): void;
  evidence(id: string): void;
  leave(): void;
}

// Review-local ids: this graph must never share element ids with the
// registered landscape graph.
const unitId = (ref: string) => `review-unit:${ref}`;
const relationId = (id: string) => `review-relation:${id}`;
const evidenceId = (id: string) => `review-evidence:${id}`;

function reviewStyles(): StylesheetJson {
  const t = graphPalette();
  return [
    {
      selector: "node.unit",
      style: {
        label: "data(label)", "font-size": 10, color: t.text, "text-wrap": "wrap", "text-max-width": "120px",
        "text-valign": "bottom", "text-margin-y": 4, width: 22, height: 22,
        "background-color": t.accent, "border-width": 2, "border-color": t.accent,
      },
    },
    { selector: "node.state-blocked", style: { "background-color": t.warning } },
    { selector: "node.state-pending", style: { "background-color": t.candidate } },
    { selector: "node.state-registered", style: { "background-color": t.success } },
    { selector: "node.is-selected", style: { "border-color": t.success, "border-width": 3 } },
    {
      selector: "node.is-current",
      style: { "overlay-color": t.gold, "overlay-opacity": 0.28, "overlay-padding": 7, "border-color": t.gold },
    },
    {
      selector: "node.evidence",
      style: { shape: "round-rectangle", width: 9, height: 9, "background-color": t.accent, label: "", "border-width": 0 },
    },
    {
      selector: "edge",
      style: { width: 1.5, "curve-style": "bezier", "target-arrow-shape": "triangle", "arrow-scale": 0.8 },
    },
    { selector: "edge.structure", style: { "line-style": "dashed", "line-color": t.accent, "target-arrow-color": t.accent } },
    {
      selector: "edge.semantic",
      style: {
        "line-color": t.accent, "target-arrow-color": t.accent, label: "data(label)", "font-size": 9,
        color: t.text, "text-background-color": t.canvas, "text-background-opacity": 0, "text-outline-color": t.canvas, "text-outline-width": 2, "text-background-padding": "2px",
      },
    },
    {
      selector: "node.external",
      style: {
        label: "data(label)", "font-size": 9, color: t.text, "text-wrap": "wrap", "text-max-width": "120px",
        "text-valign": "bottom", "text-margin-y": 4, width: 16, height: 16, shape: "diamond",
        "background-color": t.accent, "border-width": 2, "border-style": "dashed", "border-color": t.accent,
      },
    },
    { selector: "node.external-cross", style: { "border-color": t.candidate } },
    { selector: "edge.semantic.cross", style: { "line-style": "dashed", "line-color": t.candidate, "target-arrow-color": t.candidate } },
    { selector: "edge.evidence", style: { "line-style": "dotted", "line-color": t.accent, "target-arrow-shape": "none", width: 1 } },
    { selector: "edge.is-current", style: { "line-color": t.gold, "target-arrow-color": t.gold, width: 3 } },
  ];


}
/** Standalone Cytoscape view over one run's candidate units. */
export class ReviewGraph {
  private core: Core;
  private evidence: ReviewGraphEvidence[] = [];

  constructor(
    private host: HTMLElement,
    units: ReviewGraphUnit[],
    structure: ReviewGraphStructure[],
    relations: ReviewGraphRelation[],
    callbacks: ReviewGraphCallbacks,
    external: ReviewGraphExternal[] = [],
  ) {
    const known = new Set(units.map(unit => unit.ref));
    const outside = new Map(external.filter(node => !known.has(node.ref)).map(node => [node.ref, node]));
    const shown = (ref: string) => known.has(ref) || outside.has(ref);
    const elements: ElementDefinition[] = [
      ...units.map(unit => ({
        group: "nodes" as const,
        data: { id: unitId(unit.ref), ref: unit.ref, label: unit.label },
        classes: `unit state-${unit.state}`,
      })),
      ...[...outside.values()].map(node => ({
        group: "nodes" as const,
        data: { id: unitId(node.ref), ref: node.ref, label: node.label },
        classes: `external external-${node.kind}`,
      })),
      ...structure
        .filter(edge => known.has(edge.parent) && known.has(edge.child))
        .map((edge, index) => ({
          group: "edges" as const,
          data: { id: `review-structure:${index}`, source: unitId(edge.parent), target: unitId(edge.child) },
          classes: "structure",
        })),
      ...relations
        .filter(edge => shown(edge.source) && shown(edge.target))
        .map(edge => ({
          group: "edges" as const,
          data: { id: relationId(edge.id), relation: edge.id, source: unitId(edge.source), target: unitId(edge.target), label: edge.label },
          classes: [outside.get(edge.source), outside.get(edge.target)].some(node => node?.kind === "cross") || edge.cross ? "semantic cross" : "semantic",
        })),
    ];
    host.dataset.unitCount = String(units.length);
    host.dataset.externalCount = String(outside.size);
    host.dataset.relationCount = String(elements.filter(element => element.classes?.toString().startsWith("semantic")).length);
    this.core = cytoscape({ container: host, elements, style: reviewStyles(), minZoom: 0.2, maxZoom: 3 });
    watchGraphTheme(this.core, () => this.core.style(reviewStyles()));
    const children = new Set(structure.map(edge => edge.child));
    const roots = [...units.filter(unit => !children.has(unit.ref)).map(unit => unitId(unit.ref)), ...[...outside.keys()].map(unitId)];
    this.core.layout(roots.length > 0 && roots.length < units.length + outside.size
      ? { name: "breadthfirst", directed: true, roots, spacingFactor: 1.15, padding: 20, animate: false }
      : { name: "cose", padding: 20, animate: false, randomize: false }).run();
    this.core.on("tap", "node.unit, node.external", event => callbacks.unit(event.target.data("ref"), false));
    this.core.on("mouseover", "node.unit", event => callbacks.unit(event.target.data("ref"), true));
    this.core.on("tap", "edge.semantic", event => callbacks.relation(event.target.data("relation"), false));
    this.core.on("mouseover", "edge.semantic", event => callbacks.relation(event.target.data("relation"), true));
    this.core.on("tap", "node.evidence", event => callbacks.evidence(event.target.data("evidence")));
    this.core.on("mouseout", "node.unit, edge.semantic", () => callbacks.leave());
  }

  setSelected(selected: Set<string>): void {
    this.core.nodes(".unit").forEach(node => { node.toggleClass("is-selected", selected.has(node.data("ref"))); });
  }

  setCurrent(units: string[], relation: string | null): void {
    const current = new Set(units);
    this.core.nodes(".unit").forEach(node => { node.toggleClass("is-current", current.has(node.data("ref"))); });
    this.core.edges(".semantic").forEach(edge => { edge.toggleClass("is-current", edge.data("relation") === relation); });
    this.host.dataset.current = units.join(" ");
  }

  /** SourceSpans are not map nodes here; evidence mode shows them as markers beside their unit. */
  setEvidence(evidence: ReviewGraphEvidence[]): void {
    this.core.remove(".evidence");
    this.evidence = evidence;
    const perUnit = new Map<string, number>();
    this.core.add(evidence.flatMap(item => {
      const unit = this.core.getElementById(unitId(item.unit));
      if (unit.empty()) return [];
      const index = perUnit.get(item.unit) ?? 0;
      perUnit.set(item.unit, index + 1);
      const position = unit.position();
      return [
        {
          group: "nodes" as const,
          data: { id: evidenceId(item.id), evidence: item.id, label: item.label },
          position: { x: position.x + 20 + index * 12, y: position.y - 16 },
          classes: "evidence",
        },
        {
          group: "edges" as const,
          data: { id: `review-evidence-edge:${item.id}`, source: evidenceId(item.id), target: unitId(item.unit) },
          classes: "evidence",
        },
      ];
    }));
    this.host.dataset.evidenceCount = String(this.evidence.length);
  }

  resize(): void {
    this.core.resize();
    this.core.fit(undefined, 20);
  }

  destroy(): void {
    this.core.destroy();
  }
}
