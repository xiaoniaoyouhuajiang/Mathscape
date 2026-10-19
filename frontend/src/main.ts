import { invoke } from "@tauri-apps/api/core";
import { ArtifactController } from "./artifact/artifact-controller";
import type { DesktopSnapshot } from "./artifact/types";
import { eventBus } from "./core/events";
import { errorMessage } from "./core/ipc";
import { DiscoveryLedgerController } from "./discovery/discovery-ledger";
import { mountDiscoveryTask } from "./discovery-task";
import { GraphController } from "./graph/graph-controller";
import { InspectorController } from "./inspector/inspector-controller";
import { LlmSettingsController } from "./llm/settings";
import { DesktopInkWorkspace } from "./ink/desktop";
import { connectInkViews, type InkViewSync } from "./ink/views";
import {
  createInkNoteFromContext,
  setInkContextCreator,
  setInkSourceCandidateProvider,
} from "./ink/links";
import { ArtifactReader } from "./reader/reader";
import { mountReviewWorkspace } from "./review/workspace";
import { AppShell } from "./shell/app-shell";
import "./styles.css";

const app = document.querySelector<HTMLDivElement>("#app");
if (app === null) {
  throw new Error("application root is missing");
}

let artifactController!: ArtifactController;
const shell = new AppShell(app, {
  isParseActive: () => artifactController?.isParseActive() ?? false,
});
const el = shell.elements;

let inkWorkspace!: DesktopInkWorkspace;
let inkOpen = false;
const closeInkWorkspace = () => {
  inkOpen = false;
  el.inkHost.hidden = true;
  el.workspaceGrid.hidden = false;
  el.inkButton.textContent = "手写";
  // Safety net for changes made from the tablet while the map was hidden.
  loadInkOverlay();
};
inkWorkspace = new DesktopInkWorkspace(el.inkHost, closeInkWorkspace, (message, kind) => shell.showToast(message, kind));
el.inkButton.addEventListener("click", () => {
  inkOpen = !inkOpen;
  if (inkOpen) {
    el.workspaceGrid.hidden = true;
    el.inkHost.hidden = false;
    el.inkButton.textContent = "返回地图";
    void inkWorkspace.open();
  } else {
    inkWorkspace.close();
  }
});

mountDiscoveryTask();
mountReviewWorkspace(el.reviewWorkspaceHost);
const reader = new ArtifactReader(el.readerHost);

const llmSettings = new LlmSettingsController(
  el.llmSettingsHost,
  el.llmSettingsButton,
  el.llmSettingsLabel,
  (msg, kind) => shell.showToast(msg, kind),
);

let currentSnapshot: DesktopSnapshot | undefined;
let inspectorController!: InspectorController;

const discoveryLedger = new DiscoveryLedgerController(
  el.discoveryRunsList,
  el.refreshDiscoveryRunsButton,
  {
    getSnapshot: () => currentSnapshot,
    showToast: (msg, kind) => shell.showToast(msg, kind),
  },
);

const graphController = new GraphController(
  {
    graphContainer: el.graphContainer,
    graphLegend: el.graphLegend,
    focusBar: el.focusBar,
    focusRootLabel: el.focusRootLabel,
    focusSummary: el.focusSummary,
    nodeFilterGroup: el.nodeFilterGroup,
    renderModeToggle: el.renderModeToggle,
    emptyState: el.emptyState,
    emptyStateTitle: el.emptyStateTitle,
    emptyStateMessage: el.emptyStateMessage,
  },
  {
    onSelectNode: (nodeId) => inspectorController.selectNodeById(nodeId),
    onRequestCreateUnit: (parentId) => void inspectorController.enterCreateUnitMode(parentId),
    onRequestCreateRelation: (sourceId, targetId) =>
      void inspectorController.enterCreateRelationMode(sourceId, targetId),
    onBeforeRenderGraph: () => {
      if (reader.isInkOpen()) return;
      reader.close();
      el.readerHost.hidden = true;
    },
    onGraphRendered: () => artifactController.refreshParseState(),
    showToast: (msg, kind) => shell.showToast(msg, kind),
    getSnapshot: () => currentSnapshot,
    onOpenInkNote: (documentId) => eventBus.emit("open-ink-document", { documentId }),
  },
);

inspectorController = new InspectorController(el.nodeDetail, {
  getSnapshot: () => currentSnapshot,
  showToast: (msg, kind) => shell.showToast(msg, kind),
  copyIdentifier: (id) => shell.copyIdentifier(id),
  onFocusNode: (nodeId) => graphController.setFocus(nodeId, "all"),
  onOpenReader: (artifactId, label, inkFocus) => {
    el.readerHost.hidden = false;
    void reader.open(artifactId, label, { inkFocus });
  },
  onOpenReaderWithFocus: (artifactId, title, focus) => {
    el.readerHost.hidden = false;
    void reader.open(artifactId, title, { focus });
  },
  createParseForm: (pdf, marker) => artifactController.createParseForm(pdf, marker),
  refreshSnapshot: () => loadSnapshot(),
  canFocus: (node) => graphController.canFocus(node),
  isFocusRoot: (nodeId) => graphController.getFocusRootId() === nodeId,
});

artifactController = new ArtifactController(
  {
    markerState: el.markerState,
    markerVersion: el.markerVersion,
    markerProblem: el.markerProblem,
    lastImport: el.lastImport,
    lastParse: el.lastParse,
  },
  {
    showToast: (msg, kind) => shell.showToast(msg, kind),
    setBusy: (busy, msg) => shell.setBusy(busy, msg),
    onSnapshotChanged: (snapshot) => renderSnapshot(snapshot),
    getSnapshot: () => currentSnapshot,
    onParseStateChanged: () => inspectorController.refreshIfUnlocked(),
  },
);

function renderSnapshot(snapshot: DesktopSnapshot): void {
  currentSnapshot = snapshot;
  inspectorController.updateSelectedNodeFromSnapshot(snapshot);
  shell.renderWorkspaceInfo({
    artifactCount: snapshot.artifact_count,
    conversionCount: snapshot.conversion_count,
    workspaceId: snapshot.workspace_id,
    workspaceRoot: snapshot.workspace_root,
  });
  artifactController.renderMarkerStatus(snapshot.marker);
  graphController.renderCurrentGraph(snapshot);
  inspectorController.refreshIfUnlocked();
  loadInkOverlay();
}

// N2: map count markers and "我的手稿" come from a separate overlay query that
// is re-read after link writes and note lifecycle/title writes.
let inkViews: InkViewSync | undefined;
function loadInkOverlay(): void {
  void inkViews?.reloadOverlay();
}

async function loadSnapshot(): Promise<void> {
  shell.setBusy(true, "正在读取 Workspace");
  try {
    const snapshot = await invoke<DesktopSnapshot>("get_desktop_snapshot");
    renderSnapshot(snapshot);
    void discoveryLedger.loadDiscoveryRuns();
    shell.updateServiceState("本地内核已连接", true);
  } catch (error: unknown) {
    shell.updateServiceState("Workspace 初始化失败", false);
    shell.showToast(errorMessage(error), "error");
  } finally {
    shell.setBusy(false);
  }
}

el.importButton.addEventListener("click", () => void artifactController.importPdf());

eventBus.on("source-map-changed", () => void loadSnapshot());
function showInkWorkspace(): void {
  reader.close(); el.readerHost.hidden = true;
  inkOpen = true; el.workspaceGrid.hidden = true; el.inkHost.hidden = false;
  el.inkButton.textContent = "返回地图";
}
eventBus.on("open-ink-document", ({ documentId }) => {
  showInkWorkspace();
  void inkWorkspace.openDocumentId(documentId);
});

// N2 context creation (§6.5): one transaction creates the note and its first
// link; then the note opens on the desktop or with the tablet pairing panel.
setInkContextCreator(async (target, choice, draft) => {
  const receipt = await createInkNoteFromContext(draft, target, choice.role, choice.title);
  eventBus.emit("ink-links-changed");
  showInkWorkspace();
  await inkWorkspace.openDocumentId(receipt.document_id);
  if (choice.mode === "tablet") await inkWorkspace.pairTablet(receipt.document_id);
});
setInkSourceCandidateProvider(() => {
  const nodes = currentSnapshot?.landscape_graph.nodes ?? [];
  return nodes
    .filter((node) => node.domain_kind === "SourceSemanticUnit" && node.domain_id !== null)
    .filter((node) => !["superseded", "rejected", "deprecated"].includes(node.lifecycle ?? ""))
    .map((node) => ({ object_id: node.domain_id!, label: node.label, kind: node.domain_kind! }));
});
inkViews = connectInkViews({
  applyOverlay: (overlay) => graphController.setInkOverlay(overlay),
  refreshInspector: () => inspectorController.refreshIfUnlocked(),
});
eventBus.on("open-ink-link-target", ({ link }) => {
  const target = link.target;
  if (target.kind === "ink_note") {
    eventBus.emit("open-ink-document", { documentId: target.document_id });
    return;
  }
  inkWorkspace.close();
  const nodes = currentSnapshot?.landscape_graph.nodes ?? [];
  if (target.kind === "source_object") {
    const node = nodes.find((item) => item.domain_id === target.object_id && item.element_kind === "entity");
    if (node === undefined) { shell.showToast("关联的来源对象不在当前地图中。", "error"); return; }
    inspectorController.selectNodeById(node.id);
    return;
  }
  const artifact = nodes.find((item) => item.domain_kind === "SourceArtifact" && item.domain_id === target.artifact_id);
  el.readerHost.hidden = false;
  void reader.open(target.artifact_id, artifact?.label ?? link.target_label, {
    focus: { start: target.start, end: target.end, blockIds: target.block_ids },
  });
});
eventBus.on("open-source", (detail) => {
  void (async () => {
    if (detail.refreshMap) await loadSnapshot();
    el.readerHost.hidden = false;
    await reader.open(detail.artifactId, detail.title, {
      focus: detail.focus,
      pdfPageIndex: detail.pdfPageIndex,
      conversionId: detail.conversionId,
      patch: detail.patchRunId
        ? { runId: detail.patchRunId, mode: detail.patchMode ?? "workflow" }
        : undefined,
    });
  })();
});

artifactController.startPolling(1000);

void loadSnapshot();
void llmSettings.loadSummary();
