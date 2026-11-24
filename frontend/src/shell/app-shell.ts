import { mountDetailResizer } from "./panel-resizer";
import { requiredElement } from "../core/dom";
import { mountThemeToggle } from "./theme-manager";

export interface AppElements {
  shell: HTMLElement;
  importButton: HTMLButtonElement;
  sourceToggle: HTMLButtonElement;
  workspaceGrid: HTMLElement;
  serviceState: HTMLElement;
  artifactCount: HTMLElement;
  conversionCount: HTMLElement;
  workspaceId: HTMLElement;
  workspaceName: HTMLElement;
  workspaceRoot: HTMLElement;
  copyWorkspaceIdButton: HTMLButtonElement;
  canvasPanel: HTMLElement;
  graphContainer: HTMLElement;
  graphLegend: HTMLElement;
  focusBar: HTMLElement;
  focusRootLabel: HTMLElement;
  focusSummary: HTMLElement;
  graphViewTitle: HTMLElement;
  nodeFilterGroup: HTMLElement;
  renderModeToggle: HTMLInputElement;
  emptyState: HTMLElement;
  emptyStateTitle: HTMLElement;
  emptyStateMessage: HTMLElement;
  nodeDetail: HTMLElement;
  lastImport: HTMLElement;
  lastParse: HTMLElement;
  discoveryRunsList: HTMLElement;
  refreshDiscoveryRunsButton: HTMLButtonElement;
  markerState: HTMLElement;
  markerVersion: HTMLElement;
  markerProblem: HTMLElement;
  leanState: HTMLElement;
  leanSummary: HTMLElement;
  leanProfiles: HTMLElement;
  leanProblem: HTMLElement;
  toast: HTMLElement;
  readerHost: HTMLElement;
  reviewWorkspaceHost: HTMLElement;
  llmSettingsHost: HTMLElement;
  llmSettingsButton: HTMLButtonElement;
  llmSettingsLabel: HTMLElement;
  sandboxHost: HTMLElement;
  sandboxButton: HTMLButtonElement;
  inkButton: HTMLButtonElement;
  searchButton: HTMLButtonElement;
  learningViewButton: HTMLButtonElement;
  helpButton: HTMLButtonElement;
  helpMenu: HTMLElement;
  tutorialMenuItem: HTMLButtonElement;
  /** Outer overlay that may host the N3c split layout. */
  inkWorkspace: HTMLElement;
  inkSplitReader: HTMLElement;
  inkSplitResizer: HTMLElement;
  /** DesktopInkWorkspace mounts here (right pane in split mode). */
  inkHost: HTMLElement;
}

export class AppShell {
  readonly elements: AppElements;
  private toastTimer: number | undefined;
  private isParseActive: () => boolean;

  constructor(
    root: HTMLElement,
    options: {
      isParseActive?: () => boolean;
    } = {},
  ) {
    this.isParseActive = options.isParseActive ?? (() => false);
    this.elements = this.mount(root);
    mountThemeToggle(requiredElement<HTMLButtonElement>("#theme-toggle-button", root));
    this.initSidebarToggle();
    this.initViewMenu();
    this.initCanvasToolbar();
    const detailPanel = this.elements.workspaceGrid.querySelector<HTMLElement>(".detail-panel");
    if (detailPanel) mountDetailResizer(this.elements.workspaceGrid, detailPanel);
  }

  private mount(root: HTMLElement): AppElements {
    root.innerHTML = `
  <div class="shell">
    <header class="topbar">
      <div class="brand-block">
        <svg class="brand-mark" width="38" height="38" viewBox="0 0 38 38" aria-hidden="true" focusable="false">
  <defs>
    <radialGradient id="ms-brand-flare" cx="50%" cy="50%" r="50%">
      <stop offset="0%" stop-color="#ffe29e" stop-opacity="0.95" />
      <stop offset="45%" stop-color="#eab665" stop-opacity="0.35" />
      <stop offset="100%" stop-color="#050811" stop-opacity="0" />
    </radialGradient>
  </defs>
  <rect x="0.5" y="0.5" width="37" height="37" rx="9.5" fill="#060913" stroke="#8ec0ff" stroke-opacity="0.26" />
  <polyline
    points="6.5,23.5 13,13 17.5,25.5 23,23.8 27.8,13.5 32.5,22.5"
    fill="none"
    stroke="#b8d6ff"
    stroke-opacity="0.72"
    stroke-width="1.35"
    stroke-linecap="round"
    stroke-linejoin="round"
  />
  <circle cx="13" cy="13" r="7.5" fill="url(#ms-brand-flare)" />
  <path d="M9,13 H17 M13,9 V17" stroke="#ffd88a" stroke-width="0.9" stroke-linecap="round" stroke-opacity="0.85" />
  <circle cx="6.5" cy="23.5" r="1.35" fill="#dceaff" />
  <circle cx="13" cy="13" r="2.2" fill="#fffdf7" />
  <circle cx="17.5" cy="25.5" r="1.45" fill="#dceaff" />
  <circle cx="23" cy="23.8" r="1.35" fill="#dceaff" />
  <circle cx="27.8" cy="13.5" r="1.65" fill="#e8f2ff" />
  <circle cx="32.5" cy="22.5" r="1.2" fill="#bfd8ff" />
</svg>
        <div>
          <div class="brand">Mathscape</div>
          <div class="tagline">数学景观 · 来源视图</div>
        </div>
      </div>
      <nav class="topbar-center" aria-label="视图切换">
        <button id="view-menu-button" class="view-menu-button" type="button" aria-haspopup="true" aria-expanded="false" aria-controls="view-switch">视图 <span aria-hidden="true">▾</span></button>
        <div id="view-switch" class="view-switch" role="group" aria-label="视图">
          <button id="learning-view-button" class="view-switch-button" type="button" title="学习视图：每份材料里哪些内容是新的、哪些在别的材料里见过">学习视图</button>
          <button id="ink-button" class="view-switch-button" type="button" title="手写：打开手稿工作区">手写</button>
          <button id="sandbox-button" class="view-switch-button" type="button" title="沙箱：从检查点复制隔离工作区，可丢弃或导出">沙箱</button>
        </div>
      </nav>
      <div class="topbar-actions">
        <span id="service-state" class="service-state">正在连接本地内核</span>
        <button id="workspace-search-button" class="secondary-button workspace-search-button" type="button" title="搜索手稿、材料、来源单元和命名快照（⌘K / Ctrl+K）" aria-keyshortcuts="Meta+K Control+K"><svg class="topbar-icon" viewBox="0 0 16 16" aria-hidden="true"><circle cx="7" cy="7" r="4.5" fill="none" stroke="currentColor" stroke-width="1.6"/><path d="M10.4 10.4 14 14" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg><span>搜索</span></button>
        <button id="theme-toggle-button" class="icon-button" type="button" aria-label="切换主题"><svg class="topbar-icon" viewBox="0 0 20 20" aria-hidden="true"><circle cx="10" cy="10" r="6.5" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M10 3.5a6.5 6.5 0 0 0 0 13Z" fill="currentColor"/></svg></button>
        <button id="llm-settings-button" class="icon-button settings-button" type="button" title="设置">
          <span aria-hidden="true">⚙</span>
          <span id="llm-settings-label" class="sr-only">设置</span>
        </button>
        <div class="help-menu">
          <button id="help-button" class="icon-button" type="button" title="帮助" aria-label="帮助" aria-haspopup="menu" aria-expanded="false" aria-controls="help-menu"><span aria-hidden="true">?</span></button>
          <div id="help-menu" class="help-menu-list" role="menu" hidden>
            <button id="tutorial-menu-item" type="button" role="menuitem">教程：两本书讲的是不是同一个东西</button>
          </div>
        </div>
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
        <section class="lean-card" aria-label="Lean Profile">
          <div class="runtime-heading">
            <span class="field-label">Lean Profile · 环境 / 外部库</span>
            <span id="lean-state" class="runtime-state">检查中</span>
          </div>
          <strong id="lean-summary" class="runtime-version">—</strong>
          <ul id="lean-profiles" class="lean-profile-list"></ul>
          <p id="lean-problem" class="runtime-problem">正在读取 Lean profile 记录。</p>
        </section>
        <section id="last-import" class="import-card is-empty">
          <span class="field-label">最近一次导入</span>
          <p>选择一份 PDF，将其复制到自包含 Artifact Pool。</p>
        </section>
        <section id="last-parse" class="import-card is-empty">
          <span class="field-label">最近一次解析</span>
          <p>选择图中的 PDF 节点以配置连续页码范围。</p>
        </section>
        <section id="discovery-ledger" class="discovery-ledger">
          <div class="ledger-heading">
            <span class="field-label">内容发现记录</span>
            <button id="refresh-discovery-runs" class="copy-button" type="button">刷新</button>
          </div>
          <div id="discovery-runs-list" class="discovery-runs-list">
            <p class="ledger-empty">正在读取运行记录…</p>
          </div>
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
              <button class="preset-button" data-preset="study" type="button" title="研读透镜：按材料章节顺序动态展开双轨线性书脊与共享语义伴星">研读</button>
            </div>
            <span class="filter-divider">│</span>
            <div class="filter-group" id="node-filter-group">
              <label class="graph-evidence-toggle"><input type="checkbox" data-node-kind="SourceArtifact"> 材料文件</label>
              <label class="graph-evidence-toggle"><input type="checkbox" data-node-kind="SourceSemanticUnit"> 内容单元</label>
              <label class="graph-evidence-toggle"><input type="checkbox" data-node-kind="SourceSpan"> 原文证据</label>
              <label class="graph-evidence-toggle"><input type="checkbox" data-node-kind="SemanticEntity"> 共享对象</label>
            </div>
            <span class="filter-divider">│</span>
            <label class="graph-evidence-toggle"><input id="render-mode-toggle" type="checkbox" checked> 语义边</label>
          </div>
        </div>
        <div id="graph" class="graph-canvas"></div>
        <div id="focus-bar" class="focus-bar" aria-label="聚焦范围" hidden>
          <span class="focus-label">聚焦：<strong id="focus-root">—</strong></span>
          <span class="focus-depth" aria-label="聚焦视角">
            视角
            <button class="lens-button" data-lens="heuristic" type="button" title="跨书启发邻域：穿透共享对象关联其他教材与引理，理论书架止步防爆炸">启发关联</button>
            <button class="lens-button" data-lens="structural" type="button" title="仅展开单份材料内部的章节与书内证明关系">来源结构</button>
            <button class="lens-button" data-lens="semantic_only" type="button" title="仅展示共享对象、理论与挂账组成的抽象语义骨架">纯语义网</button>
          </span>
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
        <section id="review-workspace" hidden></section>
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

    <section id="ink-workspace" class="ink-workspace" hidden aria-label="手稿工作区">
      <div id="ink-split-reader" class="ink-split-reader" hidden aria-label="并排阅读原文"></div>
      <div id="ink-split-resizer" class="ink-split-resizer" hidden></div>
      <div id="ink-desktop-host" class="ink-desktop-host" aria-label="手稿"></div>
    </section>

    <div id="llm-settings-host"></div>
    <div id="sandbox-host"></div>
    <div id="toast" class="toast" role="status" aria-live="polite"></div>
  </div>
`;

    return {
      shell: requiredElement<HTMLElement>(".shell", root),
      importButton: requiredElement<HTMLButtonElement>("#import-pdf", root),
      sourceToggle: requiredElement<HTMLButtonElement>("#toggle-sources", root),
      workspaceGrid: requiredElement<HTMLElement>(".workspace-grid", root),
      serviceState: requiredElement<HTMLElement>("#service-state", root),
      artifactCount: requiredElement<HTMLElement>("#artifact-count", root),
      conversionCount: requiredElement<HTMLElement>("#conversion-count", root),
      workspaceId: requiredElement<HTMLElement>("#workspace-id", root),
      workspaceName: requiredElement<HTMLElement>("#workspace-name", root),
      workspaceRoot: requiredElement<HTMLElement>("#workspace-root", root),
      copyWorkspaceIdButton: requiredElement<HTMLButtonElement>("#copy-workspace-id", root),
      canvasPanel: requiredElement<HTMLElement>(".canvas-panel", root),
      graphContainer: requiredElement<HTMLElement>("#graph", root),
      graphLegend: requiredElement<HTMLElement>("#graph-legend", root),
      focusBar: requiredElement<HTMLElement>("#focus-bar", root),
      focusRootLabel: requiredElement<HTMLElement>("#focus-root", root),
      focusSummary: requiredElement<HTMLElement>("#focus-summary", root),
      graphViewTitle: requiredElement<HTMLElement>("#graph-view-title", root),
      nodeFilterGroup: requiredElement<HTMLElement>("#node-filter-group", root),
      renderModeToggle: requiredElement<HTMLInputElement>("#render-mode-toggle", root),
      emptyState: requiredElement<HTMLElement>("#empty-state", root),
      emptyStateTitle: requiredElement<HTMLElement>("#empty-state-title", root),
      emptyStateMessage: requiredElement<HTMLElement>("#empty-state-message", root),
      nodeDetail: requiredElement<HTMLElement>("#node-detail", root),
      lastImport: requiredElement<HTMLElement>("#last-import", root),
      lastParse: requiredElement<HTMLElement>("#last-parse", root),
      discoveryRunsList: requiredElement<HTMLElement>("#discovery-runs-list", root),
      refreshDiscoveryRunsButton: requiredElement<HTMLButtonElement>("#refresh-discovery-runs", root),
      markerState: requiredElement<HTMLElement>("#marker-state", root),
      markerVersion: requiredElement<HTMLElement>("#marker-version", root),
      markerProblem: requiredElement<HTMLElement>("#marker-problem", root),
      leanState: requiredElement<HTMLElement>("#lean-state", root),
      leanSummary: requiredElement<HTMLElement>("#lean-summary", root),
      leanProfiles: requiredElement<HTMLElement>("#lean-profiles", root),
      leanProblem: requiredElement<HTMLElement>("#lean-problem", root),
      toast: requiredElement<HTMLElement>("#toast", root),
      readerHost: requiredElement<HTMLElement>("#reader", root),
      reviewWorkspaceHost: requiredElement<HTMLElement>("#review-workspace", root),
      llmSettingsHost: requiredElement<HTMLElement>("#llm-settings-host", root),
      llmSettingsButton: requiredElement<HTMLButtonElement>("#llm-settings-button", root),
      llmSettingsLabel: requiredElement<HTMLElement>("#llm-settings-label", root),
      sandboxHost: requiredElement<HTMLElement>("#sandbox-host", root),
      sandboxButton: requiredElement<HTMLButtonElement>("#sandbox-button", root),
      inkButton: requiredElement<HTMLButtonElement>("#ink-button", root),
      searchButton: requiredElement<HTMLButtonElement>("#workspace-search-button", root),
      learningViewButton: requiredElement<HTMLButtonElement>("#learning-view-button", root),
      helpButton: requiredElement<HTMLButtonElement>("#help-button", root),
      helpMenu: requiredElement<HTMLElement>("#help-menu", root),
      tutorialMenuItem: requiredElement<HTMLButtonElement>("#tutorial-menu-item", root),
      inkWorkspace: requiredElement<HTMLElement>("#ink-workspace", root),
      inkSplitReader: requiredElement<HTMLElement>("#ink-split-reader", root),
      inkSplitResizer: requiredElement<HTMLElement>("#ink-split-resizer", root),
      inkHost: requiredElement<HTMLElement>("#ink-desktop-host", root),
    };
  }

  /**
   * 顶栏中间的视图切换（学习视图 / 手写 / 沙箱）。窄窗口下收进「视图」下拉：
   * 还是同一组按钮，只是改成弹出菜单，点一项后收起。
   */
  private initViewMenu(): void {
    const root = this.elements.shell;
    const button = root.querySelector<HTMLButtonElement>("#view-menu-button");
    const center = root.querySelector<HTMLElement>(".topbar-center");
    const group = root.querySelector<HTMLElement>("#view-switch");
    if (!button || !center || !group) return;
    const setOpen = (open: boolean) => {
      center.classList.toggle("is-open", open);
      button.setAttribute("aria-expanded", String(open));
    };
    button.addEventListener("click", (event) => {
      event.stopPropagation();
      setOpen(!center.classList.contains("is-open"));
    });
    group.addEventListener("click", () => setOpen(false));
    document.addEventListener("click", (event) => {
      if (!center.contains(event.target as Node)) setOpen(false);
    });
    document.addEventListener("keydown", (event) => {
      if (event.key === "Escape" && center.classList.contains("is-open")) setOpen(false);
    });
  }

  private initSidebarToggle(): void {
    const { sourceToggle, workspaceGrid } = this.elements;
    const shell = workspaceGrid.closest<HTMLElement>(".shell")!;
    const sourcePanel = workspaceGrid.querySelector<HTMLElement>(".source-panel")!;
    let preferredCollapsed = false;
    let compact = false;
    let drawerOpen = false;
    try {
      preferredCollapsed = localStorage.getItem("mathscape:sources-collapsed") === "true";
    } catch { /* Optional presentation preference. */ }

    const update = () => {
      // Only keep the full directory if the map still has ~560px beside it.
      // This is temporary: closing the dock restores the user's directory preference.
      const nextCompact = shell.classList.contains("has-sandbox-sidebar") && workspaceGrid.clientWidth < 1100;
      if (nextCompact !== compact) drawerOpen = false;
      compact = nextCompact;
      const collapsed = compact ? !drawerOpen : preferredCollapsed;
      const changed = workspaceGrid.classList.contains("sources-collapsed") !== collapsed
        || workspaceGrid.classList.contains("sources-compact") !== compact;
      workspaceGrid.classList.toggle("sources-compact", compact);
      workspaceGrid.classList.toggle("sources-collapsed", collapsed);
      sourceToggle.textContent = collapsed ? "›" : "‹";
      sourceToggle.setAttribute("aria-expanded", String(!collapsed));
      sourceToggle.setAttribute("aria-label", collapsed ? "展开来源目录" : "折叠来源目录");
      if (changed) window.dispatchEvent(new Event("resize"));
    };
    update();
    sourceToggle.onclick = () => {
      if (compact) {
        drawerOpen = !drawerOpen;
      } else {
        preferredCollapsed = !preferredCollapsed;
        try {
          localStorage.setItem("mathscape:sources-collapsed", String(preferredCollapsed));
        } catch { /* Storage may be unavailable. */ }
      }
      update();
    };
    shell.addEventListener("click", (event) => {
      if (!drawerOpen || sourcePanel.contains(event.target as Node)) return;
      drawerOpen = false;
      update();
    });
    shell.addEventListener("keydown", (event) => {
      if (event.key !== "Escape" || !drawerOpen) return;
      drawerOpen = false;
      update();
      sourceToggle.focus();
    });
    window.addEventListener("resize", update);
    new ResizeObserver(update).observe(workspaceGrid);

    this.elements.copyWorkspaceIdButton.addEventListener("click", () => {
      void this.copyIdentifier(this.elements.workspaceId.textContent ?? "");
    });
  }

  /** Reader/review overlays follow the toolbar when its controls wrap. */
  private initCanvasToolbar(): void {
    const panel = this.elements.workspaceGrid.querySelector<HTMLElement>(".canvas-panel")!;
    const toolbar = panel.querySelector<HTMLElement>(".canvas-toolbar")!;
    let height = 0;
    new ResizeObserver(() => {
      const next = Math.ceil(toolbar.getBoundingClientRect().height);
      if (next === height) return;
      height = next;
      panel.style.setProperty("--canvas-toolbar-height", `${height}px`);
      window.dispatchEvent(new Event("resize"));
    }).observe(toolbar);
  }

  showToast(message: string, kind: "success" | "error"): void {
    window.clearTimeout(this.toastTimer);
    const { toast } = this.elements;
    toast.textContent = message;
    toast.dataset.kind = kind;
    toast.classList.add("is-visible");
    this.toastTimer = window.setTimeout(() => toast.classList.remove("is-visible"), 5000);
  }

  setBusy(busy: boolean, message = ""): void {
    if (this.isParseActive()) {
      busy = true;
    }
    const { importButton, serviceState } = this.elements;
    importButton.disabled = busy;
    importButton.classList.toggle("is-busy", busy);
    if (busy) {
      serviceState.textContent = message;
    } else if (!serviceState.textContent?.includes("失败")) {
      serviceState.textContent = "本地内核已连接";
    }
  }

  updateServiceState(text: string, isReady = false): void {
    const { serviceState } = this.elements;
    serviceState.textContent = text;
    serviceState.classList.toggle("is-ready", isReady);
  }

  renderWorkspaceInfo(info: {
    artifactCount: number;
    conversionCount: number;
    workspaceId: string;
    workspaceRoot: string;
  }): void {
    this.elements.artifactCount.textContent = info.artifactCount.toString();
    this.elements.conversionCount.textContent = info.conversionCount.toString();
    this.elements.workspaceId.textContent = info.workspaceId;
    const folder = info.workspaceRoot.split(/[\\/]/).filter(Boolean).at(-1);
    this.elements.workspaceName.textContent = !folder || folder === "workspace" ? "我的数学景观" : folder;
    this.elements.workspaceName.title = this.elements.workspaceName.textContent;
    this.elements.workspaceRoot.textContent = info.workspaceRoot;
  }

  async copyIdentifier(value: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(value);
      this.showToast("已复制完整 ID", "success");
    } catch {
      this.showToast("剪贴板不可用，请展开技术详情后选择 ID 复制。", "error");
    }
  }
}
