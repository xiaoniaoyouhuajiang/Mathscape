/**
 * Discard confirmation shared by the batch dialog and the review workspace:
 * lists who a discard touches before anything happens. Default (unchecked)
 * turns the dependents 「阻塞」; checking 「连带丢弃」 discards them too.
 */
import { titleOf, type PendingChangeSet } from "./proposal-model";

export interface DiscardChoice { cascade: boolean }

export function renderDiscardConfirm(
  preview: { roots: PendingChangeSet[]; dependents: PendingChangeSet[] },
  done: (choice: DiscardChoice | null) => void,
): HTMLElement {
  const names = (items: PendingChangeSet[]) => items.map(item => `${titleOf(item)} v${item.version}`).join("、");
  const box = document.createElement("div");
  box.className = "proposal-discard-confirm";
  box.setAttribute("role", "alertdialog");
  const lead = document.createElement("p");
  lead.textContent = `将丢弃：${names(preview.roots)}。丢弃只改提案状态，不写正式地图。`;
  box.append(lead);
  const cascade = document.createElement("input");
  cascade.type = "checkbox";
  if (preview.dependents.length) {
    const dependents = document.createElement("p");
    dependents.className = "proposal-discard-dependents";
    dependents.textContent = `依赖它们的提案（${preview.dependents.length} 个，含间接依赖）：${names(preview.dependents)}`;
    const label = document.createElement("label");
    label.className = "proposal-discard-cascade";
    label.append(cascade, document.createTextNode(" 连带丢弃这些依赖提案（不勾选则它们转为「阻塞」，等待 Agent 重新提交）"));
    box.append(dependents, label);
  } else {
    const none = document.createElement("p");
    none.className = "proposal-discard-dependents";
    none.textContent = "没有其他提案依赖它们。";
    box.append(none);
  }
  const actions = document.createElement("div");
  actions.className = "proposal-discard-actions";
  const cancel = document.createElement("button");
  cancel.type = "button";
  cancel.textContent = "取消";
  cancel.onclick = () => done(null);
  const confirm = document.createElement("button");
  confirm.type = "button";
  confirm.className = "review-danger";
  confirm.textContent = "确认丢弃";
  confirm.onclick = () => done({ cascade: cascade.checked });
  actions.append(cancel, confirm);
  box.append(actions);
  return box;
}
