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
import { ArtifactReader, type ReaderOpenOptions } from "./reader/reader";
import { sourceNodeDetail } from "./reader/content";
import { mountReviewWorkspace } from "./review/workspace";
import { AppShell } from "./shell/app-shell";
import {
  INK_SPLIT_WINDOW_MIN,
  mountInkSplitResizer,
  type InkSplitHandle,
} from "./shell/ink-split";
import {
  ensureSplitEmptyHost,
  hideSplitEmpty,
  showSplitEmpty,
  type SplitSourceItem,
} from "./shell/ink-split-empty";
import { listInkNoteLinks, type InkNoteLink } from "./ink/links";
import { searchWorkspace, type SearchHit } from "./search/api";
import { WorkspaceSearchPanel, installWorkspaceSearch, type SearchAction } from "./search/search-panel";
import { installInkCloseGuard } from "./ink/close-guard";
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
let inkSplit = false;
/** Last pane that received a pointer: used so Delete in the reader never erases ink. */
let inkPaneFocused = true;
const readerHome = el.readerHost.parentElement!;
let inkSplitHandle: InkSplitHandle | undefined;
/** Bumped on every exit from split/ink so in-flight left-pane loads cannot resurrect the overlay. */
let splitGeneration = 0;
let splitReaderRequest = 0;
let splitReaderError: string | undefined;

function placeReaderInSplit(inSplit: boolean): void {
  if (inSplit) {
    // Never move #reader into the overlay after an exit (in-flight ensure/open).
    if (!inkSplit || !inkOpen) return;
    if (el.readerHost.parentElement !== el.inkSplitReader) {
      el.inkSplitReader.append(el.readerHost);
    }
    el.readerHost.classList.add("reader-panel");
    // Keep host visible only when it has a document; empty state covers the blank case.
    el.readerHost.hidden = !reader.hasContent() && !reader.isLoading();
  } else {
    restoreReaderHome({ hide: false });
  }
}

/** Put #reader back under the map canvas and clear the split empty state. */
function restoreReaderHome(opts: { hide: boolean }): void {
  if (el.readerHost.parentElement !== readerHome) {
    readerHome.append(el.readerHost);
  }
  hideSplitEmpty(el.inkSplitReader.querySelector("#ink-split-empty"));
  if (opts.hide) el.readerHost.hidden = true;
}

function splitSourceItems(): SplitSourceItem[] {
  const items: SplitSourceItem[] = [];
  const seen = new Set<string>();
  for (const pdf of currentSnapshot?.pdf_artifacts ?? []) {
    if (seen.has(pdf.artifact_id)) continue;
    seen.add(pdf.artifact_id);
    items.push({ artifactId: pdf.artifact_id, label: pdf.filename || pdf.artifact_id });
  }
  for (const node of currentSnapshot?.landscape_graph.nodes ?? []) {
    if (node.domain_kind !== "SourceArtifact" || !node.domain_id || seen.has(node.domain_id)) continue;
    seen.add(node.domain_id);
    items.push({ artifactId: node.domain_id, label: node.label || node.domain_id });
  }
  return items;
}

function primaryIsOpenable(link: InkNoteLink | undefined): boolean {
  if (!link) return false;
  // 并排左侧只用于原文；不并排打开另一份手稿。
  if (link.target.kind === "ink_note") return false;
  return link.target.kind === "markdown" || link.target.kind === "source_object";
}

type SplitReaderTarget =
  | { kind: "artifact"; id: string; title: string; options?: ReaderOpenOptions }
  | { kind: "source_object"; id: string; title: string };

async function splitPrimaryLink(): Promise<InkNoteLink | undefined> {
  const primary = inkWorkspace.primaryLink();
  if (primary) return primary;
  const documentId = inkWorkspace.currentDocumentId();
  if (!documentId) return;
  try {
    const links = await listInkNoteLinks(documentId);
    if (inkWorkspace.currentDocumentId() === documentId) {
      return links.find(link => link.is_primary && link.state === "active");
    }
  } catch { /* The source picker remains available. */ }
}

async function openSplitReader(target: SplitReaderTarget): Promise<void> {
  if (!inkSplit || !inkOpen) return;
  const generation = splitGeneration;
  const request = ++splitReaderRequest;
  const stillSplit = () => generation === splitGeneration && request === splitReaderRequest && inkSplit && inkOpen;
  splitReaderError = undefined;
  placeReaderInSplit(true);
  hideSplitEmpty(ensureSplitEmptyHost(el.inkSplitReader));
  el.readerHost.hidden = false;
  inkPaneFocused = false;
  try {
    let artifactId = target.id;
    let options: ReaderOpenOptions = target.kind === "artifact" ? target.options ?? {} : {};
    if (target.kind === "source_object") {
      const node = currentSnapshot?.landscape_graph.nodes.find(item => item.domain_id === target.id);
      if (node?.domain_kind !== "SourceArtifact") {
        // Units/spans/relations are anchors, not files. Reuse the inspector's locator.
        const detail = await sourceNodeDetail(target.id);
        if (!stillSplit()) return;
        if (!detail.artifact_id) throw new Error("关联的来源对象没有可打开的原文");
        artifactId = detail.artifact_id;
        options = {
          focus: detail.byte_range ? { ...detail.byte_range, blockIds: detail.markdown_block_ids } : undefined,
          inkFocus: detail.ink,
        };
      }
    }
    if (!stillSplit()) return;
    const opened = await reader.open(artifactId, target.title, options);
    if (!stillSplit()) return;
    if (!opened) {
      if (reader.isLoading() || reader.hasContent()) return; // A different reader action took over.
      throw new Error(reader.lastError() ?? "原文未能打开");
    }
    window.dispatchEvent(new Event("resize"));
  } catch (error: unknown) {
    if (!stillSplit()) return;
    splitReaderError = errorMessage(error);
    reader.close();
    const primary = await splitPrimaryLink();
    if (stillSplit()) renderSplitEmpty(primary);
  }
}

function openPrimaryInSplit(link: InkNoteLink): void {
  const target = link.target;
  if (target.kind === "ink_note") {
    shell.showToast("并排阅读左侧用于原文，不能并排打开另一份手稿。", "error");
    return;
  }
  if (target.kind === "markdown") {
    void openSplitReader({
      kind: "artifact", id: target.artifact_id, title: link.target_label,
      options: { focus: { start: target.start, end: target.end, blockIds: target.block_ids } },
    });
    return;
  }
  void openSplitReader({ kind: "source_object", id: target.object_id, title: link.target_label });
}

function renderSplitEmpty(primary: InkNoteLink | undefined): void {
  if (!inkSplit || !inkOpen) return;
  const host = ensureSplitEmptyHost(el.inkSplitReader);
  el.readerHost.hidden = true;
  showSplitEmpty(host, {
    primaryLabel: primary?.target_label,
    error: splitReaderError,
    primaryOpenable: primaryIsOpenable(primary),
    sources: splitSourceItems(),
    onOpenPrimary: () => { if (primary) openPrimaryInSplit(primary); },
    onOpenSource: (item) => void openSplitReader({ kind: "artifact", id: item.artifactId, title: item.label }),
    onImportPdf: () => void artifactController.importPdf(),
    onBackToMap: () => void inkWorkspace.close(),
  });
  inkPaneFocused = false;
}

/** Fill the left pane: keep / restore reader, auto-open primary markdown, or show a picker. */
async function ensureSplitLeftPane(): Promise<void> {
  if (!inkSplit || !inkOpen) return;
  const generation = splitGeneration;
  const request = splitReaderRequest;
  const stillSplit = () => generation === splitGeneration && request === splitReaderRequest && inkSplit && inkOpen;
  placeReaderInSplit(true);
  if (!stillSplit()) return;
  if (reader.hasContent()) {
    splitReaderError = undefined;
    hideSplitEmpty(ensureSplitEmptyHost(el.inkSplitReader));
    el.readerHost.hidden = false;
    window.dispatchEvent(new Event("resize"));
    return;
  }
  if (reader.isLoading()) { el.readerHost.hidden = false; return; }
  // A failure must return to a recoverable picker, not retry a remembered target in a loop.
  if (splitReaderError) {
    const primary = await splitPrimaryLink();
    if (stillSplit()) renderSplitEmpty(primary);
    return;
  }
  // Restore the last reader document if we only hid it when opening 手稿.
  const last = reader.lastOpened();
  if (last) {
    if (!stillSplit()) return;
    void openSplitReader({ kind: "artifact", id: last.id, title: last.title, options: last.options });
    return;
  }
  const primary = await splitPrimaryLink();
  if (!stillSplit()) return;
  if (primary?.target.kind === "markdown") {
    openPrimaryInSplit(primary);
    return;
  }
  renderSplitEmpty(primary);
}

function setInkSplit(enabled: boolean, opts: { hint?: boolean } = {}): void {
  if (enabled && window.innerWidth < INK_SPLIT_WINDOW_MIN) {
    shell.showToast(`窗口窄于 ${INK_SPLIT_WINDOW_MIN}px，暂无法并排阅读。`, "error");
    enabled = false;
  }
  if (enabled === inkSplit) {
    if (enabled) {
      inkSplitHandle?.setEnabled(true);
      void ensureSplitLeftPane();
    }
    return;
  }
  if (!enabled) splitGeneration += 1;
  inkSplit = enabled;
  el.inkWorkspace.classList.toggle("is-split", enabled);
  el.inkSplitReader.hidden = !enabled;
  el.inkSplitResizer.hidden = !enabled;
  placeReaderInSplit(enabled);
  inkSplitHandle?.setEnabled(enabled);
  // Per-layout notes-list preference, and fit the page once on entering 并排.
  inkWorkspace.setSplitMode(enabled);
  const toggle = el.inkHost.querySelector<HTMLButtonElement>("#ink-split-toggle");
  if (toggle) {
    toggle.setAttribute("aria-pressed", String(enabled));
    toggle.textContent = enabled ? "退出并排" : "并排阅读";
  }
  if (enabled) void ensureSplitLeftPane();
  else {
    restoreReaderHome({ hide: true });
    hideSplitEmpty(el.inkSplitReader.querySelector("#ink-split-empty"));
  }
  window.dispatchEvent(new Event("resize"));
  if (!enabled && opts.hint) {
    shell.showToast("窗口过窄，已退出并排阅读。", "error");
  }
}

const closeInkWorkspace = () => {
  // Drop split first (bumps splitGeneration via setInkSplit) so in-flight left-pane
  // loads cannot move #reader back into the overlay or unhide it over the map.
  setInkSplit(false);
  inkOpen = false;
  el.inkWorkspace.hidden = true;
  el.workspaceGrid.hidden = false;
  el.inkButton.textContent = "手写";
  restoreReaderHome({ hide: true });
  // Graph lived under display:none while 手写 was open — reflow after layout.
  requestAnimationFrame(() => {
    graphController.resize();
    window.dispatchEvent(new Event("resize"));
  });
  loadInkOverlay();
};
inkWorkspace = new DesktopInkWorkspace(el.inkHost, closeInkWorkspace, (message, kind) => shell.showToast(message, kind));
inkSplitHandle = mountInkSplitResizer(el.inkWorkspace, el.inkSplitReader, el.inkSplitResizer);
el.inkHost.addEventListener("pointerdown", () => { inkPaneFocused = true; }, true);
el.inkSplitReader.addEventListener("pointerdown", () => { inkPaneFocused = false; }, true);
// Expose focus probe for the desktop Delete handler.
(window as unknown as { __mathscapeInkPaneFocused?: () => boolean }).__mathscapeInkPaneFocused = () => inkPaneFocused;

function openInkWorkspaceFromButton(): Promise<void> {
  inkOpen = true;
  el.workspaceGrid.hidden = true;
  el.inkWorkspace.hidden = false;
  el.inkButton.textContent = "返回地图";
  return inkWorkspace.open();
}
el.inkButton.addEventListener("click", () => {
  // H5: closing may ask first (unsaved operations); it closes itself when allowed.
  if (inkOpen) void inkWorkspace.close();
  else void openInkWorkspaceFromButton();
});
// H5: the Tauri window close (and a browser unload) waits for the 手稿 save
// and asks before discarding operations that are still pending.
installInkCloseGuard({
  confirm: async () => {
    if (!inkOpen) {
      if (!await inkWorkspace.unsavedAfterFlush()) return true;
      await openInkWorkspaceFromButton(); // The prompt lives in the 手稿 workspace.
    }
    return inkWorkspace.confirmLeave("window");
  },
  hasUnsavedHint: () => inkWorkspace.hasUnsavedHint(),
});
window.addEventListener("resize", () => {
  if (inkSplit && window.innerWidth < INK_SPLIT_WINDOW_MIN) setInkSplit(false, { hint: true });
});
eventBus.on("ink-split-toggle", (detail) => {
  if (!inkOpen) return;
  const next = detail && typeof detail === "object" && detail !== null && "enabled" in detail && detail.enabled !== undefined
    ? !!detail.enabled
    : !inkSplit;
  setInkSplit(next);
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
      if (inkSplit && inkOpen) return;
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
    if (inkSplit && inkOpen) {
      placeReaderInSplit(true);
      hideSplitEmpty(ensureSplitEmptyHost(el.inkSplitReader));
    } else if (el.readerHost.parentElement !== readerHome) {
      restoreReaderHome({ hide: false });
    }
    el.readerHost.hidden = false;
    void reader.open(artifactId, label, { inkFocus });
  },
  onOpenReaderWithFocus: (artifactId, title, focus) => {
    if (inkSplit && inkOpen) {
      placeReaderInSplit(true);
      hideSplitEmpty(ensureSplitEmptyHost(el.inkSplitReader));
    } else if (el.readerHost.parentElement !== readerHome) {
      restoreReaderHome({ hide: false });
    }
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
  // Keep the last reader document for a later 并排阅读; only hide the host.
  if (!inkSplit) el.readerHost.hidden = true;
  inkOpen = true; el.workspaceGrid.hidden = true; el.inkWorkspace.hidden = false;
  el.inkButton.textContent = "返回地图";
}
eventBus.on("open-ink-document", ({ documentId }) => {
  if (inkSplit && inkOpen) {
    inkPaneFocused = true;
    void inkWorkspace.openDocumentId(documentId).then(() => {
      // Left pane keeps its document; if it was empty, refresh primary / picker for the new note.
      if (!reader.hasContent()) void ensureSplitLeftPane();
    });
    return;
  }
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
    if (inkSplit && inkOpen) {
      // Right pane only — never two notes side by side.
      void inkWorkspace.openDocumentId(target.document_id).then(() => {
        if (!reader.hasContent()) void ensureSplitLeftPane();
      });
      inkPaneFocused = true;
      return;
    }
    eventBus.emit("open-ink-document", { documentId: target.document_id });
    return;
  }
  if (inkSplit && inkOpen) {
    openPrimaryInSplit(link);
    return;
  }
  const nodes = currentSnapshot?.landscape_graph.nodes ?? [];
  void (async () => {
    if (!await inkWorkspace.close()) return;
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
  })();
});
eventBus.on("open-source", (detail) => {
  void (async () => {
    if (detail.refreshMap) await loadSnapshot();
    if (inkSplit && inkOpen) {
      placeReaderInSplit(true);
      hideSplitEmpty(ensureSplitEmptyHost(el.inkSplitReader));
    } else if (el.readerHost.parentElement !== readerHome) {
      restoreReaderHome({ hide: false });
    }
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

// N3d workspace search. The panel renders; navigation lives here because it
// reuses the reader / split / map plumbing above.
let searchNavigation = 0;
const nextFrame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));

/** Select + center a map node by domain id. Leaves 手稿 first if needed. */
async function locateOnMap(domainId: string, current: () => boolean): Promise<boolean> {
  const fromInk = inkOpen;
  if (inkOpen && !await inkWorkspace.close()) return false;
  if (reader.hasContent() || reader.isLoading() || !el.readerHost.hidden) { reader.close(); el.readerHost.hidden = true; }
  if (fromInk) { await nextFrame(); await nextFrame(); }
  if (!current()) return false;
  const node = currentSnapshot?.landscape_graph.nodes.find(item => item.domain_id === domainId && item.element_kind === "entity");
  if (!node) { shell.showToast("该结果不在当前地图中，请刷新后重试。", "error"); return false; }
  inspectorController.selectNodeById(node.id);
  if (graphController.revealNode(node.id) === "hidden") shell.showToast("该节点被当前筛选隐藏，详情已显示在右侧检查器。", "success");
  return true;
}

/** Open a source result in the map-page reader (non-split). Resolves
 * source objects to their material + focus first (as in PR #7). */
async function openMapReader(target: SplitReaderTarget, current: () => boolean): Promise<void> {
  try {
    let artifactId = target.id;
    let options: ReaderOpenOptions = target.kind === "artifact" ? target.options ?? {} : {};
    if (target.kind === "source_object") {
      const node = currentSnapshot?.landscape_graph.nodes.find(item => item.domain_id === target.id);
      if (node?.domain_kind !== "SourceArtifact") {
        const detail = await sourceNodeDetail(target.id);
        if (!current() || inkOpen) return;
        if (!detail.artifact_id) throw new Error("该来源对象没有可打开的原文");
        artifactId = detail.artifact_id;
        options = {
          focus: detail.byte_range ? { ...detail.byte_range, blockIds: detail.markdown_block_ids } : undefined,
          inkFocus: detail.ink,
        };
      }
    }
    if (!current() || inkOpen) return;
    restoreReaderHome({ hide: false });
    el.readerHost.hidden = false;
    const opened = await reader.open(artifactId, target.title, options);
    if (!current() || opened) return;
    if (reader.isLoading() || reader.hasContent()) return; // Another reader action took over.
    throw new Error(reader.lastError() ?? "原文未能打开");
  } catch (error: unknown) {
    if (!current()) return;
    shell.showToast("无法打开原文：" + errorMessage(error), "error");
  }
}

async function openSearchResult(hit: SearchHit, action: SearchAction): Promise<void> {
  const request = ++searchNavigation;
  const current = () => request === searchNavigation;
  const target = hit.target;
  if (target.kind === "ink_note") {
    eventBus.emit("open-ink-document", { documentId: target.document_id });
    return;
  }
  const split = inkSplit && inkOpen;
  if (target.kind === "ink_snapshot") {
    const readerTarget: SplitReaderTarget = { kind: "artifact", id: target.artifact_id, title: hit.title };
    if (split) { await openSplitReader(readerTarget); return; }
    if (inkOpen && !await inkWorkspace.close()) return;
    await openMapReader(readerTarget, current);
    return;
  }
  const domainId = target.kind === "artifact" ? target.artifact_id : target.kind === "source_object" ? target.object_id : target.node_id;
  const readerTarget: SplitReaderTarget | undefined = target.kind === "artifact"
    ? { kind: "artifact", id: domainId, title: hit.title }
    : target.kind === "source_object" ? { kind: "source_object", id: domainId, title: hit.title } : undefined;
  // 并排阅读: source results open in the left pane, the note stays.
  if (action === "open" && readerTarget && split) { await openSplitReader(readerTarget); return; }
  const located = await locateOnMap(domainId, current);
  if (!located || action === "locate" || !readerTarget || !current()) return;
  await openMapReader(readerTarget, current);
}

const workspaceSearch = new WorkspaceSearchPanel({
  search: (query) => searchWorkspace(query),
  activate: (hit, action) => openSearchResult(hit, action),
});
installWorkspaceSearch(workspaceSearch);
el.searchButton.addEventListener("click", () => workspaceSearch.open());

artifactController.startPolling(1000);

void loadSnapshot();
void llmSettings.loadSummary();
