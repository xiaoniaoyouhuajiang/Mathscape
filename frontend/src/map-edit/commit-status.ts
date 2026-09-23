/**
 * Commit status components — loading, success, and error states shown in the
 * Inspector panel during and after a map change commit.
 */
import type { CommitReceipt } from "./types";

/**
 * Render a "committing…" loading state that blocks the Inspector.
 */
export function renderCommitting(host: HTMLElement): void {
  host.replaceChildren();
  const wrapper = document.createElement("div");
  wrapper.className = "commit-status commit-status--loading";
  const spinner = document.createElement("div");
  spinner.className = "commit-spinner";
  const text = document.createElement("p");
  text.textContent = "正在提交变更…";
  wrapper.append(spinner, text);
  host.append(wrapper);
}

/**
 * Render a success receipt after a commit completes.
 */
export function renderCommitSuccess(
  host: HTMLElement,
  receipt: CommitReceipt,
  onDismiss: () => void,
): void {
  host.replaceChildren();
  const wrapper = document.createElement("div");
  wrapper.className = "commit-status commit-status--success";

  const heading = document.createElement("span");
  heading.className = "eyebrow";
  heading.textContent = "提交成功";
  wrapper.append(heading);

  const badge = document.createElement("div");
  badge.className = "impact-status impact-status--ready";
  badge.textContent = "✓ 已提交";
  wrapper.append(badge);

  // Summary
  const created = receipt.objects.filter((o) => o.action === "create").length;
  const revised = receipt.objects.filter((o) => o.action === "revise").length;
  const retired = receipt.objects.filter((o) => o.action === "retire").length;
  const parts: string[] = [];
  if (created > 0) parts.push(`创建 ${created}`);
  if (revised > 0) parts.push(`修订 ${revised}`);
  if (retired > 0) parts.push(`退役 ${retired}`);
  const summary = document.createElement("p");
  summary.className = "form-note";
  summary.textContent = parts.length > 0 ? parts.join("，") : "无变更";
  wrapper.append(summary);

  // Commit ID
  const idRow = document.createElement("code");
  idRow.className = "identifier-value";
  idRow.textContent = receipt.commit_id;
  idRow.title = receipt.commit_id;
  wrapper.append(idRow);

  const dismiss = document.createElement("button");
  dismiss.type = "button";
  dismiss.className = "parse-button";
  dismiss.textContent = "完成";
  dismiss.onclick = onDismiss;
  wrapper.append(dismiss);

  host.append(wrapper);
}

/**
 * Render an error state with retry/cancel options.
 */
export function renderCommitError(
  host: HTMLElement,
  error: string,
  onRetry: () => void,
  onCancel: () => void,
): void {
  host.replaceChildren();
  const wrapper = document.createElement("div");
  wrapper.className = "commit-status commit-status--error";

  const heading = document.createElement("span");
  heading.className = "eyebrow";
  heading.textContent = "提交失败";

  const badge = document.createElement("div");
  badge.className = "impact-status impact-status--conflicted";
  badge.textContent = "提交未成功";

  const msg = document.createElement("p");
  msg.className = "detail-error";
  msg.textContent = error;

  const actions = document.createElement("div");
  actions.className = "edit-actions";
  const retryButton = document.createElement("button");
  retryButton.type = "button";
  retryButton.className = "parse-button";
  retryButton.textContent = "重新预览";
  retryButton.onclick = onRetry;
  const cancelButton = document.createElement("button");
  cancelButton.type = "button";
  cancelButton.className = "retire-button cancel-button";
  cancelButton.textContent = "取消";
  cancelButton.onclick = onCancel;
  actions.append(retryButton, cancelButton);

  wrapper.append(heading, badge, msg, actions);
  host.append(wrapper);
}
