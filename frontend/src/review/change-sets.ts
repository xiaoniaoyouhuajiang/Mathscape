/**
 * Review of Phase2 staged proposals (「提案」 data source).
 *
 * Two entry points:
 * - {@link openProposalReview}: manual review. Opens the full-screen review
 *   workspace in proposal mode (two columns, tabs, highlighted anchors,
 *   KaTeX, click-to-locate) with 「接受 / 丢弃本提案」 for the whole change set.
 * - {@link reviewBeforeLeaving}: the blocking batch dialog shown before a
 *   sandbox session switch, session discard, export or app exit. It resolves
 *   `true` to let the action continue and `false` to cancel it.
 *
 * The backend owns every rule: the investigation id
 * (`get_change_set_investigation`), the anchors (`list_pending_change_sets`),
 * the snapshot read (`get_change_set_snapshot`), dependency checks, the plan
 * computed against the current head and the single-commit acceptance.
 * Unchecked proposals stay staged; accepting never raises Trust and does not
 * mean the mathematics is correct.
 */
import { invoke } from "@tauri-apps/api/core";
import { eventBus } from "../core/events";
import type { AnchoredMarkdown } from "../reader/selection";
import {
  debtLabel, defaultSelection, dependencyGroups, deselect, discardPreview, impactSummary, keyOf,
  proposalDebts, proposalDiscoveryOutcome, proposalOverview, proposalReviewProjection, select, selectionOf, selectionRefs, stateLabel, titleOf,
  type BatchPlan, type ChangeSetQueue, type ChangeSetSelection, type ImpactPreview, type PendingChangeSet,
  type ProposalDecision, type ProposalWorkspaceController,
} from "./proposal-model";
import { renderDiscardConfirm, type DiscardChoice } from "./proposal-discard";
import { findProposalTargetForDomainObject } from "./map-review-sync";
import { openEmptyProposalReview, openReviewWorkspace } from "./workspace";
import "./review.css";
import "./change-sets.css";

export type ReviewTrigger = "switch_session" | "discard" | "export" | "exit";
type Toast = (message: string, kind: "success" | "error") => void;

const TRIGGER_TEXT: Record<ReviewTrigger, { title: string; proceed: string; cancel: string; note: string }> = {
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

function button(label: string, onClick: () => void, className = ""): HTMLButtonElement {
  const node = el("button", className, label);
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

function listQueue(investigationId: string): Promise<ChangeSetQueue> {
  return invoke<ChangeSetQueue>("list_pending_change_sets", { investigationId });
}

/** Plans `selections` against the current head and accepts them in one commit. */
async function acceptSelections(investigationId: string, selections: ChangeSetSelection[]): Promise<BatchPlan> {
  const plan = await invoke<BatchPlan>("plan_change_set_batch", { investigationId, selections });
  if (plan.impact.status !== "ready") throw new Error(impactSummary(plan.impact));
  await invoke("accept_change_set_batch", {
    investigationId,
    selections: plan.selections,
    baseCommit: plan.baseCommit,
    batchPlanFingerprint: plan.batchPlanFingerprint,
  });
  eventBus.emit("source-map-changed", { reason: "change-set-accepted" });
  return plan;
}

async function discardSelections(investigationId: string, selections: ChangeSetSelection[], cascade: boolean): Promise<string> {
  const outcome = await invoke<{ discarded: unknown[]; blocked: unknown[] }>("discard_change_set_batch", {
    investigationId, selections, cascade, reason: null,
  });
  eventBus.emit("source-map-changed", { reason: "change-set-discarded" });
  return `已丢弃 ${outcome.discarded.length} 个提案${outcome.blocked.length ? `，${outcome.blocked.length} 个依赖方转为阻塞` : ""}`;
}

export interface OpenProposalReviewOptions {
  changeSetId?: string | null;
  objectId?: string | null;
  mapLinked?: boolean;
}

/** Opens manual review of a session (or this window) in the review workspace. */
export async function openProposalReview(
  sandboxSessionId: string | null,
  toast: Toast,
  options?: OpenProposalReviewOptions,
): Promise<void> {
  let investigationId: string | null;
  try { investigationId = await investigationFor(sandboxSessionId); } catch (error) { toast(message(error), "error"); return; }
  if (!investigationId) { toast("当前工作区没有调查（只有沙箱会话会产生提案）", "error"); return; }
  const session = new ProposalWorkspaceSession(investigationId, options?.mapLinked);
  try {
    await session.reload();
  } catch (error) {
    toast(`无法读取提案：${message(error)}`, "error");
    return;
  }
  const first = session.items[0];
  // Open the workspace on an explicit empty state: a toast alone is easy to
  // miss (the dialogue dock can sit on top of it).
  if (!first) { openEmptyProposalReview(investigationId); return; }
  if (options?.objectId || options?.changeSetId) {
    const located = findProposalTargetForDomainObject(session.items, options.objectId, options.changeSetId);
    if (located) {
      await session.open(located.changeSetKey, undefined, located.focusTarget ?? undefined);
      return;
    }
  }
  await session.open(keyOf(first));
}

/**
 * Drives the review workspace for one investigation: reads the queue and
 * the snapshots through the backend, and routes 接受 / 丢弃 to the batch
 * commands (a single accept is a batch of one).
 */
class ProposalWorkspaceSession implements ProposalWorkspaceController {
  items: PendingChangeSet[] = [];
  private docs = new Map<string, Promise<{ document: AnchoredMarkdown; conversionId: string | null }>>();

  constructor(readonly investigationId: string, private initialMapLinked?: boolean) {}

  async reload(): Promise<void> {
    this.items = (await listQueue(this.investigationId)).items;
  }

  /** The snapshot as the backend reads it for this investigation; failures keep the real reason. */
  private snapshot(artifactId: string): Promise<{ document: AnchoredMarkdown; conversionId: string | null }> {
    let doc = this.docs.get(artifactId);
    if (!doc) {
      doc = invoke<{ document: AnchoredMarkdown; conversionId: string | null }>("get_change_set_snapshot", { investigationId: this.investigationId, artifactId });
      doc.catch(() => this.docs.delete(artifactId));
      this.docs.set(artifactId, doc);
    }
    return doc;
  }

  async open(key: string, groupIndex?: number, focusTarget?: string): Promise<void> {
    const item = this.items.find(candidate => keyOf(candidate) === key) ?? this.items[0];
    if (!item) return;
    const groups = proposalReviewProjection(item);
    const overview = proposalOverview(item, groups);
    const index = Math.min(Math.max(groupIndex ?? overview.defaultGroup, 0), groups.length - 1);
    const group = groups[index];
    let document: AnchoredMarkdown | null = null;
    let documentError: string | null = null;
    if (group.source.artifact_id) {
      try {
        const loaded = await this.snapshot(group.source.artifact_id);
        document = loaded.document;
        group.source.conversion_id = loaded.conversionId;
      } catch (error) {
        documentError = `无法读取提案引用的 Markdown 快照：${message(error)}`;
      }
    } else {
      documentError = "该提案没有 Markdown 锚点（只引用了手稿证据或不含证据）。";
    }
    const initialMapLinked = this.initialMapLinked;
    this.initialMapLinked = undefined;
    openReviewWorkspace(proposalDiscoveryOutcome(item, group, document?.snapshot.content_hash ?? ""), {
      title: titleOf(item),
      ...(initialMapLinked !== undefined ? { mapLinked: initialMapLinked } : {}),
      proposal: {
        changeSetId: item.changeSetId, version: item.version, state: item.state, proposedBy: item.proposedBy,
        snapshot: group.source.artifact_id, item, items: this.items, groupIndex: index, groupCount: groups.length, overview,
        document, documentError, anchors: group.anchors, focusTarget: focusTarget ?? null, controller: this,
      },
    });
  }

  preview(item: PendingChangeSet): Promise<ImpactPreview> {
    return invoke<ImpactPreview>("preview_change_set_impact", {
      investigationId: this.investigationId, changeSetId: item.changeSetId, version: item.version,
    });
  }

  async accept(item: PendingChangeSet): Promise<ProposalDecision> {
    const plan = await acceptSelections(this.investigationId, [selectionOf(item)]);
    return this.after(plan.selections.length > 1 ? `已在一次提交中接受 ${plan.selections.length} 个提案` : "已接受提案");
  }

  async discard(item: PendingChangeSet, cascade: boolean): Promise<ProposalDecision> {
    return this.after(await discardSelections(this.investigationId, [selectionOf(item)], cascade));
  }

  private async after(text: string): Promise<ProposalDecision> {
    await this.reload();
    return { message: text, remaining: this.items.length };
  }
}

/**
 * Blocking batch review before `trigger`. Resolves `true` when the action
 * may proceed (also when there is nothing to review or the review cannot be
 * loaded) and `false` when the user cancels it.
 */
export async function reviewBeforeLeaving(sandboxSessionId: string | null, trigger: ReviewTrigger, toast: Toast): Promise<boolean> {
  let investigationId: string | null;
  try { investigationId = await investigationFor(sandboxSessionId); } catch { return true; }
  if (!investigationId) return true;
  let queue: ChangeSetQueue;
  try { queue = await listQueue(investigationId); } catch { return true; }
  if (queue.items.length === 0) return true;
  return new BatchReview(investigationId, trigger, toast, queue).run();
}

/** The blocking dialog: checkboxes, total impact, 接受所选 / 丢弃所选, proceed or cancel. */
class BatchReview {
  private readonly overlay = el("div", "change-set-review-overlay");
  private readonly dialog = el("section", "change-set-review-dialog");
  private readonly list = el("div", "change-set-review-list");
  private readonly feedback = el("p", "change-set-review-feedback review-action-feedback");
  private readonly summary = el("p", "change-set-review-summary");
  private readonly confirmHost = el("div", "change-set-review-confirm");
  private summaryGeneration = 0;
  private items: PendingChangeSet[] = [];
  private selected = new Set<string>();
  private previews = new Map<string, ImpactPreview | string>();
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
      el("p", "review-note", `接受会以一次提交写入当前沙箱副本；未勾选的提案保持「未审批」。接受不提升 Trust，也不代表数学上正确。${text.note}`),
    );
    this.confirmHost.hidden = true;
    // Grid rows: header, summary, list, confirm, feedback, footer.
    this.dialog.append(header, this.summary, this.list, this.confirmHost, this.feedback, this.footer());
    this.overlay.append(this.dialog);
    document.body.append(this.overlay);
    const done = new Promise<boolean>(resolve => { this.resolve = resolve; });
    void this.reload(this.queue);
    return done;
  }

  private footer(): HTMLElement {
    const text = TRIGGER_TEXT[this.trigger];
    const actions = el("footer", "change-set-review-actions review-footer-actions");
    actions.append(
      button(text.cancel, () => this.finish(false)),
      el("span", "change-set-review-spacer"),
      button("丢弃所选", () => void this.discardSelected(), "review-danger"),
      button("接受所选", () => void this.accept(selectionRefs(this.items, this.selected)), "review-accept"),
      button(text.proceed, () => this.finish(true)),
    );
    return actions;
  }

  private finish(proceed: boolean): void {
    if (this.busy) return;
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
    this.renderList();
    void this.refreshSummary();
    for (const item of this.items) void this.loadPreview(item);
  }

  /** Total impact of the current selection, planned as one batch against the head. */
  private async refreshSummary(): Promise<void> {
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
    head.append(el("span"), el("span", "", "提案"), el("span", "", "状态"), el("span", "", "Debt"), el("span", "", "影响预览"));
    table.append(head);
    if (this.items.length === 0) table.append(el("p", "review-empty", "当前调查没有待审核提案。"));
    groups.forEach((group, index) => {
      if (groups.length > 1 || group.length > 1) {
        table.append(el("div", "change-set-review-group review-subheading", group.length > 1 ? `依赖组 ${index + 1}（${group.length} 个，上游在前）` : "独立提案"));
      }
      for (const item of group) table.append(this.row(item));
    });
    this.list.replaceChildren(table);
  }

  private row(item: PendingChangeSet): HTMLElement {
    const key = keyOf(item);
    const row = el("label", "change-set-review-row");
    row.dataset.key = key;
    const check = el("input");
    check.type = "checkbox";
    check.checked = this.selected.has(key);
    check.disabled = item.state !== "staged";
    check.setAttribute("aria-label", `选择提案 ${titleOf(item)}`);
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
    title.append(el("strong", "", titleOf(item)), el("small", "review-item-meta", `v${item.version}${item.history?.length ? ` · 另有 ${item.history.length} 个旧版本` : ""}`));
    if (item.dependencies.length) {
      title.append(el("small", "review-item-meta", `依赖 ${item.dependencies.map(dep => {
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
    row.append(check, title, el("span", `change-set-review-state state-${item.state}`, stateLabel(item.state)), debt, impact);
    return row;
  }

  private async accept(selections: ChangeSetSelection[]): Promise<void> {
    if (this.busy) return;
    if (!selections.length) { this.feedback.textContent = "请选择至少一个未审批提案。"; return; }
    this.busy = true;
    this.feedback.textContent = "正在按当前地图重新规划…";
    try {
      const plan = await acceptSelections(this.investigationId, selections);
      this.toast(`已在一次提交中接受 ${plan.selections.length} 个提案`, "success");
      this.feedback.textContent = "";
    } catch (error) {
      this.feedback.textContent = `接受失败：${message(error)}`;
    } finally {
      this.busy = false;
    }
    await this.reload();
  }

  private async discardSelected(): Promise<void> {
    if (this.busy) return;
    const preview = discardPreview(this.items, [...this.selected]);
    if (!preview.roots.length) { this.feedback.textContent = "请选择至少一个提案。"; return; }
    const choice = await new Promise<DiscardChoice | null>(resolve => {
      this.confirmHost.replaceChildren(renderDiscardConfirm(preview, value => {
        this.confirmHost.hidden = true;
        this.confirmHost.replaceChildren();
        resolve(value);
      }));
      this.confirmHost.hidden = false;
    });
    if (choice === null) return;
    this.busy = true;
    try {
      this.toast(await discardSelections(this.investigationId, preview.roots.map(selectionOf), choice.cascade), "success");
    } catch (error) {
      this.feedback.textContent = `丢弃失败：${message(error)}`;
    } finally {
      this.busy = false;
    }
    await this.reload();
  }
}
