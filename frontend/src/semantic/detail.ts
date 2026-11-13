/**
 * Inspector 里的语义对象详情：共享对象、对齐线、一笔账、约定卡。
 *
 * 只显示已接受地图里的内容。用户专用的对象级按钮（审阅对齐线、结账）走普通
 * 地图编辑命令（plan → commit），与「接受提案」分开；接受提案从不改变它们。
 */
import { textElement } from "../core/dom";
import { commitMapChange, generateRequestId, getEditContext, planMapChange } from "../map-edit/api";
import type { UserChangeOperation } from "../map-edit/types";
import type { SemanticObjectDetail } from "./api";
import { feedbackButton } from "./feedback";
import {
  alignmentPlainLabel, alignmentStatusLabel, baselineStatusLabel, debtStatusLabel, entityPlainLabel,
  failureModeLabel, lostDirectionLabel, relationLabel, semanticKindLabel, semanticObjectNoun,
} from "./labels";

type Json = Record<string, unknown>;
const str = (value: unknown): string | null => typeof value === "string" ? value : null;
const arr = (value: unknown): Json[] => Array.isArray(value) ? value.filter((v): v is Json => v !== null && typeof v === "object") : [];

export interface SemanticDetailDeps {
  showToast: (message: string, kind: "success" | "error") => void;
  /** 地图已改变（用户审阅 / 结账 / 核对之后）。 */
  refresh: () => Promise<void>;
  /** 「重新核对 mathlib 名称」。 */
  recheck: () => Promise<void>;
  /** 选中另一个地图对象。 */
  focusObject?: (objectId: string) => void;
}

function row(term: string, value: string | HTMLElement): HTMLElement {
  const item = document.createElement("div");
  item.className = "semantic-row";
  item.append(textElement("span", "semantic-term", term));
  if (typeof value === "string") item.append(textElement("span", "semantic-value", value));
  else item.append(value);
  return item;
}

function list(items: string[], className = "semantic-list"): HTMLElement {
  const ul = textElement("ul", className, "");
  for (const text of items) ul.append(textElement("li", "", text));
  return ul;
}

function title(detail: SemanticObjectDetail, id: string | null): string {
  if (!id) return "—";
  return detail.titles[id] ?? (id.length > 36 ? `${id.slice(0, 28)}…` : id);
}

/** 约定卡：每条约定一行，显示原文说法、是否明说。 */
export function conventionsCard(conventions: Json[], heading: string): HTMLElement {
  const card = textElement("section", "semantic-card semantic-dialect-card", "");
  card.append(textElement("h4", "", heading));
  if (conventions.length === 0) card.append(textElement("p", "semantic-muted", "没有约定。"));
  for (const convention of conventions) {
    const line = textElement("p", "semantic-convention", str(convention.text) ?? "—");
    const tags: string[] = [];
    if (convention.explicitness === "explicit") tags.push("书里明说");
    else if (convention.explicitness === "inherited") tags.push("沿用上一级");
    else if (convention.explicitness) tags.push("推断");
    if (convention.basis === "mathlib") tags.push("按 mathlib");
    if (tags.length) line.append(textElement("small", "semantic-tag", ` · ${tags.join(" · ")}`));
    card.append(line);
  }
  return card;
}

/** 一笔账的徽章：暗金虚线；已还清时金色实线。 */
export function debtBadge(debt: Json): HTMLElement {
  const status = str(debt.status) ?? "open";
  const badge = textElement("article", `semantic-debt-badge status-${status}`, "");
  badge.append(
    textElement("strong", "", str(debt.obligation) ?? "一笔账"),
    textElement("span", "semantic-debt-status", debtStatusLabel(status)),
  );
  const meta = [str(debt.quantifier), debt.failure_mode ? `不还清会怎样：${failureModeLabel(str(debt.failure_mode))}` : null]
    .filter(Boolean) as string[];
  if (meta.length) badge.append(textElement("small", "semantic-debt-meta", meta.join(" · ")));
  return badge;
}

async function runUserOperation(
  objectId: string,
  build: (revision: string) => UserChangeOperation,
  reason: string,
  deps: SemanticDetailDeps,
): Promise<void> {
  try {
    const context = await getEditContext(objectId);
    const request = {
      request_id: generateRequestId(),
      base_commit: context.base_commit,
      reason,
      operations: [build(context.current_revision)],
    };
    const plan = await planMapChange(request);
    if (plan.status !== "ready") {
      deps.showToast(plan.diagnostics.map(item => item.message).join("；") || "无法写入", "error");
      return;
    }
    await commitMapChange(request, plan.plan_fingerprint);
    deps.showToast("已写入地图（新修订，可回溯）", "success");
    await deps.refresh();
  } catch (error) {
    deps.showToast(error instanceof Error ? error.message : String(error), "error");
  }
}

function actionButton(label: string, handler: () => void, className = "parse-button"): HTMLButtonElement {
  const button = textElement("button", className, label);
  button.type = "button";
  button.onclick = handler;
  return button;
}

/** 渲染一个语义对象。返回的元素直接放进 Inspector。 */
export function renderSemanticDetail(detail: SemanticObjectDetail, deps: SemanticDetailDeps): HTMLElement {
  const root = textElement("section", `semantic-detail kind-${detail.kind}`, "");
  root.setAttribute("aria-label", semanticObjectNoun(detail.kind));
  const p = detail.payload;
  const retired = detail.lifecycle !== "active";
  const feedback = (name: string, extra: string | null = null) => feedbackButton(
    { noun: semanticObjectNoun(detail.kind), name, detail: extra, id: detail.object_id },
    (inserted) => deps.showToast(inserted ? "已插入对话输入框（未发送）" : "没有打开可输入的沙箱对话；请先打开沙箱对话", inserted ? "success" : "error"),
  );

  if (detail.kind === "SemanticEntity") {
    const name = entityPlainLabel(p);
    root.append(row("种类", semanticKindLabel(str(p.semantic_kind))));
    const labels = arr(p.preferred_labels).map(label => `${str(label.label) ?? ""}（${str(label.lang) ?? "?"}）`);
    if (labels.length) root.append(row("名称", labels.join(" / ")));
    if (p.home_theory) root.append(row("所在理论", title(detail, str(p.home_theory))));
    if (str(p.description)) root.append(textElement("p", "semantic-description", str(p.description)!));
    const declaration = p.declaration as Json | null | undefined;
    if (declaration) {
      const statement = (declaration.statement ?? declaration.definiens) as Json | undefined;
      if (statement && str(statement.text)) root.append(row("精确说法", str(statement.text)!));
    }
    root.append(conventionsCard(arr(p.conventions), "约定注记"));
    const baseline = p.mathlib_baseline as Json | null | undefined;
    const baselineCard = textElement("section", "semantic-card semantic-baseline", "");
    baselineCard.append(textElement("h4", "", "mathlib 参照"));
    if (!baseline || !str(baseline.decl)) {
      baselineCard.append(textElement("p", "semantic-muted", str(baseline?.note) ?? "mathlib 里暂无对应名称。"));
    } else {
      const status = str(baseline.status) ?? "unchecked";
      baselineCard.append(row("名称", str(baseline.decl)!), row("状态", baselineStatusLabel(status)));
      const checked = baseline.checked as Json | undefined;
      if (checked) {
        baselineCard.append(row("核对于", `profile ${str(checked.profile_id) ?? "?"} · mathlib ${str(checked.mathlib_version) ?? "?"}`));
        if (str(checked.location)) baselineCard.append(row("位置", str(checked.location)!));
      }
      baselineCard.append(textElement("p", "semantic-muted", "核对只说明这个名字在本机 mathlib 里存在，不说明它和教材里的命题是一回事。"));
    }
    const recheck = actionButton("重新核对 mathlib 名称", () => {
      recheck.disabled = true;
      void deps.recheck().finally(() => { recheck.disabled = false; });
    });
    baselineCard.append(recheck);
    root.append(baselineCard);
    root.append(feedback(name));
  }

  if (detail.kind === "Alignment") {
    const plain = alignmentPlainLabel(p);
    root.append(
      row("白话", plain),
      row("关系", relationLabel(str(p.relation))),
      row("来源", title(detail, str(p.source))),
      row("共享对象", title(detail, str(p.target))),
      row("状态", alignmentStatusLabel(str(p.status))),
    );
    const params = arr(p.parameter_map);
    if (params.length) {
      const table = textElement("table", "semantic-table", "");
      table.append(Object.assign(document.createElement("caption"), { textContent: "参数对照（第几条 ↔ 第几条）" }));
      for (const entry of params) {
        const tr = document.createElement("tr");
        tr.append(textElement("td", "", str(entry.source) ?? ""), textElement("td", "", "↔"), textElement("td", "", str(entry.target) ?? ""),
          textElement("td", "", str(entry.map) ?? ""));
        table.append(tr);
      }
      root.append(table);
    }
    const terms = arr(p.term_index_map);
    if (terms.length) {
      root.append(row("数列项对照", terms.map(entry => entry.index_shift === 0
        ? `数列逐项对齐：${str(entry.source)} = ${str(entry.target)}`
        : `${str(entry.source)} 对应 ${str(entry.target)}（相差 ${String(entry.index_shift)} 项）`).join("；")));
    }
    const lost = arr(p.lost_information);
    if (lost.length) {
      root.append(textElement("h4", "", "对不上的部分"),
        list(lost.map(item => `${str(item.short) ?? ""}${item.direction ? `（${lostDirectionLabel(str(item.direction))}）` : ""}${str(item.claim) ? `：${str(item.claim)}` : ""}`)));
    }
    const assumptions = arr(p.required_assumptions);
    if (assumptions.length) root.append(textElement("h4", "", "需要的前提"), list(assumptions.map(item => str(item.text) ?? "")));
    if (!retired) {
      const actions = textElement("div", "semantic-actions", "");
      actions.append(textElement("span", "semantic-muted", "你的审阅（对象级，和“接受提案”分开）："));
      const review = (status: string, label: string) => actionButton(label, () => void runUserOperation(detail.object_id, (revision) => ({
        operation: "set_alignment_review", object_id: detail.object_id, expected_revision: revision, status, note: null,
      }), `审阅对齐线：${label}`, deps));
      actions.append(review("semantically_reviewed", "我看过了"), review("accepted", "我确认"), review("rejected", "我否决"));
      root.append(actions);
    }
    root.append(feedback(str(p.map_label) ?? plain, plain));
  }

  if (detail.kind === "Debt") {
    root.append(debtBadge(p));
    root.append(row("挂在", title(detail, str(p.attached_to))));
    if (str(p.payoff_hint)) root.append(row("怎么还", str(p.payoff_hint)!));
    const paidBy = arr(p.paid_by).map(ref => title(detail, str(ref.ref)));
    if (paidBy.length) root.append(row("由谁还清", paidBy.join("、")));
    const status = str(p.status);
    if (!retired && status !== "obsolete") {
      const actions = textElement("div", "semantic-actions", "");
      const settle = (settlement: "paid" | "waived" | "open", label: string, note: string | null = null) =>
        void runUserOperation(detail.object_id, (revision) => ({
          operation: "set_debt_settlement", object_id: detail.object_id, expected_revision: revision, settlement, note,
        }), `结账：${label}`, deps);
      if (status === "claimed_paid") actions.append(actionButton("确认已还清", () => settle("paid", "确认已还清")));
      if (status === "open" || status === "claimed_paid") {
        actions.append(actionButton("免除…", () => {
          const note = window.prompt("免除这笔账的理由（必填）");
          if (note && note.trim()) settle("waived", "免除", note.trim());
        }));
      }
      if (status !== "open") actions.append(actionButton("重新打开", () => settle("open", "重新打开")));
      root.append(actions);
    }
    root.append(feedback(str(p.obligation) ?? "一笔账"));
  }

  if (detail.kind === "DialectScope") {
    root.append(row("适用于", `${title(detail, str(p.anchor_unit))} 及其下的内容`));
    root.append(conventionsCard(arr(p.conventions), str(p.label) ?? "约定卡"));
    root.append(feedback(str(p.label) ?? "约定卡"));
  }

  if (detail.debts.length) {
    root.append(textElement("h4", "", `挂在这里的账 ${detail.debts.length}`));
    for (const debt of detail.debts) {
      const badge = debtBadge(debt);
      if (deps.focusObject) {
        badge.tabIndex = 0;
        badge.onclick = () => deps.focusObject?.(debt.object_id);
      }
      root.append(badge);
    }
  }
  root.append(textElement("p", "semantic-muted semantic-trust-note", "这里显示的状态都不是可信度：接受提案只是写进地图，不代表数学上已验证。"));
  return root;
}
