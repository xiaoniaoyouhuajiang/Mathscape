import { invoke } from "@tauri-apps/api/core";
import type { Core, EdgeSingular, NodeSingular } from "cytoscape";

import {
  applyFocusScope,
  mountCytoscapeGraph,
  syncCytoscapeGraph,
  type RenderMode,
} from "./graph/cytoscape";
import {
  hideContextMenu,
  isContextMenuOpen,
  showContextMenu,
  type ContextMenuItem,
} from "./graph/context-menu";
import { focusSubgraph, type FocusDepth, type FocusResult } from "./graph/focus";
import type { IncidenceGraph, IncidenceNode } from "./graph/projection";
import {
  EDGE_TYPES,
  NODE_TYPES,
  NODE_TYPE_ORDER,
  domainKindLabel,
  nodeTypeLabel,
  type EdgeTypeKey,
  type EdgeTypeVisual,
  type NodeTypeKey,
  type NodeTypeVisual,
} from "./graph/taxonomy";
import "./styles.css";
import { ArtifactReader } from "./reader/reader";
import { sourceNodeDetail } from "./reader/content";
import { markdownRenderer } from "./reader/markdown";
import { LlmSettingsController } from "./llm/settings";
import { mountDiscoveryTask } from "./discovery-task";
import {
  getEditContext,
  planMapChange,
  commitMapChange,
  renderEditActions,
  renderEditForm,
  renderCreateUnitForm,
  renderCreateRelationForm,
  renderRetireForm,
  renderImpactPreview,
  renderCommitting,
  renderCommitSuccess,
  renderCommitError,
  type UserChangeRequest,
} from "./map-edit";

interface DesktopSnapshot {
  workspace_id: string;
  workspace_root: string;
  artifact_count: number;
  conversion_count: number;
  marker: MarkerRuntimeStatus;
  pdf_artifacts: PdfArtifactSummary[];
  landscape_graph: IncidenceGraph;
}

interface MarkerRuntimeStatus {
  configured: boolean;
  ready: boolean;
  executable: string | null;
  version: string | null;
  problem: string | null;
}

interface PdfArtifactSummary {
  artifact_id: string;
  filename: string;
  page_count: number | null;
}

type PdfImportOutcome = "inserted" | "deduplicated";

interface PdfImportResult {
  artifact_id: string;
  content_hash: string;
  byte_size: number;
  filename: string;
  page_count: number;
  document_title: string | null;
  content_kind: "text_based" | "scanned" | "image_based" | "mixed";
  classification_confidence: number;
  sampled_ocr_page_count: number;
  outcome: PdfImportOutcome;
  snapshot: DesktopSnapshot;
}

type MarkerProfileChoice = "fast_native_v1" | "math_ocr_v1";

interface ParseRequest {
  artifact_id: string;
  first_page: number;
  last_page: number;
  profile: MarkerProfileChoice;
}

interface ParsedArtifactOutput {
  artifact_id: string;
  role: string;
  relative_path: string;
  byte_size: number;
}

interface PdfParseResult {
  conversion_id: string;
  input_artifact_id: string;
  first_page: number;
  last_page: number;
  profile: MarkerProfileChoice;
  marker_version: string;
  elapsed_ms: number;
  outputs: ParsedArtifactOutput[];
  cleanup_warning: string | null;
  snapshot: DesktopSnapshot;
}

const app = document.querySelector<HTMLDivElement>("#app");
if (app === null) {
  throw new Error("application root is missing");
}

app.innerHTML = `
  <div class="shell">
    <header class="topbar">
      <div class="brand-block">
        <div class="brand-mark">M</div>
        <div>
          <div class="brand">Mathscape</div>
          <div class="tagline">数学景观 · 来源视图</div>
        </div>
      </div>
      <div class="topbar-actions">
        <span id="service-state" class="service-state">正在连接本地内核</span>
        <button id="llm-settings-button" class="settings-button" type="button">
          <span aria-hidden="true">⚙</span>
          <span id="llm-settings-label">设置</span>
        </button>
        <button id="import-pdf" class="primary-button" type="button">
          <span aria-hidden="true">＋</span> 导入 PDF
        </button>
      </div>
    </header>

    <main class="workspace-grid">
      <aside class="panel source-panel">
        <button id="toggle-sources" type="button" aria-expanded="true" aria-label="折叠来源目录">‹</button>
        <div class="panel-heading">
          <span class="eyebrow">SOURCE LAYER</span>
          <h1>来源目录</h1>
        </div>
        <div class="stat-grid">
          <article class="stat-card">
            <span>Artifacts</span>
            <strong id="artifact-count">—</strong>
          </article>
          <article class="stat-card">
            <span>Conversions</span>
            <strong id="conversion-count">—</strong>
          </article>
        </div>
        <section class="workspace-card">
          <span class="field-label">Workspace</span>
          <strong id="workspace-name" class="import-name">我的数学景观</strong>
          <details class="technical-details">
          <summary>技术详情</summary>
          <code id="workspace-id">正在初始化…</code>
          <button id="copy-workspace-id" class="copy-button" type="button">复制 ID</button>
          <span class="field-label">本地路径</span>
          <p id="workspace-root" class="path-value">—</p>
          </details>
        </section>
        <section class="marker-card">
          <div class="runtime-heading">
            <span class="field-label">Marker Runtime</span>
            <span id="marker-state" class="runtime-state">检查中</span>
          </div>
          <strong id="marker-version" class="runtime-version">—</strong>
          <p id="marker-problem" class="runtime-problem">正在探测本地 Marker 环境。</p>
        </section>
        <section id="last-import" class="import-card is-empty">
          <span class="field-label">最近一次导入</span>
          <p>选择一份 PDF，将其复制到自包含 Artifact Pool。</p>
        </section>
        <section id="last-parse" class="import-card is-empty">
          <span class="field-label">最近一次解析</span>
          <p>选择图中的 PDF 节点以配置连续页码范围。</p>
        </section>
      </aside>

      <section class="canvas-panel" aria-label="数学地图">
        <div class="canvas-toolbar">
          <div>
            <span class="eyebrow">MAP</span>
            <h2 id="graph-view-title">数学地图</h2>
          </div>
          <div class="graph-controls" aria-label="过滤">
            <div class="preset-group">
              <button class="preset-button" data-preset="source" type="button">来源内容</button>
              <button class="preset-button" data-preset="material" type="button">材料</button>
              <button class="preset-button" data-preset="overview" type="button">总览</button>
            </div>
            <span class="filter-divider">│</span>
            <div class="filter-group" id="node-filter-group">
              <label class="graph-evidence-toggle"><input type="checkbox" data-node-kind="SourceArtifact"> 材料文件</label>
              <label class="graph-evidence-toggle"><input type="checkbox" data-node-kind="SourceSemanticUnit"> 内容单元</label>
              <label class="graph-evidence-toggle"><input type="checkbox" data-node-kind="SourceSpan"> 原文证据</label>
            </div>
            <span class="filter-divider">│</span>
            <label class="graph-evidence-toggle"><input id="render-mode-toggle" type="checkbox" checked> 语义边</label>
          </div>
        </div>
        <div id="graph" class="graph-canvas"></div>
        <div id="focus-bar" class="focus-bar" aria-label="聚焦范围" hidden>
          <span class="focus-label">聚焦：<strong id="focus-root">—</strong></span>
          <span class="focus-depth">
            深度
            <button class="depth-button" data-depth="1" type="button">1</button>
            <button class="depth-button" data-depth="2" type="button">2</button>
            <button class="depth-button" data-depth="all" type="button">全部</button>
          </span>
          <span id="focus-summary" class="focus-summary"></span>
          <button id="exit-focus" class="focus-exit" type="button">返回全图</button>
        </div>
        <div id="graph-legend" class="map-legend" aria-label="图例"></div>
        <section id="reader" class="reader-panel" aria-label="文件阅读器" hidden></section>
        <div id="empty-state" class="empty-state">
          <strong id="empty-state-title">尚未导入材料</strong>
          <span id="empty-state-message">导入第一份 PDF 后，节点会出现在这里。</span>
        </div>
      </section>

      <aside class="panel detail-panel">
        <div class="panel-heading">
          <span class="eyebrow">INSPECTOR</span>
          <h2>节点详情</h2>
        </div>
        <div id="node-detail" class="node-detail empty-detail">
          <div class="detail-symbol">⌁</div>
          <p>点击图中的节点以查看它在来源层中的身份和状态。</p>
        </div>
      </aside>
    </main>

    <div id="llm-settings-host"></div>
    <div id="toast" class="toast" role="status" aria-live="polite"></div>
  </div>
`;

const importButton = requiredElement<HTMLButtonElement>("#import-pdf");
mountDiscoveryTask();
const sourceToggle = requiredElement<HTMLButtonElement>("#toggle-sources");
const workspaceGrid = requiredElement<HTMLElement>(".workspace-grid");
const setSourcesCollapsed = (collapsed: boolean) => {
  workspaceGrid.classList.toggle("sources-collapsed", collapsed);
  sourceToggle.textContent = collapsed ? "›" : "‹";
  sourceToggle.setAttribute("aria-expanded", String(!collapsed));
  sourceToggle.setAttribute("aria-label", collapsed ? "展开来源目录" : "折叠来源目录");
  window.dispatchEvent(new Event("resize"));
};
try { setSourcesCollapsed(localStorage.getItem("mathscape:sources-collapsed") === "true"); } catch { /* Optional presentation preference. */ }
sourceToggle.onclick = () => {
  const collapsed = !workspaceGrid.classList.contains("sources-collapsed");
  setSourcesCollapsed(collapsed);
  try { localStorage.setItem("mathscape:sources-collapsed", String(collapsed)); } catch { /* Storage may be unavailable. */ }
};
const serviceState = requiredElement<HTMLElement>("#service-state");
const artifactCount = requiredElement<HTMLElement>("#artifact-count");
const conversionCount = requiredElement<HTMLElement>("#conversion-count");
const workspaceId = requiredElement<HTMLElement>("#workspace-id");
const workspaceName = requiredElement<HTMLElement>("#workspace-name");
const workspaceRoot = requiredElement<HTMLElement>("#workspace-root");
const graphContainer = requiredElement<HTMLElement>("#graph");
const graphLegend = requiredElement<HTMLElement>("#graph-legend");
const focusBar = requiredElement<HTMLElement>("#focus-bar");
const focusRootLabel = requiredElement<HTMLElement>("#focus-root");
const focusSummary = requiredElement<HTMLElement>("#focus-summary");
const graphViewTitle = requiredElement<HTMLElement>("#graph-view-title");
const nodeFilterGroup = requiredElement<HTMLElement>("#node-filter-group");
const emptyState = requiredElement<HTMLElement>("#empty-state");
const emptyStateTitle = requiredElement<HTMLElement>("#empty-state-title");
const emptyStateMessage = requiredElement<HTMLElement>("#empty-state-message");
const nodeDetail = requiredElement<HTMLElement>("#node-detail");
const lastImport = requiredElement<HTMLElement>("#last-import");
const lastParse = requiredElement<HTMLElement>("#last-parse");
const markerState = requiredElement<HTMLElement>("#marker-state");
const markerVersion = requiredElement<HTMLElement>("#marker-version");
const markerProblem = requiredElement<HTMLElement>("#marker-problem");
const toast = requiredElement<HTMLElement>("#toast");
const readerHost = requiredElement<HTMLElement>("#reader");
const reader = new ArtifactReader(readerHost);
const llmSettings = new LlmSettingsController(
  requiredElement<HTMLElement>("#llm-settings-host"),
  requiredElement<HTMLButtonElement>("#llm-settings-button"),
  requiredElement<HTMLElement>("#llm-settings-label"),
  showToast,
);
document.querySelectorAll<HTMLButtonElement>(".preset-button").forEach((button) => {
  button.onclick = () => {
    const preset = button.dataset.preset as PresetName;
    if (preset in PRESET_NODE_KINDS) {
      activePreset = preset;
      visibleNodeKinds = new Set(PRESET_NODE_KINDS[preset]);
      syncCheckboxesToFilter();
      persistFilter();
      renderCurrentGraph();
    }
  };
});
nodeFilterGroup.addEventListener("change", () => {
  syncFilterFromCheckboxes();
  persistFilter();
  renderCurrentGraph();
});
const renderModeToggle = requiredElement<HTMLInputElement>("#render-mode-toggle");
renderModeToggle.onchange = () => {
  renderMode = renderModeToggle.checked ? "semantic" : "hyperedge";
  try { localStorage.setItem("mathscape:render-mode", renderMode); } catch { /* Optional presentation preference. */ }
  renderCurrentGraph();
};
document.querySelectorAll<HTMLButtonElement>(".depth-button").forEach((button) => {
  button.onclick = () => {
    focusDepth = button.dataset.depth === "all" ? "all" : (Number(button.dataset.depth) as 1 | 2);
    renderCurrentGraph();
  };
});
requiredElement<HTMLButtonElement>("#exit-focus").onclick = () => exitFocus();
document.addEventListener("keydown", (event) => {
  if (event.key !== "Escape") return;
  if (isContextMenuOpen()) hideContextMenu();
  else if (focusRootId !== null) exitFocus();
});
const showMap = () => {
  reader.close();
  readerHost.hidden = true;
  graphCore?.resize();
};

let graphCore: Core | null = null;
let toastTimer: number | undefined;
let activeParse: ParseRequest | null = null;
let submittingParse = false;
let pollingParse = false;
let selectedNode: IncidenceNode | undefined;
let currentSnapshot: DesktopSnapshot | undefined;
// Snapshot refresh must not wipe an in-progress edit / retire / preview.
let inspectorLocked = false;
// The elements currently mounted, so interaction handlers bound once against a
// persistent Cytoscape instance can still resolve a node back to its model.
let displayedGraph: IncidenceGraph | undefined;
let renderMode: RenderMode = "semantic";
// Focus is a transient exploration state: unlike the filter and render mode it
// is deliberately not persisted, so a reload always returns to the whole map.
let focusRootId: string | null = null;
let focusDepth: FocusDepth = "all";
let pendingRelationSourceId: string | null = null;

type PresetName = "overview" | "material" | "source";
const PRESET_NODE_KINDS: Record<PresetName, ReadonlySet<string>> = {
  overview: new Set(["SourceArtifact", "SourceSemanticUnit", "SourceSpan"]),
  material: new Set(["SourceArtifact"]),
  source: new Set(["SourceArtifact", "SourceSemanticUnit"]),
};
let visibleNodeKinds = new Set<string>(PRESET_NODE_KINDS.source);
let activePreset: PresetName | "custom" = "source";

function syncCheckboxesToFilter(): void {
  nodeFilterGroup.querySelectorAll<HTMLInputElement>("input[data-node-kind]").forEach((cb) => {
    cb.checked = visibleNodeKinds.has(cb.dataset.nodeKind!);
  });
  syncPresetHighlight();
}
function syncFilterFromCheckboxes(): void {
  activePreset = "custom";
  visibleNodeKinds = new Set<string>();
  nodeFilterGroup.querySelectorAll<HTMLInputElement>("input[data-node-kind]").forEach((cb) => {
    if (cb.checked) visibleNodeKinds.add(cb.dataset.nodeKind!);
  });
  syncPresetHighlight();
}
function syncPresetHighlight(): void {
  document.querySelectorAll<HTMLButtonElement>(".preset-button").forEach((button) => {
    const preset = button.dataset.preset as PresetName;
    const presetKinds = PRESET_NODE_KINDS[preset];
    const active = presetKinds !== undefined && setsEqual(presetKinds, visibleNodeKinds);
    button.classList.toggle("is-active", active);
    button.setAttribute("aria-pressed", String(active));
  });
}
function setsEqual<T>(a: ReadonlySet<T>, b: ReadonlySet<T>): boolean {
  if (a.size !== b.size) return false;
  for (const item of a) if (!b.has(item)) return false;
  return true;
}
function persistFilter(): void {
  try { localStorage.setItem("mathscape:graph-filter", JSON.stringify([...visibleNodeKinds])); } catch { /* Optional presentation preference. */ }
}
try {
  const savedFilter = localStorage.getItem("mathscape:graph-filter");
  if (savedFilter !== null) {
    const parsed = JSON.parse(savedFilter) as string[];
    if (Array.isArray(parsed) && parsed.every((k: unknown) => typeof k === "string")) {
      visibleNodeKinds = new Set(parsed);
      const matchingPreset = (Object.keys(PRESET_NODE_KINDS) as PresetName[])
        .find((preset) => setsEqual(PRESET_NODE_KINDS[preset], visibleNodeKinds));
      activePreset = matchingPreset ?? "custom";
    }
  }
} catch { /* Optional presentation preference. */ }
syncCheckboxesToFilter();
try {
  const savedMode = localStorage.getItem("mathscape:render-mode");
  if (savedMode === "semantic" || savedMode === "hyperedge") renderMode = savedMode;
  renderModeToggle.checked = renderMode === "semantic";
} catch { /* Optional presentation preference. */ }

importButton.addEventListener("click", () => void importPdf());
requiredElement<HTMLButtonElement>("#copy-workspace-id").addEventListener("click", () => {
  void copyIdentifier(workspaceId.textContent ?? "");
});
void loadSnapshot();
void llmSettings.loadSummary();
window.addEventListener("mathscape:source-map-changed", () => void loadSnapshot());
window.addEventListener("mathscape:open-source", (event) => {
  const detail = (event as CustomEvent<{ artifactId: string; title: string; focus?: { start: number; end: number; blockIds?: string[] }; refreshMap?: boolean }>).detail;
  void (async () => {
    // Rendering the map closes the reader, so refresh before opening it.
    if (detail.refreshMap) await loadSnapshot();
    readerHost.hidden = false;
    await reader.open(detail.artifactId, detail.title, detail.focus);
  })();
});
window.setInterval(() => void syncParseState(), 1000);

async function syncParseState(): Promise<void> {
  if (submittingParse || pollingParse) return;
  pollingParse = true;
  try {
    const task = await invoke<ParseRequest | null>("get_active_parse");
    if (submittingParse) return;
    const finished = activeParse !== null && task === null;
    if (JSON.stringify(task) !== JSON.stringify(activeParse)) {
      activeParse = task;
      refreshParseState();
      if (finished) {
        lastParse.replaceChildren(
          textElement("span", "field-label", "后台任务已结束"),
          textElement("p", "", "正在重新读取来源图；转换结果以目录记录为准。"),
        );
        await loadSnapshot();
      }
    }
  } catch {
    // A transient IPC error must not clear the last known running task.
  } finally {
    pollingParse = false;
  }
}

function refreshParseState(): void {
  if (activeParse !== null) {
    const task = activeParse;
    lastParse.replaceChildren(
      textElement("span", "field-label", "正在解析"),
      textElement("strong", "import-name",
        currentSnapshot?.pdf_artifacts.find(pdf => pdf.artifact_id === task.artifact_id)?.filename ?? "PDF"),
      textElement("p", "", `PDF 第 ${task.first_page}–${task.last_page} 页 · ${task.profile}`),
    );
    setBusy(true, `Marker 正在解析第 ${task.first_page}–${task.last_page} 页`);
  } else {
    setBusy(false);
  }
  if (!inspectorLocked && selectedNode !== undefined && currentSnapshot !== undefined) {
    renderNodeDetail(selectedNode,
      currentSnapshot.pdf_artifacts.find(pdf => pdf.artifact_id === selectedNode?.domain_id),
      currentSnapshot.marker);
  }
}

async function loadSnapshot(): Promise<void> {
  setBusy(true, "正在读取 Workspace");
  try {
    const snapshot = await invoke<DesktopSnapshot>("get_desktop_snapshot");
    renderSnapshot(snapshot);
    serviceState.textContent = "本地内核已连接";
    serviceState.classList.add("is-ready");
  } catch (error: unknown) {
    serviceState.textContent = "Workspace 初始化失败";
    showToast(errorMessage(error), "error");
  } finally {
    setBusy(false);
  }
}

async function importPdf(): Promise<void> {
  setBusy(true, "正在检查并导入 PDF");
  try {
    const result = await invoke<PdfImportResult | null>(
      "choose_and_import_pdf",
    );
    if (result === null) {
      return;
    }
    renderSnapshot(result.snapshot);
    renderImportResult(result);
    showToast(
      result.outcome === "inserted"
        ? `已导入 ${result.filename}`
        : `${result.filename} 已存在，复用了原 Artifact`,
      "success",
    );
  } catch (error: unknown) {
    showToast(errorMessage(error), "error");
  } finally {
    setBusy(false);
  }
}

function renderSnapshot(snapshot: DesktopSnapshot): void {
  currentSnapshot = snapshot;
  if (selectedNode !== undefined) {
    selectedNode = snapshot.landscape_graph.nodes.find((node) => node.id === selectedNode?.id);
  }
  artifactCount.textContent = snapshot.artifact_count.toString();
  conversionCount.textContent = snapshot.conversion_count.toString();
  workspaceId.textContent = snapshot.workspace_id;
  const folder = snapshot.workspace_root.split(/[\\/]/).filter(Boolean).at(-1);
  workspaceName.textContent = !folder || folder === "workspace" ? "我的数学景观" : folder;
  workspaceName.title = workspaceName.textContent;
  workspaceRoot.textContent = snapshot.workspace_root;
  renderMarkerStatus(snapshot.marker);

  renderCurrentGraph(snapshot);
}

function renderCurrentGraph(snapshot = currentSnapshot): void {
  if (snapshot === undefined) return;
  showMap();
  hideContextMenu();
  if (focusRootId !== null && !snapshot.landscape_graph.nodes.some((n) => n.id === focusRootId)) {
    focusRootId = null;
  }
  // Focus traverses the unfiltered projection and contracts paths that cross
  // hidden nodes, so the visibility filter is passed in rather than applied first.
  const focus =
    focusRootId === null
      ? null
      : focusSubgraph(snapshot.landscape_graph, focusRootId, {
          depth: focusDepth,
          isEntityVisible,
        });
  const filtered = filterGraph(snapshot.landscape_graph);
  // The focused region is emphasized inside the whole filtered map rather than
  // replacing it, so the user keeps the surrounding context and the layout.
  displayedGraph = focus === null ? filtered : mergeGraphs(filtered, focus.graph);
  const focusScope =
    focus === null ? null : new Set(focus.graph.nodes.map((node) => node.id));
  syncPresetHighlight();
  renderFocusBar(focus, snapshot.landscape_graph);
  const entityCount = (focus === null ? filtered : focus.graph).nodes.filter(
    (n) => n.element_kind === "entity",
  ).length;
  const hasAnyData = snapshot.landscape_graph.nodes.some((n) => n.element_kind !== "group");
  if (focus !== null) {
    emptyStateTitle.textContent = "聚焦范围内没有可显示的节点";
    emptyStateMessage.textContent = "该来源尚未派生出内容，或当前过滤条件隐藏了全部后代。";
  } else {
    emptyStateTitle.textContent = hasAnyData ? "当前过滤条件下无可见节点" : "尚未导入材料";
    emptyStateMessage.textContent = hasAnyData
      ? "尝试切换显示预设或勾选更多节点类型。"
      : "导入第一份 PDF 后，节点会出现在这里。";
  }
  if (graphCore === null) {
    graphCore = mountCytoscapeGraph(graphContainer, displayedGraph, renderMode);
    bindGraphInteractions(graphCore);
  } else {
    syncCytoscapeGraph(graphCore, displayedGraph, renderMode);
  }
  applyFocusScope(graphCore, focusScope);
  renderLegend(graphCore);
  // In focus mode the root is always rendered, so "nothing to show" means the
  // root came back without any visible descendant.
  emptyState.classList.toggle("is-hidden", entityCount > (focus === null ? 0 : 1));
  refreshParseState();
}

/** Union by element ID; `overlay` wins so contracted paths keep their labels. */
function mergeGraphs(base: IncidenceGraph, overlay: IncidenceGraph): IncidenceGraph {
  const nodes = new Map(base.nodes.map((node) => [node.id, node]));
  for (const node of overlay.nodes) nodes.set(node.id, node);
  const edges = new Map(base.edges.map((edge) => [edge.id, edge]));
  for (const edge of overlay.edges) edges.set(edge.id, edge);
  return { ...base, nodes: [...nodes.values()], edges: [...edges.values()] };
}

function bindGraphInteractions(core: Core): void {
  core.on("tap", "node", (event) => {
    const id = (event.target as NodeSingular).id();
    if (pendingRelationSourceId !== null) {
      const target = displayedGraph?.nodes.find((node) => node.id === id);
      if (target?.domain_kind === "SourceSemanticUnit" && target.id !== pendingRelationSourceId) {
        const sourceId = pendingRelationSourceId;
        pendingRelationSourceId = null;
        void enterCreateRelationMode(sourceId, target.domain_id!);
      } else {
        showToast("请选择另一个内容单元作为目标节点", "error");
      }
      return;
    }
    selectNode(id);
  });
  core.on("tap", "edge", (event) => {
    const relationId = (event.target as EdgeSingular).data("relationId");
    if (typeof relationId !== "string") return;
    const relation = displayedGraph?.nodes.find(
      (node) => node.element_kind === "relation" && node.domain_id === relationId,
    );
    if (relation !== undefined) selectNode(relation.id);
  });
  core.on("cxttap", "node", (event) => {
    const node = event.target as NodeSingular;
    selectNode(node.id());
    const position = event.renderedPosition ?? { x: 0, y: 0 };
    const bounds = graphContainer.getBoundingClientRect();
    showContextMenu(
      bounds.left + position.x,
      bounds.top + position.y,
      nodeMenuItems(node.id()),
    );
  });
  core.on("cxttap", (event) => {
    if (event.target !== core) return;
    if (focusRootId === null) return;
    showContextMenu(event.originalEvent.clientX, event.originalEvent.clientY, [
      { label: "返回全图", onSelect: exitFocus },
    ]);
  });
}

function selectNode(id: string): void {
  const model = displayedGraph?.nodes.find((node) => node.id === id);
  if (model === undefined || currentSnapshot === undefined) return;
  inspectorLocked = false;
  selectedNode = model;
  const pdf = currentSnapshot.pdf_artifacts.find(
    (artifact) => artifact.artifact_id === model.domain_id,
  );
  renderNodeDetail(model, pdf, currentSnapshot.marker);
}

function nodeMenuItems(id: string): ContextMenuItem[] {
  const node = displayedGraph?.nodes.find((candidate) => candidate.id === id);
  const items: ContextMenuItem[] = [];
  if (node?.domain_kind === "SourceSemanticUnit" && node.domain_id !== null) {
    items.push({
      label: "以此为父节点新建内容单元",
      onSelect: () => void enterCreateUnitMode(node.domain_id!),
    });
    items.push({
      label: "从此节点创建来源关系",
      onSelect: () => {
        pendingRelationSourceId = node.id;
        showToast("请点击地图中的另一个内容单元作为目标", "success");
      },
    });
  }
  if (node !== undefined && canFocus(node)) {
    items.push(
      {
        label: "以此为中心聚焦",
        disabled: focusRootId === id && focusDepth === "all",
        onSelect: () => setFocus(id, "all"),
      },
      {
        label: "仅展开一层",
        disabled: focusRootId === id && focusDepth === 1,
        onSelect: () => setFocus(id, 1),
      },
    );
  }
  items.push({
    label: "返回全图",
    disabled: focusRootId === null,
    onSelect: exitFocus,
  });
  return items;
}

/** Only entities that sit upstream of a derivation chain can anchor a focus. */
function canFocus(node: IncidenceNode): boolean {
  return (
    node.element_kind === "entity" &&
    (node.domain_kind === "SourceArtifact" || node.domain_kind === "SourceSemanticUnit")
  );
}

function setFocus(id: string, depth: FocusDepth): void {
  focusRootId = id;
  focusDepth = depth;
  renderCurrentGraph();
}

function exitFocus(): void {
  focusRootId = null;
  renderCurrentGraph();
}

function renderImportResult(result: PdfImportResult): void {
  lastImport.classList.remove("is-empty");
  lastImport.replaceChildren(
    textElement("span", "field-label", "最近一次导入"),
    textElement("strong", "import-name", result.filename),
    definitionList([
      ["结果", result.outcome === "inserted" ? "已写入" : "已去重"],
      ["页数", `${result.page_count}`],
      ["类型", contentKindLabel(result.content_kind)],
      ["抽样 OCR 页", `${result.sampled_ocr_page_count}`],
    ]),
  );
}

function renderNodeDetail(
  node: IncidenceNode,
  pdf: PdfArtifactSummary | undefined,
  marker: MarkerRuntimeStatus,
): void {
  nodeDetail.classList.remove("empty-detail");
  nodeDetail.replaceChildren(
    textElement("span", "kind-pill", nodeTypeLabel(node)),
    textElement("h3", "detail-title", node.label),
    definitionList([
      ["类型", domainKindLabel(node.domain_kind)],
      ["层级", displayLayer(node.layer)],
      ["生命周期", displayLifecycle(node.lifecycle)],
      ["摘要", node.summary ?? "—"],
    ]),
  );
  if (node.domain_id !== null && (node.domain_kind === "SourceSemanticUnit" || node.domain_kind === "SourceSpan")) void renderSourceDetail(node, nodeDetail);
  const technical = document.createElement("details");
  technical.className = "technical-details";
  technical.append(textElement("summary", "", "技术详情"));
  technical.append(textElement("code", "identifier-value", node.domain_id ?? "—"));
  if (node.domain_id !== null) {
    const copy = textElement("button", "copy-button", "复制 ID");
    copy.type = "button";
    copy.addEventListener("click", () => void copyIdentifier(node.domain_id!));
    technical.append(copy);
  }
  nodeDetail.append(technical);
  if (canFocus(node)) {
    const focused = focusRootId === node.id;
    const focusButton = textElement(
      "button",
      "parse-button",
      focused ? "已以此为中心" : "以此为中心聚焦",
    );
    focusButton.type = "button";
    focusButton.disabled = focused;
    focusButton.onclick = () => setFocus(node.id, "all");
    nodeDetail.append(focusButton);
  }
  if (node.domain_kind === "SourceArtifact" && node.domain_id !== null) {
    const read = textElement("button", "parse-button", "阅读文件");
    read.type = "button";
    read.onclick = () => {
      readerHost.hidden = false;
      void reader.open(node.domain_id!, node.label);
    };
    nodeDetail.append(read);
  }
  if (pdf !== undefined) {
    nodeDetail.append(createParseForm(pdf, marker));
  }
  // Map-edit: add edit/retire buttons for editable domain objects
  if (
    node.domain_id !== null &&
    (node.domain_kind === "SourceSemanticUnit" || node.domain_kind === "SourceUnitRelation")
  ) {
    void mountEditActions(node.domain_id);
  }
}

// ---------------------------------------------------------------------------
// Map-edit flow orchestrator
// ---------------------------------------------------------------------------

async function mountEditActions(objectId: string): Promise<void> {
  try {
    const editCtx = await getEditContext(objectId);
    if (inspectorLocked || selectedNode?.domain_id !== objectId) return;
    renderEditActions(nodeDetail, editCtx, () => enterEditMode(editCtx), () => enterRetireMode(editCtx));
  } catch (error) {
    const message = errorMessage(error);
    // Projection-synthesized relations (has_part / contains_span / located_in)
    // are not stored objects and have no edit context.
    if (message.includes("对象不存在")) return;
    nodeDetail.append(textElement("p", "detail-error", `编辑上下文读取失败：${message}`));
  }
}

function enterEditMode(editCtx: import("./map-edit").EditContext): void {
  inspectorLocked = true;
  nodeDetail.classList.remove("empty-detail");
  nodeDetail.replaceChildren();
  let draftForm: HTMLFormElement | null = null;
  const { form } = renderEditForm(
    editCtx,
    (request) => void enterPreviewMode(request, () => {
      if (draftForm !== null) { nodeDetail.replaceChildren(draftForm); inspectorLocked = true; }
    }),
    () => restoreNodeDetail(),
  );
  draftForm = form;
  nodeDetail.append(form);
}

function enterRetireMode(editCtx: import("./map-edit").EditContext): void {
  inspectorLocked = true;
  nodeDetail.classList.remove("empty-detail");
  nodeDetail.replaceChildren();
  let draftForm: HTMLFormElement | null = null;
  nodeDetail.append(
    draftForm = renderRetireForm(
      editCtx,
      (request) => void enterPreviewMode(request, () => {
        if (draftForm !== null) { nodeDetail.replaceChildren(draftForm); inspectorLocked = true; }
      }),
      () => restoreNodeDetail(),
    ),
  );
}

async function enterCreateUnitMode(parentId: string): Promise<void> {
  try {
    const context = await getEditContext(parentId);
    // The context intentionally excludes the object itself from its edit
    // parent list to prevent cycles. For creation, the selected node is the
    // explicit parent and must be offered as the initial choice.
    if (!context.available_parents.some((option) => option.object_id === parentId)) {
      context.available_parents.unshift({
        object_id: parentId,
        label: selectedNode?.label ?? parentId,
      });
    }
    inspectorLocked = true;
    nodeDetail.replaceChildren();
    let draftForm: HTMLFormElement | null = null;
    draftForm = renderCreateUnitForm(context, parentId, (request) => void enterPreviewMode(request, () => {
      if (draftForm !== null) { nodeDetail.replaceChildren(draftForm); inspectorLocked = true; }
    }), () => restoreNodeDetail());
    nodeDetail.append(draftForm);
  } catch (error) {
    showToast(`读取创建上下文失败：${errorMessage(error)}`, "error");
  }
}

async function enterCreateRelationMode(sourceId: string, targetId: string): Promise<void> {
  try {
    const context = await getEditContext(sourceId);
    inspectorLocked = true;
    nodeDetail.replaceChildren();
    let draftForm: HTMLFormElement | null = null;
    draftForm = renderCreateRelationForm(context, sourceId, targetId, (request) => void enterPreviewMode(request, () => {
      if (draftForm !== null) { nodeDetail.replaceChildren(draftForm); inspectorLocked = true; }
    }), () => restoreNodeDetail());
    nodeDetail.append(draftForm);
  } catch (error) {
    showToast(`读取关系创建上下文失败：${errorMessage(error)}`, "error");
  }
}

async function enterPreviewMode(
  request: UserChangeRequest,
  onBack: () => void,
): Promise<void> {
  inspectorLocked = true;
  nodeDetail.replaceChildren();
  nodeDetail.append(textElement("p", "", "正在分析变更影响…"));
  try {
    const plan = await planMapChange(request);
    nodeDetail.replaceChildren();
    renderImpactPreview(
      nodeDetail,
      plan,
      () => void executeCommit(request, plan.plan_fingerprint, onBack),
      onBack,
    );
  } catch (error) {
    renderCommitError(
      nodeDetail,
      errorMessage(error),
      () => void enterPreviewMode(request, onBack),
      () => restoreNodeDetail(),
    );
  }
}

async function executeCommit(
  request: UserChangeRequest,
  fingerprint: string,
  onBack: () => void,
): Promise<void> {
  inspectorLocked = true;
  renderCommitting(nodeDetail);
  try {
    const receipt = await commitMapChange(request, fingerprint);
    try {
      await loadSnapshot();
    } catch {
      // The commit already succeeded; a snapshot refresh failure must not hide the receipt.
    }
    renderCommitSuccess(nodeDetail, receipt, () => restoreNodeDetail());
    showToast("变更已提交", "success");
  } catch (error) {
    renderCommitError(
      nodeDetail,
      errorMessage(error),
      () => void enterPreviewMode(request, onBack),
      () => restoreNodeDetail(),
    );
  }
}

function restoreNodeDetail(): void {
  inspectorLocked = false;
  if (selectedNode !== undefined && currentSnapshot !== undefined) {
    renderNodeDetail(
      selectedNode,
      currentSnapshot.pdf_artifacts.find((pdf) => pdf.artifact_id === selectedNode?.domain_id),
      currentSnapshot.marker,
    );
  } else {
    nodeDetail.classList.add("empty-detail");
    nodeDetail.replaceChildren(
      textElement("div", "detail-symbol", "⌁"),
      textElement("p", "", "点击图中的节点以查看它在来源层中的身份和状态。"),
    );
  }
}

function displayLayer(layer: string): string { return ({ source: "来源层", local_semantic: "局部语义层", global_semantic: "全局语义层", alignment: "对齐层", rdtx: "RDTX 层", formalization: "形式化层" } as Record<string, string>)[layer] ?? layer; }
function displayLifecycle(lifecycle: string | null): string {
  if (lifecycle === null) return "—";
  return ({
    active: "活动",
    deprecated: "已退役",
    superseded: "已被替代",
    rejected: "已拒绝",
    deleted: "已删除",
  } as Record<string, string>)[lifecycle] ?? lifecycle;
}
async function renderSourceDetail(node: IncidenceNode, host: HTMLElement): Promise<void> {
  try { const detail = await sourceNodeDetail(node.domain_id!); if (detail.excerpt) { const preview = document.createElement("div"); preview.className = "source-preview markdown-body"; preview.innerHTML = markdownRenderer().render(detail.excerpt); host.append(preview); } if (detail.artifact_id) { const button = textElement("button", "parse-button", "打开并定位原文"); button.type = "button"; button.onclick = () => { readerHost.hidden = false; void reader.open(detail.artifact_id!, detail.display_title ?? node.label, detail.byte_range ? { ...detail.byte_range, blockIds: detail.markdown_block_ids } : undefined); }; host.append(button); } }
  catch (error) { host.append(textElement("p", "detail-error", `来源详情读取失败：${error instanceof Error ? error.message : String(error)}`)); }
}

function renderFocusBar(focus: FocusResult | null, graph: IncidenceGraph): void {
  focusBar.hidden = focus === null;
  if (focus === null) return;
  focusRootLabel.textContent =
    graph.nodes.find((node) => node.id === focusRootId)?.label ?? "—";
  const contracted =
    focus.contractedCount > 0 ? ` · 折叠了 ${focus.contractedCount} 个被过滤的中间节点` : "";
  focusSummary.textContent = `展开 ${focus.reachedCount} 个节点${contracted}`;
  document.querySelectorAll<HTMLButtonElement>(".depth-button").forEach((button) => {
    const active = button.dataset.depth === String(focusDepth);
    button.classList.toggle("is-active", active);
    button.setAttribute("aria-pressed", String(active));
  });
}

function isEntityVisible(node: IncidenceNode): boolean {
  if (node.domain_kind === null) return false;
  const knownNodeKinds = new Set(["SourceArtifact", "SourceSemanticUnit", "SourceSpan"]);
  return (
    (activePreset === "overview" && !knownNodeKinds.has(node.domain_kind)) ||
    visibleNodeKinds.has(node.domain_kind)
  );
}

function filterGraph(graph: IncidenceGraph): IncidenceGraph {
  // Unknown types are intentionally visible only in the all-layer overview.
  // Source/material presets remain stable until their layer-specific controls
  // are added, so a future semantic node cannot silently leak into them.
  const knownRelationKinds = new Set(["ConversionRun", "SourceUnitRelation"]);
  const allowUnknownKinds = activePreset === "overview";
  const relationKinds = new Set<string>();
  if (visibleNodeKinds.has("SourceArtifact")) relationKinds.add("ConversionRun");
  if (visibleNodeKinds.has("SourceSemanticUnit")) relationKinds.add("SourceUnitRelation");
  const nodes = graph.nodes.filter((node) => {
    if (node.element_kind === "group") return true;
    if (node.element_kind === "relation") {
      if (node.domain_kind === null) return false;
      return (allowUnknownKinds && !knownRelationKinds.has(node.domain_kind))
        || relationKinds.has(node.domain_kind);
    }
    return isEntityVisible(node);
  });
  const nodeIds = new Set(nodes.map((n) => n.id));
  const edges = graph.edges.filter((e) => nodeIds.has(e.source) && nodeIds.has(e.target));
  return { ...graph, nodes, edges };
}

// The legend is read back from the mounted graph so it can never describe a
// type that the current filter and render mode did not actually produce.
function renderLegend(core: Core): void {
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
    if (nodeKeys.has(key)) items.push(legendItem(nodeSwatch(NODE_TYPES[key]), NODE_TYPES[key].label));
  }
  for (const key of ["relation", "evidence", "contracted", "incidence"] as EdgeTypeKey[]) {
    if (edgeKeys.has(key)) items.push(legendItem(edgeSwatch(EDGE_TYPES[key]), EDGE_TYPES[key].label));
  }
  graphLegend.replaceChildren(...items);
}

function legendItem(swatch: HTMLElement, label: string): HTMLElement {
  const item = document.createElement("span");
  item.className = "legend-item";
  item.append(swatch, document.createTextNode(label));
  return item;
}

function nodeSwatch(visual: NodeTypeVisual): HTMLElement {
  const shape = visual.shape === "ellipse" ? "is-round" : visual.shape === "diamond" ? "is-diamond" : "is-rect";
  const swatch = document.createElement("span");
  swatch.className = `legend-dot ${shape}${visual.borderStyle === "dashed" ? " is-dashed" : ""}`;
  swatch.style.backgroundColor = visual.background;
  swatch.style.borderColor = visual.border;
  return swatch;
}

function edgeSwatch(visual: EdgeTypeVisual): HTMLElement {
  const swatch = document.createElement("span");
  swatch.className = `legend-line${visual.dashed ? " is-dashed" : ""}`;
  swatch.style.color = visual.color;
  return swatch;
}

function createParseForm(
  pdf: PdfArtifactSummary,
  marker: MarkerRuntimeStatus,
): HTMLFormElement {
  const form = document.createElement("form");
  form.className = "parse-form";
  const heading = textElement("span", "field-label", "Marker 解析范围");
  const range = document.createElement("div");
  range.className = "range-fields";
  const firstPage = pageInput("起始页", 1, pdf.page_count);
  const lastPage = pageInput("结束页", pdf.page_count ?? 1, pdf.page_count);
  const running = activeParse?.artifact_id === pdf.artifact_id ? activeParse : null;
  if (running !== null) {
    firstPage.input.value = String(running.first_page);
    lastPage.input.value = String(running.last_page);
  }
  firstPage.input.disabled = activeParse !== null;
  lastPage.input.disabled = activeParse !== null;
  range.append(firstPage.label, lastPage.label);

  const profileLabel = document.createElement("label");
  profileLabel.className = "form-field profile-field";
  profileLabel.append(textElement("span", "", "识别策略"));
  const profile = document.createElement("select");
  profile.append(
    new Option("Math OCR · balanced", "math_ocr_v1"),
    new Option("Fast native", "fast_native_v1"),
  );
  profileLabel.append(profile);
  if (running !== null) profile.value = running.profile;
  profile.disabled = activeParse !== null;

  const submit = document.createElement("button");
  submit.type = "submit";
  submit.className = "parse-button";
  submit.textContent = marker.ready ? "解析选定页面" : "Marker 尚未就绪";
  submit.disabled = !marker.ready || pdf.page_count === null;
  if (activeParse !== null) {
    submit.disabled = true;
    submit.textContent = running !== null ? "正在解析…" : "请等待当前解析完成";
  }
  const note = textElement(
    "p",
    "form-note",
    pdf.page_count === null
      ? "该 Artifact 缺少页数，暂不能构造安全页码范围。"
      : `页码按 PDF 页面计算，共 ${pdf.page_count} 页；一次请求必须连续。`,
  );
  form.append(heading, range, profileLabel, submit, note);
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    const first = firstPage.input.valueAsNumber;
    const last = lastPage.input.valueAsNumber;
    if (
      !Number.isInteger(first) ||
      !Number.isInteger(last) ||
      first < 1 ||
      last < first ||
      pdf.page_count === null ||
      last > pdf.page_count
    ) {
      showToast("请输入 PDF 页数范围内的连续页码。", "error");
      return;
    }
    void parsePdf(
      pdf,
      first,
      last,
      profile.value as MarkerProfileChoice,
      submit,
    );
  });
  return form;
}

function pageInput(
  caption: string,
  value: number,
  maximum: number | null,
): { label: HTMLLabelElement; input: HTMLInputElement } {
  const label = document.createElement("label");
  label.className = "form-field";
  label.append(textElement("span", "", caption));
  const input = document.createElement("input");
  input.type = "number";
  input.min = "1";
  input.step = "1";
  input.value = value.toString();
  if (maximum !== null) {
    input.max = maximum.toString();
  }
  label.append(input);
  return { label, input };
}

async function parsePdf(
  pdf: PdfArtifactSummary,
  firstPage: number,
  lastPage: number,
  profile: MarkerProfileChoice,
  submit: HTMLButtonElement,
): Promise<void> {
  if (activeParse !== null || submittingParse) return;
  submittingParse = true;
  activeParse = {
    artifact_id: pdf.artifact_id, first_page: firstPage, last_page: lastPage, profile,
  };
  refreshParseState();
  setBusy(true, `Marker 正在解析第 ${firstPage}–${lastPage} 页`);
  submit.disabled = true;
  submit.textContent = "正在解析…";
  try {
    const result = await invoke<PdfParseResult>("parse_pdf", {
      request: {
        artifact_id: pdf.artifact_id,
        first_page: firstPage,
        last_page: lastPage,
        profile,
      },
    });
    renderSnapshot(result.snapshot);
    renderParseResult(result, pdf.filename);
    showToast(
      `已解析 ${pdf.filename} 第 ${firstPage}–${lastPage} 页，生成 ${result.outputs.length} 个产物`,
      "success",
    );
  } catch (error: unknown) {
    submit.disabled = false;
    submit.textContent = "解析选定页面";
    showToast(errorMessage(error), "error");
    lastParse.replaceChildren(
      textElement("span", "field-label", "解析失败"),
      textElement("p", "", errorMessage(error)),
    );
  } finally {
    activeParse = null;
    submittingParse = false;
    refreshParseState();
    void syncParseState();
    setBusy(false);
  }
}

function renderMarkerStatus(marker: MarkerRuntimeStatus): void {
  markerState.textContent = marker.ready ? "可用" : marker.configured ? "异常" : "未配置";
  markerState.classList.toggle("is-ready", marker.ready);
  markerVersion.textContent = marker.version === null ? "—" : `marker-pdf ${marker.version}`;
  markerProblem.textContent = marker.ready
    ? marker.executable ?? "Marker 已通过版本探测"
    : marker.problem ?? "Marker 不可用";
}

function renderParseResult(result: PdfParseResult, filename: string): void {
  lastParse.classList.remove("is-empty");
  lastParse.replaceChildren(
    textElement("span", "field-label", "最近一次解析"),
    textElement("strong", "import-name", filename),
    definitionList([
      ["页面", `${result.first_page}–${result.last_page}`],
      ["Profile", result.profile],
      ["产物", `${result.outputs.length}`],
      ["耗时", formatDuration(result.elapsed_ms)],
    ]),
  );
  if (result.cleanup_warning !== null) {
    lastParse.append(
      textElement("p", "runtime-problem", result.cleanup_warning),
    );
  }
}

function definitionList(rows: ReadonlyArray<readonly [string, string]>): HTMLElement {
  const list = document.createElement("dl");
  list.className = "definition-list";
  for (const [term, value] of rows) {
    list.append(textElement("dt", "", term), textElement("dd", "", value));
  }
  return list;
}

function textElement<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className: string,
  value: string,
): HTMLElementTagNameMap[K] {
  const element = document.createElement(tag);
  element.className = className;
  element.textContent = value;
  return element;
}

function requiredElement<T extends Element>(selector: string): T {
  const element = document.querySelector<T>(selector);
  if (element === null) {
    throw new Error(`required element is missing: ${selector}`);
  }
  return element;
}

function setBusy(busy: boolean, message = ""): void {
  if (activeParse !== null) {
    busy = true;
    message = `Marker 正在解析第 ${activeParse.first_page}–${activeParse.last_page} 页`;
  }
  importButton.disabled = busy;
  importButton.classList.toggle("is-busy", busy);
  if (busy) {
    serviceState.textContent = message;
  } else if (!serviceState.textContent?.includes("失败")) {
    serviceState.textContent = "本地内核已连接";
  }
}

function showToast(message: string, kind: "success" | "error"): void {
  window.clearTimeout(toastTimer);
  toast.textContent = message;
  toast.dataset.kind = kind;
  toast.classList.add("is-visible");
  toastTimer = window.setTimeout(() => toast.classList.remove("is-visible"), 5000);
}

function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "发生了未知错误";
}

async function copyIdentifier(value: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(value);
    showToast("已复制完整 ID", "success");
  } catch {
    showToast("剪贴板不可用，请展开技术详情后选择 ID 复制。", "error");
  }
}

function contentKindLabel(kind: PdfImportResult["content_kind"]): string {
  return {
    text_based: "文本型",
    scanned: "扫描型",
    image_based: "图像型",
    mixed: "混合型",
  }[kind];
}

function formatDuration(milliseconds: number): string {
  if (milliseconds < 1_000) {
    return `${milliseconds} ms`;
  }
  const seconds = milliseconds / 1_000;
  return seconds < 60 ? `${seconds.toFixed(1)} s` : `${(seconds / 60).toFixed(1)} min`;
}
