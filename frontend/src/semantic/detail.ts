/**
 * Inspector 里的语义对象详情：共享对象、对齐线、一笔账、约定卡。
 *
 * 同时支持已接受对象与沙箱暂存（`〔未审批〕`）语义对象：
 * - 暂存对象完整展示 KaTeX 公式、参数对照与修改前后差异，不再隐藏为空壳；
 * - 用户专用的对象级按钮（审阅对齐线、结账）走普通地图编辑命令（plan → commit），
 *   仅对已接受对象开放，与「接受提案」严格分开。
 */
import { textElement } from "../core/dom";
import { t } from "../i18n";
import { commitMapChange, generateRequestId, getEditContext, planMapChange } from "../map-edit/api";
import type { UserChangeOperation } from "../map-edit/types";
import { markdownRenderer } from "../reader/markdown";
import type { SemanticObjectDetail } from "./api";
import { feedbackButton } from "./feedback";
import {
  alignmentPlainLabel,
  alignmentStatusLabel,
  baselineStatusLabel,
  debtStatusLabel,
  entityPlainLabel,
  failureModeLabel,
  lostDirectionLabel,
  relationLabel,
  representationBindingPlainLabel,
  semanticObjectNoun,
} from "./labels";
import {
  computePayloadRevisionDiff,
  formatTermIndexEntry,
  renderFieldDiffTable,
  renderLocalSymbolsCard,
  renderMathNode,
  renderRepresentationCoreCardBody,
  renderSemanticEntityCardBody,
  renderSymbolMapTable,
} from "./semantic-diff";

type Json = Record<string, unknown>;
const str = (value: unknown): string | null => (typeof value === "string" ? value : null);
const arr = (value: unknown): Json[] =>
  Array.isArray(value) ? value.filter((v): v is Json => v !== null && typeof v === "object") : [];

const sharedMd = markdownRenderer();

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

function title(detail: SemanticObjectDetail, id: string | null): string {
  if (!id) return "—";
  return detail.titles[id] ?? (id.length > 36 ? `${id.slice(0, 28)}…` : id);
}

/** 约定卡：每条约定一行，支持 KaTeX 渲染，显示原文说法、是否明说。 */
export function conventionsCard(conventions: Json[], heading: string): HTMLElement {
  const card = textElement("section", "semantic-card semantic-dialect-card", "");
  card.append(textElement("h4", "", heading));
  if (conventions.length === 0) card.append(textElement("p", "semantic-muted", "没有约定。"));
  for (const convention of conventions) {
    const line = document.createElement("div");
    line.className = "semantic-convention";
    line.append(renderMathNode(sharedMd, str(convention.text) ?? "—", "semantic-markdown"));
    const tags: string[] = [];
    if (convention.explicitness === "explicit") tags.push("书里明说");
    else if (convention.explicitness === "inherited") tags.push("沿用上一级");
    else if (convention.explicitness) tags.push("推断");
    if (convention.basis === "mathlib") tags.push("按 mathlib");
    if (tags.length) line.append(textElement("small", "semantic-tag", tags.join(" · ")));
    card.append(line);
  }
  return card;
}

/** 一笔账的徽章：暗金虚线；已还清时金色实线。 */
export function debtBadge(debt: Json): HTMLElement {
  const status = str(debt.status) ?? "open";
  const badge = textElement("article", `semantic-debt-badge status-${status}`, "");
  const titleEl = renderMathNode(sharedMd, str(debt.obligation) ?? "一笔账", "semantic-debt-title semantic-markdown");
  badge.append(
    titleEl,
    textElement("span", "semantic-debt-status", debtStatusLabel(status)),
  );
  const meta = [
    str(debt.quantifier),
    debt.failure_mode ? `不还清会怎样：${failureModeLabel(str(debt.failure_mode))}` : null,
  ].filter(Boolean) as string[];
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
      deps.showToast(plan.diagnostics.map((item) => item.message).join("；") || "无法写入", "error");
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
  const isStaged = Boolean(detail.staged);
  const resolveTitle = (id: string | null) => title(detail, id);
  const feedback = (name: string, extra: string | null = null) =>
    feedbackButton(
      { noun: semanticObjectNoun(detail.kind), name, detail: extra, id: detail.object_id },
      (inserted) =>
        deps.showToast(
          inserted ? "已插入对话输入框（未发送）" : "没有打开可输入的沙箱对话；请先打开沙箱对话",
          inserted ? "success" : "error",
        ),
    );

  // 若该对象是暂存 edit（替换已接受版本），直接在顶部可展开展示前后字段差异
  if (detail.staged?.accepted_payload) {
    const diffs = computePayloadRevisionDiff(detail.kind, detail.staged.accepted_payload, p, resolveTitle);
    const diffDetails = document.createElement("details");
    diffDetails.className = "semantic-staged-diff";
    diffDetails.open = true;
    diffDetails.append(
      textElement("summary", "", `对比已接受旧版本（${diffs.length} 处字段变化）`),
      renderFieldDiffTable(diffs, sharedMd, "已接受版本 → 当前暂存提案"),
    );
    root.append(diffDetails);
  }

  if (detail.kind === "SemanticEntity") {
    const name = entityPlainLabel(p);
    root.append(renderSemanticEntityCardBody(sharedMd, p, resolveTitle));
    const baseline = p.mathlib_baseline as Json | null | undefined;
    const checked = baseline?.checked as Json | undefined;
    if (checked) {
      const checkInfo = textElement("section", "semantic-card semantic-baseline-check", "");
      checkInfo.append(
        row("核对于", `profile ${str(checked.profile_id) ?? "?"} · mathlib ${str(checked.mathlib_version) ?? "?"}`),
      );
      if (str(checked.location)) checkInfo.append(row("源码位置", str(checked.location)!));
      root.append(checkInfo);
    }
    if (!isStaged) {
      const recheckWrap = textElement("div", "semantic-actions", "");
      const recheck = actionButton("重新核对 mathlib 名称", () => {
        recheck.disabled = true;
        void deps.recheck().finally(() => {
          recheck.disabled = false;
        });
      });
      recheckWrap.append(
        recheck,
        textElement("p", "semantic-muted", baseline && str(baseline.decl)
          ? `当前状态：${baselineStatusLabel(str(baseline.status) ?? "unchecked")}。核对只说明名字在 mathlib 中存在，不说明与教材命题等价。`
          : "核对本工作区所有写了 mathlib 名称的共享对象。"),
      );
      root.append(recheckWrap);
    } else {
      root.append(textElement("p", "semantic-muted", "此共享对象处于未审批提案中；接受提案写入地图后可核对本机 mathlib 名称。"));
    }
    root.append(feedback(name));
  }

  if (detail.kind === "Alignment") {
    const plain = alignmentPlainLabel(p);
    root.append(
      row("白话", plain),
      row("关系", relationLabel(str(p.relation))),
      row("来源", resolveTitle(str(p.source))),
      row("共享对象", resolveTitle(str(p.target))),
      row("状态", alignmentStatusLabel(str(p.status))),
    );
    if (str(p.plain_note)) {
      root.append(row("补充说明", renderMathNode(sharedMd, str(p.plain_note)!, "semantic-value semantic-markdown")));
    }
    const targetBranches = Array.isArray(p.target_branches)
      ? p.target_branches.map(str).filter((v): v is string => !!v)
      : [];
    if (targetBranches.length > 0) {
      root.append(row(t("semantic.s2.targetBranches"), targetBranches.join(", ")));
    }
    const params = arr(p.parameter_map);
    if (params.length) {
      const table = textElement("table", "semantic-table", "");
      table.append(Object.assign(document.createElement("caption"), { textContent: "参数对照（第几条 ↔ 第几条）" }));
      for (const entry of params) {
        const tr = document.createElement("tr");
        const srcText = `${str(entry.source) ?? ""}${str(entry.source_range) ? `（${str(entry.source_range)}）` : ""}`;
        const dstText = `${str(entry.target) ?? ""}${str(entry.target_range) ? `（${str(entry.target_range)}）` : ""}`;
        tr.append(
          renderMathNode(sharedMd, srcText, "semantic-td-cell"),
          textElement("td", "", "↔"),
          renderMathNode(sharedMd, dstText, "semantic-td-cell"),
          renderMathNode(sharedMd, str(entry.map) ?? "", "semantic-td-cell"),
        );
        table.append(tr);
      }
      root.append(table);
    }
    const terms = arr(p.term_index_map);
    if (terms.length) {
      const termTexts = terms.map(formatTermIndexEntry).join("；");
      root.append(row("数列项对照", renderMathNode(sharedMd, termTexts, "semantic-value semantic-markdown")));
    }
    const lost = arr(p.lost_information);
    if (lost.length) {
      root.append(textElement("h4", "", "对不上的部分"));
      const ul = textElement("ul", "semantic-list", "");
      for (const item of lost) {
        const li = document.createElement("li");
        const head = `${str(item.short) ?? ""}${item.direction ? `（${lostDirectionLabel(str(item.direction))}）` : ""}`;
        li.append(textElement("strong", "", head));
        if (str(item.claim)) li.append(renderMathNode(sharedMd, str(item.claim)!, "semantic-markdown"));
        if (str(item.see_also)) li.append(textElement("small", "semantic-muted", `另见：${resolveTitle(str(item.see_also))}`));
        ul.append(li);
      }
      root.append(ul);
    }
    const assumptions = arr(p.required_assumptions);
    if (assumptions.length) {
      root.append(textElement("h4", "", "需要的前提"));
      const ul = textElement("ul", "semantic-list", "");
      for (const item of assumptions) {
        const li = document.createElement("li");
        li.append(renderMathNode(sharedMd, str(item.text) ?? "", "semantic-markdown"));
        if (str(item.debt)) li.append(textElement("small", "semantic-muted", `关联账目：${resolveTitle(str(item.debt))}`));
        ul.append(li);
      }
      root.append(ul);
    }
    if (!retired && !isStaged) {
      const actions = textElement("div", "semantic-actions", "");
      actions.append(textElement("span", "semantic-muted", "你的审阅（对象级，和“接受提案”分开）："));
      const review = (status: string, label: string) =>
        actionButton(label, () =>
          void runUserOperation(
            detail.object_id,
            (revision) => ({
              operation: "set_alignment_review",
              object_id: detail.object_id,
              expected_revision: revision,
              status,
              note: null,
            }),
            `审阅对齐线：${label}`,
            deps,
          ),
        );
      actions.append(review("semantically_reviewed", "我看过了"), review("accepted", "我确认"), review("rejected", "我否决"));
      root.append(actions);
    } else if (isStaged) {
      root.append(textElement("p", "semantic-muted", "此对齐线尚未接受入图；请先接受提案，再执行「我看过了 / 我确认 / 我否决」对象级审阅。"));
    }
    root.append(feedback(str(p.map_label) ?? plain, plain));
  }

  if (detail.kind === "Debt") {
    root.append(debtBadge(p));
    root.append(row("挂在", resolveTitle(str(p.attached_to))));
    if (str(p.failure_note)) {
      root.append(row("风险说明", renderMathNode(sharedMd, str(p.failure_note)!, "semantic-value semantic-markdown")));
    }
    if (str(p.payoff_hint)) {
      root.append(row("怎么还", renderMathNode(sharedMd, str(p.payoff_hint)!, "semantic-value semantic-markdown")));
    }
    const paidBy = arr(p.paid_by).map((ref) => resolveTitle(str(ref.ref)));
    if (paidBy.length) root.append(row("由谁还清", paidBy.join("、")));
    const status = str(p.status);
    if (!retired && status !== "obsolete" && !isStaged) {
      const actions = textElement("div", "semantic-actions", "");
      const settle = (settlement: "paid" | "waived" | "open", label: string, note: string | null = null) =>
        void runUserOperation(
          detail.object_id,
          (revision) => ({
            operation: "set_debt_settlement",
            object_id: detail.object_id,
            expected_revision: revision,
            settlement,
            note,
          }),
          `结账：${label}`,
          deps,
        );
      if (status === "claimed_paid") actions.append(actionButton("确认已还清", () => settle("paid", "确认已还清")));
      if (status === "open" || status === "claimed_paid") {
        actions.append(
          actionButton("免除…", () => {
            const note = window.prompt("免除这笔账的理由（必填）");
            if (note && note.trim()) settle("waived", "免除", note.trim());
          }),
        );
      }
      if (status !== "open") actions.append(actionButton("重新打开", () => settle("open", "重新打开")));
      root.append(actions);
    } else if (isStaged) {
      root.append(textElement("p", "semantic-muted", "此账目处于未审批提案中；接受提案写入地图后可确认还清或免除。"));
    }
    root.append(feedback(str(p.obligation) ?? "一笔账"));
  }

  if (detail.kind === "DialectScope") {
    root.append(row("适用于", `${resolveTitle(str(p.anchor_unit))} 及其下的内容`));
    root.append(conventionsCard(arr(p.conventions), str(p.label) ?? "约定卡"));
    const localSymbols = arr(p.local_symbols);
    if (localSymbols.length > 0) {
      root.append(renderLocalSymbolsCard(sharedMd, localSymbols));
    }
    root.append(feedback(str(p.label) ?? "约定卡"));
  }

  if (detail.kind === "RepresentationCore") {
    const name = entityPlainLabel(p);
    root.append(renderRepresentationCoreCardBody(sharedMd, p, resolveTitle));
    const baseline = p.mathlib_baseline as Json | null | undefined;
    const checked = baseline?.checked as Json | undefined;
    if (checked) {
      const checkInfo = textElement("section", "semantic-card semantic-baseline-check", "");
      checkInfo.append(
        row(t("semantic.common.checkedAt"), `profile ${str(checked.profile_id) ?? "?"} · mathlib ${str(checked.mathlib_version) ?? "?"}`),
      );
      if (str(checked.location)) checkInfo.append(row(t("semantic.common.sourceLoc"), str(checked.location)!));
      root.append(checkInfo);
    }
    if (!isStaged) {
      const recheckWrap = textElement("div", "semantic-actions", "");
      const recheck = actionButton(t("semantic.common.recheckBtn"), () => {
        recheck.disabled = true;
        void deps.recheck().finally(() => {
          recheck.disabled = false;
        });
      });
      recheckWrap.append(
        recheck,
        textElement(
          "p",
          "semantic-muted",
          baseline && str(baseline.decl)
            ? t("semantic.common.recheckStatusHint", { status: baselineStatusLabel(str(baseline.status) ?? "unchecked") })
            : t("semantic.common.recheckAllHint"),
        ),
      );
      root.append(recheckWrap);
    } else {
      root.append(textElement("p", "semantic-muted", t("semantic.s3.stagedCoreHint")));
    }
    root.append(feedback(name));
  }

  if (detail.kind === "RepresentationBinding") {
    const plain = representationBindingPlainLabel(p);
    root.append(
      row(t("semantic.common.plainLabel"), plain),
      row(t("semantic.s3.dialectScope"), resolveTitle(str(p.dialect_scope))),
      row(t("semantic.s3.representationCore"), resolveTitle(str(p.representation))),
      row(t("semantic.common.baselineStatus"), alignmentStatusLabel(str(p.status))),
    );
    if (str(p.plain_note)) {
      root.append(row(t("semantic.s3.noteCol"), renderMathNode(sharedMd, str(p.plain_note)!, "semantic-value semantic-markdown")));
    }
    const symbolMap = arr(p.symbol_map);
    if (symbolMap.length > 0) {
      root.append(renderSymbolMapTable(sharedMd, symbolMap));
    }
    const adaptations = arr(p.index_and_domain_adaptations);
    if (adaptations.length > 0) {
      root.append(textElement("h4", "", t("semantic.s3.adaptationsHeading")));
      const ul = textElement("ul", "semantic-list", "");
      for (const ad of adaptations) {
        if (!str(ad.text)) continue;
        const li = document.createElement("li");
        li.append(renderMathNode(sharedMd, str(ad.text)!, "semantic-markdown"));
        ul.append(li);
      }
      root.append(ul);
    }
    const lost = arr(p.lost_information);
    if (lost.length > 0) {
      const ul = textElement("ul", "semantic-list", "");
      for (const entry of lost) {
        const li = document.createElement("li");
        const head = `${str(entry.short) ?? ""}${entry.direction ? ` (${lostDirectionLabel(str(entry.direction))})` : ""}`;
        li.append(textElement("strong", "", head));
        if (str(entry.claim)) li.append(renderMathNode(sharedMd, str(entry.claim)!, "semantic-markdown"));
        ul.append(li);
      }
      root.append(ul);
    }
    if (!retired && !isStaged) {
      const actions = textElement("div", "semantic-actions", "");
      actions.append(textElement("span", "semantic-muted", t("semantic.common.reviewPrompt")));
      const review = (status: string, label: string) =>
        actionButton(label, () =>
          void runUserOperation(
            detail.object_id,
            (revision) => ({
              operation: "set_representation_binding_review",
              object_id: detail.object_id,
              expected_revision: revision,
              status,
              note: null,
            }),
            t("semantic.s3.reviewBindingReason", { label }),
            deps,
          ),
        );
      actions.append(
        review("semantically_reviewed", t("semantic.common.reviewedBtn")),
        review("accepted", t("semantic.common.acceptedBtn")),
        review("rejected", t("semantic.common.rejectedBtn")),
      );
      root.append(actions);
    } else if (isStaged) {
      root.append(textElement("p", "semantic-muted", t("semantic.s3.stagedBindingHint")));
    }
    root.append(feedback(str(p.map_label) ?? plain, plain));
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
  root.append(
    textElement("p", "semantic-muted semantic-trust-note", "这里显示的状态都不是可信度：接受提案只是写进地图，不代表数学上已验证。"),
  );
  return root;
}
