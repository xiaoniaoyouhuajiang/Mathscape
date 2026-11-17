import type { Core, EdgeSingular, NodeSingular } from "cytoscape";
import type { DesktopSnapshot } from "../artifact/types";
import {
  hideContextMenu,
  isContextMenuOpen,
  showContextMenu,
  type ContextMenuItem,
} from "./context-menu";
import { applyStagedOverlay } from "./staged-overlay";
import {
  applyFocusScope,
  applySelectionSpotlight,
  FOCUS_ROOT_CLASS,
  mountCytoscapeGraph,
  OUT_OF_FOCUS_CLASS,
  SELECTED_PRIMARY_CLASS,
  syncCytoscapeGraph,
  type RenderMode,
} from "./cytoscape";
import { focusSubgraph, type FocusDepth, type FocusLens, type FocusResult } from "./focus";
import { applyInkOverlay, clearInkOverlay, INK_NOTE_NODE_CLASS, type InkOverlayResult } from "./ink-overlay";
import {
  collapseRedundantConversions,
  isAuxiliaryArtifactNode,
  resolveIncrementalPositions,
  type MapPoint,
} from "./layout";
import type { InkLinkOverlay } from "../ink/links";
import type { IncidenceEdge, IncidenceGraph, IncidenceNode } from "./projection";
import { activityGraph } from "./lifecycle";
import {
  EDGE_TYPES,
  NODE_TYPES,
  NODE_TYPE_ORDER,
  type EdgeTypeKey,
  type EdgeTypeVisual,
  type NodeTypeKey,
  type NodeTypeVisual,
} from "./taxonomy";

export type PresetName = "overview" | "material" | "source";

export const PRESET_NODE_KINDS: Record<PresetName, ReadonlySet<string>> = {
  overview: new Set(["SourceArtifact", "SourceSemanticUnit", "SourceSpan", "SemanticEntity"]),
  material: new Set(["SourceArtifact"]),
  source: new Set(["SourceArtifact", "SourceSemanticUnit", "SemanticEntity"]),
};

/** 「共享对象」开关同时控制语义层的约定卡、一笔账和它们的连线。 */
export const SEMANTIC_NODE_KINDS: ReadonlySet<string> = new Set(["SemanticEntity", "DialectScope", "Debt"]);
export const SEMANTIC_RELATION_KINDS: ReadonlySet<string> = new Set([
  "Alignment",
  "SemanticEntity",
  "Debt",
  "DialectScope",
]);

/** 语义层上线前保存的过滤设置：没有 SemanticEntity 时补上，保持原预设。 */
function migrateSavedFilter(saved: Set<string>): Set<string> {
  const legacy: string[][] = [
    ["SourceArtifact", "SourceSemanticUnit", "SourceSpan"],
    ["SourceArtifact", "SourceSemanticUnit"],
  ];
  const isLegacy = legacy.some((kinds) => kinds.length === saved.size && kinds.every((kind) => saved.has(kind)));
  if (isLegacy) saved.add("SemanticEntity");
  return saved;
}

export interface GraphControllerElements {
  graphContainer: HTMLElement;
  graphLegend: HTMLElement;
  focusBar: HTMLElement;
  focusRootLabel: HTMLElement;
  focusSummary: HTMLElement;
  nodeFilterGroup: HTMLElement;
  renderModeToggle: HTMLInputElement;
  emptyState: HTMLElement;
  emptyStateTitle: HTMLElement;
  emptyStateMessage: HTMLElement;
}

export interface GraphControllerDelegate {
  onSelectNode: (nodeId: string | null) => void;
  onRequestCreateUnit: (parentId: string) => void;
  onRequestCreateRelation: (sourceId: string, targetId: string) => void;
  onBeforeRenderGraph?: () => void;
  onGraphRendered?: () => void;
  showToast: (message: string, kind: "success" | "error") => void;
  getSnapshot: () => DesktopSnapshot | undefined;
  /** N2: a synthesized "我的手稿" node was clicked. */
  onOpenInkNote?: (documentId: string) => void;
}

export class GraphController {
  private graphCore: Core | null = null;
  private displayedGraph?: IncidenceGraph;
  private renderMode: RenderMode = "semantic";
  private focusRootId: string | null = null;
  private focusDepth: FocusDepth = 2;
  private focusLens: FocusLens = "heuristic";
  private pendingRelationSourceId: string | null = null;
  /** Last applied focus-mode scope, so a map scene can put it back. */
  private focusScope: ReadonlySet<string> | null = null;
  private focusRelationScope: ReadonlySet<string> | null = null;
  /** Temporary scope override from Map-Linked Review Drawer, kept separate so closing review restores Focus Mode. */
  private customFocusScope: {
    nodeIds: ReadonlySet<string>;
    relationIds: ReadonlySet<string> | null;
  } | null = null;
  /** Currently single-clicked projection element id for 1-hop Selection Spotlight. */
  private selectedElementId: string | null = null;

  private visibleNodeKinds = new Set<string>(PRESET_NODE_KINDS.source);
  private activePreset: PresetName | "custom" = "source";
  private showRetired = false;
  private inkOverlay: InkLinkOverlay | undefined;
  private showInkNotes = false;
  private inkOverlayResult: InkOverlayResult = { drawn: 0, hidden: 0 };
  private inkToggleLabel: HTMLElement | undefined;
  /** Order-invariant base positions computed by deterministic `cose` on the full active landscape. */
  private basePositions = new Map<string, MapPoint>();
  /** User-dragged node positions preserved across filter toggles until "整理布局". */
  private userPinnedPositions = new Map<string, MapPoint>();
  private lastWorkspaceId: string | null = null;

  constructor(
    private readonly elements: GraphControllerElements,
    private readonly delegate: GraphControllerDelegate,
  ) {
    this.initFilterSettings();
    this.bindDOMControls();
  }

  getGraphCore(): Core | null {
    return this.graphCore;
  }

  getDisplayedGraph(): IncidenceGraph | undefined {
    return this.displayedGraph;
  }

  getFocusRootId(): string | null {
    return this.focusRootId;
  }

  setPendingRelationSourceId(id: string | null): void {
    this.pendingRelationSourceId = id;
  }

  /** Re-apply the focus-mode dim and selection spotlight (after a sandbox map scene or ink refresh). */
  reapplyFocusScope(): void {
    this.applyEffectiveFocusAndSelection();
  }

  private applyEffectiveFocusAndSelection(): void {
    if (!this.graphCore) return;
    const snapshot = this.delegate.getSnapshot();
    const rootModel =
      this.focusRootId !== null
        ? snapshot?.landscape_graph.nodes.find((n) => n.id === this.focusRootId)
        : undefined;
    const effectiveNodes = this.customFocusScope ? this.customFocusScope.nodeIds : this.focusScope;
    const effectiveRels = this.customFocusScope ? this.customFocusScope.relationIds : this.focusRelationScope;
    applyFocusScope(
      this.graphCore,
      effectiveNodes,
      effectiveRels,
      this.customFocusScope ? null : this.focusRootId,
      this.customFocusScope ? null : (rootModel?.domain_id ?? null),
    );
    applySelectionSpotlight(this.graphCore, this.selectedElementId, this.displayedGraph);
  }

  private fitInFocusElements(): void {
    const core = this.graphCore;
    if (!core || core.width() <= 0 || core.height() <= 0) return;
    const active = core.elements().not(`.${OUT_OF_FOCUS_CLASS}`);
    if (active.empty()) return;
    core.fit(active, 56);
    if (core.zoom() > 1.35) {
      core.zoom({ level: 1.35, renderedPosition: { x: core.width() / 2, y: core.height() / 2 } });
    }
  }

  resize(): void {
    this.graphCore?.resize();
  }

  defaultFocusDepthFor(node: IncidenceNode | undefined): FocusDepth {
    if (node?.domain_kind === "SourceArtifact") return "all";
    return 2;
  }

  setFocus(id: string, depth?: FocusDepth, lens?: FocusLens): void {
    const node = this.delegate.getSnapshot()?.landscape_graph.nodes.find((n) => n.id === id);
    this.focusRootId = id;
    this.selectedElementId = null;
    this.focusDepth = depth ?? this.defaultFocusDepthFor(node);
    if (lens !== undefined) {
      this.focusLens = lens;
    } else if (
      node &&
      (SEMANTIC_NODE_KINDS.has(node.domain_kind ?? "") || node.domain_kind === "Alignment") &&
      this.focusLens === "structural"
    ) {
      // Semantic nodes have no intra-book derivation tree; switch back to heuristic.
      this.focusLens = "heuristic";
    }
    if (
      node &&
      (SEMANTIC_NODE_KINDS.has(node.domain_kind ?? "") ||
        node.domain_kind === "Alignment" ||
        this.focusLens === "semantic_only") &&
      !this.visibleNodeKinds.has("SemanticEntity")
    ) {
      this.visibleNodeKinds.add("SemanticEntity");
      this.activePreset = "custom";
      this.syncCheckboxesToFilter();
      this.persistFilter();
    }
    this.renderCurrentGraph();
    this.fitInFocusElements();
  }

  /** Apply an arbitrary multi-node focus scope (e.g. from the Map-Linked Review Drawer). */
  setCustomFocusScope(nodeIds: ReadonlySet<string> | null, relationIds?: ReadonlySet<string> | null): void {
    this.customFocusScope =
      nodeIds === null ? null : { nodeIds, relationIds: relationIds ?? null };
    this.applyEffectiveFocusAndSelection();
  }

  centerDomainObject(domainId: string): void {
    const core = this.graphCore;
    const snapshot = this.delegate.getSnapshot();
    if (!core || !snapshot) return;
    const node = snapshot.landscape_graph.nodes.find((n) => n.domain_id === domainId);
    if (!node) return;
    this.selectedElementId = node.id;
    this.applyEffectiveFocusAndSelection();
    const elem =
      node.element_kind === "relation"
        ? core.edges().filter((e) => e.data("relationId") === (node.domain_id ?? node.id))
        : core.getElementById(node.id);
    if (!elem || elem.empty()) return;
    core.elements().unselect();
    elem.select();
    core.center(elem);
  }

  exitFocus(): void {
    this.focusRootId = null;
    this.focusScope = null;
    this.focusRelationScope = null;
    this.renderCurrentGraph();
  }

  /** N3d search: select and center a node. "hidden" means the current
   * type filter hides it (focus mode is left so a hidden-by-focus node shows). */
  revealNode(nodeId: string): "shown" | "hidden" | "missing" {
    const snapshot = this.delegate.getSnapshot();
    if (!snapshot?.landscape_graph.nodes.some((node) => node.id === nodeId)) return "missing";
    let element = this.graphCore?.getElementById(nodeId);
    if ((!element || element.empty() || element.hasClass(OUT_OF_FOCUS_CLASS)) && this.focusRootId !== null) {
      this.exitFocus();
      element = this.graphCore?.getElementById(nodeId);
    }
    const core = this.graphCore;
    if (!core || !element || element.empty()) return "hidden";
    core.resize();
    core.elements().unselect();
    element.select();
    this.selectedElementId = nodeId;
    this.applyEffectiveFocusAndSelection();
    core.center(element);
    return "shown";
  }

  canFocus(node: IncidenceNode): boolean {
    if (node.element_kind === "relation") {
      return node.domain_kind === "Alignment";
    }
    return (
      node.element_kind === "entity" &&
      (node.domain_kind === "SourceArtifact" ||
        node.domain_kind === "SourceSemanticUnit" ||
        node.domain_kind === "SemanticEntity" ||
        node.domain_kind === "DialectScope" ||
        node.domain_kind === "Debt")
    );
  }

  private initFilterSettings(): void {
    try {
      const savedFilter = localStorage.getItem("mathscape:graph-filter");
      if (savedFilter !== null) {
        const parsed = JSON.parse(savedFilter) as string[];
        if (Array.isArray(parsed) && parsed.every((k: unknown) => typeof k === "string")) {
          this.visibleNodeKinds = migrateSavedFilter(new Set(parsed));
          const matchingPreset = (Object.keys(PRESET_NODE_KINDS) as PresetName[]).find((preset) =>
            this.setsEqual(PRESET_NODE_KINDS[preset], this.visibleNodeKinds),
          );
          this.activePreset = matchingPreset ?? "custom";
        }
      }
    } catch {
      /* Optional presentation preference. */
    }

    try {
      const savedMode = localStorage.getItem("mathscape:render-mode");
      if (savedMode === "semantic" || savedMode === "hyperedge") {
        this.renderMode = savedMode;
      }
      this.elements.renderModeToggle.checked = this.renderMode === "semantic";
    } catch {
      /* Optional presentation preference. */
    }
  }

  private bindDOMControls(): void {
    const retiredLabel=document.createElement("label");retiredLabel.className="graph-evidence-toggle";
    const retired=document.createElement("input");retired.type="checkbox";retired.setAttribute("aria-label","显示退役对象");
    retired.onchange=()=>{this.showRetired=retired.checked;this.renderCurrentGraph();};
    retiredLabel.append(retired,document.createTextNode(" 显示退役对象（当前状态）"));
    this.elements.renderModeToggle.parentElement?.after(retiredLabel);
    try { this.showInkNotes = localStorage.getItem("mathscape:show-ink-notes") === "1"; } catch { /* Optional preference. */ }
    const inkLabel = document.createElement("label"); inkLabel.className = "graph-evidence-toggle";
    const ink = document.createElement("input"); ink.type = "checkbox"; ink.checked = this.showInkNotes;
    ink.setAttribute("aria-label", "我的手稿");
    ink.onchange = () => {
      this.showInkNotes = ink.checked;
      try { localStorage.setItem("mathscape:show-ink-notes", ink.checked ? "1" : "0"); } catch { /* Optional preference. */ }
      this.refreshInkOverlay();
    };
    const inkText = document.createElement("span");
    inkLabel.append(ink, inkText);
    this.inkToggleLabel = inkText;
    this.renderInkToggle();
    retiredLabel.after(inkLabel);

    const relayoutBtn = document.createElement("button");
    relayoutBtn.type = "button";
    relayoutBtn.className = "graph-relayout-button";
    relayoutBtn.textContent = "整理布局";
    relayoutBtn.title = "重新计算有机星团布局并适配视野";
    relayoutBtn.onclick = () => this.relayout();
    inkLabel.after(relayoutBtn);
    document.querySelectorAll<HTMLButtonElement>(".preset-button[data-preset]").forEach((button) => {
      button.onclick = () => {
        const preset = button.dataset.preset as PresetName;
        if (preset in PRESET_NODE_KINDS) {
          this.activePreset = preset;
          this.visibleNodeKinds = new Set(PRESET_NODE_KINDS[preset]);
          this.syncCheckboxesToFilter();
          this.persistFilter();
          this.renderCurrentGraph();
        }
      };
    });

    this.elements.nodeFilterGroup.addEventListener("change", () => {
      this.syncFilterFromCheckboxes();
      this.persistFilter();
      this.renderCurrentGraph();
    });

    this.elements.renderModeToggle.onchange = () => {
      this.renderMode = this.elements.renderModeToggle.checked ? "semantic" : "hyperedge";
      try {
        localStorage.setItem("mathscape:render-mode", this.renderMode);
      } catch {
        /* Optional presentation preference. */
      }
      this.renderCurrentGraph();
    };

    document.querySelectorAll<HTMLButtonElement>(".depth-button").forEach((button) => {
      button.onclick = () => {
        this.focusDepth =
          button.dataset.depth === "all" ? "all" : (Number(button.dataset.depth) as 1 | 2);
        this.renderCurrentGraph();
      };
    });

    document.querySelectorAll<HTMLButtonElement>(".lens-button").forEach((button) => {
      button.onclick = () => {
        const lens = button.dataset.lens as FocusLens | undefined;
        if (lens === "heuristic" || lens === "structural" || lens === "semantic_only") {
          this.focusLens = lens;
          if (lens === "semantic_only" && !this.visibleNodeKinds.has("SemanticEntity")) {
            this.visibleNodeKinds.add("SemanticEntity");
            this.activePreset = "custom";
            this.syncCheckboxesToFilter();
            this.persistFilter();
          }
          this.renderCurrentGraph();
        }
      };
    });

    const exitFocusBtn = document.querySelector<HTMLButtonElement>("#exit-focus");
    if (exitFocusBtn) {
      exitFocusBtn.onclick = () => this.exitFocus();
    }

    document.addEventListener("keydown", (event) => {
      if (event.key !== "Escape") return;
      if (isContextMenuOpen()) {
        hideContextMenu();
      } else if (this.focusRootId !== null) {
        this.exitFocus();
      }
    });

    this.syncCheckboxesToFilter();
  }

  private syncCheckboxesToFilter(): void {
    this.elements.nodeFilterGroup
      .querySelectorAll<HTMLInputElement>("input[data-node-kind]")
      .forEach((cb) => {
        cb.checked = this.visibleNodeKinds.has(cb.dataset.nodeKind!);
      });
    this.syncPresetHighlight();
  }

  private syncFilterFromCheckboxes(): void {
    this.visibleNodeKinds = new Set<string>();
    this.elements.nodeFilterGroup
      .querySelectorAll<HTMLInputElement>("input[data-node-kind]")
      .forEach((cb) => {
        if (cb.checked) this.visibleNodeKinds.add(cb.dataset.nodeKind!);
      });
    const matchingPreset = (Object.keys(PRESET_NODE_KINDS) as PresetName[]).find((preset) =>
      this.setsEqual(PRESET_NODE_KINDS[preset], this.visibleNodeKinds),
    );
    this.activePreset = matchingPreset ?? "custom";
    this.syncPresetHighlight();
  }

  private syncPresetHighlight(): void {
    document.querySelectorAll<HTMLButtonElement>(".preset-button[data-preset]").forEach((button) => {
      const preset = button.dataset.preset as PresetName;
      const presetKinds = PRESET_NODE_KINDS[preset];
      const active = presetKinds !== undefined && this.setsEqual(presetKinds, this.visibleNodeKinds);
      button.classList.toggle("is-active", active);
      button.setAttribute("aria-pressed", String(active));
    });
  }

  private setsEqual<T>(a: ReadonlySet<T>, b: ReadonlySet<T>): boolean {
    if (a.size !== b.size) return false;
    for (const item of a) if (!b.has(item)) return false;
    return true;
  }

  private persistFilter(): void {
    try {
      localStorage.setItem("mathscape:graph-filter", JSON.stringify([...this.visibleNodeKinds]));
    } catch {
      /* Optional presentation preference. */
    }
  }

  /** N2: counts and note nodes from `get_ink_link_overlay` (§14.7). */
  setInkOverlay(overlay: InkLinkOverlay | undefined): void {
    this.inkOverlay = overlay;
    this.refreshInkOverlay();
  }

  private refreshInkOverlay(): void {
    if (this.graphCore === null) { this.renderInkToggle(); return; }
    this.inkOverlayResult = applyInkOverlay(this.graphCore, this.inkOverlay, this.showInkNotes);
    this.applyEffectiveFocusAndSelection();
    this.renderInkToggle();
    this.renderLegend(this.graphCore);
  }

  private renderInkToggle(): void {
    if (!this.inkToggleLabel) return;
    const hidden = this.showInkNotes && this.inkOverlayResult.hidden ? `（另有 ${this.inkOverlayResult.hidden} 份未显示）` : "";
    this.inkToggleLabel.textContent = ` 我的手稿${hidden}`;
  }

  /** Recompute a clean organic `cose` layout on the full map, clearing manual drag offsets. */
  relayout(): void {
    this.userPinnedPositions.clear();
    this.basePositions.clear();
    this.renderCurrentGraph(this.delegate.getSnapshot(), { forceRelayout: true, fit: true });
  }

  renderCurrentGraph(
    snapshot = this.delegate.getSnapshot(),
    options?: { forceRelayout?: boolean; fit?: boolean },
  ): void {
    if (snapshot === undefined) return;
    this.delegate.onBeforeRenderGraph?.();
    hideContextMenu();
    if (this.lastWorkspaceId !== null && this.lastWorkspaceId !== snapshot.workspace_id) {
      this.basePositions.clear();
      this.userPinnedPositions.clear();
    }
    this.lastWorkspaceId = snapshot.workspace_id;

    const landscape = activityGraph(snapshot.landscape_graph, this.showRetired);
    const rect = this.elements.graphContainer.getBoundingClientRect();
    const viewport = { width: rect.width, height: rect.height };
    this.basePositions = resolveIncrementalPositions(
      landscape,
      options?.forceRelayout ? new Map() : this.basePositions,
      this.userPinnedPositions,
      viewport,
    );

    if (
      this.focusRootId !== null &&
      !landscape.nodes.some((n) => n.id === this.focusRootId)
    ) {
      this.focusRootId = null;
    }

    const focus =
      this.focusRootId === null
        ? null
        : focusSubgraph(landscape, this.focusRootId, {
            depth: this.focusDepth,
            lens: this.focusLens,
            isEntityVisible: (node) => this.isEntityVisible(node),
          });

    const filtered = this.filterGraph(landscape);
    this.displayedGraph = focus === null ? filtered : this.mergeGraphs(filtered, focus.graph);
    const positions = this.positionsWithSynthetics(this.displayedGraph, this.basePositions);
    const focusScope =
      focus === null ? null : new Set(focus.graph.nodes.map((node) => node.id));
    const focusRelationScope =
      focus === null ? null : new Set(focus.graph.edges.map((edge) => edge.relation_id));

    this.syncPresetHighlight();
    this.renderFocusBar(focus, landscape);

    const entityCount = (focus === null ? filtered : focus.graph).nodes.filter(
      (n) => n.element_kind === "entity",
    ).length;
    const hasAnyData = snapshot.landscape_graph.nodes.some((n) => n.element_kind !== "group");

    if (focus !== null) {
      this.elements.emptyStateTitle.textContent = "聚焦范围内没有可显示的关联节点";
      this.elements.emptyStateMessage.textContent =
        "可尝试切换聚焦视角（启发关联 / 来源结构 / 纯语义网）、增大深度或开启更多节点类型。";
    } else {
      this.elements.emptyStateTitle.textContent = hasAnyData
        ? "当前过滤条件下无可见节点"
        : "尚未导入材料";
      this.elements.emptyStateMessage.textContent = hasAnyData
        ? "尝试切换显示预设或勾选更多节点类型。"
        : "导入第一份 PDF 后，节点会出现在这里。";
    }

    if (this.graphCore !== null) clearInkOverlay(this.graphCore);
    if (this.graphCore === null) {
      this.graphCore = mountCytoscapeGraph(
        this.elements.graphContainer,
        this.displayedGraph,
        this.renderMode,
        {
          positions,
          labelHover: {
            isFull: (node) =>
              node.hasClass(SELECTED_PRIMARY_CLASS) || node.hasClass(FOCUS_ROOT_CLASS),
          },
        },
      );
      this.bindGraphInteractions(this.graphCore);
    } else {
      syncCytoscapeGraph(this.graphCore, this.displayedGraph, this.renderMode, {
        positions,
        fit: options?.fit,
        updateExistingPositions: options?.forceRelayout,
      });
    }

    if (
      this.selectedElementId !== null &&
      !this.displayedGraph.nodes.some((n) => n.id === this.selectedElementId)
    ) {
      this.selectedElementId = null;
    }

    this.focusScope = focusScope;
    this.focusRelationScope = focusRelationScope;
    applyStagedOverlay(this.graphCore, this.delegate.getSnapshot()?.investigation);
    this.inkOverlayResult = applyInkOverlay(this.graphCore, this.inkOverlay, this.showInkNotes);
    this.applyEffectiveFocusAndSelection();
    this.renderInkToggle();
    this.renderLegend(this.graphCore);
    this.elements.emptyState.classList.toggle(
      "is-hidden",
      entityCount > (focus === null ? 0 : 1),
    );
    this.delegate.onGraphRendered?.();
  }

  private positionsWithSynthetics(
    displayed: IncidenceGraph,
    base: ReadonlyMap<string, MapPoint>,
  ): Map<string, MapPoint> {
    const out = new Map(base);
    const byRel = new Map<string, IncidenceEdge[]>();
    for (const edge of displayed.edges) {
      const list = byRel.get(edge.relation_id) ?? [];
      list.push(edge);
      byRel.set(edge.relation_id, list);
    }
    for (const node of displayed.nodes) {
      if (node.element_kind !== "relation" || out.has(node.id)) continue;
      const relKey = node.domain_id ?? node.id;
      const relEdges = byRel.get(relKey) ?? [];
      const pts = relEdges
        .map((e) => out.get(e.source === node.id ? e.target : e.source))
        .filter((p): p is MapPoint => !!p);
      if (pts.length === 0) continue;
      const cx = Math.round(pts.reduce((a, p) => a + p.x, 0) / pts.length);
      const cy = Math.round(pts.reduce((a, p) => a + p.y, 0) / pts.length);
      out.set(node.id, { x: cx, y: cy });
      out.set(`junc~${relKey}`, { x: cx, y: cy });
    }
    return out;
  }

  private mergeGraphs(base: IncidenceGraph, overlay: IncidenceGraph): IncidenceGraph {
    const nodes = new Map(base.nodes.map((node) => [node.id, node]));
    for (const node of overlay.nodes) nodes.set(node.id, node);
    const edges = new Map(base.edges.map((edge) => [edge.id, edge]));
    for (const edge of overlay.edges) edges.set(edge.id, edge);
    return { ...base, nodes: [...nodes.values()], edges: [...edges.values()] };
  }

  private filterGraph(graph: IncidenceGraph): IncidenceGraph {
    graph = activityGraph(graph, this.showRetired);
    const knownRelationKinds = new Set([
      "ConversionRun",
      "SourceUnitRelation",
      "Evidence",
      ...SEMANTIC_RELATION_KINDS,
    ]);
    const allowUnknownKinds = this.activePreset === "overview";
    const showSpans = this.visibleNodeKinds.has("SourceSpan");
    const showArtifacts = this.visibleNodeKinds.has("SourceArtifact");
    const showUnits = this.visibleNodeKinds.has("SourceSemanticUnit");
    const relationKinds = new Set<string>();
    if (showArtifacts) relationKinds.add("ConversionRun");
    if (showUnits || showSpans) relationKinds.add("SourceUnitRelation");
    if (showSpans && showArtifacts) relationKinds.add("Evidence");
    if (this.visibleNodeKinds.has("SemanticEntity")) SEMANTIC_RELATION_KINDS.forEach((kind) => relationKinds.add(kind));

    let nodes = graph.nodes.filter((node) => {
      if (node.element_kind === "group") return true;
      if (node.element_kind === "relation") {
        if (node.domain_kind === null) return false;
        return (
          (allowUnknownKinds && !knownRelationKinds.has(node.domain_kind)) ||
          relationKinds.has(node.domain_kind)
        );
      }
      return this.isEntityVisible(node);
    });

    const nodeIds = new Set(nodes.map((n) => n.id));
    let edges = graph.edges.filter((e) => nodeIds.has(e.source) && nodeIds.has(e.target));

    if (this.activePreset !== "overview" && showArtifacts) {
      const collapsed = collapseRedundantConversions(nodes, edges);
      nodes = collapsed.nodes;
      edges = collapsed.edges;
    }

    return { ...graph, nodes, edges };
  }

  private isEntityVisible(node: IncidenceNode): boolean {
    if (node.domain_kind === null) return false;
    if (isAuxiliaryArtifactNode(node) && this.activePreset !== "overview") {
      return false;
    }
    const knownNodeKinds = new Set(["SourceArtifact", "SourceSemanticUnit", "SourceSpan", ...SEMANTIC_NODE_KINDS]);
    if (SEMANTIC_NODE_KINDS.has(node.domain_kind)) return this.visibleNodeKinds.has("SemanticEntity");
    return (
      (this.activePreset === "overview" && !knownNodeKinds.has(node.domain_kind)) ||
      this.visibleNodeKinds.has(node.domain_kind)
    );
  }

  private bindGraphInteractions(core: Core): void {
    core.on("tap", "node", (event) => {
      const tapped = event.target as NodeSingular;
      if (tapped.hasClass(INK_NOTE_NODE_CLASS)) {
        this.delegate.onOpenInkNote?.(String(tapped.data("documentId")));
        return;
      }
      const id = tapped.id();
      if (this.pendingRelationSourceId !== null) {
        const target = this.displayedGraph?.nodes.find((node) => node.id === id);
        if (
          target?.domain_kind === "SourceSemanticUnit" &&
          target.id !== this.pendingRelationSourceId
        ) {
          const sourceId = this.pendingRelationSourceId;
          this.pendingRelationSourceId = null;
          this.delegate.onRequestCreateRelation(sourceId, target.domain_id!);
        } else {
          this.delegate.showToast("请选择另一个内容单元作为目标节点", "error");
        }
        return;
      }
      core.elements().unselect();
      tapped.select();
      this.selectedElementId = id;
      this.applyEffectiveFocusAndSelection();
      this.delegate.onSelectNode(id);
    });

    core.on("tap", "edge", (event) => {
      const edge = event.target as EdgeSingular;
      const relationId = edge.data("relationId");
      if (typeof relationId !== "string") return;
      const relation = this.displayedGraph?.nodes.find(
        (node) => node.element_kind === "relation" && node.domain_id === relationId,
      );
      if (relation !== undefined) {
        core.elements().unselect();
        edge.select();
        this.selectedElementId = relation.id;
        this.applyEffectiveFocusAndSelection();
        this.delegate.onSelectNode(relation.id);
      }
    });

    core.on("tap", (event) => {
      if (event.target !== core) return;
      if (this.selectedElementId !== null) {
        core.elements().unselect();
        this.selectedElementId = null;
        this.applyEffectiveFocusAndSelection();
        this.delegate.onSelectNode(null);
      }
    });

    core.on("cxttap", "edge", (event) => {
      const relationId = (event.target as EdgeSingular).data("relationId");
      if (typeof relationId !== "string") return;
      const relation = this.displayedGraph?.nodes.find(
        (node) => node.element_kind === "relation" && node.domain_id === relationId,
      );
      if (relation === undefined) return;
      this.delegate.onSelectNode(relation.id);
      const position = event.renderedPosition ?? { x: 0, y: 0 };
      const bounds = this.elements.graphContainer.getBoundingClientRect();
      showContextMenu(
        bounds.left + position.x,
        bounds.top + position.y,
        this.nodeMenuItems(relation.id),
      );
    });

    core.on("cxttap", "node", (event) => {
      const node = event.target as NodeSingular;
      if (node.hasClass(INK_NOTE_NODE_CLASS)) return;
      this.delegate.onSelectNode(node.id());
      const position = event.renderedPosition ?? { x: 0, y: 0 };
      const bounds = this.elements.graphContainer.getBoundingClientRect();
      showContextMenu(
        bounds.left + position.x,
        bounds.top + position.y,
        this.nodeMenuItems(node.id()),
      );
    });

    core.on("dragfreeon", "node", (event) => {
      const node = event.target as NodeSingular;
      if (node.hasClass(INK_NOTE_NODE_CLASS) || node.data("elementKind") === "junction") return;
      const pos = node.position();
      const point = { x: Math.round(pos.x), y: Math.round(pos.y) };
      this.userPinnedPositions.set(node.id(), point);
      this.basePositions.set(node.id(), point);
    });

    core.on("cxttap", (event) => {
      if (event.target !== core) return;
      const items: ContextMenuItem[] = [
        { label: "整理全图布局", onSelect: () => this.relayout() },
      ];
      if (this.focusRootId !== null) {
        items.push({ label: "返回全图", onSelect: () => this.exitFocus() });
      }
      showContextMenu(event.originalEvent.clientX, event.originalEvent.clientY, items);
    });
  }

  private nodeMenuItems(id: string): ContextMenuItem[] {
    const node = this.displayedGraph?.nodes.find((candidate) => candidate.id === id);
    const items: ContextMenuItem[] = [];
    if (node?.domain_kind === "SourceSemanticUnit" && node.domain_id !== null) {
      items.push({
        label: "以此为父节点新建内容单元",
        onSelect: () => this.delegate.onRequestCreateUnit(node.domain_id!),
      });
      items.push({
        label: "从此节点创建来源关系",
        onSelect: () => {
          this.pendingRelationSourceId = node.id;
          this.delegate.showToast("请点击地图中的另一个内容单元作为目标", "success");
        },
      });
    }
    if (node !== undefined && this.canFocus(node)) {
      const defaultDepth = this.defaultFocusDepthFor(node);
      items.push(
        {
          label: "以此为中心聚焦",
          disabled: this.focusRootId === id && this.focusDepth === defaultDepth,
          onSelect: () => this.setFocus(id, defaultDepth),
        },
        {
          label: "仅展开一层",
          disabled: this.focusRootId === id && this.focusDepth === 1,
          onSelect: () => this.setFocus(id, 1),
        },
      );
      if (defaultDepth !== "all") {
        items.push({
          label: "展开全部关联",
          disabled: this.focusRootId === id && this.focusDepth === "all",
          onSelect: () => this.setFocus(id, "all"),
        });
      }
    }
    items.push({
      label: "返回全图",
      disabled: this.focusRootId === null,
      onSelect: () => this.exitFocus(),
    });
    return items;
  }

  private renderFocusBar(focus: FocusResult | null, graph: IncidenceGraph): void {
    const { focusBar, focusRootLabel, focusSummary } = this.elements;
    focusBar.hidden = focus === null;
    if (focus === null) return;
    focusRootLabel.textContent =
      graph.nodes.find((node) => node.id === this.focusRootId)?.label ?? "—";
    const contracted =
      focus.contractedCount > 0 ? ` · 折叠了 ${focus.contractedCount} 个被过滤的中间节点` : "";
    focusSummary.textContent = `展开 ${focus.reachedCount} 个节点${contracted}`;
    document.querySelectorAll<HTMLButtonElement>(".depth-button").forEach((button) => {
      const active = button.dataset.depth === String(this.focusDepth);
      button.classList.toggle("is-active", active);
      button.setAttribute("aria-pressed", String(active));
    });
    document.querySelectorAll<HTMLButtonElement>(".lens-button").forEach((button) => {
      const active = button.dataset.lens === this.focusLens;
      button.classList.toggle("is-active", active);
      button.setAttribute("aria-pressed", String(active));
    });
  }

  private renderLegend(core: Core): void {
    const nodeKeys = new Set<NodeTypeKey>();
    core.nodes().forEach((node) => {
      const key = node.data("nodeType") as NodeTypeKey | undefined;
      if (key !== undefined && key in NODE_TYPES) nodeKeys.add(key);
    });
    const edgeKeys = new Set<EdgeTypeKey>();
    core.edges().forEach((edge) => {
      if (edge.hasClass("ink-note-edge")) return;
      if (edge.hasClass("contracted-edge")) edgeKeys.add("contracted");
      else if (edge.hasClass("alignment-edge")) edgeKeys.add("alignment");
      else if (edge.hasClass("debt-edge")) edgeKeys.add("debt");
      else if (edge.hasClass("evidence-edge")) edgeKeys.add("evidence");
      else if (edge.hasClass("semantic-edge")) edgeKeys.add("relation");
      else edgeKeys.add("incidence");
    });

    const items: HTMLElement[] = [];
    for (const key of NODE_TYPE_ORDER) {
      if (nodeKeys.has(key)) {
        items.push(this.legendItem(this.nodeSwatch(NODE_TYPES[key]), NODE_TYPES[key].label));
      }
    }
    for (const key of ["relation", "alignment", "debt", "evidence", "contracted", "incidence"] as EdgeTypeKey[]) {
      if (edgeKeys.has(key)) {
        items.push(this.legendItem(this.edgeSwatch(EDGE_TYPES[key]), EDGE_TYPES[key].label));
      }
    }
    if (core.nodes(`.${INK_NOTE_NODE_CLASS}`).nonempty()) {
      items.push(this.legendItem(this.nodeSwatch({ shape: "round-rectangle", background: "#3d3157", border: "#b79be0", borderStyle: "solid" } as NodeTypeVisual), "我的手稿"));
    }
    this.elements.graphLegend.replaceChildren(...items);
  }

  private legendItem(swatch: HTMLElement, label: string): HTMLElement {
    const item = document.createElement("span");
    item.className = "legend-item";
    item.append(swatch, document.createTextNode(label));
    return item;
  }

  private nodeSwatch(visual: NodeTypeVisual): HTMLElement {
    const shape =
      visual.shape === "ellipse" ? "is-round" : visual.shape === "diamond" ? "is-diamond" : "is-rect";
    const swatch = document.createElement("span");
    swatch.className = `legend-dot ${shape}${visual.borderStyle === "dashed" ? " is-dashed" : ""}`;
    swatch.style.backgroundColor = visual.background;
    swatch.style.borderColor = visual.border;
    return swatch;
  }

  private edgeSwatch(visual: EdgeTypeVisual): HTMLElement {
    const swatch = document.createElement("span");
    swatch.className = `legend-line${visual.dashed ? " is-dashed" : ""}`;
    swatch.style.color = visual.color;
    return swatch;
  }
}
