/**
 * ImpactPlan preview component — shows the user what will happen before they
 * confirm a map change.
 */
import type { ImpactPlan, ImpactItem, ImpactDiagnostic } from "./types";

const ACTION_LABELS: Record<string, string> = {
  create: "创建",
  revise: "修订",
  retire: "退役",
  preserve: "保留",
  detach: "分离",
  orphan: "孤立",
  recheck_evidence: "重检证据",
  block: "阻断",
};

const KIND_LABELS: Record<string, string> = {
  SourceSemanticUnit: "内容单元",
  SourceUnitRelation: "来源关系",
  SourceSpan: "原文证据",
  SourceArtifact: "材料文件",
};

const STATUS_LABELS: Record<string, string> = {
  ready: "可提交",
  blocked: "存在阻断项",
  conflicted: "地图已更新",
};

/**
 * Render the ImpactPlan preview into the given host element.
 * Returns cleanup function (currently no-op).
 */
export function renderImpactPreview(
  host: HTMLElement,
  plan: ImpactPlan,
  onConfirm: () => void,
  onCancel: () => void,
): void {
  host.replaceChildren();

  const heading = document.createElement("span");
  heading.className = "eyebrow";
  heading.textContent = "变更预览";
  host.append(heading);

  // Status badge
  const statusBadge = document.createElement("div");
  statusBadge.className = `impact-status impact-status--${plan.status}`;
  statusBadge.textContent = STATUS_LABELS[plan.status] ?? plan.status;
  host.append(statusBadge);

  // Direct changes
  if (plan.direct_changes.length > 0) {
    host.append(renderItemSection("直接变更", plan.direct_changes));
  }

  // Affected objects
  if (plan.affected_objects.length > 0) {
    host.append(renderItemSection("级联影响", plan.affected_objects));
  }

  // Diagnostics
  if (plan.diagnostics.length > 0) {
    const section = document.createElement("div");
    section.className = "impact-diagnostics";
    const title = document.createElement("span");
    title.className = "field-label";
    title.textContent = "诊断信息";
    section.append(title);
    for (const diag of plan.diagnostics) {
      section.append(renderDiagnostic(diag));
    }
    host.append(section);
  }

  // Summary counts
  const total = plan.direct_changes.length + plan.affected_objects.length;
  const retireCount = [...plan.direct_changes, ...plan.affected_objects].filter(
    (i) => i.action === "retire",
  ).length;
  const summary = document.createElement("p");
  summary.className = "form-note impact-summary";
  summary.textContent = `共 ${total} 个对象受影响` +
    (retireCount > 0 ? `，其中 ${retireCount} 个将退役` : "");
  host.append(summary);

  // Action buttons
  const actions = document.createElement("div");
  actions.className = "edit-actions";

  const confirmButton = document.createElement("button");
  confirmButton.type = "button";
  confirmButton.className = "parse-button";
  confirmButton.textContent = "确认提交";
  confirmButton.disabled = plan.status !== "ready";
  confirmButton.onclick = onConfirm;

  const cancelButton = document.createElement("button");
  cancelButton.type = "button";
  cancelButton.className = "retire-button cancel-button";
  cancelButton.textContent = "返回编辑";
  cancelButton.onclick = onCancel;

  actions.append(confirmButton, cancelButton);
  host.append(actions);
}

// ---------------------------------------------------------------------------
// Internal renderers
// ---------------------------------------------------------------------------

function renderItemSection(title: string, items: ImpactItem[]): HTMLElement {
  const section = document.createElement("div");
  section.className = "impact-section";
  const heading = document.createElement("span");
  heading.className = "field-label";
  heading.textContent = `${title}（${items.length}）`;
  section.append(heading);

  const list = document.createElement("ul");
  list.className = "impact-list";
  for (const item of items) {
    list.append(renderItem(item));
  }
  section.append(list);
  return section;
}

function renderItem(item: ImpactItem): HTMLElement {
  const li = document.createElement("li");
  li.className = `impact-item impact-item--${item.action}`;

  const actionBadge = document.createElement("span");
  actionBadge.className = `impact-action impact-action--${item.action}`;
  actionBadge.textContent = ACTION_LABELS[item.action] ?? item.action;

  const kindLabel = document.createElement("span");
  kindLabel.className = "impact-kind";
  kindLabel.textContent = KIND_LABELS[item.kind] ?? item.kind;

  const idLabel = document.createElement("code");
  idLabel.className = "impact-id";
  // Show a short suffix to help identification
  const idParts = item.object_id.split("/");
  idLabel.textContent = item.display_title ?? idParts.at(-1) ?? item.object_id;
  idLabel.title = item.object_id;

  li.append(actionBadge, kindLabel, idLabel);
  const reason = document.createElement("p");
  reason.textContent = item.reason;
  li.append(reason);
  if (item.related_objects?.length || item.protected_by_decisions?.length) {
    const details = document.createElement("details");
    const summary = document.createElement("summary"); summary.textContent = "关联依赖与登记";
    details.append(summary);
    for (const ref of item.related_objects ?? []) {
      const line = document.createElement("p"); line.textContent = `${ref.display_title ?? ref.object_id} · ${ref.reference_kind}`; details.append(line);
    }
    for (const id of item.protected_by_decisions ?? []) {
      const line = document.createElement("p"); line.textContent = `有效登记：${id}`; details.append(line);
    }
    li.append(details);
  }
  return li;
}

function renderDiagnostic(diag: ImpactDiagnostic): HTMLElement {
  const div = document.createElement("div");
  div.className = "impact-diagnostic";
  const code = document.createElement("code");
  code.className = "diagnostic-code";
  code.textContent = diag.code;
  const msg = document.createElement("span");
  msg.textContent = diag.message;
  div.append(code, msg);
  return div;
}
