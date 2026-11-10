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
  debtLabel, defaultSelection, dependencyGroups, deselect, diagnosticLabel, discardPreview, impactSummary, keyOf,
  proposalDebts, proposalDiscoveryOutcome, proposalReviewProjection, select, selectionOf, selectionRefs, titleOf,
  type BatchPlan, type ChangeSetQueue, type ChangeSetSelection, type ImpactPreview, type PendingChangeSet,
  type ProposalReviewProjection,
} from "./proposal-model";
import { ReviewGraph } from "./review-graph";
import { openReviewWorkspace } from "./workspace";

export type ReviewTrigger = "manual" | "switch_session" | "discard" | "export" | "exit";
type Toast = (message: string, kind: "success" | "error") => void;

const TRIGGER_TEXT: Record<ReviewTrigger, { title: string; proceed: string; cancel: string; note: string }> = {
  manual: { title: "审核提案", proceed: "关闭", cancel: "", note: "" },
  switch_session: { title: "切换沙箱前审核提案", proceed: "保留未审批并切换", cancel: "取消切换", note: "切换后仍可回到这个沙箱继续审核。" },
  discard: { title: "丢弃沙箱前审核提案", proceed: "继续丢弃沙箱", cancel: "取消丢弃", note: "丢弃沙箱会删除副本中的全部提案与已接受内容。" },
  export: { title: "导出前审核提案", proceed: "保留未审批并导出", cancel: "取消导出", note: "导出包只含对话记录与事件，不含地图或提案；未审批提案留在沙箱副本中，之后仍可审核。" },
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
  private readonly summary = el("p", "change-set-review-summary");
  private readonly confirmHost = el("div", "change-set-review-confirm");
  private summaryGeneration = 0;
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
    this.summary.hidden = this.trigger === "manual";
    this.confirmHost.hidden = true;
    this.dialog.append(header, this.summary, body, this.confirmHost, this.feedback, this.footer());
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
    actions.append(
      button(text.cancel, () => this.finish(false), "quiet"),
      el("span", "sandbox-actions-spacer"),
      button("丢弃所选", () => void this.discardKeys([...this.selected]), "danger"),
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
    void this.refreshSummary();
    for (const item of this.items) void this.loadPreview(item);
  }

  /** Total impact of the current selection, planned as one batch against the head. */
  private async refreshSummary(): Promise<void> {
    if (this.trigger === "manual") return;
    const generation = ++this.summaryGeneration;
    const selections = selectionRefs(this.items, this.selected);
    if (!selections.length) { this.summary.textContent = "所选合计：未选择提案。"; return; }
    this.summary.textContent = `所选 ${selections.length} 个提案合计：计算中…`;
    try {
      const plan = await invoke<BatchPlan>("plan_change_set_batch", { investigationId: this.investigationId, selections });
      if (generation !== this.summaryGeneration) return;
      this.summary.textContent = `所选 ${selections.length} 个提案合计（一次提交）：${impactSummary(plan.impact)}`;
    } catch (error) {
      if (generation !== this.summaryGeneration) return;
      this.summary.textContent = `所选合计：${message(error)}`;
    }
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
      void this.refreshSummary();
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
    const heading = el("div", "change-set-review-snapshot-heading");
    heading.append(el("h4", "", group.source.artifact_id ? `快照 ${group.source.artifact_id.slice(-12)}` : "引用已有证据"));
    const item = this.items.find(candidate => keyOf(candidate) === this.current);
    // The blocking dialog must not be dismissed by opening the workspace, so
    // the full workspace is offered from the review panel only.
    if (this.trigger === "manual" && item && group.source.artifact_id && group.units.length) {
      heading.append(button("在审核工作区打开", () => void this.openInWorkspace(item, group), "quiet"));
    }
    section.append(heading);
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

  /** Opens one snapshot group in the discovery review workspace, read-only. */
  private async openInWorkspace(item: PendingChangeSet, group: ProposalReviewProjection): Promise<void> {
    const artifactId = group.source.artifact_id;
    if (!artifactId) return;
    const doc = await this.document(artifactId);
    if (!doc) { this.feedback.textContent = "无法读取该提案引用的 Markdown 快照。"; return; }
    this.finish(true);
    openReviewWorkspace(proposalDiscoveryOutcome(item, group, doc.snapshot.content_hash), {
      title: titleOf(item),
      proposal: { changeSetId: item.changeSetId, version: item.version, state: item.state, proposedBy: item.proposedBy, snapshot: artifactId },
    });
  }

  private document(artifactId: string): Promise<AnchoredMarkdown | null> {
    let doc = this.docs.get(artifactId);
    if (!doc) { doc = markdownDocument(artifactId, null).catch(() => null); this.docs.set(artifactId, doc); }
    return doc;
  }

  private anchorView(artifactId: string | null, anchor: ProposalReviewProjection["units"][number]["anchors"][number]): HTMLElement {
    const host = el("div", "change-set-review-anchor");
    host.append(el("blockquote", "", anchor.excerpt || `[${anchor.range.start}, ${anchor.range.end})`));
    if (!artifactId) return host;
    void this.document(artifactId).then(loaded => {
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
    await this.discardKeys([keyOf(item)]);
  }

  /**
   * Discard preview: lists the proposals that depend on `keys` and lets the
   * user choose. Default (unchecked) turns the dependents blocked; checking
   * 「连带丢弃」 discards them too. Nothing happens until 「确认丢弃」.
   */
  private async discardKeys(keys: string[]): Promise<void> {
    if (this.busy) return;
    const preview = discardPreview(this.items, keys);
    if (!preview.roots.length) { this.feedback.textContent = "请选择至少一个提案。"; return; }
    const choice = await this.askDiscard(preview);
    if (choice === null) return;
    await this.discard(preview.roots.map(selectionOf), choice.cascade);
  }

  private askDiscard(preview: ReturnType<typeof discardPreview>): Promise<{ cascade: boolean } | null> {
    return new Promise(resolve => {
      const box = this.confirmHost;
      const names = (items: PendingChangeSet[]) => items.map(item => `${titleOf(item)} v${item.version}`).join("、");
      const children: HTMLElement[] = [el("p", "", `将丢弃：${names(preview.roots)}。丢弃只改提案状态，不写正式地图。`)];
      const cascade = el("input");
      cascade.type = "checkbox";
      if (preview.dependents.length) {
        children.push(el("p", "change-set-review-confirm-dependents", `依赖它们的提案（${preview.dependents.length} 个，含间接依赖）：${names(preview.dependents)}`));
        const label = el("label", "change-set-review-cascade");
        label.append(cascade, document.createTextNode(" 连带丢弃这些依赖提案（不勾选则它们转为「阻塞」，等待 Agent 重新提交）"));
        children.push(label);
      }
      const done = (value: { cascade: boolean } | null) => { box.hidden = true; box.replaceChildren(); resolve(value); };
      const actions = el("div", "change-set-review-confirm-actions");
      actions.append(button("取消", () => done(null), "quiet"), button("确认丢弃", () => done({ cascade: cascade.checked }), "danger"));
      children.push(actions);
      box.replaceChildren(...children);
      box.hidden = false;
    });
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
