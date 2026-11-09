/**
 * Review of Phase2 staged proposals (「提案」 data source).
 *
 * Two entry points share one view:
 * - {@link openProposalReview}: the review panel (per-proposal 接受 / 丢弃).
 * - {@link reviewBeforeLeaving}: the blocking batch dialog shown before a
 *   sandbox session switch, session discard, export or app exit. It resolves
 *   `true` to let the action continue and `false` to cancel it.
 *
 * The backend owns every rule: the investigation id
 * (`get_change_set_investigation`), dependency checks, the plan computed
 * against the current head and the single-commit acceptance. Unchecked
 * proposals stay staged; accepting never raises Trust and does not mean the
 * mathematics is correct.
 */
import { invoke } from "@tauri-apps/api/core";
import { eventBus } from "../core/events";
import { markdownDocument, type AnchoredMarkdown } from "../reader/selection";
import { checkAnchor, renderAnchorFragment, reviewAnchor } from "./anchor-fragment";
import { completenessLabel, relationLabel, unitKindLabel } from "./labels";
import {
  debtLabel, defaultSelection, dependencyGroups, deselect, diagnosticLabel, discardImpact, impactSummary, keyOf,
  proposalDebts, proposalReviewProjection, select, selectionOf, selectionRefs, titleOf,
  type BatchPlan, type ChangeSetQueue, type ChangeSetSelection, type ImpactPreview, type PendingChangeSet,
  type ProposalReviewProjection,
} from "./proposal-model";
import { ReviewGraph } from "./review-graph";

export type ReviewTrigger = "manual" | "switch_session" | "discard" | "export" | "exit";
type Toast = (message: string, kind: "success" | "error") => void;

const TRIGGER_TEXT: Record<ReviewTrigger, { title: string; proceed: string; cancel: string; note: string }> = {
  manual: { title: "审核提案", proceed: "关闭", cancel: "", note: "" },
  switch_session: { title: "切换沙箱前审核提案", proceed: "保留未审批并切换", cancel: "取消切换", note: "切换后仍可回到这个沙箱继续审核。" },
  discard: { title: "丢弃沙箱前审核提案", proceed: "继续丢弃沙箱", cancel: "取消丢弃", note: "丢弃沙箱会删除副本中的全部提案与已接受内容。" },
  export: { title: "导出前审核提案", proceed: "保留未审批并导出", cancel: "取消导出", note: "只有已接受的内容属于正式地图；未审批提案随会话一起导出但不进入正式 head。" },
  exit: { title: "退出前审核提案", proceed: "保留未审批并退出", cancel: "取消退出", note: "未审批提案会保留，下次打开该沙箱可继续审核。" },
};

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className = "", text = ""): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text) node.textContent = text;
  return node;
}

function button(label: string, onClick: () => void, variant = "secondary"): HTMLButtonElement {
  const node = el("button", `sandbox-btn sandbox-btn-${variant}`, label);
  node.type = "button";
  node.onclick = onClick;
  return node;
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** Backend-issued investigation id of a sandbox session (or of this window). */
export function investigationFor(sandboxSessionId: string | null): Promise<string | null> {
  return invoke<string | null>("get_change_set_investigation", { sandboxSessionId });
}

/** Opens the review panel for a session (or this window). */
export async function openProposalReview(sandboxSessionId: string | null, toast: Toast): Promise<void> {
  const investigationId = await investigationFor(sandboxSessionId).catch(error => { toast(message(error), "error"); return null; });
  if (!investigationId) { toast("当前工作区没有调查（只有沙箱会话会产生提案）", "error"); return; }
  await new ProposalReview(investigationId, "manual", toast).run();
}

/**
 * Blocking batch review before `trigger`. Resolves `true` when the action
 * may proceed (also when there is nothing to review or the review cannot be
 * loaded) and `false` when the user cancels it.
 */
export async function reviewBeforeLeaving(sandboxSessionId: string | null, trigger: Exclude<ReviewTrigger, "manual">, toast: Toast): Promise<boolean> {
  let investigationId: string | null;
  try { investigationId = await investigationFor(sandboxSessionId); } catch { return true; }
  if (!investigationId) return true;
  let queue: ChangeSetQueue;
  try { queue = await listQueue(investigationId); } catch { return true; }
  if (queue.items.length === 0) return true;
  return new ProposalReview(investigationId, trigger, toast, queue).run();
}

function listQueue(investigationId: string): Promise<ChangeSetQueue> {
  return invoke<ChangeSetQueue>("list_pending_change_sets", { investigationId });
}

class ProposalReview {
  private readonly overlay = el("div", "change-set-review-overlay");
  private readonly dialog = el("section", "change-set-review-dialog");
  private readonly list = el("div", "change-set-review-list");
  private readonly detail = el("div", "change-set-review-detail");
  private readonly feedback = el("p", "change-set-review-feedback");
  private readonly cascade = el("input");
  private items: PendingChangeSet[] = [];
  private selected = new Set<string>();
  private current: string | null = null;
  private previews = new Map<string, ImpactPreview | string>();
  private graphs: ReviewGraph[] = [];
  private docs = new Map<string, Promise<AnchoredMarkdown | null>>();
  private busy = false;
  private resolve: (proceed: boolean) => void = () => {};

  constructor(
    private readonly investigationId: string,
    private readonly trigger: ReviewTrigger,
    private readonly toast: Toast,
    private queue?: ChangeSetQueue,
  ) {}

  run(): Promise<boolean> {
    const text = TRIGGER_TEXT[this.trigger];
    this.dialog.setAttribute("role", "dialog");
    this.dialog.setAttribute("aria-modal", "true");
    this.dialog.dataset.trigger = this.trigger;
    const header = el("header", "change-set-review-header");
    header.append(
      el("h2", "", text.title),
      el("p", "change-set-review-note", `接受会以一次提交写入当前沙箱副本；未勾选的提案保持「未审批」。接受不提升 Trust，也不代表数学上正确。${text.note}`),
    );
    const body = el("div", "change-set-review-body");
    body.append(this.list, this.detail);
    this.dialog.append(header, body, this.feedback, this.footer());
    this.overlay.append(this.dialog);
    document.body.append(this.overlay);
    const done = new Promise<boolean>(resolve => { this.resolve = resolve; });
    void this.reload(this.queue);
    return done;
  }

  private footer(): HTMLElement {
    const text = TRIGGER_TEXT[this.trigger];
    const actions = el("footer", "change-set-review-actions");
    if (this.trigger === "manual") {
      actions.append(button("关闭", () => this.finish(true), "quiet"));
      return actions;
    }
    this.cascade.type = "checkbox";
    const cascadeLabel = el("label", "change-set-review-cascade");
    cascadeLabel.append(this.cascade, document.createTextNode(" 同时丢弃依赖它们的提案"));
    actions.append(
      button(text.cancel, () => this.finish(false), "quiet"),
      cascadeLabel,
      el("span", "sandbox-actions-spacer"),
      button("丢弃所选", () => void this.discard(selectionRefs(this.items, this.selected), this.cascade.checked), "danger"),
      button("接受所选", () => void this.accept(selectionRefs(this.items, this.selected)), "primary"),
      button(text.proceed, () => this.finish(true), "secondary"),
    );
    return actions;
  }

  private finish(proceed: boolean): void {
    if (this.busy) return;
    this.graphs.forEach(graph => graph.destroy());
    this.overlay.remove();
    this.resolve(proceed);
  }

  private async reload(queue?: ChangeSetQueue): Promise<void> {
    try {
      this.queue = queue ?? await listQueue(this.investigationId);
    } catch (error) {
      this.feedback.textContent = message(error);
      return;
    }
    const previous = new Set(this.items.map(keyOf));
    this.items = this.queue.items;
    // New items start selected; the user's unchecks survive a reload.
    const fresh = defaultSelection(this.items);
    this.selected = new Set([...fresh].filter(key => this.selected.has(key) || !previous.has(key)));
    this.previews.clear();
    if (this.current && !this.items.some(item => keyOf(item) === this.current)) this.current = null;
    this.current ??= this.items[0] ? keyOf(this.items[0]) : null;
    this.renderList();
    this.renderDetail();
    for (const item of this.items) void this.loadPreview(item);
  }

  private async loadPreview(item: PendingChangeSet): Promise<void> {
    const key = keyOf(item);
    if (item.state !== "staged") { this.previews.set(key, "阻塞，不能接受"); this.renderList(); return; }
    try {
      const preview = await invoke<ImpactPreview>("preview_change_set_impact", {
        investigationId: this.investigationId, changeSetId: item.changeSetId, version: item.version,
      });
      this.previews.set(key, preview);
    } catch (error) {
      this.previews.set(key, message(error));
    }
    this.renderList();
  }

  private renderList(): void {
    const groups = dependencyGroups(this.items);
    const table = el("div", "change-set-review-table");
    const head = el("div", "change-set-review-row change-set-review-head");
    head.append(el("span"), el("span", "", "提案"), el("span", "", "状态"), el("span", "", "Debt"), el("span", "", "影响预览"), el("span"));
    table.append(head);
    if (this.items.length === 0) table.append(el("p", "change-set-review-empty", "当前调查没有待审核提案。"));
    groups.forEach((group, index) => {
      if (groups.length > 1 || group.length > 1) {
        table.append(el("div", "change-set-review-group", group.length > 1 ? `依赖组 ${index + 1}（${group.length} 个，上游在前）` : `独立提案`));
      }
      for (const item of group) table.append(this.row(item));
    });
    this.list.replaceChildren(table);
  }

  private row(item: PendingChangeSet): HTMLElement {
    const key = keyOf(item);
    const row = el("div", "change-set-review-row");
    row.classList.toggle("is-current", key === this.current);
    row.dataset.key = key;
    const check = el("input");
    check.type = "checkbox";
    check.checked = this.selected.has(key);
    check.disabled = item.state !== "staged" || this.trigger === "manual";
    check.hidden = this.trigger === "manual";
    check.onclick = event => event.stopPropagation();
    check.onchange = () => {
      if (check.checked) {
        const next = select(this.items, this.selected, key);
        if (next === null) { check.checked = false; this.feedback.textContent = "它依赖的提案已阻塞，不能接受。"; return; }
        this.selected = next;
      } else {
        this.selected = deselect(this.items, this.selected, key);
      }
      this.renderList();
    };
    const title = el("span", "change-set-review-title");
    title.append(el("strong", "", titleOf(item)), el("small", "", ` v${item.version}`));
    if (item.dependencies.length) {
      title.append(el("small", "change-set-review-deps", `依赖 ${item.dependencies.map(dep => {
        const target = this.items.find(candidate => keyOf(candidate) === keyOf(dep));
        return target ? titleOf(target) : `${dep.changeSetId.slice(-8)}@${dep.version}（已处理）`;
      }).join("、")}`));
    }
    const debts = proposalDebts(item);
    const debt = el("span", `change-set-review-debt${debts.length ? " has-debt" : ""}`, debts.length ? `${debts.length} 项` : "无");
    debt.title = debts.map(debtLabel).join("\n");
    const preview = this.previews.get(key);
    const impact = el("span", "change-set-review-impact",
      preview === undefined ? "计算中…" : typeof preview === "string" ? preview : impactSummary(preview.impact));
    if (preview && typeof preview !== "string" && preview.plannedWith.length) impact.title = `与 ${preview.plannedWith.length} 个上游提案一起规划`;
    const actions = el("span", "change-set-review-row-actions");
    if (this.trigger === "manual") {
      const accept = button("接受", () => void this.accept([selectionOf(item)], true), "primary");
      accept.disabled = item.state !== "staged";
      actions.append(accept, button("丢弃", () => void this.discardOne(item), "danger"));
    }
    row.append(check, title, el("span", `change-set-review-state state-${item.state}`, item.state === "staged" ? "未审批" : "阻塞"), debt, impact, actions);
    row.onclick = () => { this.current = key; this.renderList(); this.renderDetail(); };
    return row;
  }

  private renderDetail(): void {
    this.graphs.forEach(graph => graph.destroy());
    this.graphs = [];
    const item = this.items.find(candidate => keyOf(candidate) === this.current);
    if (!item) { this.detail.replaceChildren(el("p", "change-set-review-empty", "选择左侧提案查看详情。")); return; }
    const children: HTMLElement[] = [el("h3", "", titleOf(item)), el("p", "change-set-review-meta", `${item.proposedBy} · round ${item.roundId} · v${item.version}`)];
    const debts = proposalDebts(item);
    if (debts.length) {
      const list = el("ul", "change-set-review-debts");
      debts.forEach(debt => list.append(el("li", "", debtLabel(debt))));
      children.push(el("h4", "", "Debt"), list);
    }
    for (const group of proposalReviewProjection(item)) children.push(this.groupView(group));
    this.detail.replaceChildren(...children);
    this.graphs.forEach(graph => graph.resize());
  }

  private groupView(group: ProposalReviewProjection): HTMLElement {
    const section = el("section", "change-set-review-snapshot");
    section.append(el("h4", "", group.source.artifact_id ? `快照 ${group.source.artifact_id.slice(-12)}` : "引用已有证据"));
    if (group.units.length) {
      const host = el("div", "review-graph change-set-review-graph");
      section.append(host);
      queueMicrotask(() => {
        if (!host.isConnected) return;
        const graph = new ReviewGraph(
          host,
          group.units.map(unit => ({ ref: unit.unit_ref, label: unit.display_title ?? unit.unit_ref, state: "selectable" as const })),
          group.structure_relations.map(edge => ({ parent: edge.parent, child: edge.child })),
          group.relations.map(relation => ({ id: relation.relation_ref, source: relation.source, target: relation.target, label: relationLabel(relation.relation) })),
          { unit: () => {}, relation: () => {}, evidence: () => {}, leave: () => {} },
        );
        this.graphs.push(graph);
        graph.resize();
      });
    }
    for (const unit of group.units) {
      const card = el("article", "change-set-review-unit");
      card.append(el("strong", "", `${unitKindLabel(unit.unit_kind)} · ${unit.display_title ?? unit.unit_ref}`),
        el("small", "", ` ${unit.acceptance === "edit" ? "修改" : "新建"} · ${completenessLabel(unit.completeness)}`));
      for (const anchor of unit.anchors) card.append(this.anchorView(group.source.artifact_id, anchor));
      section.append(card);
    }
    for (const relation of group.relations) {
      section.append(el("p", "change-set-review-relation", `${relation.source} —${relationLabel(relation.relation)}→ ${relation.target}`));
    }
    for (const diagnostic of group.diagnostics) {
      section.append(el("p", `change-set-review-diagnostic level-${diagnostic.severity}`, `${diagnosticLabel(diagnostic.code)}${diagnostic.path ? `（${diagnostic.path}）` : ""}：${diagnostic.message}`));
    }
    return section;
  }

  private anchorView(artifactId: string | null, anchor: ProposalReviewProjection["units"][number]["anchors"][number]): HTMLElement {
    const host = el("div", "change-set-review-anchor");
    host.append(el("blockquote", "", anchor.excerpt || `[${anchor.range.start}, ${anchor.range.end})`));
    if (!artifactId) return host;
    let doc = this.docs.get(artifactId);
    if (!doc) { doc = markdownDocument(artifactId, null).catch(() => null); this.docs.set(artifactId, doc); }
    void doc.then(loaded => {
      if (!loaded || !host.isConnected) return;
      const bytes = new TextEncoder().encode(loaded.source);
      const review = reviewAnchor(anchor);
      const check = checkAnchor(loaded, bytes, review);
      host.replaceChildren(check.ok
        ? renderAnchorFragment(loaded, bytes, review, check.blocks)
        : el("p", "review-stale", `锚点无法在本窗口的快照中定位：${check.reason}`));
    });
    return host;
  }

  private async accept(selections: ChangeSetSelection[], single = false): Promise<void> {
    if (this.busy) return;
    if (!selections.length) { this.feedback.textContent = "请选择至少一个未审批提案。"; return; }
    this.busy = true;
    this.feedback.textContent = "正在按当前地图重新规划…";
    try {
      const plan = await invoke<BatchPlan>("plan_change_set_batch", { investigationId: this.investigationId, selections });
      if (plan.impact.status !== "ready") {
        this.feedback.textContent = impactSummary(plan.impact);
        return;
      }
      await invoke("accept_change_set_batch", {
        investigationId: this.investigationId,
        selections: plan.selections,
        baseCommit: plan.baseCommit,
        batchPlanFingerprint: plan.batchPlanFingerprint,
      });
      this.toast(single ? "已接受提案" : `已在一次提交中接受 ${plan.selections.length} 个提案`, "success");
      this.feedback.textContent = "";
      eventBus.emit("source-map-changed", { reason: "change-set-accepted" });
    } catch (error) {
      this.feedback.textContent = `接受失败：${message(error)}`;
    } finally {
      this.busy = false;
    }
    await this.reload();
  }

  private async discardOne(item: PendingChangeSet): Promise<void> {
    const impact = discardImpact(this.items, [keyOf(item)], false);
    let cascade = false;
    if (impact.blocked.size) {
      cascade = window.confirm(`有 ${impact.blocked.size} 个提案依赖它。\n确定：一并丢弃它们（逐条记录）。\n取消：只丢弃这一个，依赖方转为阻塞。`);
    }
    await this.discard([selectionOf(item)], cascade);
  }

  private async discard(selections: ChangeSetSelection[], cascade: boolean): Promise<void> {
    if (this.busy) return;
    if (!selections.length) { this.feedback.textContent = "请选择至少一个提案。"; return; }
    this.busy = true;
    try {
      const outcome = await invoke<{ discarded: unknown[]; blocked: unknown[] }>("discard_change_set_batch", {
        investigationId: this.investigationId, selections, cascade, reason: null,
      });
      this.toast(`已丢弃 ${outcome.discarded.length} 个提案${outcome.blocked.length ? `，${outcome.blocked.length} 个依赖方转为阻塞` : ""}`, "success");
      eventBus.emit("source-map-changed", { reason: "change-set-discarded" });
    } catch (error) {
      this.feedback.textContent = `丢弃失败：${message(error)}`;
    } finally {
      this.busy = false;
    }
    await this.reload();
  }
}
