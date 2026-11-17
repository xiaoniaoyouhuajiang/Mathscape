/**
 * In-Map Study HUD (`#study-hud`) & Node Hover Briefing Popover (`StudyBriefingPopover`).
 *
 * Replaces the map-blocking modal list with an integrated spatial lens overlay:
 * - Top floating bar: scrollable material selector, non-destructive spotlight
 *   signal pills, collapsible conventions/debts drawer, staged toggle, and exit.
 * - Hover briefing card: appears beside hovered spine/semantic nodes with
 *   alignment differences (KaTeX), 1-hop intra-book topology summary, and
 *   1-click `未读 / 在读 / 已掌握` status toggles (stored in `study_marks` sidecar).
 */
import type { Core, EventObject, NodeSingular } from "cytoscape";
import { textElement } from "../core/dom";
import { errorMessage } from "../core/ipc";
import { OUT_OF_FOCUS_CLASS } from "../graph/cytoscape";
import type { IncidenceGraph } from "../graph/projection";
import {
  buildStudyNodeBriefing,
  type StudyNodeBriefing,
  type StudySignalMode,
} from "../graph/study-posture";
import { markdownRenderer } from "../reader/markdown";
import {
  getLearningView,
  recheckMathlibBaselines,
  recheckSummary,
  setStudyMarks,
  type LearningMaterial,
  type LearningView,
  type StudyStatus,
} from "./api";
import { debtStatusLabel } from "./labels";

const md = markdownRenderer();

function renderInlineMath(text: string, className = "study-inline-math"): HTMLElement {
  const span = document.createElement("span");
  span.className = className;
  span.innerHTML = md.renderInline(text);
  return span;
}

const STUDY_STATUS_ITEMS: ReadonlyArray<[StudyStatus, string]> = [
  ["unread", "未读"],
  ["studying", "在读"],
  ["mastered", "已掌握"],
];

export interface StudyHudDelegate {
  materialName: (artifactId: string) => string;
  hasStagedProposals: () => boolean;
  getDisplayedGraph: () => IncidenceGraph | undefined;
  getGraphCore: () => Core | null;
  onSelectMaterial: (materialId: string | null) => void;
  onSelectSignal: (signal: StudySignalMode) => void;
  onDataUpdated: (view: LearningView) => void;
  onFocusDomainObject: (domainId: string) => void;
  onSelectProjectionNode: (nodeId: string) => void;
  onExitStudy: () => void;
  showToast: (message: string, kind: "success" | "error") => void;
}

export class StudyHudController {
  private readonly hudBar: HTMLElement;
  private readonly drawer: HTMLElement;
  private readonly briefingCard: HTMLElement;
  private active = false;
  private view: LearningView = { materials: [] };
  private lastViewJson = "";
  private selectedMaterialId: string | null = null;
  private signalMode: StudySignalMode = "all";
  private includeStaged = false;
  private stagedTouchedByUser = false;
  private drawerOpen = false;

  private boundCore: Core | null = null;
  private hoverEnterTimer: number | null = null;
  private hoverLeaveTimer: number | null = null;
  private hoveredNodeId: string | null = null;
  private pointerInsideCard = false;

  constructor(
    private readonly canvasPanel: HTMLElement,
    private readonly delegate: StudyHudDelegate,
  ) {
    this.hudBar = textElement("div", "study-hud", "");
    this.hudBar.id = "study-hud";
    this.hudBar.setAttribute("role", "region");
    this.hudBar.setAttribute("aria-label", "材料研读控制栏");
    this.hudBar.hidden = true;

    this.drawer = textElement("div", "study-hud-drawer", "");
    this.drawer.hidden = true;

    this.briefingCard = textElement("aside", "study-briefing-popover", "");
    this.briefingCard.setAttribute("aria-label", "节点研读简报");
    this.briefingCard.hidden = true;
    this.briefingCard.addEventListener("mouseenter", () => {
      this.pointerInsideCard = true;
      if (this.hoverLeaveTimer !== null) {
        window.clearTimeout(this.hoverLeaveTimer);
        this.hoverLeaveTimer = null;
      }
    });
    this.briefingCard.addEventListener("mouseleave", () => {
      this.pointerInsideCard = false;
      this.scheduleHideBriefing(120);
    });

    this.canvasPanel.append(this.hudBar, this.drawer, this.briefingCard);
  }

  isActive(): boolean {
    return this.active;
  }

  getLearningViewData(): LearningView {
    return this.view;
  }

  getSelectedMaterial(): LearningMaterial | null {
    if (!this.selectedMaterialId) return null;
    return this.view.materials.find((m) => m.artifact_id === this.selectedMaterialId) ?? null;
  }

  getSelectedMaterialId(): string | null {
    return this.selectedMaterialId;
  }

  getSignalMode(): StudySignalMode {
    return this.signalMode;
  }

  /** Lookup helper so `InspectorController` can also display & toggle a unit's `StudyStatus`. */
  getUnitStudyStatus(unitDomainId: string): StudyStatus | null {
    for (const mat of this.view.materials) {
      const found = (mat.outline_units ?? []).find((u) => u.unit_id === unitDomainId);
      if (found) {
        return found.study_status === "mastered" || found.study_status === "studying"
          ? found.study_status
          : "unread";
      }
    }
    return null;
  }

  async setUnitStudyStatus(unitDomainId: string, status: StudyStatus): Promise<void> {
    try {
      await setStudyMarks([{ object_id: unitDomainId, status }]);
      await this.reloadData(true);
    } catch (error) {
      this.delegate.showToast(`无法更新学习状态：${errorMessage(error)}`, "error");
    }
  }

  async activate(initialMaterialId?: string | null): Promise<void> {
    if (!this.stagedTouchedByUser) {
      this.includeStaged = this.delegate.hasStagedProposals();
    }
    if (initialMaterialId !== undefined) {
      this.selectedMaterialId = initialMaterialId;
    }
    this.active = true;
    this.hudBar.hidden = false;
    this.ensureCoreListeners();
    this.renderHud();
    await this.reloadData();
  }

  deactivate(): void {
    this.active = false;
    this.drawerOpen = false;
    this.hudBar.hidden = true;
    this.drawer.hidden = true;
    this.hideBriefingNow();
  }

  async reloadData(forceNotify = false): Promise<void> {
    if (!this.stagedTouchedByUser && this.delegate.hasStagedProposals()) {
      this.includeStaged = true;
    }
    let nextView: LearningView;
    try {
      nextView = await getLearningView(this.includeStaged);
    } catch {
      nextView = this.synthesizeFallbackView();
    }
    const nextJson = JSON.stringify(nextView);
    const changed = forceNotify || nextJson !== this.lastViewJson;
    this.view = nextView;
    this.lastViewJson = nextJson;

    if (
      this.selectedMaterialId !== null &&
      !this.view.materials.some((m) => m.artifact_id === this.selectedMaterialId)
    ) {
      this.selectedMaterialId = null;
    }
    if (!this.active) {
      // Cache silently in GraphController (no render while inactive).
      this.delegate.onDataUpdated(this.view);
      return;
    }
    this.renderHud();
    if (changed) {
      this.delegate.onDataUpdated(this.view);
    }
    if (this.hoveredNodeId !== null) {
      this.refreshBriefingIfOpen();
    }
  }

  notifyGraphMounted(): void {
    this.ensureCoreListeners();
  }

  private synthesizeFallbackView(): LearningView {
    const graph = this.delegate.getDisplayedGraph();
    if (!graph) return { materials: [] };
    const artifacts = graph.nodes.filter(
      (n) => n.element_kind === "entity" && n.domain_kind === "SourceArtifact" && n.domain_id,
    );
    const units = graph.nodes.filter(
      (n) => n.element_kind === "entity" && n.domain_kind === "SourceSemanticUnit" && n.domain_id,
    );
    if (artifacts.length === 0 || units.length === 0) return { materials: [] };
    const primary = artifacts.find((a) => a.label.endsWith(".md") || a.label.includes("规范解析正文")) ?? artifacts[0];
    return {
      materials: [
        {
          artifact_id: primary.domain_id!,
          units: [],
          new_conventions: [],
          open_debts: [],
          outline_units: units.map((u) => ({
            unit_id: u.domain_id!,
            unit_title: u.label,
            unit_kind: "statement",
            parent_unit: null,
            study_status: "unread",
            category: "unaligned",
            targets: [],
          })),
        },
      ],
    };
  }

  private renderHud(): void {
    this.hudBar.replaceChildren();
    const row = textElement("div", "study-hud-main", "");

    const badge = textElement("span", "study-hud-badge", "研读透镜");
    row.append(badge);

    // 1. Scrollable Material Selector
    const select = document.createElement("select");
    select.className = "study-material-select";
    select.setAttribute("aria-label", "选择研读材料");

    const allOpt = document.createElement("option");
    allOpt.value = "";
    allOpt.textContent = `全部材料 · 语义总览（${this.view.materials.length} 份）`;
    select.append(allOpt);

    for (const mat of this.view.materials) {
      const opt = document.createElement("option");
      opt.value = mat.artifact_id;
      const outline = mat.outline_units ?? [];
      const mastered = outline.filter((u) => u.study_status === "mastered").length;
      const aligned = outline.filter((u) => u.category !== "unaligned").length;
      const masteredSuffix = mastered > 0 ? ` · 掌握 ${mastered}` : "";
      opt.textContent = `${this.delegate.materialName(mat.artifact_id)}（${outline.length} 单元 · 对齐 ${aligned}${masteredSuffix}）`;
      select.append(opt);
    }
    select.value = this.selectedMaterialId ?? "";
    select.onchange = () => {
      this.selectedMaterialId = select.value || null;
      this.signalMode = "all";
      this.drawerOpen = false;
      this.hideBriefingNow();
      this.renderHud();
      this.delegate.onSelectMaterial(this.selectedMaterialId);
    };
    row.append(select);

    // 2. Material-specific Spotlight Signal Pills & Conventions/Debts Drawer Toggle
    const selectedMat = this.getSelectedMaterial();
    if (selectedMat) {
      const outline = selectedMat.outline_units ?? [];
      const novelDiffCount = outline.filter(
        (u) => u.category === "fresh" || u.targets.some((t) => t.only_differs_by.length > 0),
      ).length;
      const crossCount = outline.filter(
        (u) => u.category === "seen_elsewhere" || u.category === "mastered_elsewhere",
      ).length;
      const unalignedCount = outline.filter((u) => u.category === "unaligned").length;
      const debtCount = selectedMat.open_debts.length;

      const pillsWrap = textElement("div", "study-signal-pills", "");
      pillsWrap.setAttribute("role", "group");
      pillsWrap.setAttribute("aria-label", "书脊高亮信号");

      const signalSpecs: Array<{ mode: StudySignalMode; label: string; show: boolean }> = [
        { mode: "all", label: `全部 (${outline.length})`, show: true },
        { mode: "novel_and_diff", label: `已对齐/局部差异 (${novelDiffCount})`, show: novelDiffCount > 0 },
        { mode: "cross_book", label: `跨书见过 (${crossCount})`, show: crossCount > 0 },
        { mode: "unaligned", label: `尚未对齐 (${unalignedCount})`, show: unalignedCount > 0 },
        { mode: "debts", label: `挂账 (${debtCount})`, show: debtCount > 0 },
      ];

      for (const spec of signalSpecs) {
        if (!spec.show && this.signalMode !== spec.mode) continue;
        const btn = textElement(
          "button",
          `study-signal-pill${this.signalMode === spec.mode ? " is-active" : ""}`,
          spec.label,
        );
        btn.type = "button";
        btn.dataset.studySignal = spec.mode;
        btn.setAttribute("aria-pressed", String(this.signalMode === spec.mode));
        btn.onclick = () => {
          this.signalMode = this.signalMode === spec.mode && spec.mode !== "all" ? "all" : spec.mode;
          this.renderHud();
          this.delegate.onSelectSignal(this.signalMode);
        };
        pillsWrap.append(btn);
      }
      row.append(pillsWrap);

      const convCount = selectedMat.conventions_detail?.length ?? selectedMat.new_conventions.length;
      if (convCount > 0 || debtCount > 0) {
        const drawerBtn = textElement(
          "button",
          `study-drawer-toggle${this.drawerOpen ? " is-active" : ""}`,
          `约定 ${convCount} · 账单 ${debtCount} ${this.drawerOpen ? "▴" : "▾"}`,
        );
        drawerBtn.type = "button";
        drawerBtn.setAttribute("aria-expanded", String(this.drawerOpen));
        drawerBtn.onclick = () => {
          this.drawerOpen = !this.drawerOpen;
          this.renderHud();
        };
        row.append(drawerBtn);
      }
    } else {
      const hint = textElement(
        "span",
        "study-hud-hint",
        "从下拉框选择一份材料，可将散落单元动态排成「左轨章节书脊 + 右轨共享语义伴星」",
      );
      row.append(hint);
    }

    // 3. Right-side controls: Staged Toggle + Mathlib Recheck + Exit
    const rightControls = textElement("div", "study-hud-right", "");
    const stagedLbl = textElement("label", "study-staged-check", "");
    const stagedInput = document.createElement("input");
    stagedInput.type = "checkbox";
    stagedInput.checked = this.includeStaged;
    stagedInput.onchange = () => {
      this.stagedTouchedByUser = true;
      this.includeStaged = stagedInput.checked;
      void this.reloadData();
    };
    stagedLbl.append(stagedInput, document.createTextNode(" 含暂存提案"));
    rightControls.append(stagedLbl);

    const recheckBtn = textElement("button", "study-mini-btn", "核对 mathlib");
    recheckBtn.type = "button";
    recheckBtn.title = "用本地 Lean profile 核对共享对象声明的 mathlib 名称";
    recheckBtn.onclick = async () => {
      recheckBtn.disabled = true;
      try {
        const report = await recheckMathlibBaselines();
        this.delegate.showToast(recheckSummary(report), report.status === "unavailable" ? "error" : "success");
      } catch (error) {
        this.delegate.showToast(errorMessage(error), "error");
      } finally {
        recheckBtn.disabled = false;
      }
    };
    rightControls.append(recheckBtn);

    const exitBtn = textElement("button", "study-exit-btn", "退出研读");
    exitBtn.type = "button";
    exitBtn.onclick = () => this.delegate.onExitStudy();
    rightControls.append(exitBtn);

    row.append(rightControls);
    this.hudBar.append(row);

    this.renderDrawer(selectedMat);
  }

  private renderDrawer(material: LearningMaterial | null): void {
    if (!this.active || !this.drawerOpen || !material) {
      this.drawer.hidden = true;
      this.drawer.replaceChildren();
      return;
    }
    this.drawer.hidden = false;
    this.drawer.replaceChildren();

    const detailConvs = material.conventions_detail ?? [];
    if (detailConvs.length > 0 || material.new_conventions.length > 0) {
      const sec = textElement("section", "study-drawer-section", "");
      sec.append(textElement("h4", "", "这份材料的记号与编号约定"));
      const ul = textElement("ul", "study-drawer-list", "");
      if (detailConvs.length > 0) {
        for (const c of detailConvs) {
          const li = textElement("li", "study-drawer-item", "");
          li.append(
            textElement(
              "span",
              c.is_baseline_match ? "study-tag study-tag-ok" : "study-tag study-tag-warn",
              c.is_baseline_match ? "同共享基线" : "特有约定",
            ),
            renderInlineMath(c.text),
          );
          if (c.diff_note) {
            li.append(textElement("span", "study-drawer-muted", `（${c.diff_note}）`));
          }
          li.onclick = () => this.delegate.onFocusDomainObject(c.scope_id);
          ul.append(li);
        }
      } else {
        for (const c of material.new_conventions) {
          const li = textElement("li", "study-drawer-item", "");
          li.append(textElement("span", "study-tag study-tag-warn", c.scope_label), renderInlineMath(c.text));
          li.onclick = () => this.delegate.onFocusDomainObject(c.scope_id);
          ul.append(li);
        }
      }
      sec.append(ul);
      this.drawer.append(sec);
    }

    if (material.open_debts.length > 0) {
      const sec = textElement("section", "study-drawer-section", "");
      sec.append(textElement("h4", "", `未还清的数学账（${material.open_debts.length}）`));
      const ul = textElement("ul", "study-drawer-list", "");
      for (const d of material.open_debts) {
        const li = textElement("li", "study-drawer-item is-clickable", "");
        li.append(
          textElement("span", "study-tag study-tag-debt", debtStatusLabel(d.status)),
          renderInlineMath(d.obligation),
        );
        li.title = "点击在右侧轨道居中定位这笔账";
        li.onclick = () => this.delegate.onFocusDomainObject(d.debt_id);
        ul.append(li);
      }
      sec.append(ul);
      this.drawer.append(sec);
    }
  }

  private ensureCoreListeners(): void {
    const core = this.delegate.getGraphCore();
    if (!core || core === this.boundCore) return;
    this.boundCore = core;

    core.on("mouseover", "node", (event: EventObject) => {
      if (!this.active) return;
      const node = event.target as NodeSingular;
      if (
        node.hasClass(OUT_OF_FOCUS_CLASS) ||
        node.data("elementKind") === "junction" ||
        node.hasClass("ink-note-node")
      ) {
        return;
      }
      if (this.hoverLeaveTimer !== null) {
        window.clearTimeout(this.hoverLeaveTimer);
        this.hoverLeaveTimer = null;
      }
      if (this.hoverEnterTimer !== null) {
        window.clearTimeout(this.hoverEnterTimer);
      }
      const targetId = node.id();
      this.hoverEnterTimer = window.setTimeout(() => {
        this.hoverEnterTimer = null;
        if (!this.active || core.destroyed()) return;
        this.showBriefingForNode(node);
      }, 110);
      void targetId;
    });

    core.on("mouseout", "node", () => {
      if (this.hoverEnterTimer !== null) {
        window.clearTimeout(this.hoverEnterTimer);
        this.hoverEnterTimer = null;
      }
      if (!this.active) return;
      this.scheduleHideBriefing(170);
    });

    core.on("pan zoom drag", () => {
      if (!this.briefingCard.hidden && !this.pointerInsideCard) {
        this.hideBriefingNow();
      }
    });
  }

  private scheduleHideBriefing(delayMs: number): void {
    if (this.hoverLeaveTimer !== null) window.clearTimeout(this.hoverLeaveTimer);
    this.hoverLeaveTimer = window.setTimeout(() => {
      this.hoverLeaveTimer = null;
      if (!this.pointerInsideCard) {
        this.hideBriefingNow();
      }
    }, delayMs);
  }

  private hideBriefingNow(): void {
    if (this.hoverEnterTimer !== null) {
      window.clearTimeout(this.hoverEnterTimer);
      this.hoverEnterTimer = null;
    }
    if (this.hoverLeaveTimer !== null) {
      window.clearTimeout(this.hoverLeaveTimer);
      this.hoverLeaveTimer = null;
    }
    this.hoveredNodeId = null;
    this.briefingCard.hidden = true;
  }

  private refreshBriefingIfOpen(): void {
    const core = this.delegate.getGraphCore();
    if (!core || !this.hoveredNodeId || this.briefingCard.hidden) return;
    const node = core.getElementById(this.hoveredNodeId);
    if (node.empty() || !node.isNode()) {
      this.hideBriefingNow();
      return;
    }
    this.showBriefingForNode(node);
  }

  private showBriefingForNode(node: NodeSingular): void {
    const graph = this.delegate.getDisplayedGraph();
    if (!graph) return;
    const briefing = buildStudyNodeBriefing(
      node.id(),
      graph,
      this.view,
      this.delegate.materialName,
    );
    if (!briefing) return;

    this.hoveredNodeId = node.id();
    this.renderBriefingContent(briefing);
    this.briefingCard.hidden = false;
    this.positionBriefingBesideNode(node);
  }

  private renderBriefingContent(b: StudyNodeBriefing): void {
    this.briefingCard.replaceChildren();

    const head = textElement("div", "study-briefing-head", "");
    const titleEl = textElement("strong", "study-briefing-title", "");
    titleEl.append(renderInlineMath(b.title));
    head.append(titleEl);
    if (b.alignmentBadge) {
      head.append(
        textElement("span", `study-briefing-badge is-${b.alignmentTone}`, b.alignmentBadge),
      );
    }
    this.briefingCard.append(head);

    // Quick StudyStatus toggle bar for SourceSemanticUnit
    if (b.studyStatus !== null && b.domainId) {
      const statusRow = textElement("div", "study-briefing-status-row", "");
      statusRow.append(textElement("span", "study-briefing-label", "个人掌握度："));
      const seg = textElement("div", "study-status-segmented", "");
      for (const [st, lbl] of STUDY_STATUS_ITEMS) {
        const btn = textElement(
          "button",
          `study-status-chip${b.studyStatus === st ? ` is-active is-${st}` : ""}`,
          lbl,
        );
        btn.type = "button";
        btn.setAttribute("aria-pressed", String(b.studyStatus === st));
        btn.onclick = (ev) => {
          ev.stopPropagation();
          void this.setUnitStudyStatus(b.domainId!, st);
        };
        seg.append(btn);
      }
      statusRow.append(seg);
      this.briefingCard.append(statusRow);
    }

    // Semantic Alignments & `only_differs_by`
    if (b.targets.length > 0) {
      const sec = textElement("div", "study-briefing-section", "");
      sec.append(textElement("div", "study-briefing-section-title", "关联共享语义对象"));
      for (const t of b.targets) {
        const item = textElement("div", "study-briefing-target", "");
        const top = textElement("div", "study-briefing-target-line", "");
        top.append(
          textElement("span", "study-rel-pill", t.relationText),
          renderInlineMath(t.entityLabel, "study-target-name"),
        );
        if (t.staged) {
          top.append(textElement("span", "study-staged-pill", "未审批"));
        }
        top.onclick = () => this.delegate.onFocusDomainObject(t.entityId);
        item.append(top);
        if (t.diffs.length > 0) {
          const diffLine = textElement("div", "study-briefing-diff", "只差：");
          diffLine.append(renderInlineMath(t.diffs.join("；")));
          item.append(diffLine);
        }
        if (t.seenInNames.length > 0) {
          item.append(
            textElement(
              "div",
              "study-briefing-cross",
              `${t.masteredElsewhere ? "已在其他材料掌握" : "其他材料见过"}：${t.seenInNames.join("、")}`,
            ),
          );
        }
        sec.append(item);
      }
      this.briefingCard.append(sec);
    }

    // 1-Hop Intra-Book / Semantic Neighbors Summary
    if (b.intraBookNeighbors.length > 0) {
      const sec = textElement("div", "study-briefing-section", "");
      sec.append(textElement("div", "study-briefing-section-title", "周边 1 跳关联节点"));
      const chips = textElement("div", "study-briefing-neighbors", "");
      for (const nb of b.intraBookNeighbors) {
        const chip = textElement("button", "study-neighbor-chip", "");
        chip.type = "button";
        chip.append(
          textElement("span", "study-neighbor-rel", `${nb.relationLabel}：`),
          document.createTextNode(nb.label.length > 20 ? `${nb.label.slice(0, 20)}…` : nb.label),
        );
        chip.onclick = (ev) => {
          ev.stopPropagation();
          this.hideBriefingNow();
          this.delegate.onSelectProjectionNode(nb.nodeId);
        };
        chips.append(chip);
      }
      sec.append(chips);
      this.briefingCard.append(sec);
    }

    // Attached Open Debts
    if (b.openDebts.length > 0) {
      const sec = textElement("div", "study-briefing-section", "");
      sec.append(textElement("div", "study-briefing-section-title", "关联未还清的账"));
      for (const d of b.openDebts) {
        const line = textElement("div", "study-briefing-debt", "⚠ ");
        line.append(renderInlineMath(d.obligation));
        sec.append(line);
      }
      this.briefingCard.append(sec);
    }
  }

  private positionBriefingBesideNode(node: NodeSingular): void {
    const panelRect = this.canvasPanel.getBoundingClientRect();
    const graphEl = this.canvasPanel.querySelector<HTMLElement>("#graph");
    if (!graphEl) return;
    const graphRect = graphEl.getBoundingClientRect();
    const offsetLeft = graphRect.left - panelRect.left;
    const offsetTop = graphRect.top - panelRect.top;

    const rpos = node.renderedPosition();
    const cardW = this.briefingCard.offsetWidth || 310;
    const cardH = this.briefingCard.offsetHeight || 180;

    // On a spine node, prefer placing below-left or below so horizontal right-going alignment bridges stay visible!
    const isSpineUnit = node.hasClass("spine-unit-node");
    let left = isSpineUnit
      ? offsetLeft + rpos.x - cardW - 22
      : offsetLeft + rpos.x + 26;
    let top = offsetTop + rpos.y - 18;

    if (left < 14) {
      // Not enough room on the left of the spine: place to the right of the spine title
      // (just below the outgoing alignment arrow) so the spine rows underneath stay completely visible!
      const rightOfSpine = offsetLeft + rpos.x + 215;
      if (rightOfSpine + cardW <= panelRect.width - 14) {
        left = rightOfSpine;
        top = offsetTop + rpos.y + 18;
      } else {
        left = Math.min(Math.max(14, offsetLeft + rpos.x + 20), panelRect.width - cardW - 14);
        top = offsetTop + rpos.y + 24;
      }
    }
    if (left + cardW > panelRect.width - 14) {
      left = Math.max(14, panelRect.width - cardW - 14);
    }
    if (top + cardH > panelRect.height - 14) {
      top = Math.max(68, panelRect.height - cardH - 14);
    }
    if (top < 66) top = 66;

    this.briefingCard.style.left = `${Math.round(left)}px`;
    this.briefingCard.style.top = `${Math.round(top)}px`;
  }
}
