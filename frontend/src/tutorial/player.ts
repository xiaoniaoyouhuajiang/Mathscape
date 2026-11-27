/**
 * 教程播放器：在地图区上方的独立画布里，用主地图同一套 Cytoscape 样式与
 * 场景播放器的镜头节奏（SCENE_TIMING）回放 11 镜。
 *
 * - 只读：不调用任何 Tauri 命令，不碰用户的地图实例和工作区。
 * - 「减少动效」：状态瞬间切换，不做镜头滑动和编号牌移动，不自动推进，
 *   HUD 显示「下一步 ›」；字幕照常。
 */
import type { Core, ElementDefinition, NodeSingular } from "cytoscape";
import { openExternalLink } from "../core/external-link";
import { reducedMotion as defaultReducedMotion } from "../core/motion";
import { mountCytoscapeGraph } from "../graph/cytoscape";
import { SCENE_TIMING } from "../graph/scene-player";
import { EDGE_TYPES, NODE_TYPES, type NodeTypeVisual } from "../graph/taxonomy";
import { graphPalette } from "../graph/theme";
import { LABEL_HOVER, LABEL_RECESS, LABEL_HOVER_STYLE, LABEL_RECESS_STYLE, refreshLabels, syncNodeLabel } from "../graph/label-hover";
import { texToPlain, truncatePlain } from "../graph/tex-plain";
import { t } from "../i18n";
import { EP2_TYPE_NOTES } from "../i18n/tutorial-ep2-copy";
import { getTheme } from "../shell/theme-manager";
import { inlineMathHtml, markdownHtml } from "./math";
import { TUTORIAL_EP2_TITLE, buildEpisode2Shots } from "./episode2";
import type { TutorialEpisodeId, TutorialProjection } from "./projection";
import { loadEpisodeProjection, tutorialGraph } from "./projection";
import type { ShotExtra, TutorialShot } from "./script";
import { TYPE_NOTES, buildTutorialShots, stripTypePrefix } from "./script";

export const TUTORIAL_TITLE = "教程（一）：两本书讲的是不是同一个东西";

/** 两条派生虚线：材料 → 从它切出来的那条定义（只标两个“中心”）。 */
const DERIVED: Record<string, string> = { jia: "jia.u.def", yi: "yi.u.def" };

const SEMANTIC_KIND: Record<string, { name: string; code: string }> = {
  concept: { name: TYPE_NOTES.concept.name, code: "concept" },
  declaration: { name: TYPE_NOTES.declaration.name, code: "declaration" },
  theory: { name: TYPE_NOTES.theory.name, code: "theory" },
  problem_schema: { name: TYPE_NOTES.problem.name, code: "problem_schema" },
};

export interface TutorialPlayerDeps {
  load?: () => Promise<TutorialProjection>;
  reducedMotion?: () => boolean;
  /** 自动推进的倍速（测试用）；默认 1。 */
  speed?: number;
}

const HIDDEN = "tutorial-hidden";
const FOCUS = "tutorial-focus";
const DIM = "tutorial-dim";
/** 连线：这一镜要读的线才显示文字。 */
const EDGE_FOCUS = "tutorial-edge-focus";
/** 连线：显示关系文字（回顾镜默认只加粗高亮，不写字，避免和单元标签挤在一起）。 */
const EDGE_LABELED = "tutorial-edge-labeled";
/** 回顾镜：别的材料里见过，绿色描边。 */
const SEEN = "tutorial-seen";
/** 回顾镜：新的，黄色描边。 */
const FRESH = "tutorial-new";
/** 回顾镜：约定卡、账、共享对象等一并点亮，但不展开成长标签。 */
const ACCENT = "tutorial-accent";
/** 面板行悬停 / 点击时，对应的地图元素。 */
const ROW_FOCUS = "tutorial-row-focus";
/** 非焦点节点的短标签最多几个字。 */
const COMPACT_CHARS = 12;
/** 焦点一笔账的标签最多几个字（全文在说明页）。 */
const DEBT_FULL_CHARS = 40;

/**
 * 教程地图的固定布局（模型坐标）：材料 | 内容单元（星港在上、灯塔在下）|
 * 共享对象与理论 | 一笔账（和它挂的对象同一行）。夹具是固定的，手排比力导向
 * 稳定，也能按标签大小留够间距：列宽 ≥ 焦点标签宽度，行距 ≥ 焦点标签高度。
 */
const UNIT_STEP = 96;
const COL = { material: -470, section: -230, unit: 0, shared: 440, debt: 800 } as const;
const TUTORIAL_LAYOUT: Record<string, [number, number]> = {
  "artifact:jia": [COL.material, -520],
  "artifact:yi": [COL.material, 90],
  "ds.jia": [COL.section, -480],
  "jia.u.sec": [COL.section, -280],
  "jia.u.def": [COL.unit, -520],
  "jia.u.lem": [COL.unit, -520 + UNIT_STEP],
  "jia.u.lem_pf": [COL.unit, -520 + UNIT_STEP * 2],
  "jia.u.thm": [COL.unit, -520 + UNIT_STEP * 3],
  "jia.u.thm_pf": [COL.unit, -520 + UNIT_STEP * 4],
  "jia.u.rem": [COL.unit, -520 + UNIT_STEP * 5],
  "ds.yi": [COL.section, 110],
  "yi.u.sec": [COL.section, 300],
  "yi.u.def": [COL.unit, 90],
  "yi.u.lem": [COL.unit, 90 + UNIT_STEP],
  "yi.u.thm": [COL.unit, 90 + UNIT_STEP * 2],
  "yi.u.ex": [COL.unit, 90 + UNIT_STEP * 3],
  "yi.u.hint": [COL.unit, 90 + UNIT_STEP * 4],
  "e.theory": [COL.shared, -420],
  "e.fib": [COL.shared, -290],
  "e.fib_def": [COL.shared, -160],
  "e.qpow": [COL.shared, -30],
  "e.cassini": [COL.shared, 100],
  "e.puzzle": [COL.shared, 240],
  "d.yi.f_eq_F": [COL.debt, -160],
  "d.yi.lem.forward": [COL.debt, -30],
  "d.yi.thm.forward": [COL.debt, 100],
  "d.puzzle.all_k": [COL.debt, 240],
};

/** 第 2 集固定布局：保证焦点多行标签与表示绑定/对齐线水平胶囊互不遮挡。 */
const TUTORIAL_EP2_LAYOUT: Record<string, [number, number]> = {
  "artifact:jia": [-560, -460],
  "artifact:yi": [-560, 180],
  "jia.u.sec": [-300, -260],
  "yi.u.sec": [-300, 380],
  "ds.jia": [0, -600],
  "jia.u.def": [0, -460],
  "jia.u.thm": [0, -310],
  "jia.u.cor": [0, -110],
  "ds.yi": [0, 40],
  "yi.u.def": [0, 180],
  "yi.u.prop": [0, 310],
  "yi.u.rem": [0, 510],
  "e.theory": [520, -640],
  "e.ogf_concept": [560, -500],
  "r.seq": [480, -420],
  "r.fps": [480, -260],
  "e.geom_fin": [560, -20],
  "e.geom_inf_fps": [560, 260],
  "d.yi.analytic_radius": [920, -120],
};

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className = "", text = ""): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text) node.textContent = text;
  return node;
}

export class TutorialPlayer {
  readonly root: HTMLElement;
  private readonly titleEl: HTMLElement;
  private readonly episodeButtons: Record<TutorialEpisodeId, HTMLButtonElement>;
  private readonly canvas: HTMLElement;
  private readonly card: HTMLElement;
  private readonly legend: HTMLElement;
  private readonly caption: HTMLElement;
  private readonly counter: HTMLElement;
  private readonly provenance: HTMLElement;
  private readonly playButton: HTMLButtonElement;
  private readonly prevButton: HTMLButtonElement;
  private readonly nextButton: HTMLButtonElement;
  private core: Core | null = null;
  private episode: TutorialEpisodeId = "ep1";
  private readonly projections = new Map<TutorialEpisodeId, TutorialProjection>();
  private projection: TutorialProjection | null = null;
  private shots: TutorialShot[] = [];
  private index = -1;
  private playing = false;
  /** 镜内动画（编号牌右移）的计时器；换镜或关闭时清掉。 */
  private timers: number[] = [];
  /** 自动推进的计时器。 */
  private advanceTimer: number | null = null;

  constructor(host: HTMLElement, private readonly deps: TutorialPlayerDeps = {}) {
    this.root = el("section", "tutorial-overlay");
    this.root.hidden = true;
    this.root.setAttribute("role", "dialog");
    this.root.setAttribute("aria-label", t("tutorial.ep1.menuTitle"));
    const header = el("header", "tutorial-header");
    this.titleEl = el("strong", "tutorial-title", t("tutorial.ep1.menuTitle"));
    const switcher = el("div", "tutorial-episodes");
    switcher.setAttribute("role", "group");
    switcher.setAttribute("aria-label", t("tutorial.switcherLabel"));
    const ep1Btn = el("button", "tutorial-episode-btn is-active", t("tutorial.ep1.short"));
    const ep2Btn = el("button", "tutorial-episode-btn", t("tutorial.ep2.short"));
    ep1Btn.type = "button";
    ep2Btn.type = "button";
    ep1Btn.dataset.episode = "ep1";
    ep2Btn.dataset.episode = "ep2";
    ep1Btn.setAttribute("aria-pressed", "true");
    ep2Btn.setAttribute("aria-pressed", "false");
    ep1Btn.addEventListener("click", () => { if (this.episode !== "ep1") void this.open("ep1"); });
    ep2Btn.addEventListener("click", () => { if (this.episode !== "ep2") void this.open("ep2"); });
    this.episodeButtons = { ep1: ep1Btn, ep2: ep2Btn };
    switcher.append(ep1Btn, ep2Btn);
    header.append(this.titleEl, switcher);
    this.provenance = el("span", "tutorial-provenance");
    this.counter = el("span", "tutorial-counter");
    const close = el("button", "tutorial-close", "关闭教程");
    close.type = "button";
    close.addEventListener("click", () => this.close());
    header.append(this.provenance, this.counter, close);
    const stage = el("div", "tutorial-stage");
    const mapWrap = el("div", "tutorial-map-wrap");
    this.canvas = el("div", "tutorial-map");
    this.canvas.setAttribute("aria-label", "教程地图");
    this.legend = el("div", "map-legend tutorial-legend");
    this.legend.setAttribute("aria-label", "图例");
    mapWrap.append(this.canvas, this.legend);
    this.card = el("aside", "tutorial-card");
    this.card.setAttribute("aria-live", "polite");
    // 说明页里的外部链接（mathlib 的维基百科）交给系统浏览器，不在应用窗口里跳转。
    this.card.addEventListener("click", (event) => {
      const link = (event.target as HTMLElement | null)?.closest<HTMLAnchorElement>("a[data-external]");
      if (!link) return;
      event.preventDefault();
      void openExternalLink(link.href).catch(() => undefined);
    });
    stage.append(mapWrap, this.card);
    this.caption = el("p", "tutorial-caption");
    this.caption.setAttribute("aria-live", "polite");
    const hud = el("footer", "tutorial-hud");
    this.prevButton = el("button", "tutorial-prev", "‹ 上一步");
    this.playButton = el("button", "tutorial-play", "暂停");
    this.nextButton = el("button", "tutorial-next", "下一步 ›");
    for (const button of [this.prevButton, this.playButton, this.nextButton]) button.type = "button";
    this.prevButton.addEventListener("click", () => this.go(this.index - 1));
    this.nextButton.addEventListener("click", () => (this.index >= this.shots.length - 1 ? this.close() : this.go(this.index + 1)));
    this.playButton.addEventListener("click", () => this.setPlaying(!this.playing));
    hud.append(this.prevButton, this.playButton, this.nextButton);
    this.root.append(header, stage, this.caption, hud);
    this.root.addEventListener("keydown", (event) => {
      if (event.key === "Escape") this.close();
      else if (event.key === "ArrowRight") this.go(this.index + 1);
      else if (event.key === "ArrowLeft") this.go(this.index - 1);
    });
    host.append(this.root);
  }

  private reduced(): boolean {
    return (this.deps.reducedMotion ?? defaultReducedMotion)();
  }

  isOpen(): boolean {
    return !this.root.hidden;
  }

  currentShot(): TutorialShot | null {
    return this.shots[this.index] ?? null;
  }

  currentEpisode(): TutorialEpisodeId {
    return this.episode;
  }

  async open(episode: TutorialEpisodeId = "ep1"): Promise<void> {
    this.clearTimers();
    this.episode = episode;
    const activeTitle = episode === "ep2" ? TUTORIAL_EP2_TITLE : t("tutorial.ep1.menuTitle");
    this.titleEl.textContent = activeTitle;
    this.root.setAttribute("aria-label", activeTitle);
    for (const id of ["ep1", "ep2"] as const) {
      const active = id === episode;
      this.episodeButtons[id].classList.toggle("is-active", active);
      this.episodeButtons[id].setAttribute("aria-pressed", String(active));
    }
    this.root.hidden = false;
    this.root.tabIndex = -1;
    this.root.focus();
    let projection = this.projections.get(episode) ?? null;
    if (!projection) {
      this.caption.textContent = "正在载入教程…";
      try {
        const loader = episode === "ep1" && this.deps.load
          ? this.deps.load
          : () => loadEpisodeProjection(episode);
        projection = await loader();
      } catch (error) {
        this.caption.textContent = `教程载入失败：${error instanceof Error ? error.message : String(error)}`;
        return;
      }
      this.projections.set(episode, projection);
    }
    this.projection = projection;
    this.shots = episode === "ep2" ? buildEpisode2Shots(projection) : buildTutorialShots(projection);
    this.core?.destroy();
    this.core = mountCytoscapeGraph(this.canvas, tutorialGraph(this.projection), "semantic", {
      labelHover: { isFull: (node) => node.hasClass(FOCUS) },
    });
    this.decorate(this.core);
    this.applyTutorialStyles(this.core);
    this.core.on("appearancechange", () => {
      if (!this.core) return;
      this.applyTutorialStyles(this.core);
      this.renderLegend(this.core, this.currentShot() ?? undefined);
      refreshLabels(this.core, this.core.elements() as unknown as Parameters<typeof refreshLabels>[1]);
    });
    // 悬停由 mountCytoscapeGraph 绑定：完整标签浮到最上层，邻接短标签暂时避让。
    this.core.autoungrabify(true);
    // 画面上对象少时（前两镜）不要放得太大，节点标签留在画布里。
    this.core.maxZoom(1.4);
    this.go(0);
    this.setPlaying(!this.reduced());
  }

  private applyTutorialStyles(core: Core): void {
    const p = graphPalette();
    const light = getTheme() === "parchment";
    const derivedColor = light ? "#2a7a7c" : "#7fc5c6";
    const derivedPillText = light ? "#1b595b" : "#7fc5c6";
    const derivedPillBorder = light ? "#5e9da3" : "#2f5b5c";
    const bindingColor = p.candidate;
    const bindingPillText = light ? "#4a2f75" : "#ebd9ff";
    const bindingPillBorder = light ? "#8d71b8" : "#6b4ca8";
    const pillText = light ? p.text : "#ffe6a6";
    const pillBorder = light ? "#c59b57" : "#6e5a2a";

    core.style().selector(`.${HIDDEN}`).style({ display: "none" })
      .selector("edge.tutorial-derived-edge").style({
        label: "data(label)", width: 1.6, "line-style": "dashed", "line-color": derivedColor,
        "target-arrow-color": derivedColor, "target-arrow-shape": "triangle", color: derivedColor, "font-size": "9px",
      })
      .selector("edge.tutorial-binding-edge").style({
        "line-style": "dashed", "line-dash-pattern": [8, 4, 2, 4], "line-color": bindingColor,
        "target-arrow-color": bindingColor, color: bindingColor,
      })
      .selector(`.${DIM}`).style({ opacity: 0.35 })
      // 节点：默认一行短名（小字、不换行），焦点 / 悬停才显示〔类型〕+ 全名。
      // 字号、换行宽度、垫底留白在各状态下保持一致：只换文字（syncLabel），
      // 不让 Cytoscape 沿用旧的标签尺寸把新文字裁掉。
      .selector("node").style({
        "font-size": "12px", "text-wrap": "wrap", "text-max-width": "220px",
        "text-background-shape": "roundrectangle", "text-background-padding": "3px", "z-index": 1,
      })
      .selector(`node.${FOCUS}, node.${LABEL_HOVER}`).style(LABEL_HOVER_STYLE)
      .selector(`node.${FOCUS}`).style({ "border-width": 4, "border-color": p.gold, "underlay-color": p.gold, "underlay-opacity": 0.25, "underlay-padding": 6, "z-index": 30 })
      .selector(`node.${SEEN}`).style({ "border-width": 4, "border-color": p.success, "underlay-color": p.success, "underlay-opacity": 0.28, "underlay-padding": 6, "z-index": 32 })
      .selector(`node.${FRESH}`).style({ "border-width": 4, "border-color": p.gold, "underlay-color": p.gold, "underlay-opacity": 0.3, "underlay-padding": 6, "z-index": 32 })
      .selector(`node.${ACCENT}`).style({ "border-width": 3, "border-color": p.accent, "underlay-color": p.accent, "underlay-opacity": 0.22, "underlay-padding": 5, "z-index": 28, opacity: 1 })
      .selector(`node.${ROW_FOCUS}`).style({ "underlay-color": p.accent, "underlay-opacity": light ? 0.28 : 0.35, "underlay-padding": 8, "z-index": 50, opacity: 1 })
      .selector(`node.${LABEL_HOVER}`).style({ "z-index": 1000, "z-index-compare": "manual", "z-compound-depth": "top", opacity: 1 })
      .selector(`.${LABEL_RECESS}`).style(LABEL_RECESS_STYLE)
      // 连线：默认不显示文字；这一镜要读的线（或悬停的线）才显示，水平放、垫与当前主题一致的圆角底，关闭文字描边防止亮色主题反色泛白。
      .selector("edge").style({ label: "", "text-rotation": "none", "z-index": 0 })
      .selector(`edge.${EDGE_FOCUS}`).style({ width: 3.2, opacity: 1 })
      .selector(`edge.${EDGE_LABELED}, edge.${LABEL_HOVER}, edge.${ROW_FOCUS}`).style({
        label: "data(label)", "font-size": "12px", "text-wrap": "wrap", "text-max-width": "220px",
        "text-opacity": 1, "text-outline-width": 0,
        color: pillText, "text-background-color": p.panel, "text-background-opacity": 0.96,
        "text-background-shape": "roundrectangle", "text-background-padding": "3px",
        "text-border-color": pillBorder, "text-border-width": 1, "text-border-opacity": 1, "z-index": 20,
      })
      .selector(`edge.${LABEL_HOVER}`).style({ opacity: 1, "z-index": 1000, "z-index-compare": "manual", "z-compound-depth": "top" })
      .selector(`edge.${ROW_FOCUS}`).style({ opacity: 1, width: 3.4, "z-index": 40 })
      .selector(`edge.tutorial-derived-edge.${EDGE_LABELED}, edge.tutorial-derived-edge.${LABEL_HOVER}, edge.tutorial-derived-edge.${ROW_FOCUS}`).style({
        color: derivedPillText, "text-border-color": derivedPillBorder,
      })
      .selector(`edge.tutorial-binding-edge.${EDGE_LABELED}, edge.tutorial-binding-edge.${LABEL_HOVER}, edge.tutorial-binding-edge.${ROW_FOCUS}`).style({
        color: bindingPillText, "text-border-color": bindingPillBorder,
      })
      .selector(`edge.debt-edge.${EDGE_LABELED}, edge.debt-edge.${LABEL_HOVER}, edge.debt-edge.${ROW_FOCUS}`).style({
        color: p.warning, "text-border-color": p.warning,
      })
      .update();
  }

  close(): void {
    this.clearTimers();
    this.playing = false;
    this.root.hidden = true;
    this.core?.destroy();
    this.core = null;
    this.index = -1;
  }

  private clearTimers(): void {
    for (const timer of this.timers) window.clearTimeout(timer);
    this.timers = [];
    this.clearAdvance();
  }

  private clearAdvance(): void {
    if (this.advanceTimer !== null) window.clearTimeout(this.advanceTimer);
    this.advanceTimer = null;
  }

  private setPlaying(playing: boolean): void {
    this.playing = playing;
    this.playButton.textContent = playing ? "暂停" : "播放";
    this.playButton.hidden = this.reduced();
    this.schedule();
  }

  private schedule(): void {
    this.clearAdvance();
    const shot = this.currentShot();
    if (!shot || !this.playing || this.reduced()) return;
    const ms = (shot.seconds * 1000) / (this.deps.speed ?? 1);
    this.advanceTimer = window.setTimeout(() => {
      if (this.index < this.shots.length - 1) this.go(this.index + 1);
      else this.setPlaying(false);
    }, ms);
  }

  /** local_ref（或 artifact:<key>、derived:<key>）→ 地图上的领域 id。 */
  private domainId(local: string): string | undefined {
    const projection = this.projection!;
    if (local.startsWith("artifact:")) return projection.artifacts[local.slice("artifact:".length)];
    if (local.startsWith("derived:")) return local;
    return projection.refs[local];
  }

  /**
   * 地图标注：每个节点第一行写类型名（共享对象再加类型代码），对齐线写
   * 关系类型代码 + 白话；再补上两条“派生”虚线。只改教程自己的画布。
   */
  private decorate(core: Core): void {
    const projection = this.projection!;
    const byId = new Map(Object.values(projection.objects).map((object) => [object.id, object]));
    core.batch(() => {
      core.nodes().forEach((node: NodeSingular) => {
        const label = String(node.data("label") ?? "");
        const domainKind = node.data("domainKind") as string | null;
        let type = "";
        if (domainKind === "SourceArtifact") type = TYPE_NOTES.material.name;
        else if (domainKind === "SourceSemanticUnit") type = TYPE_NOTES.unit.name;
        else if (domainKind === "DialectScope") type = `${TYPE_NOTES.dialect.name} · dialect scope`;
        else if (domainKind === "Debt") type = `${TYPE_NOTES.debt.name} · debt`;
        else if (domainKind === "RepresentationCore") type = `${EP2_TYPE_NOTES.representationCore.name} · representation core`;
        else if (domainKind === "SemanticEntity") {
          const kind = SEMANTIC_KIND[String(byId.get(node.data("domainId"))?.payload.semantic_kind ?? "")];
          if (kind) type = `${kind.name} · ${kind.code}`;
        }
        if (!type) {
          node.data({ fullLabel: label, compactLabel: truncatePlain(label, COMPACT_CHARS), label: truncatePlain(label, COMPACT_CHARS) });
          return;
        }
        let body = stripTypePrefix(label);
        if (domainKind === "Debt") {
          // 投影标签在 24 个字处截断；焦点时用对象里的原文（转成纯文字）重新截。
          const obligation = byId.get(node.data("domainId"))?.payload.obligation;
          body = typeof obligation === "string"
            ? truncatePlain(texToPlain(obligation), DEBT_FULL_CHARS)
            : label.replace(/^一笔账：/, "");
        }
        // 焦点：〔类型〕+ 全名；其他：一行短名（一笔账只写“一笔账”），悬停看全文。
        const compact = domainKind === "Debt" ? TYPE_NOTES.debt.name : truncatePlain(body, COMPACT_CHARS);
        node.data({ fullLabel: `〔${type}〕\n${body}`, compactLabel: compact, label: compact });
      });
      core.edges(".alignment-edge").forEach((edge) => {
        if (edge.data("domainKind") === "RepresentationBinding") {
          edge.addClass("tutorial-binding-edge");
          edge.data("label", `binds_representation\n${edge.data("label")}`);
          return;
        }
        const relation = String(byId.get(edge.data("relationId"))?.payload.relation ?? "");
        // 两行：关系类型代码 / 白话。
        if (relation) edge.data("label", `${relation}\n${edge.data("label")}`);
      });
      this.layout(core);
      for (const [artifact, unit] of Object.entries(DERIVED)) {
        const source = core.nodes().filter((node) => node.data("domainId") === projection.artifacts[artifact])[0] as NodeSingular | undefined;
        const target = core.nodes().filter((node) => node.data("domainId") === projection.refs[unit])[0] as NodeSingular | undefined;
        if (!source || !target) continue;
        core.add({
          group: "edges", classes: "tutorial-derived-edge",
          data: { id: `derived~${artifact}`, source: source.id(), target: target.id(), label: "派生", relationId: `derived:${artifact}` },
        });
      }
    });
  }

  /** 按对应剧集的固定坐标表摆放；表里没有的节点留在力导向的位置。 */
  private layout(core: Core): void {
    const projection = this.projection!;
    const coords = this.episode === "ep2" ? TUTORIAL_EP2_LAYOUT : TUTORIAL_LAYOUT;
    const local = new Map<string, string>();
    for (const [key, id] of Object.entries(projection.refs)) local.set(id, key);
    for (const [key, id] of Object.entries(projection.artifacts)) local.set(id, `artifact:${key}`);
    core.nodes().forEach((node: NodeSingular) => {
      const key = local.get(String(node.data("domainId") ?? ""));
      const at = key ? coords[key] : undefined;
      if (at) node.position({ x: at[0], y: at[1] });
    });
    // 汇合点（多端关系）放在它连着的节点中间。
    core.nodes(`[elementKind = "junction"]`).forEach((junction: NodeSingular) => {
      const around = junction.neighborhood().nodes();
      if (around.empty()) return;
      const sum = around.reduce((acc, node) => ({ x: acc.x + node.position("x"), y: acc.y + node.position("y") }), { x: 0, y: 0 });
      junction.position({ x: sum.x / around.length, y: sum.y / around.length });
    });
  }

  go(index: number): void {
    if (!this.projection || index < 0 || index >= this.shots.length) return;
    this.clearTimers();
    this.index = index;
    const shot = this.shots[index];
    this.applyMap(index);
    this.renderShot(shot);
    this.prevButton.disabled = index === 0;
    this.nextButton.textContent = index === this.shots.length - 1 ? "结束 ›" : "下一步 ›";
    this.schedule();
  }

  private applyMap(index: number): void {
    const core = this.core;
    if (!core) return;
    const revealed = new Set<string>();
    for (const shot of this.shots.slice(0, index + 1)) {
      for (const local of shot.reveal) {
        const id = this.domainId(local);
        if (id) revealed.add(id);
      }
    }
    const shot = this.shots[index];
    const referenced = new Set(Object.values(this.projection!.refs));
    const focus = new Set(shot.focus.map((local) => this.domainId(local)).filter(Boolean) as string[]);
    const seen = new Set((shot.seen ?? []).map((local) => this.domainId(local)).filter(Boolean) as string[]);
    const fresh = new Set((shot.fresh ?? []).map((local) => this.domainId(local)).filter(Boolean) as string[]);
    const accent = new Set((shot.accent ?? []).map((local) => this.domainId(local)).filter(Boolean) as string[]);
    const emphasized = new Set([...focus, ...seen, ...fresh, ...accent]);
    let relabeled = core.collection();
    const revealedNow = new Set(shot.reveal.map((local) => this.domainId(local)).filter(Boolean) as string[]);
    core.batch(() => {
      core.nodes().forEach((node: NodeSingular) => {
        const domain = node.data("domainId") as string | null;
        const junction = node.data("elementKind") === "junction";
        const visible = junction || (domain !== null && revealed.has(domain));
        const inFocus = domain !== null && focus.has(domain);
        const inEmphasized = domain !== null && emphasized.has(domain);
        node.toggleClass(HIDDEN, !visible);
        node.toggleClass(FOCUS, inFocus);
        node.toggleClass(SEEN, domain !== null && seen.has(domain));
        node.toggleClass(FRESH, domain !== null && fresh.has(domain));
        node.toggleClass(ACCENT, domain !== null && accent.has(domain));
        node.toggleClass(ROW_FOCUS, false);
        node.toggleClass(DIM, emphasized.size > 0 && visible && !inEmphasized);
        if (syncNodeLabel(node, node.hasClass(FOCUS))) relabeled = relabeled.union(node);
      });
      core.edges().forEach((edge) => {
        const relation = edge.data("relationId") as string | undefined;
        const endpointsVisible = !edge.source().hasClass(HIDDEN) && !edge.target().hasClass(HIDDEN);
        // 有 local_ref 的关系（对齐线、书内关系）和两条派生虚线按镜头出现；
        // 投影算出的连线（属于理论、挂账、约定适用）随两端出现。
        const scripted = relation !== undefined && (referenced.has(relation) || relation.startsWith("derived:"));
        const allowed = !relation || !scripted || revealed.has(relation);
        const shown = endpointsVisible && allowed;
        edge.toggleClass(HIDDEN, !shown);
        // 线上加粗：两端都被点亮，或这一镜新出现的对齐线 / 派生线。
        // 线上的文字：只在两端都是焦点时显示（回顾镜共享对象只点亮不展开，避免和单元标签挤在一起）。
        const ends = [edge.source().data("domainId"), edge.target().data("domainId")] as Array<string | null>;
        const bothFocused = ends.every((id) => id !== null && focus.has(id));
        const bothEmphasized = ends.every((id) => id !== null && emphasized.has(id));
        const newHere = scripted && revealedNow.has(relation!);
        edge.toggleClass(EDGE_FOCUS, shown && (newHere || bothEmphasized));
        edge.toggleClass(EDGE_LABELED, shown && (newHere || bothFocused));
        edge.toggleClass(ROW_FOCUS, false);
        edge.toggleClass(DIM, shown && emphasized.size > 0 && !(newHere || bothEmphasized));
      });
      core.nodes(`[elementKind = "junction"]`).forEach((node) => {
        node.toggleClass(HIDDEN, node.connectedEdges().filter(`.${HIDDEN}`).length === node.connectedEdges().length);
      });
    });
    refreshLabels(core, relabeled.nodes() as unknown as Parameters<typeof refreshLabels>[1]);
    this.renderLegend(core, shot);
    const target = core.elements().filter((element) => !element.hasClass(HIDDEN));
    const fitTo = emphasized.size
      ? core.nodes().filter((node) => emphasized.has(node.data("domainId") as string))
      : target;
    if (fitTo.empty()) return;
    const view = this.viewportFor(core, fitTo);
    if (this.reduced()) core.viewport(view);
    else core.animate(view, { duration: SCENE_TIMING.fitMs });
  }

  /**
   * 像 `fit` 一样把焦点（含标签）放进画面。
   * 画布与底部图例各占一行，无需再从画布高度扣掉图例。
   */
  private viewportFor(core: Core, eles: ReturnType<Core["elements"]>): { zoom: number; pan: { x: number; y: number } } {
    const pad = 56;
    const width = this.canvas.clientWidth || core.width();
    const height = this.canvas.clientHeight || core.height();
    // 不用 includeLabels：标签文字刚换过，此时 Cytoscape 量到的还是旧标签的尺寸，
    // 而且会把这个旧尺寸缓存下来，导致新标签被裁掉。按标签的最大尺寸手动留边：
    // 标签在节点上方，最宽 text-max-width（220px）。
    const nodes = eles.boundingBox({ includeLabels: false, includeOverlays: false });
    const box = { x1: nodes.x1 - 110, x2: nodes.x2 + 110, y1: nodes.y1 - 72, y2: nodes.y2 + 8, w: 0, h: 0 };
    box.w = box.x2 - box.x1;
    box.h = box.y2 - box.y1;
    const availW = Math.max(40, width - pad * 2);
    const availH = Math.max(40, height - pad * 2);
    const zoom = Math.min(core.maxZoom(), Math.max(core.minZoom(), Math.min(availW / Math.max(box.w, 1), availH / Math.max(box.h, 1))));
    return {
      zoom,
      pan: { x: width / 2 - zoom * (box.x1 + box.w / 2), y: pad + availH / 2 - zoom * (box.y1 + box.h / 2) },
    };
  }

  /** 图例：和主地图底栏同一套样式，只列当前画面上出现的类型。 */
  private renderLegend(core: Core, shot?: TutorialShot): void {
    const projection = this.projection!;
    const byId = new Map(Object.values(projection.objects).map((object) => [object.id, object]));
    const visibleNodes = core.nodes().filter((node) => !node.hasClass(HIDDEN) && node.data("elementKind") !== "junction");
    const visibleEdges = core.edges().filter((edge) => !edge.hasClass(HIDDEN));
    const nodeTypes = new Set(visibleNodes.map((node) => String(node.data("nodeType"))));
    const sharedKinds = new Set(visibleNodes
      .filter((node) => node.data("domainKind") === "SemanticEntity")
      .map((node) => String(byId.get(node.data("domainId"))?.payload.semantic_kind ?? "")));
    const items: HTMLElement[] = [];
    const node = (key: keyof typeof NODE_TYPES, text: string) => {
      if (nodeTypes.has(key)) items.push(this.legendItem(this.nodeSwatch(NODE_TYPES[key]), text));
    };
    node("markdown", TYPE_NOTES.material.name);
    node("semantic_unit", TYPE_NOTES.unit.name);
    const shared = ["concept", "declaration", "problem_schema"].filter((kind) => sharedKinds.has(kind))
      .map((kind) => `${SEMANTIC_KIND[kind].name} ${kind}`);
    if (nodeTypes.has("shared_entity") && shared.length) {
      items.push(this.legendItem(this.nodeSwatch(NODE_TYPES.shared_entity), `共享对象：${shared.join(" · ")}`));
    }
    node("shared_theory", `${TYPE_NOTES.theory.name} theory`);
    node("representation_core", t("tutorial.legend.representationCore"));
    node("dialect_scope", `${TYPE_NOTES.dialect.name} dialect scope`);
    node("debt", `${TYPE_NOTES.debt.name} debt`);
    const special = ["tutorial-derived-edge", "tutorial-binding-edge", "alignment-edge", "debt-edge", "semantic-link-edge", "evidence-edge"];
    const edge = (match: (classes: string[]) => boolean, color: string, dashed: boolean, text: string) => {
      if (visibleEdges.some((item) => match(item.classes()))) items.push(this.legendItem(this.edgeSwatch(color, dashed), text));
    };
    const has = (name: string) => (classes: string[]) => classes.includes(name);
    const derivedSwatch = getTheme() === "parchment" ? "#2a7a7c" : "#7fc5c6";
    edge(has("tutorial-derived-edge"), derivedSwatch, true, "派生（虚线：来自哪份材料）");
    edge((classes) => classes.includes("semantic-edge") && !special.some((name) => classes.includes(name)), EDGE_TYPES.relation.color, false, "书内关系（箭头表示方向）");
    edge((classes) => classes.includes("alignment-edge") && !classes.includes("tutorial-binding-edge"), EDGE_TYPES.alignment.color, false, "对齐线 alignment（线上：关系类型 · 白话）");
    edge(has("tutorial-binding-edge"), "var(--ms-candidate)", true, t("tutorial.legend.representationBinding"));
    edge(has("semantic-link-edge"), EDGE_TYPES.relation.color, true, "属于理论 / 约定适用于");
    edge(has("debt-edge"), EDGE_TYPES.debt.color, true, "挂账");
    if (shot?.legendNote) {
      const note = el("span", "tutorial-legend-note");
      note.textContent = shot.legendNote;
      items.push(note);
    }
    this.legend.replaceChildren(...items);
  }

  private legendItem(swatch: HTMLElement, label: string): HTMLElement {
    const item = el("span", "legend-item");
    item.append(swatch, document.createTextNode(label));
    return item;
  }

  private nodeSwatch(visual: NodeTypeVisual): HTMLElement {
    const shape = visual.shape === "ellipse" ? "is-round" : visual.shape === "diamond" ? "is-diamond" : "is-rect";
    const swatch = el("span", `legend-dot ${shape} is-${visual.shape}${visual.borderStyle === "dashed" ? " is-dashed" : ""}`);
    swatch.style.backgroundColor = visual.background;
    swatch.style.borderColor = visual.border;
    return swatch;
  }

  private edgeSwatch(color: string, dashed: boolean): HTMLElement {
    const swatch = el("span", `legend-line${dashed ? " is-dashed" : ""}`);
    swatch.style.color = color;
    return swatch;
  }

  private html<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, html: string): HTMLElementTagNameMap[K] {
    const node = el(tag, className);
    node.innerHTML = html;
    return node;
  }

  private renderShot(shot: TutorialShot): void {
    this.counter.textContent = `第 ${this.index + 1} / ${this.shots.length} 镜 · ${shot.title}`;
    this.provenance.textContent = `数据：${shot.provenance.join(" + ")}`;
    this.caption.innerHTML = inlineMathHtml(shot.narration);
    this.card.replaceChildren();
    this.card.dataset.shot = shot.id;
    this.card.append(this.html("h3", "tutorial-card-caption", inlineMathHtml(shot.caption)));
    this.card.append(this.html("strong", "tutorial-card-title", inlineMathHtml(shot.card.title)));
    if (shot.card.lines.length) {
      const ul = el("ul", "tutorial-card-lines");
      for (const line of shot.card.lines) ul.append(this.html("li", "", inlineMathHtml(line)));
      this.card.append(ul);
    }
    if (shot.extra) this.card.append(this.renderExtra(shot.extra));
    for (const tag of shot.tags ?? []) {
      const box = el("div", "tutorial-tag-intro");
      box.append(el("span", `tutorial-learning-label${tag.tag === "新的" ? " is-new" : ""}`, tag.tag), this.html("p", "", inlineMathHtml(tag.text)));
      this.card.append(box);
    }
    if (shot.types.length) {
      const section = el("section", "tutorial-types");
      section.append(el("h4", "", "这一镜出现的类型"));
      const dl = el("dl");
      for (const type of shot.types) {
        const dt = el("dt");
        dt.append(el("span", "tutorial-type-name", type.name), el("code", "tutorial-type-code", type.code));
        dl.append(dt, this.html("dd", "", inlineMathHtml(type.plain)));
      }
      section.append(dl);
      this.card.append(section);
    }
  }

  private renderExtra(extra: ShotExtra): HTMLElement {
    const reduced = this.reduced();
    switch (extra.kind) {
      case "materials": {
        const wrap = el("div", "tutorial-extra tutorial-materials");
        for (const item of extra.items) {
          const page = el("article", "tutorial-material");
          const head = el("header", "tutorial-material-head");
          head.append(el("strong", "", item.book), el("code", "tutorial-material-file", item.file));
          page.append(head, this.html("div", "tutorial-material-body markdown-body", markdownHtml(item.markdown)));
          wrap.append(page);
        }
        return wrap;
      }
      case "sequence": {
        const table = el("table", "tutorial-sequence");
        const head = el("tr");
        head.append(el("th", "", "k"), ...extra.ks.map((k) => el("th", "", String(k))));
        table.append(head);
        for (const row of extra.rows) {
          const tr = el("tr");
          tr.append(el("th", "", row.label));
          row.cells.forEach((cell, i) => {
            const same = extra.rows.every((other) => other.cells[i] === cell);
            tr.append(el("td", cell === null ? "is-blank" : same ? "is-same" : "", cell === null ? "—" : String(cell)));
          });
          table.append(tr);
        }
        const wrap = el("div", "tutorial-extra");
        wrap.append(table, el("p", "tutorial-extra-note", extra.note));
        return wrap;
      }
      case "numbering": {
        const wrap = el("div", `tutorial-extra tutorial-numbering${reduced ? " is-static" : ""}`);
        const formulas = el("div", "tutorial-formulas");
        for (const formula of extra.formulas) {
          const row = el("div", "tutorial-formula");
          row.append(this.html("span", "tutorial-formula-book", inlineMathHtml(`${formula.book}（${formula.range}）`)),
            this.html("span", "tutorial-formula-tex", inlineMathHtml(`$${formula.tex}$`)));
          formulas.append(row);
        }
        wrap.append(formulas);
        const explain = el("ul", "tutorial-formula-explain");
        for (const line of extra.explain) explain.append(this.html("li", "", inlineMathHtml(line)));
        wrap.append(explain, el("p", "tutorial-sequence-fixed", extra.sequenceNote));
        const grid = el("div", "tutorial-plates");
        grid.style.setProperty("--columns", String(extra.shared.length));
        const shared = el("div", "tutorial-plate-row is-shared");
        for (const plate of extra.shared) {
          shared.append(el("span", plate.n === extra.missing.n ? "tutorial-plate is-missing" : "tutorial-plate", plate.text));
        }
        const yi = el("div", "tutorial-plate-row is-yi");
        yi.style.setProperty("--shift", String(extra.shift));
        for (const plate of extra.yi) {
          const matched = extra.shared.find((s) => s.n === plate.matches);
          const ok = matched && matched.text.split(" = ").pop() === plate.text.split(" = ").pop();
          yi.append(el("span", "tutorial-plate", `${plate.text}${ok ? " ✓" : ""}`));
        }
        grid.append(shared, yi);
        wrap.append(grid, this.html("p", "tutorial-missing", inlineMathHtml(`${extra.missing.text} · ${extra.missing.note}`)));
        // 默认：编号牌先放在第 1 列，再整排右移到对应的共享编号下；减少动效时直接放在终位。
        if (reduced) yi.classList.add("is-shifted");
        else this.timers.push(window.setTimeout(() => yi.classList.add("is-shifted"), SCENE_TIMING.holdMs));
        return wrap;
      }
      case "revision": {
        const wrap = el("div", `tutorial-extra tutorial-revision${reduced ? " is-static" : ""}`);
        const v1 = el("div", "tutorial-revision-step is-v1");
        v1.append(el("span", "tutorial-revision-version", "v1"), el("code", "", extra.v1.relation), el("span", "", ` · ${extra.v1.plain}`), el("p", "", extra.v1.note));
        const feedback = el("div", "tutorial-revision-step is-feedback");
        const button = el("button", "secondary-button tutorial-feedback-demo", "反馈给 agent");
        button.type = "button";
        button.disabled = true;
        button.title = "教程里只演示，不会真的发送";
        feedback.append(button, el("p", "", `「${extra.feedback}」`));
        const v2 = el("div", "tutorial-revision-step is-v2");
        v2.append(el("span", "tutorial-revision-version", "v2"), el("code", "", extra.v2.relation), el("span", "", ` · ${extra.v2.plain}`),
          this.html("p", "", inlineMathHtml(`对不上的部分：${extra.v2.lost}`)), el("p", "tutorial-revision-note", extra.v2.note));
        wrap.append(v1, el("span", "tutorial-revision-arrow", "↓"), feedback, el("span", "tutorial-revision-arrow", "↓"), v2,
          this.html("p", "tutorial-extra-note", inlineMathHtml(`为什么改：${extra.why}`)));
        return wrap;
      }
      case "accept": {
        const wrap = el("div", "tutorial-extra tutorial-accept");
        const button = el("button", "primary-button tutorial-accept-demo", extra.button);
        button.type = "button";
        button.disabled = true;
        button.title = "教程里只演示，不会写入地图";
        wrap.append(button, el("p", "tutorial-banner", extra.banner), el("span", "tutorial-badge", extra.badge));
        const ul = el("ul", "tutorial-trust-axes");
        for (const axis of extra.axes) ul.append(el("li", "", axis));
        wrap.append(ul);
        return wrap;
      }
      case "learning": {
        const wrap = el("div", "tutorial-extra tutorial-learning");
        const bind = (node: HTMLElement, focus: string[]) => {
          node.dataset.focus = focus.join(",");
          node.tabIndex = 0;
          node.addEventListener("mouseenter", () => this.highlightRow(node, focus));
          node.addEventListener("mouseleave", () => this.clearRowHighlight());
          node.addEventListener("click", () => this.highlightRow(node, focus));
          node.addEventListener("focus", () => this.highlightRow(node, focus));
          node.addEventListener("blur", () => this.clearRowHighlight());
        };
        const ul = el("ul", "tutorial-learning-units");
        for (const unit of extra.units) {
          const li = el("li");
          li.append(el("span", `tutorial-learning-label${unit.label === "新的" ? " is-new" : ""}`, unit.label), el("span", "", ` ${unit.title}`));
          if (unit.detail) li.append(el("small", "", ` · ${unit.detail}`));
          if (unit.note) li.append(el("small", "tutorial-learning-note", ` · ${unit.note}`));
          bind(li, unit.focus);
          ul.append(li);
        }
        wrap.append(ul);
        for (const convention of extra.conventions) {
          const row = el("p", "tutorial-learning-convention");
          row.append(el("span", "tutorial-learning-label is-convention", "新约定"), this.html("span", "", ` ${inlineMathHtml(convention.text)}`));
          bind(row, convention.focus);
          wrap.append(row);
        }
        const summary = el("p", "tutorial-learning-summary", extra.summary);
        bind(summary, extra.summaryFocus);
        wrap.append(summary);
        if (extra.footerNote) {
          wrap.append(el("p", "tutorial-extra-note tutorial-stage-preview", extra.footerNote));
        }
        return wrap;
      }
      case "symbols": {
        const wrap = el("div", "tutorial-extra tutorial-symbols");
        wrap.append(el("h4", "tutorial-extra-heading", t("tutorial.extra.symbolsHeading")));
        for (const scope of extra.scopes) {
          const section = el("section", "tutorial-symbol-scope");
          section.dataset.scope = scope.scopeLocal;
          section.append(el("strong", "tutorial-symbol-scope-title", scope.title));
          const table = el("table", "tutorial-symbol-table");
          const thead = el("tr");
          thead.append(
            el("th", "", t("tutorial.extra.symbolCol")),
            el("th", "", t("tutorial.extra.roleCol")),
            el("th", "", t("tutorial.extra.domainCol")),
            el("th", "", t("tutorial.extra.meaningCol")),
          );
          table.append(thead);
          for (const row of scope.rows) {
            const tr = el("tr");
            tr.append(
              this.html("td", "is-symbol", inlineMathHtml(row.symbol)),
              el("td", "is-role", row.role),
              this.html("td", "is-domain", inlineMathHtml(row.domain)),
              this.html("td", "is-meaning", inlineMathHtml(row.meaning)),
            );
            table.append(tr);
          }
          section.append(table);
          wrap.append(section);
        }
        return wrap;
      }
      case "bindings": {
        const wrap = el("div", "tutorial-extra tutorial-bindings");
        const coreBox = el("div", "tutorial-binding-core");
        coreBox.append(
          this.html("p", "tutorial-binding-carrier", `${t("tutorial.extra.carrierLabel")}: ${inlineMathHtml(extra.carrier)}`),
        );
        if (extra.operations.length) {
          const ops = el("ul", "tutorial-binding-ops");
          for (const op of extra.operations) ops.append(this.html("li", "", inlineMathHtml(op)));
          coreBox.append(ops);
        }
        wrap.append(coreBox);
        for (const item of extra.items) {
          const card = el("article", "tutorial-binding-card");
          card.dataset.binding = item.bindingLocal;
          const head = el("header", "tutorial-binding-head");
          head.append(el("span", "tutorial-badge", item.book), el("strong", "", item.mapLabel));
          card.append(head);
          const mappings = el("ul", "tutorial-binding-mappings");
          for (const m of item.mappings) {
            mappings.append(this.html("li", "", inlineMathHtml(`${m.symbol} → ${m.targetExpr}${m.note ? ` · ${m.note}` : ""}`)));
          }
          card.append(mappings);
          for (const adapt of item.adaptations) {
            card.append(this.html("p", "tutorial-binding-meta", `${t("tutorial.extra.adaptLabel")}: ${inlineMathHtml(adapt)}`));
          }
          for (const lost of item.lost) {
            card.append(this.html("p", "tutorial-binding-meta is-lost", `${t("tutorial.extra.lostLabel")}: ${inlineMathHtml(lost)}`));
          }
          if (item.debt) {
            card.append(this.html("p", "tutorial-binding-debt", `${t("tutorial.extra.debtLabel")}: ${inlineMathHtml(item.debt)}`));
          }
          wrap.append(card);
        }
        return wrap;
      }
      case "branches": {
        const wrap = el("div", "tutorial-extra tutorial-branches");
        if (extra.partitionNote) {
          const banner = el("div", "tutorial-partition-banner");
          banner.append(
            el("span", "tutorial-badge", t("tutorial.extra.partitionBadge")),
            this.html("span", "", inlineMathHtml(extra.partitionNote)),
          );
          wrap.append(banner);
        }
        if (extra.branches.length) {
          const listWrap = el("div", "tutorial-branch-list");
          for (const branch of extra.branches) {
            const card = el("article", `tutorial-branch-card${branch.isDegenerate ? " is-degenerate" : ""}`);
            card.dataset.branch = branch.branchId;
            const head = el("header", "tutorial-branch-head");
            head.append(
              el("code", "", branch.branchId),
              el("strong", "", branch.label),
              el(
                "span",
                `tutorial-branch-badge${branch.isDegenerate ? " is-degenerate" : ""}`,
                t(branch.isDegenerate ? "semantic.s2.degenerateBadge" : "semantic.s2.regularBadge"),
              ),
              this.html("span", "tutorial-branch-guard", inlineMathHtml(branch.guardPredicate)),
            );
            card.append(head, this.html("div", "tutorial-branch-conclusion", inlineMathHtml(branch.conclusion)));
            listWrap.append(card);
          }
          wrap.append(listWrap);
        }
        if (extra.exclusions.length) {
          const exWrap = el("div", "tutorial-exclusion-list");
          exWrap.append(el("h4", "tutorial-extra-heading", t("semantic.s2.exclusionsHeading")));
          for (const ex of extra.exclusions) {
            const card = el("article", "tutorial-exclusion-card");
            const head = el("div", "tutorial-exclusion-head");
            head.append(el("span", "tutorial-exclusion-badge", ex.kindLabel), this.html("span", "", inlineMathHtml(ex.caseText)));
            card.append(head, this.html("p", "tutorial-extra-note", inlineMathHtml(ex.reason)));
            exWrap.append(card);
          }
          wrap.append(exWrap);
        }
        if (extra.evaluations?.length) {
          const evalWrap = el("div", "tutorial-branch-evals");
          evalWrap.append(el("h4", "tutorial-extra-heading", t("tutorial.extra.evalHeading")));
          for (const ev of extra.evaluations) {
            const row = el("div", "tutorial-eval-row");
            row.append(
              el("strong", "", ev.label),
              this.html("span", "tutorial-eval-formula", inlineMathHtml(ev.formula)),
              this.html("span", "tutorial-extra-note", inlineMathHtml(ev.note)),
            );
            evalWrap.append(row);
          }
          wrap.append(evalWrap);
        }
        return wrap;
      }
    }
  }

  private highlightRow(row: HTMLElement, locals: string[]): void {
    const core = this.core;
    this.card.querySelectorAll(".is-active").forEach((node) => node.classList.remove("is-active"));
    row.classList.add("is-active");
    if (!core) return;
    const ids = new Set(locals.map((local) => this.domainId(local)).filter((id): id is string => Boolean(id)));
    core.batch(() => {
      core.nodes().forEach((node: NodeSingular) => {
        node.toggleClass(ROW_FOCUS, ids.has(node.data("domainId") as string));
      });
      core.edges().forEach((edge) => {
        const relation = edge.data("relationId") as string | undefined;
        const ends = [edge.source().data("domainId"), edge.target().data("domainId")] as Array<string | null>;
        const both = ends.every((id) => id !== null && ids.has(id));
        const related = Boolean(relation && ids.has(relation));
        edge.toggleClass(ROW_FOCUS, !edge.hasClass(HIDDEN) && (both || related));
      });
    });
  }

  private clearRowHighlight(): void {
    this.card.querySelectorAll(".is-active").forEach((node) => node.classList.remove("is-active"));
    const core = this.core;
    if (!core) return;
    core.batch(() => {
      core.elements().forEach((element) => { element.toggleClass(ROW_FOCUS, false); });
    });
  }
}

/** 测试与调试：教程地图的元素（不含隐藏）。 */
export function visibleElementIds(core: Core): ElementDefinition["data"]["id"][] {
  return core.elements().filter((element) => !element.hasClass(HIDDEN)).map((element) => element.id());
}
