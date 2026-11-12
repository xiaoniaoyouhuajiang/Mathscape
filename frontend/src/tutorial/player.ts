/**
 * 教程播放器：在地图区上方的独立画布里，用主地图同一套 Cytoscape 样式与
 * 场景播放器的镜头节奏（SCENE_TIMING）回放 11 镜。
 *
 * - 只读：不调用任何 Tauri 命令，不碰用户的地图实例和工作区。
 * - 「减少动效」：状态瞬间切换，不做镜头滑动和编号牌移动，不自动推进，
 *   HUD 显示「下一步 ›」；字幕照常。
 */
import type { Core, ElementDefinition, NodeSingular } from "cytoscape";
import { reducedMotion as defaultReducedMotion } from "../core/motion";
import { mountCytoscapeGraph } from "../graph/cytoscape";
import { SCENE_TIMING } from "../graph/scene-player";
import { inlineMathHtml } from "./math";
import type { TutorialProjection } from "./projection";
import { loadTutorialProjection, tutorialGraph } from "./projection";
import type { ShotExtra, TutorialShot } from "./script";
import { buildTutorialShots } from "./script";

export const TUTORIAL_TITLE = "教程：两本教材是不是同一颗星";

export interface TutorialPlayerDeps {
  load?: () => Promise<TutorialProjection>;
  reducedMotion?: () => boolean;
  /** 自动推进的倍速（测试用）；默认 1。 */
  speed?: number;
}

const HIDDEN = "tutorial-hidden";
const FOCUS = "tutorial-focus";
const DIM = "tutorial-dim";

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className = "", text = ""): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text) node.textContent = text;
  return node;
}

export class TutorialPlayer {
  readonly root: HTMLElement;
  private readonly canvas: HTMLElement;
  private readonly card: HTMLElement;
  private readonly caption: HTMLElement;
  private readonly counter: HTMLElement;
  private readonly provenance: HTMLElement;
  private readonly playButton: HTMLButtonElement;
  private readonly prevButton: HTMLButtonElement;
  private readonly nextButton: HTMLButtonElement;
  private core: Core | null = null;
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
    this.root.setAttribute("aria-label", TUTORIAL_TITLE);
    const header = el("header", "tutorial-header");
    header.append(el("strong", "tutorial-title", TUTORIAL_TITLE));
    this.provenance = el("span", "tutorial-provenance");
    this.counter = el("span", "tutorial-counter");
    const close = el("button", "tutorial-close", "关闭教程");
    close.type = "button";
    close.addEventListener("click", () => this.close());
    header.append(this.provenance, this.counter, close);
    const stage = el("div", "tutorial-stage");
    this.canvas = el("div", "tutorial-map");
    this.canvas.setAttribute("aria-label", "教程地图（只读）");
    this.card = el("aside", "tutorial-card");
    this.card.setAttribute("aria-live", "polite");
    stage.append(this.canvas, this.card);
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
    const note = el("span", "tutorial-note", "只读教程 · 示例材料（虚构教材）· 不会写入你的工作区");
    hud.append(this.prevButton, this.playButton, this.nextButton, note);
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

  async open(): Promise<void> {
    this.root.hidden = false;
    this.root.tabIndex = -1;
    this.root.focus();
    if (!this.projection) {
      this.caption.textContent = "正在载入教程…";
      try {
        this.projection = await (this.deps.load ?? loadTutorialProjection)();
      } catch (error) {
        this.caption.textContent = `教程载入失败：${error instanceof Error ? error.message : String(error)}`;
        return;
      }
      this.shots = buildTutorialShots(this.projection);
    }
    this.core?.destroy();
    this.core = mountCytoscapeGraph(this.canvas, tutorialGraph(this.projection), "semantic");
    this.core.style().selector(`.${HIDDEN}`).style({ display: "none" })
      .selector(`.${DIM}`).style({ opacity: 0.35 })
      .selector(`node.${FOCUS}`).style({ "border-width": 4, "border-color": "#ffd89a", "underlay-color": "#ffd89a", "underlay-opacity": 0.25, "underlay-padding": 6 })
      .update();
    this.core.autoungrabify(true);
    this.go(0);
    this.setPlaying(!this.reduced());
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

  /** local_ref（或 artifact:<key>）→ 地图上的领域 id。 */
  private domainId(local: string): string | undefined {
    const projection = this.projection!;
    if (local.startsWith("artifact:")) return projection.artifacts[local.slice("artifact:".length)];
    return projection.refs[local];
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
    const referenced = new Set(Object.values(this.projection!.refs));
    const focus = new Set(this.shots[index].focus.map((local) => this.domainId(local)).filter(Boolean) as string[]);
    core.batch(() => {
      core.nodes().forEach((node: NodeSingular) => {
        const domain = node.data("domainId") as string | null;
        const junction = node.data("elementKind") === "junction";
        const visible = junction || (domain !== null && revealed.has(domain));
        node.toggleClass(HIDDEN, !visible);
        node.toggleClass(FOCUS, domain !== null && focus.has(domain));
        node.toggleClass(DIM, focus.size > 0 && visible && !(domain !== null && focus.has(domain)));
      });
      core.edges().forEach((edge) => {
        const relation = edge.data("relationId") as string | undefined;
        const endpointsVisible = !edge.source().hasClass(HIDDEN) && !edge.target().hasClass(HIDDEN);
        // 有 local_ref 的关系（对齐线、书内关系）按镜头出现；派生的连线（星区、挂账、约定适用）随两端出现。
        const allowed = !relation || !referenced.has(relation) || revealed.has(relation);
        edge.toggleClass(HIDDEN, !(endpointsVisible && allowed));
      });
      core.nodes(`[elementKind = "junction"]`).forEach((node) => {
        node.toggleClass(HIDDEN, node.connectedEdges().filter(`.${HIDDEN}`).length === node.connectedEdges().length);
      });
    });
    const target = core.elements().filter((element) => !element.hasClass(HIDDEN));
    const fitTo = focus.size
      ? core.nodes().filter((node) => focus.has(node.data("domainId") as string))
      : target;
    if (fitTo.empty()) return;
    if (this.reduced()) core.fit(fitTo, 48);
    else core.animate({ fit: { eles: fitTo, padding: 48 } }, { duration: SCENE_TIMING.fitMs });
  }

  private renderShot(shot: TutorialShot): void {
    this.counter.textContent = `第 ${this.index + 1} / ${this.shots.length} 镜 · ${shot.title}`;
    this.provenance.textContent = `数据：${shot.provenance.join(" + ")}`;
    this.caption.textContent = shot.narration;
    this.card.replaceChildren();
    this.card.dataset.shot = shot.id;
    this.card.append(el("h3", "tutorial-card-caption", shot.caption));
    const title = el("strong", "tutorial-card-title");
    title.innerHTML = inlineMathHtml(shot.card.title);
    this.card.append(title);
    if (shot.card.lines.length) {
      const ul = el("ul", "tutorial-card-lines");
      for (const line of shot.card.lines) {
        const li = el("li");
        li.innerHTML = inlineMathHtml(line);
        ul.append(li);
      }
      this.card.append(ul);
    }
    if (shot.extra) this.card.append(this.renderExtra(shot.extra));
  }

  private renderExtra(extra: ShotExtra): HTMLElement {
    const reduced = this.reduced();
    switch (extra.kind) {
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
        wrap.append(el("p", "tutorial-sequence-fixed", extra.sequenceNote));
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
        wrap.append(grid, el("p", "tutorial-missing", `${extra.missing.text} · ${extra.missing.note}`));
        // 默认：编号牌先放在第 1 列，再整排右移到对应的共享编号下；减少动效时直接放在终位。
        if (reduced) yi.classList.add("is-shifted");
        else this.timers.push(window.setTimeout(() => yi.classList.add("is-shifted"), SCENE_TIMING.holdMs));
        return wrap;
      }
      case "merge": {
        const wrap = el("div", `tutorial-extra tutorial-merge${reduced ? " is-static" : ""}`);
        for (const branch of extra.branches) {
          const card = el("div", "tutorial-merge-branch");
          card.append(el("span", "tutorial-merge-x", "✕"), el("strong", "", branch.title), el("p", "", branch.reason));
          wrap.append(card);
        }
        return wrap;
      }
      case "learning": {
        const wrap = el("div", "tutorial-extra tutorial-learning");
        wrap.append(el("p", "tutorial-banner", extra.banner), el("span", "tutorial-badge", extra.badge));
        const ul = el("ul", "tutorial-learning-units");
        for (const unit of extra.units) {
          const li = el("li");
          li.append(el("span", `tutorial-learning-label${unit.label === "新的" ? " is-new" : ""}`, unit.label), el("span", "", ` ${unit.title}`));
          if (unit.detail) li.append(el("small", "", ` · ${unit.detail}`));
          ul.append(li);
        }
        wrap.append(ul);
        for (const text of extra.conventions) wrap.append(el("p", "tutorial-learning-convention", text));
        wrap.append(el("p", "tutorial-learning-summary", extra.summary));
        return wrap;
      }
    }
  }
}

/** 测试与调试：教程地图的元素（不含隐藏）。 */
export function visibleElementIds(core: Core): ElementDefinition["data"]["id"][] {
  return core.elements().filter((element) => !element.hasClass(HIDDEN)).map((element) => element.id());
}
