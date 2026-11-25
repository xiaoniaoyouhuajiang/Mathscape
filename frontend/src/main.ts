import { initializeLocale, LOCALE_CHANGED } from "./i18n";
import { initializeTheme } from "./shell/theme-manager";
import { initializeMapStyle } from "./shell/map-style";
import { invoke } from "@tauri-apps/api/core";
import { ArtifactController } from "./artifact/artifact-controller";
import { renderLeanStatus } from "./artifact/lean-status";
import type { DesktopSnapshot } from "./artifact/types";
import { eventBus } from "./core/events";
import { errorMessage } from "./core/ipc";
import { reducedMotion, watchMotion } from "./core/motion";
import { DiscoveryLedgerController } from "./discovery/discovery-ledger";
import { mountDiscoveryTask, reserveDiscoveryTaskSpace } from "./discovery-task";
import { GraphController } from "./graph/graph-controller";
import { SandboxEcho } from "./graph/sandbox-echo";
import { ScenePlayer } from "./graph/scene-player";
import { InspectorController } from "./inspector/inspector-controller";
import { LlmSettingsController } from "./llm/settings";
import { mountSandboxChrome } from "./sandbox/chrome";
import { SandboxDialogueDock } from "./sandbox/dialogue";
import { INK_NOTE_CITE_PREFIX, type CiteTarget } from "./sandbox/cite";
import { SandboxPanelController } from "./sandbox/panel";
import { DesktopInkWorkspace } from "./ink/desktop";
import { listInkDocuments } from "./ink/persistence";
import { connectInkViews, type InkViewSync } from "./ink/views";
import { listUnreviewedErrataUnits } from "./reader/markdown-errata";
import {
  closeInkPopover,
  createInkNoteFromContext,
  setInkContextCreator,
  setInkSourceCandidateProvider,
} from "./ink/links";
import { ArtifactReader, type ReaderOpenOptions } from "./reader/reader";
import { readingFocusOf, sourceNodeDetail } from "./reader/content";
import { findSourceArtifactNode, isCanonicalMarkdownId, readerTargetId, splitSourceItemsFrom } from "./reader/openable";
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
import { openProposalReview, reviewBeforeLeaving } from "./review/change-sets";
import "./styles.css";
import "./semantic/semantic.css";
import { StudyHudController } from "./semantic/study-hud";
import { TutorialPlayer } from "./tutorial/player";
import "./tutorial/tutorial.css";

// The first snapshot must be named in the saved language.
const localeReady = initializeLocale();
initializeTheme();
initializeMapStyle();

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
/** Sandbox Map Scene Player (created once the graph controller exists). */
let scenePlayer: ScenePlayer | undefined;
let inkSplit = false;
/** Last pane that received a pointer: used so Delete in the reader never erases ink. */
let inkPaneFocused = true;
const readerHome = el.readerHost.parentElement!;
let inkSplitHandle: InkSplitHandle | undefined;
/** Bumped on every exit from split/ink so in-flight left-pane loads cannot resurrect the overlay. */
let splitGeneration = 0;
let splitReaderRequest = 0;
let splitReaderError: string | undefined;

type ReaderScrollAnchor =
  | { kind: "block"; block: HTMLElement; offsetFromTop: number }
  | { kind: "ratio"; ratio: number };

let pendingReaderAnchor: ReaderScrollAnchor | null = null;
let readerAnchorToken = 0;

function readerScrollContainer(): HTMLElement {
  return el.readerHost.classList.contains("has-errata-pdf-pane")
    ? (el.readerHost.querySelector<HTMLElement>(".reader-main-pane") ?? el.readerHost)
    : el.readerHost;
}

function captureReaderScrollAnchor(): ReaderScrollAnchor | null {
  if (!reader.hasContent() || el.readerHost.hidden || el.readerHost.getClientRects().length === 0) {
    return null;
  }
  const scroller = readerScrollContainer();
  const hostRect = scroller.getBoundingClientRect();
  if (hostRect.height <= 0) return null;

  const isVisible = (block: HTMLElement): DOMRect | null => {
    if (block.classList.contains("is-errata-merged-hidden")) return null;
    const rect = block.getBoundingClientRect();
    if (rect.height <= 0) return null;
    if (rect.bottom <= hostRect.top + 8 || rect.top >= hostRect.bottom - 8) return null;
    return rect;
  };

  // Prefer the active/triggered block ONLY when it is actually inside the current viewport.
  // Stale off-screen selections or initial `.source-block-focus` anchors must never yank the reader away.
  const prioritySelectors = [
    ".source-block.is-ink-menu-target",
    ".source-block.is-selected-block",
    ".source-block.is-current-block",
    ".source-block.source-block-focus",
  ];
  for (const selector of prioritySelectors) {
    for (const candidate of el.readerHost.querySelectorAll<HTMLElement>(selector)) {
      const rect = isVisible(candidate);
      if (rect) {
        const offsetFromTop = Math.max(16, Math.min(hostRect.height - 64, rect.top - hostRect.top));
        return { kind: "block", block: candidate, offsetFromTop };
      }
    }
  }

  for (const block of el.readerHost.querySelectorAll<HTMLElement>(".source-block:not(.is-errata-merged-hidden)")) {
    const rect = block.getBoundingClientRect();
    if (rect.height > 0 && rect.bottom > hostRect.top + 32) {
      const offsetFromTop = Math.max(12, Math.min(hostRect.height - 64, rect.top - hostRect.top));
      return { kind: "block", block, offsetFromTop };
    }
  }
  const maxScroll = scroller.scrollHeight - scroller.clientHeight;
  if (maxScroll > 1) {
    return { kind: "ratio", ratio: scroller.scrollTop / maxScroll };
  }
  return null;
}

function restoreReaderScrollAnchor(anchor: ReaderScrollAnchor | null): void {
  if (!anchor) return;
  const token = ++readerAnchorToken;
  requestAnimationFrame(() => {
    if (token !== readerAnchorToken || !inkSplit || !inkOpen || el.readerHost.hidden) return;
    const scroller = readerScrollContainer();
    const hostRect = scroller.getBoundingClientRect();
    if (hostRect.height <= 0) return;
    if (anchor.kind === "block" && anchor.block.isConnected) {
      const blockRect = anchor.block.getBoundingClientRect();
      if (blockRect.height <= 0) return;
      const desiredOffset = Math.max(12, Math.min(hostRect.height - 64, anchor.offsetFromTop));
      const delta = (blockRect.top - hostRect.top) - desiredOffset;
      scroller.scrollTop = Math.max(0, scroller.scrollTop + delta);
    } else if (anchor.kind === "ratio") {
      const maxScroll = scroller.scrollHeight - scroller.clientHeight;
      if (maxScroll > 0) scroller.scrollTop = anchor.ratio * maxScroll;
    }
  });
}

function placeReaderInSplit(inSplit: boolean, preCapturedAnchor?: ReaderScrollAnchor | null): void {
  if (inSplit) {
    // Never move #reader into the overlay after an exit (in-flight ensure/open).
    if (!inkSplit || !inkOpen) return;
    const movingIntoSplit = el.readerHost.parentElement !== el.inkSplitReader;
    const anchor = preCapturedAnchor ?? pendingReaderAnchor ?? (movingIntoSplit ? captureReaderScrollAnchor() : null);
    pendingReaderAnchor = null;
    if (movingIntoSplit) {
      el.inkSplitReader.append(el.readerHost);
    }
    el.readerHost.classList.add("reader-panel");
    // Keep host visible only when it has a document; empty state covers the blank case.
    el.readerHost.hidden = !reader.hasContent() && !reader.isLoading();
    if (movingIntoSplit && anchor) restoreReaderScrollAnchor(anchor);
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
  return splitSourceItemsFrom(currentSnapshot?.pdf_artifacts ?? [], currentSnapshot?.landscape_graph.nodes ?? []);
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
      if (node?.domain_kind === "SourceArtifact") {
        artifactId = readerTargetId(node);
      } else {
        // Units/spans/relations are anchors, not files. Reuse the inspector's locator.
        const detail = await sourceNodeDetail(target.id);
        if (!stillSplit()) return;
        if (!detail.artifact_id) throw new Error("关联的来源对象没有可打开的原文");
        artifactId = detail.artifact_id;
        options = {
          focus: readingFocusOf(detail, target.title),
          inkFocus: detail.ink,
          preferCanonical: true,
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
      options: {
        focus: { start: target.start, end: target.end, blockIds: target.block_ids, label: link.target_label },
        preferCanonical: true,
      },
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

function setInkSplit(enabled: boolean, opts: { hint?: boolean; anchor?: ReaderScrollAnchor | null } = {}): void {
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
  placeReaderInSplit(enabled, opts.anchor);
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
inkWorkspace = new DesktopInkWorkspace(el.inkHost, closeInkWorkspace, (message, kind) => shell.showToast(message, kind), reserveDiscoveryTaskSpace);
inkSplitHandle = mountInkSplitResizer(el.inkWorkspace, el.inkSplitReader, el.inkSplitResizer);
el.inkHost.addEventListener("pointerdown", () => { inkPaneFocused = true; }, true);
el.inkSplitReader.addEventListener("pointerdown", () => { inkPaneFocused = false; }, true);
// Expose focus probe for the desktop Delete handler.
(window as unknown as { __mathscapeInkPaneFocused?: () => boolean }).__mathscapeInkPaneFocused = () => inkPaneFocused;

function openInkWorkspaceFromButton(): Promise<void> {
  scenePlayer?.cancel();
  // Preserve reader scroll position before hiding workspaceGrid, so clicking
  // 「并排阅读」 afterwards restores the exact block the user was reading.
  pendingReaderAnchor = captureReaderScrollAnchor();
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
// Phase2: app exit is a review trigger for staged proposals. Tauri allows one
// composed close guard, so the proposal review runs inside it after 手稿.
async function reviewProposalsOnExit(): Promise<boolean> {
  if (!await reviewBeforeLeaving(null, "exit", sandboxToast)) return false;
  const docked = sandboxDock.currentSessionId;
  if (docked && docked !== sandboxWindowSessionId) return reviewBeforeLeaving(docked, "exit", sandboxToast);
  return true;
}
installInkCloseGuard({
  confirm: async () => {
    if (!inkOpen) {
      if (await inkWorkspace.unsavedAfterFlush()) {
        await openInkWorkspaceFromButton(); // The prompt lives in the 手稿 workspace.
        if (!await inkWorkspace.confirmLeave("window")) return false;
      }
    } else if (!await inkWorkspace.confirmLeave("window")) {
      return false;
    }
    return reviewProposalsOnExit();
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

let currentSnapshot: DesktopSnapshot | undefined;
let inspectorController!: InspectorController;
let studyHud: StudyHudController | undefined;

mountDiscoveryTask();
const reviewWorkspace = mountReviewWorkspace(el.reviewWorkspaceHost, {
  getGraph: () => currentSnapshot?.landscape_graph,
});
const reader = new ArtifactReader(el.readerHost);
reader.setCanonicalResolver((artifactId) => {
  const node = findSourceArtifactNode(currentSnapshot?.landscape_graph.nodes ?? [], artifactId);
  return node?.domain_id && isCanonicalMarkdownId(node.domain_id) ? { id: node.domain_id, title: node.label } : undefined;
});
reader.setOverlayCloseHandler(() => {
  reader.close();
  el.readerHost.hidden = true;
});

const llmSettings = new LlmSettingsController(
  el.llmSettingsHost,
  el.llmSettingsButton,
  el.llmSettingsLabel,
  (msg, kind) => shell.showToast(msg, kind),
);

const sandboxToast = (msg: string, kind: "success" | "error") => shell.showToast(msg, kind);
const sandboxDock = new SandboxDialogueDock(el.sandboxHost, sandboxToast);
const sandboxPanel = new SandboxPanelController(el.sandboxHost, el.sandboxButton, sandboxToast, sandboxDock);
sandboxDock.setFallbackOpen(() => void sandboxPanel.open());
let sandboxWindowSessionId: string | null = null;
void mountSandboxChrome(el.shell, { toast: sandboxToast, openDialogue: (id) => void sandboxDock.open(id) })
  .then(status => {
    sandboxWindowSessionId = status?.sandbox?.sandboxSessionId ?? null;
    sandboxDock.bindWindowSession(sandboxWindowSessionId);
  });

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
    onSelectNode: (nodeId) => {
      inspectorController.selectNodeById(nodeId);
      if (nodeId !== null && reviewWorkspace.isMapLinkedOpen()) {
        const node = currentSnapshot?.landscape_graph.nodes.find((n) => n.id === nodeId);
        if (node?.domain_id) reviewWorkspace.focusDomainObject(node.domain_id);
      }
    },
    onRequestCreateUnit: (parentId) => void inspectorController.enterCreateUnitMode(parentId),
    onRequestCreateRelation: (sourceId, targetId) =>
      void inspectorController.enterCreateRelationMode(sourceId, targetId),
    onBeforeRenderGraph: () => {
      // Clump (临时收拢) is animation state: never let a sync see it.
      scenePlayer?.beforeGraphSync();
      if (reader.isInkOpen()) return;
      if (inkSplit && inkOpen) return;
      reader.close();
      el.readerHost.hidden = true;
    },
    onGraphRendered: () => {
      artifactController.refreshParseState();
      scenePlayer?.reapply();
      reviewWorkspace.reapplyMapScope();
      studyHud?.notifyGraphMounted();
    },
    onStudyPostureChanged: (active, initialMaterialId) => {
      if (active) {
        void studyHud?.activate(initialMaterialId);
      } else {
        studyHud?.deactivate();
      }
    },
    showToast: (msg, kind) => shell.showToast(msg, kind),
    getSnapshot: () => currentSnapshot,
    onOpenInkNote: (documentId) => eventBus.emit("open-ink-document", { documentId }),
  },
);

inspectorController = new InspectorController(el.nodeDetail, {
  getSnapshot: () => currentSnapshot,
  showToast: (msg, kind) => shell.showToast(msg, kind),
  copyIdentifier: (id) => shell.copyIdentifier(id),
  onFocusNode: (nodeId) => graphController.setFocus(nodeId),
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
    // Reading from a map node continues in the stitched 规范解析正文 when the
    // snapshot belongs to one; discovery and review open snapshots themselves.
    void reader.open(artifactId, title, { focus, preferCanonical: true });
  },
  createParseForm: (pdf, marker) => artifactController.createParseForm(pdf, marker),
  refreshSnapshot: () => loadSnapshot(),
  canFocus: (node) => graphController.canFocus(node),
  isFocusRoot: (nodeId) => graphController.getFocusRootId() === nodeId,
  getStudyStatus: (unitDomainId) => studyHud?.getUnitStudyStatus(unitDomainId) ?? null,
  onSetStudyStatus: async (unitDomainId, status) => {
    await studyHud?.setUnitStudyStatus(unitDomainId, status);
  },
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

function renderSnapshot(snapshot: DesktopSnapshot, options: { preserveReader?: boolean } = {}): void {
  currentSnapshot = snapshot;
  inspectorController.updateSelectedNodeFromSnapshot(snapshot);
  shell.renderWorkspaceInfo({
    artifactCount: snapshot.artifact_count,
    conversionCount: snapshot.conversion_count,
    workspaceId: snapshot.workspace_id,
    workspaceRoot: snapshot.workspace_root,
  });
  artifactController.renderMarkerStatus(snapshot.marker);
  renderLeanStatus(
    { state: el.leanState, summary: el.leanSummary, profiles: el.leanProfiles, problem: el.leanProblem },
    snapshot.lean,
  );
  // preserveReader skips onBeforeRenderGraph, which also settles scene state.
  if (options.preserveReader) scenePlayer?.beforeGraphSync();
  graphController.renderCurrentGraph(snapshot, options.preserveReader ? { preserveReader: true } : undefined);
  inspectorController.refreshIfUnlocked();
  loadInkOverlay();
  void refreshErrataReviewOverlay();
  void refreshInkCiteTargets();
  sandboxDock.refreshCites();
  void studyHud?.reloadData();
}

// N2: map count markers and "我的手稿" come from a separate overlay query that
// is re-read after link writes and note lifecycle/title writes.
let inkViews: InkViewSync | undefined;
function loadInkOverlay(): void {
  void inkViews?.reloadOverlay();
}

let errataOverlayRevision = 0;
async function refreshErrataReviewOverlay(): Promise<void> {
  const rev = ++errataOverlayRevision;
  const units = await listUnreviewedErrataUnits();
  if (rev !== errataOverlayRevision) return;
  graphController.setErrataReviewOverlay(units);
}

let activeInkCiteTargets: CiteTarget[] = [];
let inkCiteRevision = 0;
async function refreshInkCiteTargets(): Promise<void> {
  const rev = ++inkCiteRevision;
  try {
    const docs = await listInkDocuments();
    if (rev !== inkCiteRevision) return;
    activeInkCiteTargets = docs
      .filter((doc) => doc.lifecycle !== "trashed")
      .map((doc) => ({
        nodeId: `${INK_NOTE_CITE_PREFIX}${doc.document_id}`,
        domainId: doc.document_id,
        domainKind: "InkNote",
        label: doc.title,
      }));
    sandboxDock.refreshCites();
  } catch {
    /* Non-fatal in minimal test environments without ink IPC. */
  }
}

async function loadSnapshot(options: { preserveReader?: boolean } = {}): Promise<void> {
  shell.setBusy(true, "正在读取 Workspace");
  try {
    const snapshot = await invoke<DesktopSnapshot>("get_desktop_snapshot");
    renderSnapshot(snapshot, options);
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

eventBus.on("reader-opened", () => { pendingReaderAnchor = null; });
eventBus.on("source-map-changed", (detail) => void loadSnapshot({ preserveReader: Boolean(detail && detail.preserveReader) }));
function showInkWorkspace(opts: { enterSplitWithReader?: boolean; anchor?: ReaderScrollAnchor | null } = {}): void {
  const autoSplit = Boolean(opts.enterSplitWithReader && window.innerWidth >= INK_SPLIT_WINDOW_MIN);
  const anchor = opts.anchor ?? captureReaderScrollAnchor();
  if (!autoSplit && anchor) pendingReaderAnchor = anchor;
  // Keep the last reader document for 并排阅读; only hide the host when staying full-screen.
  if (!inkSplit && !autoSplit) el.readerHost.hidden = true;
  inkOpen = true;
  el.workspaceGrid.hidden = true;
  el.inkWorkspace.hidden = false;
  el.inkButton.textContent = "返回地图";
  if (autoSplit) {
    setInkSplit(true, { anchor });
  }
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
  const readerWasOpen = !inkOpen && !el.readerHost.hidden && reader.hasContent();
  const anchor = readerWasOpen ? captureReaderScrollAnchor() : null;
  closeInkPopover();
  showInkWorkspace({ enterSplitWithReader: readerWasOpen, anchor });
  inkPaneFocused = true;
  void inkWorkspace.openDocumentId(documentId);
});

// N2 context creation (§6.5): one transaction creates the note and its first
// link; when initiated from an open reader, enter left-source + right-ink split
// automatically (for both desktop and tablet writing modes).
setInkContextCreator(async (target, choice, draft) => {
  const readerWasOpen = !inkOpen && !el.readerHost.hidden && reader.hasContent();
  const anchor = readerWasOpen ? captureReaderScrollAnchor() : null;
  const receipt = await createInkNoteFromContext(draft, target, choice.role, choice.title);
  closeInkPopover();
  eventBus.emit("ink-links-changed");
  eventBus.emit("ink-notes-changed");
  showInkWorkspace({ enterSplitWithReader: readerWasOpen, anchor });
  inkPaneFocused = true;
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
eventBus.on("ink-notes-changed", () => void refreshInkCiteTargets());
eventBus.on("ink-links-changed", () => void refreshInkCiteTargets());
eventBus.on("markdown-errata-changed", () => {
  void refreshErrataReviewOverlay();
  inspectorController.refreshIfUnlocked();
});
eventBus.on("errata-reviews-changed", () => {
  void refreshErrataReviewOverlay();
  inspectorController.refreshIfUnlocked();
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
    const artifact = findSourceArtifactNode(nodes, target.artifact_id);
    el.readerHost.hidden = false;
    void reader.open(target.artifact_id, artifact?.label ?? link.target_label, {
      focus: { start: target.start, end: target.end, blockIds: target.block_ids, label: link.target_label },
      preferCanonical: true,
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
      candidatePages: detail.candidatePages,
      backlinkNote: detail.backlinkNote,
      focusExcerpt: detail.focusExcerpt,
      returnTo: detail.returnTo,
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

/** Leave 手稿 / the reader so the map is showing. */
async function showMap(current: () => boolean): Promise<boolean> {
  const fromInk = inkOpen;
  if (inkOpen && !await inkWorkspace.close()) return false;
  if (reader.hasContent() || reader.isLoading() || !el.readerHost.hidden) { reader.close(); el.readerHost.hidden = true; }
  if (fromInk) { await nextFrame(); await nextFrame(); }
  return current();
}

/** Select + center a projection node; the inspector shows it either way. */
function selectMapNode(nodeId: string): void {
  inspectorController.selectNodeById(nodeId);
  if (graphController.revealNode(nodeId) === "hidden") shell.showToast("该节点被当前筛选隐藏，详情已显示在右侧检查器。", "success");
}

/** Select + center a map node by domain id. Leaves 手稿 first if needed. */
async function locateOnMap(domainId: string, current: () => boolean): Promise<boolean> {
  if (!await showMap(current)) return false;
  const nodes = currentSnapshot?.landscape_graph.nodes ?? [];
  // A head markdown snapshot is folded into its canonical node on the map.
  const node = nodes.find(item => item.domain_id === domainId && item.element_kind === "entity")
    ?? findSourceArtifactNode(nodes.filter(item => item.element_kind === "entity"), domainId);
  if (!node) { shell.showToast("该结果不在当前地图中，请刷新后重试。", "error"); return false; }
  selectMapNode(node.id);
  return true;
}

eventBus.on("locate-domain-object", ({ objectId }) => {
  const token = ++searchNavigation;
  void locateOnMap(objectId, () => token === searchNavigation);
});

// A-PR3: sandbox dialogue cite chips jump to the map. Map Scene Player:
// live tool events (and replays from process cards) play as map scenes.
// The dock stays open (sidebar beside the map).
const sandboxEcho = new SandboxEcho(() => graphController.getGraphCore());
const player = new ScenePlayer({
  core: () => graphController.getGraphCore(),
  hudHost: el.graphContainer.parentElement ?? el.graphContainer,
  canvas: el.graphContainer,
  // Like the A-PR3 echo: nothing plays behind the open ink pad / 手稿 layer.
  canPlay: () => !inkOpen,
  restoreFocusScope: () => graphController.reapplyFocusScope(),
  reducedMotion: () => reducedMotion(),
});
scenePlayer = player;
watchMotion();
const noNodes: never[] = [];
sandboxDock.setCiteContext({
  nodes: () => currentSnapshot?.landscape_graph.nodes ?? noNodes,
  extraTargets: () => activeInkCiteTargets,
  jump: (nodeId) => {
    if (nodeId.startsWith(INK_NOTE_CITE_PREFIX)) {
      const documentId = nodeId.slice(INK_NOTE_CITE_PREFIX.length);
      player.cancel(false);
      eventBus.emit("open-ink-document", { documentId });
      return;
    }
    const token = ++searchNavigation;
    player.cancel(false);
    void (async () => {
      if (!await showMap(() => token === searchNavigation)) return;
      if (!currentSnapshot?.landscape_graph.nodes.some(node => node.id === nodeId)) {
        shell.showToast("该对象不在当前地图中，请刷新后重试。", "error");
        return;
      }
      selectMapNode(nodeId);
      sandboxEcho.pulse("hit", [nodeId]);
    })();
  },
  // Sync ensure 「解析中」: fresh summary of one node; re-render only on change.
  // A summary is not a reason to close what is being read (its gap cards wait for these pages).
  probeSummary: async (nodeId) => {
    const snapshot = await invoke<DesktopSnapshot>("get_desktop_snapshot");
    const summaryOf = (from: DesktopSnapshot | undefined) =>
      from?.landscape_graph.nodes.find(node => node.id === nodeId)?.summary ?? null;
    const summary = summaryOf(snapshot);
    if (summary !== summaryOf(currentSnapshot)) renderSnapshot(snapshot, { preserveReader: true });
    return summary;
  },
  scenes: {
    live: (scene, part) => player.live(scene, part),
    idle: () => player.clearRunning(),
    replay: (scenes) => {
      if (inkOpen) {
        shell.showToast("手稿打开时不能在地图上回放，请先返回地图。", "error");
        return;
      }
      const token = ++searchNavigation;
      void (async () => {
        if (!await showMap(() => token === searchNavigation)) return;
        player.replay(scenes);
      })();
    },
  },
});

/** Open a source result in the map-page reader (non-split). Resolves
 * source objects to their material + focus first (as in PR #7). */
async function openMapReader(target: SplitReaderTarget, current: () => boolean): Promise<void> {
  try {
    let artifactId = target.id;
    let options: ReaderOpenOptions = target.kind === "artifact" ? target.options ?? {} : {};
    if (target.kind === "source_object") {
      const node = currentSnapshot?.landscape_graph.nodes.find(item => item.domain_id === target.id);
      if (node?.domain_kind === "SourceArtifact") {
        artifactId = readerTargetId(node);
      } else {
        const detail = await sourceNodeDetail(target.id);
        if (!current() || inkOpen) return;
        if (!detail.artifact_id) throw new Error("该来源对象没有可打开的原文");
        artifactId = detail.artifact_id;
        options = {
          focus: readingFocusOf(detail, target.title),
          inkFocus: detail.ink,
          preferCanonical: true,
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
    ? { kind: "artifact", id: domainId, title: hit.title, options: { preferCanonical: true } }
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
// Shared-object names come from the backend; re-read them in the new language.
// Re-rendering the snapshot also refreshes the inspector and learning HUD.
window.addEventListener(LOCALE_CHANGED, () => {
  void loadSnapshot({ preserveReader: true });
  workspaceSearch.refresh();
});
el.searchButton.addEventListener("click", () => workspaceSearch.open());

// 语义层研读透镜（地图内双轨线性书脊 + 右轨共享语义伴星 + 节点悬浮简报卡）。
studyHud = new StudyHudController(el.canvasPanel, {
  materialName: (artifactId) => {
    const nodes = currentSnapshot?.landscape_graph.nodes ?? [];
    const node =
      nodes.find((item) => item.domain_id === artifactId && item.element_kind === "entity") ??
      findSourceArtifactNode(
        nodes.filter((item) => item.element_kind === "entity"),
        artifactId,
      );
    return node?.label ?? artifactId.replace(/^mathscape:artifact\/sha256\//, "").slice(0, 12);
  },
  hasStagedProposals: () =>
    Boolean(
      currentSnapshot?.investigation &&
        (Object.keys(currentSnapshot.investigation.staged).length > 0 ||
          currentSnapshot.investigation.conflicts.length > 0),
    ),
  getDisplayedGraph: () => graphController.getDisplayedGraph(),
  getGraphCore: () => graphController.getGraphCore(),
  onSelectMaterial: (materialId) => {
    graphController.applyStudyData(
      studyHud!.getLearningViewData(),
      materialId,
      studyHud!.getSignalMode(),
    );
  },
  onSelectSignal: (signal) => {
    graphController.applyStudyData(
      studyHud!.getLearningViewData(),
      studyHud!.getSelectedMaterialId(),
      signal,
    );
  },
  onDataUpdated: (view) => {
    graphController.applyStudyData(
      view,
      studyHud!.getSelectedMaterialId(),
      studyHud!.getSignalMode(),
    );
  },
  onFocusDomainObject: (domainId) => {
    graphController.centerDomainObject(domainId);
    const node = currentSnapshot?.landscape_graph.nodes.find((n) => n.domain_id === domainId);
    if (node) inspectorController.selectNodeById(node.id);
  },
  onSelectProjectionNode: (nodeId) => {
    selectMapNode(nodeId);
  },
  onExitStudy: () => graphController.exitStudyPosture(),
  showToast: (msg, kind) => shell.showToast(msg, kind),
});

const toggleStudyFromTopbar = () => {
  void (async () => {
    const wasActive = graphController.isStudyPostureActive();
    if (!(await showMap(() => true))) return;
    if (wasActive) graphController.exitStudyPosture();
    else graphController.enterStudyPosture();
  })();
};
eventBus.on("open-learning-view", toggleStudyFromTopbar);

eventBus.on("open-proposal-for-object", (detail) => {
  void openProposalReview(sandboxWindowSessionId, sandboxToast, {
    changeSetId: detail.changeSetId,
    objectId: detail.objectId,
    mapLinked: detail.mapLinked ?? true,
  });
});

eventBus.on("review-map-scope", (detail) => {
  if (!detail.active) {
    graphController.setCustomFocusScope(null, null);
    return;
  }
  graphController.setCustomFocusScope(new Set(detail.nodeIds), new Set(detail.relationIds));
  if (detail.focusDomainId) {
    graphController.centerDomainObject(detail.focusDomainId);
  }
});

// 帮助菜单 → 只读教程「两本书讲的是不是同一个东西」（不写工作区）。
const tutorial = new TutorialPlayer(document.body);
const setHelpMenu = (open: boolean) => {
  el.helpMenu.hidden = !open;
  el.helpButton.setAttribute("aria-expanded", String(open));
};
el.helpButton.addEventListener("click", (event) => {
  event.stopPropagation();
  setHelpMenu(el.helpMenu.hidden !== false);
});
document.addEventListener("click", (event) => {
  if (!el.helpMenu.hidden && !el.helpMenu.contains(event.target as Node)) setHelpMenu(false);
});
el.tutorialMenuItem.addEventListener("click", () => {
  setHelpMenu(false);
  void tutorial.open("ep1");
});
el.tutorialEp2MenuItem.addEventListener("click", () => {
  setHelpMenu(false);
  void tutorial.open("ep2");
});
eventBus.on("open-tutorial", (payload) => void tutorial.open(payload?.episode ?? "ep1"));

artifactController.startPolling(1000);

void localeReady.then(() => loadSnapshot());
void llmSettings.loadSummary();
