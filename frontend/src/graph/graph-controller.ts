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
  mountCytoscapeGraph,
  syncCytoscapeGraph,
  type RenderMode,
} from "./cytoscape";
import { focusSubgraph, type FocusDepth, type FocusResult } from "./focus";
import { applyInkOverlay, clearInkOverlay, INK_NOTE_NODE_CLASS, type InkOverlayResult } from "./ink-overlay";
import type { InkLinkOverlay } from "../ink/links";
import type { IncidenceEdge, IncidenceGraph, IncidenceNode } from "./projection";
import { activityGraph } from "./lifecycle";
import {
  computeLandscapeLayout,
  landscapeLayoutKey,
  resolveIncrementalPositions,
  type MapPoint,
} from "./layout";
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
  onSelectNode: (nodeId: string) => void;
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
  private focusDepth: FocusDepth = "all";
  private pendingRelationSourceId: string | null = null;
  /** Last applied focus-mode scope, so a map scene can put it back. */
  private focusScope: ReadonlySet<string> | null = null;

  private visibleNodeKinds = new Set<string>(PRESET_NODE_KINDS.source);
  private activePreset: PresetName | "custom" = "source";
  private showRetired = false;
  private inkOverlay: InkLinkOverlay | undefined;
  private showInkNotes = false;
  private inkOverlayResult: InkOverlayResult = { drawn: 0, hidden: 0 };
  private inkToggleLabel: HTMLElement | undefined;
  /** Full-landscape coordinates reused by every filtered/render-mode view. */
  private basePositions = new Map<string, MapPoint>();
  /** Manual positions survive filter changes until the user asks to relayout. */
  private userPinnedPositions = new Map<string, MapPoint>();
  private lastWorkspaceId: string | null = null;
  private lastLayoutKey: string | null = null;

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

  /** Re-apply the focus-mode dim (after a sandbox map scene used it). */
  reapplyFocusScope(): void {
    if (this.graphCore) applyFocusScope(this.graphCore, this.focusScope);
  }

  resize(): void {
    this.graphCore?.resize();
  }

  setFocus(id: string, depth: FocusDepth): void {
    this.focusRootId = id;
    this.focusDepth = depth;
    this.renderCurrentGraph();
  }

  exitFocus(): void {
    this.focusRootId = null;
    this.renderCurrentGraph();
  }

  /** N3d search: select and center a node. "hidden" means the current
   * type filter hides it (focus mode is left so a hidden-by-focus node shows). */
  revealNode(nodeId: string): "shown" | "hidden" | "missing" {
    const snapshot = this.delegate.getSnapshot();
    if (!snapshot?.landscape_graph.nodes.some((node) => node.id === nodeId)) return "missing";
    let element = this.graphCore?.getElementById(nodeId);
    if ((!element || element.empty()) && this.focusRootId !== null) {
      this.exitFocus();
      element = this.graphCore?.getElementById(nodeId);
    }
    const core = this.graphCore;
    if (!core || !element || element.empty()) return "hidden";
    core.resize();
    core.elements().unselect();
    element.select();
    core.center(element);
    return "shown";
  }

  canFocus(node: IncidenceNode): boolean {
    return (
      node.element_kind === "entity" &&
      (node.domain_kind === "SourceArtifact" || node.domain_kind === "SourceSemanticUnit")
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
    const relayoutButton = document.createElement("button");
    relayoutButton.type = "button";
    relayoutButton.className = "graph-relayout-button";
    relayoutButton.textContent = "整理布局";
    relayoutButton.title = "按确定性物理布局重新整理全图";
    relayoutButton.onclick = () => this.relayout();
    inkLabel.after(relayoutButton);
    document.querySelectorAll<HTMLButtonElement>(".preset-button").forEach((button) => {
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
    this.activePreset = "custom";
    this.visibleNodeKinds = new Set<string>();
    this.elements.nodeFilterGroup
      .querySelectorAll<HTMLInputElement>("input[data-node-kind]")
      .forEach((cb) => {
        if (cb.checked) this.visibleNodeKinds.add(cb.dataset.nodeKind!);
      });
    this.syncPresetHighlight();
  }

  private syncPresetHighlight(): void {
    document.querySelectorAll<HTMLButtonElement>(".preset-button").forEach((button) => {
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
    this.renderInkToggle();
    this.renderLegend(this.graphCore);
  }

  private renderInkToggle(): void {
    if (!this.inkToggleLabel) return;
    const hidden = this.showInkNotes && this.inkOverlayResult.hidden ? `（另有 ${this.inkOverlayResult.hidden} 份未显示）` : "";
    this.inkToggleLabel.textContent = ` 我的手稿${hidden}`;
  }

  relayout(): void {
    this.basePositions.clear();
    this.userPinnedPositions.clear();
    this.lastLayoutKey = null;
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
      this.lastLayoutKey = null;
    }
    this.lastWorkspaceId = snapshot.workspace_id;

    const landscape = activityGraph(snapshot.landscape_graph, this.showRetired);
    const nextLayoutKey = landscapeLayoutKey(landscape);
    if (options?.forceRelayout || this.lastLayoutKey !== nextLayoutKey || this.basePositions.size === 0) {
      const fullPositions = computeLandscapeLayout(landscape);
      this.basePositions = resolveIncrementalPositions(
        landscape,
        options?.forceRelayout ? new Map() : this.basePositions,
        this.userPinnedPositions,
        fullPositions,
      );
      this.lastLayoutKey = nextLayoutKey;
    }

    if (
      this.focusRootId !== null &&
      !landscape.nodes.some((node) => node.id === this.focusRootId)
    ) {
      this.focusRootId = null;
    }

    const focus =
      this.focusRootId === null
        ? null
        : focusSubgraph(landscape, this.focusRootId, {
            depth: this.focusDepth,
            isEntityVisible: (node) => this.isEntityVisible(node),
          });

    const filtered = this.filterGraph(landscape);
    this.displayedGraph = focus === null ? filtered : this.mergeGraphs(filtered, focus.graph);
    const positions = this.positionsWithSynthetics(this.displayedGraph, this.basePositions);
    const focusScope =
      focus === null ? null : new Set(focus.graph.nodes.map((node) => node.id));

    this.syncPresetHighlight();
    this.renderFocusBar(focus, landscape);

    const entityCount = (focus === null ? filtered : focus.graph).nodes.filter(
      (node) => node.element_kind === "entity",
    ).length;
    const hasAnyData = snapshot.landscape_graph.nodes.some((node) => node.element_kind !== "group");

    if (focus !== null) {
      this.elements.emptyStateTitle.textContent = "聚焦范围内没有可显示的节点";
      this.elements.emptyStateMessage.textContent =
        "该来源尚未派生出内容，或当前过滤条件隐藏了全部后代。";
    } else {
      this.elements.emptyStateTitle.textContent = hasAnyData
        ? "当前过滤条件下无可显示节点"
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
        { positions },
      );
      this.bindGraphInteractions(this.graphCore);
    } else {
      syncCytoscapeGraph(this.graphCore, this.displayedGraph, this.renderMode, {
        positions,
        fit: options?.fit,
        updateExistingPositions: options?.forceRelayout === true,
      });
    }

    this.focusScope = focusScope;
    applyFocusScope(this.graphCore, focusScope);
    applyStagedOverlay(this.graphCore, this.delegate.getSnapshot()?.investigation);
    this.inkOverlayResult = applyInkOverlay(this.graphCore, this.inkOverlay, this.showInkNotes);
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
    const byRelation = new Map<string, IncidenceEdge[]>();
    for (const edge of displayed.edges) {
      const list = byRelation.get(edge.relation_id) ?? [];
      list.push(edge);
      byRelation.set(edge.relation_id, list);
    }
    for (const node of displayed.nodes) {
      if (node.element_kind !== "relation" || out.has(node.id)) continue;
      const relationKey = node.domain_id ?? node.id;
      const points = (byRelation.get(relationKey) ?? [])
        .map((edge) => out.get(edge.source === node.id ? edge.target : edge.source))
        .filter((point): point is MapPoint => point !== undefined);
      if (points.length === 0) continue;
      const center = {
        x: Math.round(points.reduce((sum, point) => sum + point.x, 0) / points.length),
        y: Math.round(points.reduce((sum, point) => sum + point.y, 0) / points.length),
      };
      out.set(node.id, center);
      out.set(`junc~${relationKey}`, center);
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
    graph=activityGraph(graph,this.showRetired);
    const knownRelationKinds = new Set(["ConversionRun", "SourceUnitRelation", ...SEMANTIC_RELATION_KINDS]);
    const allowUnknownKinds = this.activePreset === "overview";
    const relationKinds = new Set<string>();
    if (this.visibleNodeKinds.has("SourceArtifact")) relationKinds.add("ConversionRun");
    if (this.visibleNodeKinds.has("SourceSemanticUnit")) relationKinds.add("SourceUnitRelation");
    if (this.visibleNodeKinds.has("SemanticEntity")) SEMANTIC_RELATION_KINDS.forEach((kind) => relationKinds.add(kind));

    const nodes = graph.nodes.filter((node) => {
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
    const edges = graph.edges.filter((e) => nodeIds.has(e.source) && nodeIds.has(e.target));
    return { ...graph, nodes, edges };
  }

  private isEntityVisible(node: IncidenceNode): boolean {
    if (node.domain_kind === null) return false;
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
      this.delegate.onSelectNode(id);
    });

    core.on("tap", "edge", (event) => {
      const relationId = (event.target as EdgeSingular).data("relationId");
      if (typeof relationId !== "string") return;
      const relation = this.displayedGraph?.nodes.find(
        (node) => node.element_kind === "relation" && node.domain_id === relationId,
      );
      if (relation !== undefined) {
        this.delegate.onSelectNode(relation.id);
      }
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
      const position = node.position();
      const point = { x: Math.round(position.x), y: Math.round(position.y) };
      this.userPinnedPositions.set(node.id(), point);
      this.basePositions.set(node.id(), point);
    });

    core.on("cxttap", (event) => {
      if (event.target !== core) return;
      if (this.focusRootId === null) return;
      showContextMenu(event.originalEvent.clientX, event.originalEvent.clientY, [
        { label: "返回全图", onSelect: () => this.exitFocus() },
      ]);
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
      items.push(
        {
          label: "以此为中心聚焦",
          disabled: this.focusRootId === id && this.focusDepth === "all",
          onSelect: () => this.setFocus(id, "all"),
        },
        {
          label: "仅展开一层",
          disabled: this.focusRootId === id && this.focusDepth === 1,
          onSelect: () => this.setFocus(id, 1),
        },
      );
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
