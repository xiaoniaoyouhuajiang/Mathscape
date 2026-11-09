import { invoke } from "@tauri-apps/api/core";

export interface ChangeSetSelection {
  change_set_id: string;
  version: number;
  proposal_fingerprint: string;
}

export interface PendingChangeSet {
  changeSetId: string;
  version: number;
  state: string;
  investigationId: string;
  proposalFingerprint: string | null;
  proposal: { rationale?: string; proposed_objects?: unknown[]; proposed_relations?: unknown[] };
  result: { debts?: unknown[]; diagnostics?: unknown[] };
  dependencies: { changeSetId: string; version: number; referencedIds: string[] }[];
}

export interface ImpactPlan {
  plan_fingerprint: string;
  status: string;
  direct_changes: { object_id: string; action: string; display_title?: string | null }[];
  affected_objects: { object_id: string; action: string; display_title?: string | null }[];
  diagnostics: { code: string; message: string }[];
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className = "", text = ""): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  node.textContent = text;
  return node;
}

/** Small, dependency-aware batch reviewer shared by sandbox thread switches. */
export async function openChangeSetReview(
  investigationId: string,
  toast: (message: string, kind: "success" | "error") => void,
): Promise<void> {
  const pending = await invoke<PendingChangeSet[]>("list_pending_change_sets", { investigationId });
  if (pending.length === 0) {
    toast("当前调查没有待审核提案", "success");
    return;
  }
  const overlay = el("div", "change-set-review-overlay");
  const dialog = el("section", "change-set-review-dialog");
  dialog.setAttribute("role", "dialog");
  dialog.setAttribute("aria-modal", "true");
  dialog.append(el("h2", "", "审核暂存提案"), el("p", "change-set-review-note", "接受只写入当前沙箱副本；未勾选项保持 staged，可稍后继续审核。"));
  const list = el("div", "change-set-review-list");
  const checks = new Map<string, HTMLInputElement>();
  const keyOf = (item: PendingChangeSet) => `${item.changeSetId}@${item.version}`;
  for (const item of pending) {
    const row = el("label", "change-set-review-row");
    const check = el("input") as HTMLInputElement;
    check.type = "checkbox";
    check.checked = item.state === "staged";
    check.disabled = item.state !== "staged";
    checks.set(keyOf(item), check);
    const title = item.proposal.rationale?.trim() || item.changeSetId;
    row.append(check, el("strong", "", `${title} · v${item.version}`), el("span", "change-set-review-state", item.state));
    if (item.dependencies.length) row.append(el("small", "", `依赖 ${item.dependencies.map(dep => `${dep.changeSetId}@${dep.version}`).join("、")}`));
    list.append(row);
    check.onchange = () => {
      if (!check.checked) {
        for (const dependent of pending.filter(candidate => candidate.dependencies.some(dep => dep.changeSetId === item.changeSetId && dep.version === item.version))) {
          checks.get(keyOf(dependent))!.checked = false;
        }
      }
    };
  }
  const feedback = el("p", "change-set-review-feedback");
  const actions = el("div", "change-set-review-actions");
  const close = el("button", "sandbox-btn sandbox-btn-quiet", "稍后再审");
  const discard = el("button", "sandbox-btn sandbox-btn-danger", "丢弃所选");
  const accept = el("button", "sandbox-btn sandbox-btn-primary", "接受所选");
  actions.append(close, discard, accept);
  dialog.append(list, feedback, actions);
  overlay.append(dialog);
  document.body.append(overlay);
  const selected = () => pending.filter(item => checks.get(keyOf(item))?.checked && item.proposalFingerprint).map(item => ({ change_set_id: item.changeSetId, version: item.version, proposal_fingerprint: item.proposalFingerprint! }));
  const remove = () => overlay.remove();
  close.onclick = remove;
  discard.onclick = async () => {
    try {
      await invoke("discard_change_set_batch", { investigationId, selections: selected(), cascade: false });
      toast("已丢弃所选提案", "success");
      remove();
    } catch (error) { feedback.textContent = String(error); }
  };
  accept.onclick = async () => {
    const selections = selected();
    if (!selections.length) { feedback.textContent = "请选择至少一个 staged 提案。"; return; }
    try {
      const plan = await invoke<{ batch_plan_fingerprint: string; impact: ImpactPlan }>("plan_change_set_batch", { investigationId, selections });
      if (plan.impact.status !== "ready") { feedback.textContent = plan.impact.diagnostics.map(item => item.message).join("；") || "提案无法接受"; return; }
      await invoke("accept_change_set_batch", { investigationId, selections, batchPlanFingerprint: plan.batch_plan_fingerprint, requestId: null });
      toast(`已接受 ${selections.length} 个提案`, "success");
      remove();
    } catch (error) { feedback.textContent = String(error); }
  };
}
