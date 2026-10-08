import type { Core, EdgeSingular, NodeSingular } from "cytoscape";
import type { DesktopSnapshot } from "../artifact/types";
import {
  hideContextMenu,
  isContextMenuOpen,
  showContextMenu,
  type ContextMenuItem,
} from "./context-menu";
import {
  applyFocusScope,
  mountCytoscapeGraph,
  syncCytoscapeGraph,
  type RenderMode,
} from "./cytoscape";
import { focusSubgraph, type FocusDepth, type FocusResult } from "./focus";
import type { IncidenceGraph, IncidenceNode } from "./projection";
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
  overview: new Set(["SourceArtifact", "SourceSemanticUnit", "SourceSpan"]),
  material: new Set(["SourceArtifact"]),
  source: new Set(["SourceArtifact", "SourceSemanticUnit"]),
};

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
}

export class GraphController {
  private graphCore: Core | null = null;
  private displayedGraph?: IncidenceGraph;
  private renderMode: RenderMode = "semantic";
  private focusRootId: string | null = null;
  private focusDepth: FocusDepth = "all";
  private pendingRelationSourceId: string | null = null;

  private visibleNodeKinds = new Set<string>(PRESET_NODE_KINDS.source);
  private activePreset: PresetName | "custom" = "source";
  private showRetired = false;

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
          this.visibleNodeKinds = new Set(parsed);
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

  renderCurrentGraph(snapshot = this.delegate.getSnapshot()): void {
    if (snapshot === undefined) return;
    this.delegate.onBeforeRenderGraph?.();
    hideContextMenu();
    const landscape=activityGraph(snapshot.landscape_graph,this.showRetired);

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
            isEntityVisible: (node) => this.isEntityVisible(node),
          });

    const filtered = this.filterGraph(landscape);
    this.displayedGraph = focus === null ? filtered : this.mergeGraphs(filtered, focus.graph);
    const focusScope =
      focus === null ? null : new Set(focus.graph.nodes.map((node) => node.id));

    this.syncPresetHighlight();
    this.renderFocusBar(focus, landscape);

    const entityCount = (focus === null ? filtered : focus.graph).nodes.filter(
      (n) => n.element_kind === "entity",
    ).length;
    const hasAnyData = snapshot.landscape_graph.nodes.some((n) => n.element_kind !== "group");

    if (focus !== null) {
      this.elements.emptyStateTitle.textContent = "聚焦范围内没有可显示的节点";
      this.elements.emptyStateMessage.textContent =
        "该来源尚未派生出内容，或当前过滤条件隐藏了全部后代。";
    } else {
      this.elements.emptyStateTitle.textContent = hasAnyData
        ? "当前过滤条件下无可见节点"
        : "尚未导入材料";
      this.elements.emptyStateMessage.textContent = hasAnyData
        ? "尝试切换显示预设或勾选更多节点类型。"
        : "导入第一份 PDF 后，节点会出现在这里。";
    }

    if (this.graphCore === null) {
      this.graphCore = mountCytoscapeGraph(
        this.elements.graphContainer,
        this.displayedGraph,
        this.renderMode,
      );
      this.bindGraphInteractions(this.graphCore);
    } else {
      syncCytoscapeGraph(this.graphCore, this.displayedGraph, this.renderMode);
    }

    applyFocusScope(this.graphCore, focusScope);
    this.renderLegend(this.graphCore);
    this.elements.emptyState.classList.toggle(
      "is-hidden",
      entityCount > (focus === null ? 0 : 1),
    );
    this.delegate.onGraphRendered?.();
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
    const knownRelationKinds = new Set(["ConversionRun", "SourceUnitRelation"]);
    const allowUnknownKinds = this.activePreset === "overview";
    const relationKinds = new Set<string>();
    if (this.visibleNodeKinds.has("SourceArtifact")) relationKinds.add("ConversionRun");
    if (this.visibleNodeKinds.has("SourceSemanticUnit")) relationKinds.add("SourceUnitRelation");

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
    const knownNodeKinds = new Set(["SourceArtifact", "SourceSemanticUnit", "SourceSpan"]);
    return (
      (this.activePreset === "overview" && !knownNodeKinds.has(node.domain_kind)) ||
      this.visibleNodeKinds.has(node.domain_kind)
    );
  }

  private bindGraphInteractions(core: Core): void {
    core.on("tap", "node", (event) => {
      const id = (event.target as NodeSingular).id();
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
      this.delegate.onSelectNode(node.id());
      const position = event.renderedPosition ?? { x: 0, y: 0 };
      const bounds = this.elements.graphContainer.getBoundingClientRect();
      showContextMenu(
        bounds.left + position.x,
        bounds.top + position.y,
        this.nodeMenuItems(node.id()),
      );
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
      if (edge.hasClass("contracted-edge")) edgeKeys.add("contracted");
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
    for (const key of ["relation", "evidence", "contracted", "incidence"] as EdgeTypeKey[]) {
      if (edgeKeys.has(key)) {
        items.push(this.legendItem(this.edgeSwatch(EDGE_TYPES[key]), EDGE_TYPES[key].label));
      }
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
