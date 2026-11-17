/**
 * 学习视图 · 材料研读概览（契约 S9.1，Phase 3 增强版）：
 * - 按 canonical Markdown 折叠同一本书的多章快照，按真实章节/页码顺序展示全部已登记单元（含尚未对齐的章节）；
 * - 结合跨书共享对象对齐与本地 `study_marks` 侧车进度（未读 / 在读 / 已掌握），区分「新内容」「他处已掌握」「别的材料里见过」「尚未对齐」；
 * - 区分「与共享基线一致的约定」和「本材料特有的编号/记号约定」，并支持沙箱未审批提案预览。
 * - 本地学习标记写入独立的 `study_marks` 表，绝不修改 `map_objects` 修订或提升 Trust。
 */
import { textElement } from "../core/dom";
import { errorMessage } from "../core/ipc";
import { markdownRenderer } from "../reader/markdown";
import {
  getLearningView,
  recheckMathlibBaselines,
  recheckSummary,
  setStudyMarks,
  type LearningMaterial,
  type LearningUnit,
  type LearningView,
  type StudyOutlineUnit,
  type StudyStatus,
} from "./api";
import { debtStatusLabel, relationLabel } from "./labels";

const md = markdownRenderer();

/** 一条内容单元上的一条对齐（合并后仍保留，便于列出多个见过的对象）。 */
export interface MergedLearningTarget {
  alignment_id: string;
  entity_id: string;
  entity_label: string;
  relation: string;
  only_differs_by: string[];
}

/** 按 unit_id 合并后的学习视图行：同一单元只出现一次。 */
export interface MergedLearningUnit {
  unit_id: string;
  unit_title: string;
  label: "新的" | "别的材料里见过" | string;
  seen_in: string[];
  targets: MergedLearningTarget[];
}

export type OutlineFilterMode = "all" | "novel_and_diff" | "unaligned";

/** 去掉共享对象标签前的类型前缀，便于在一行里列出多个对象。 */
export function stripEntityTypePrefix(label: string): string {
  return label.replace(/^(概念|精确说法|理论|题型)：/, "");
}

/** 同一内容单元的多条对齐合并成一行；出现顺序与首次出现的 unit_id 一致。 */
export function mergeLearningUnits(units: LearningUnit[]): MergedLearningUnit[] {
  const order: string[] = [];
  const groups = new Map<string, LearningUnit[]>();
  for (const unit of units) {
    if (!groups.has(unit.unit_id)) {
      order.push(unit.unit_id);
      groups.set(unit.unit_id, []);
    }
    groups.get(unit.unit_id)!.push(unit);
  }
  return order.map((unit_id) => {
    const group = groups.get(unit_id)!;
    const seen = group.some((unit) => unit.label === "别的材料里见过");
    const allNew = group.every((unit) => unit.label === "新的");
    return {
      unit_id,
      unit_title: group[0].unit_title,
      label: allNew ? "新的" : seen ? "别的材料里见过" : group[0].label,
      seen_in: [...new Set(group.flatMap((unit) => unit.seen_in))],
      targets: group.map((unit) => ({
        alignment_id: unit.alignment_id,
        entity_id: unit.entity_id,
        entity_label: unit.entity_label,
        relation: unit.relation,
        only_differs_by: unit.only_differs_by,
      })),
    };
  });
}

/** 回顾面板上的明细：见过的写成「见过：对象（只差：…）」；多个对象用分号连接。 */
export function formatMergedUnitDetail(
  unit: MergedLearningUnit,
  entityName: (label: string) => string = stripEntityTypePrefix,
): string {
  const parts = unit.targets
    .map((target) => {
      const name = entityName(target.entity_label);
      const diff = target.only_differs_by.length ? `（只差：${target.only_differs_by.join("；")}）` : "";
      return `${name}${diff}`;
    })
    .filter(Boolean);
  if (!parts.length) return "";
  return unit.label === "别的材料里见过" ? `见过：${parts.join("；")}` : parts.join("；");
}

/** 纯函数：一份材料的汇总行，按合并后的已对齐单元计数（与教程第 13 镜保持一致）。 */
export function materialSummary(material: LearningMaterial): string {
  const units = mergeLearningUnits(material.units);
  const seen = units.filter((unit) => unit.label === "别的材料里见过").length;
  const fresh = units.filter((unit) => unit.label === "新的").length;
  return `新的 ${fresh} · 别的材料里见过 ${seen} · 新约定 ${material.new_conventions.length} · 未还清的账 ${material.open_debts.length}`;
}

/** 纯函数：基于完整提纲 `outline_units` 的研读进度汇总。 */
export function studyProgressSummary(material: LearningMaterial): string {
  const outline = material.outline_units ?? [];
  if (outline.length === 0) return materialSummary(material);
  const mastered = outline.filter((u) => u.study_status === "mastered").length;
  const studying = outline.filter((u) => u.study_status === "studying").length;
  const fresh = outline.filter((u) => u.category === "fresh").length;
  const masteredElsewhere = outline.filter((u) => u.category === "mastered_elsewhere").length;
  const seenElsewhere = outline.filter((u) => u.category === "seen_elsewhere").length;
  const unaligned = outline.filter((u) => u.category === "unaligned").length;
  const diffConvs = material.conventions_detail
    ? material.conventions_detail.filter((c) => !c.is_baseline_match).length
    : material.new_conventions.length;
  const parts = [
    `研读进度：已掌握 ${mastered}/${outline.length}${studying ? `（在读 ${studying}）` : ""}`,
    `新内容 ${fresh}`,
    masteredElsewhere ? `他处已掌握 ${masteredElsewhere}` : null,
    seenElsewhere ? `别的材料里见过 ${seenElsewhere}` : null,
    unaligned ? `待对齐 ${unaligned}` : null,
    `特有约定 ${diffConvs}`,
    `未还清的账 ${material.open_debts.length}`,
  ].filter(Boolean);
  return parts.join(" · ");
}

export function filterOutlineUnits(units: StudyOutlineUnit[], mode: OutlineFilterMode): StudyOutlineUnit[] {
  if (mode === "unaligned") return units.filter((u) => u.category === "unaligned");
  if (mode === "novel_and_diff") {
    return units.filter((u) => {
      if (u.category === "fresh") return true;
      if (u.targets.some((t) => t.only_differs_by.length > 0)) return true;
      if (u.category === "seen_elsewhere") return true;
      return false;
    });
  }
  return units;
}

export interface LearningViewDeps {
  /** 材料 id → 显示名。 */
  materialName: (artifactId: string) => string;
  showToast: (message: string, kind: "success" | "error") => void;
  /** 点击一条内容时在地图上选中它。 */
  focusObject?: (objectId: string) => void;
  /** 测试注入。 */
  load?: (includeStaged?: boolean) => Promise<LearningView>;
}

const CATEGORY_LABELS: Record<string, string> = {
  fresh: "新的",
  mastered_elsewhere: "他处已掌握",
  seen_elsewhere: "别的材料里见过",
  unaligned: "尚未对齐",
};

const STUDY_STATUS_LABELS: Array<[StudyStatus, string]> = [
  ["unread", "未读"],
  ["studying", "在读"],
  ["mastered", "已掌握"],
];

function renderInlineMath(text: string, className = "learning-math"): HTMLElement {
  const span = document.createElement("span");
  span.className = className;
  span.innerHTML = md.renderInline(text);
  return span;
}

export class LearningViewPanel {
  private readonly root: HTMLElement;
  private readonly body: HTMLElement;
  private readonly picker: HTMLSelectElement;
  private readonly stagedToggle: HTMLInputElement;
  private view: LearningView = { materials: [] };
  private filterMode: OutlineFilterMode = "all";
  private includeStaged = false;

  constructor(host: HTMLElement, private readonly deps: LearningViewDeps) {
    this.root = textElement("section", "learning-view", "");
    this.root.hidden = true;
    this.root.setAttribute("role", "dialog");
    this.root.setAttribute("aria-label", "学习视图");

    const header = textElement("header", "learning-view-header", "");
    const titleWrap = textElement("div", "learning-view-title-wrap", "");
    titleWrap.append(
      textElement("h2", "", "学习视图 · 材料研读概览"),
      textElement(
        "p",
        "semantic-muted learning-view-subtitle",
        "按章节顺序梳理解析单元，对比其他材料标出新内容与局部条件差异，并记录你的个人掌握进度（不改动正式地图）。",
      ),
    );

    this.picker = document.createElement("select");
    this.picker.className = "learning-view-picker";
    this.picker.setAttribute("aria-label", "选择材料");
    this.picker.onchange = () => this.renderMaterial();

    const stagedLabel = textElement("label", "learning-staged-toggle", "");
    this.stagedToggle = document.createElement("input");
    this.stagedToggle.type = "checkbox";
    this.stagedToggle.onchange = () => {
      this.includeStaged = this.stagedToggle.checked;
      void this.reload();
    };
    stagedLabel.append(this.stagedToggle, document.createTextNode(" 含未审批提案预览"));

    const recheck = textElement("button", "secondary-button", "重新核对 mathlib 名称");
    recheck.type = "button";
    recheck.onclick = async () => {
      recheck.disabled = true;
      try {
        const report = await recheckMathlibBaselines();
        this.deps.showToast(recheckSummary(report), report.status === "unavailable" ? "error" : "success");
      } catch (error) {
        this.deps.showToast(errorMessage(error), "error");
      } finally {
        recheck.disabled = false;
      }
    };

    const close = textElement("button", "secondary-button", "关闭");
    close.type = "button";
    close.onclick = () => this.close();

    const controls = textElement("div", "learning-view-header-controls", "");
    controls.append(this.picker, stagedLabel, recheck, close);
    header.append(titleWrap, controls);

    this.body = textElement("div", "learning-view-body", "");
    this.root.append(header, this.body);
    this.root.addEventListener("keydown", (event) => {
      if (event.key === "Escape") this.close();
    });
    host.append(this.root);
  }

  get isOpen(): boolean {
    return !this.root.hidden;
  }

  async open(): Promise<void> {
    this.root.hidden = false;
    await this.reload();
    this.picker.focus();
  }

  private async reload(): Promise<void> {
    this.body.replaceChildren(textElement("p", "semantic-muted", "正在计算材料研读概览…"));
    try {
      this.view = await (this.deps.load ?? getLearningView)(this.includeStaged);
    } catch (error) {
      this.body.replaceChildren(textElement("p", "runtime-problem", errorMessage(error)));
      return;
    }
    const previous = this.picker.value;
    this.picker.replaceChildren(
      ...this.view.materials.map((material) => {
        const option = document.createElement("option");
        option.value = material.artifact_id;
        const foldedCount = material.aliased_artifact_ids?.length ?? 0;
        const suffix = foldedCount > 1 ? `（已合并 ${foldedCount} 份章节快照）` : "";
        option.textContent = `${this.deps.materialName(material.artifact_id)}${suffix}`;
        return option;
      }),
    );
    if (this.view.materials.some((material) => material.artifact_id === previous)) {
      this.picker.value = previous;
    }
    this.renderMaterial();
  }

  close(): void {
    this.root.hidden = true;
  }

  private async updateMarks(unitIds: string[], status: StudyStatus): Promise<void> {
    if (unitIds.length === 0) return;
    try {
      await setStudyMarks(unitIds.map((object_id) => ({ object_id, status })));
      await this.reload();
    } catch (error) {
      this.deps.showToast(`无法更新学习状态：${errorMessage(error)}`, "error");
    }
  }

  private computeOutlineDepths(units: StudyOutlineUnit[]): Map<string, number> {
    const byId = new Map(units.map((u) => [u.unit_id, u]));
    const depths = new Map<string, number>();
    const depthOf = (u: StudyOutlineUnit, seen: Set<string>): number => {
      if (depths.has(u.unit_id)) return depths.get(u.unit_id)!;
      if (!u.parent_unit || !byId.has(u.parent_unit) || seen.has(u.unit_id)) {
        depths.set(u.unit_id, 0);
        return 0;
      }
      seen.add(u.unit_id);
      const d = Math.min(3, depthOf(byId.get(u.parent_unit)!, seen) + 1);
      depths.set(u.unit_id, d);
      return d;
    };
    for (const u of units) depthOf(u, new Set());
    return depths;
  }

  private renderFilterPills(outline: StudyOutlineUnit[]): HTMLElement {
    const bar = textElement("div", "learning-filter-bar", "");
    const modes: Array<[OutlineFilterMode, string]> = [
      ["all", `全部章节单元（${outline.length}）`],
      ["novel_and_diff", `新内容与局部差异（${filterOutlineUnits(outline, "novel_and_diff").length}）`],
      ["unaligned", `尚未对齐（${filterOutlineUnits(outline, "unaligned").length}）`],
    ];
    for (const [mode, label] of modes) {
      const btn = textElement("button", `learning-filter-pill${this.filterMode === mode ? " is-active" : ""}`, label);
      btn.type = "button";
      btn.onclick = () => {
        this.filterMode = mode;
        this.renderMaterial();
      };
      bar.append(btn);
    }
    return bar;
  }

  private renderMaterial(): void {
    const material =
      this.view.materials.find((item) => item.artifact_id === this.picker.value) ?? this.view.materials[0];
    if (!material) {
      this.body.replaceChildren(
        textElement(
          "p",
          "semantic-muted",
          "工作区还没有已登记的来源单元或语义对齐线。先从 PDF 解析件登记章节单元，或让 Agent 提议语义对齐后再打开此处。",
        ),
      );
      return;
    }

    const children: HTMLElement[] = [
      textElement("p", "learning-view-summary", studyProgressSummary(material)),
    ];
    if (this.view.includes_staged) {
      children.push(
        textElement(
          "p",
          "learning-staged-note",
          "当前正在预览沙箱中的未审批提案（标有〔未审批〕的单元尚未正式写入地图）。",
        ),
      );
    }

    const outline = material.outline_units ?? [];
    if (outline.length > 0) {
      children.push(this.renderFilterPills(outline));
      const depths = this.computeOutlineDepths(outline);
      const shown = filterOutlineUnits(outline, this.filterMode);
      const listEl = textElement("ul", "learning-view-units", "");

      if (shown.length === 0) {
        listEl.append(textElement("li", "semantic-muted", "当前筛选条件下没有匹配的章节单元。"));
      }

      for (const unit of shown) {
        const item = textElement(
          "li",
          `learning-unit category-${unit.category} status-${unit.study_status}`,
          "",
        );
        item.style.setProperty("--outline-depth", String(depths.get(unit.unit_id) ?? 0));

        const topRow = textElement("div", "learning-unit-top", "");
        const catBadge = textElement(
          "span",
          `learning-label is-${unit.category}`,
          CATEGORY_LABELS[unit.category] ?? unit.category,
        );
        topRow.append(catBadge);
        if (unit.staged) {
          topRow.append(textElement("span", "learning-label is-staged", "未审批"));
        }
        const titleBtn = textElement("button", "learning-unit-title", "");
        titleBtn.type = "button";
        titleBtn.append(renderInlineMath(unit.unit_title));
        if (this.deps.focusObject) {
          titleBtn.title = "在数学地图上定位此单元";
          titleBtn.onclick = () => this.deps.focusObject?.(unit.unit_id);
        }
        topRow.append(titleBtn);

        // 学习进度状态切换（未读 / 在读 / 已掌握）+ 章节一键掌握
        const markGroup = textElement("div", "learning-mark-group", "");
        const childIds = outline.filter((c) => c.parent_unit === unit.unit_id).map((c) => c.unit_id);
        if (childIds.length > 0) {
          const batchBtn = textElement("button", "learning-mark-chip is-batch", "本节全标已掌握");
          batchBtn.type = "button";
          batchBtn.onclick = (e) => {
            e.stopPropagation();
            void this.updateMarks([unit.unit_id, ...childIds], "mastered");
          };
          markGroup.append(batchBtn);
        }
        for (const [st, label] of STUDY_STATUS_LABELS) {
          const chip = textElement(
            "button",
            `learning-mark-chip${unit.study_status === st ? " is-active" : ""}`,
            label,
          );
          chip.type = "button";
          chip.onclick = (e) => {
            e.stopPropagation();
            if (unit.study_status !== st) void this.updateMarks([unit.unit_id], st);
          };
          markGroup.append(chip);
        }
        topRow.append(markGroup);
        item.append(topRow);

        for (const target of unit.targets) {
          const line = textElement("div", "learning-target", "");
          line.append(
            textElement("span", "learning-relation", `${relationLabel(target.relation)} `),
            renderInlineMath(target.entity_label, "learning-entity"),
          );
          if (target.only_differs_by.length) {
            line.append(
              renderInlineMath(` · 只差：${target.only_differs_by.join("；")}`, "learning-diff"),
            );
          }
          if (target.mastered_in?.length) {
            line.append(
              textElement(
                "small",
                "learning-seen-in is-mastered",
                ` · 已掌握于 ${target.mastered_in.map((id) => this.deps.materialName(id)).join("、")}`,
              ),
            );
          } else if (target.seen_in.length) {
            line.append(
              textElement(
                "small",
                "learning-seen-in",
                ` · 见于 ${target.seen_in.map((id) => this.deps.materialName(id)).join("、")}`,
              ),
            );
          }
          item.append(line);
        }
        listEl.append(item);
      }
      children.push(listEl);
    } else {
      // 兼容旧投影结构（无 outline_units 时按已对齐 units 展示）
      const units = textElement("ul", "learning-view-units", "");
      for (const unit of mergeLearningUnits(material.units)) {
        const item = textElement("li", `learning-unit is-${unit.label === "新的" ? "new" : "seen"}`, "");
        item.append(
          textElement("span", "learning-label", unit.label),
          textElement("strong", "", unit.unit_title),
        );
        for (const target of unit.targets) {
          const line = textElement("div", "learning-target", "");
          line.append(
            textElement("span", "learning-relation", ` ${relationLabel(target.relation)} `),
            renderInlineMath(target.entity_label, "learning-entity"),
          );
          if (target.only_differs_by.length) {
            line.append(textElement("small", "learning-diff", ` · 只差：${target.only_differs_by.join("；")}`));
          }
          item.append(line);
        }
        if (unit.seen_in.length) {
          item.append(
            textElement("small", "learning-seen-in", ` · 见于 ${unit.seen_in.map((id) => this.deps.materialName(id)).join("、")}`),
          );
        }
        if (this.deps.focusObject) {
          item.tabIndex = 0;
          item.onclick = () => this.deps.focusObject?.(unit.unit_id);
        }
        units.append(item);
      }
      children.push(units);
    }

    // 约定对比展示：区分「本材料特有约定」与「同共享基线」
    const convDetails = material.conventions_detail;
    if (convDetails && convDetails.length > 0) {
      const conventions = textElement("ul", "learning-view-conventions", "");
      for (const conv of convDetails) {
        const item = textElement("li", "learning-convention", "");
        const badgeText = conv.is_baseline_match ? "同共享基线" : "特有约定";
        const badgeClass = conv.is_baseline_match
          ? "learning-label is-baseline-convention"
          : "learning-label is-convention";
        item.append(textElement("span", badgeClass, badgeText), renderInlineMath(conv.text));
        if (conv.diff_note) {
          item.append(textElement("small", "learning-diff", `（${conv.diff_note}）`));
        }
        conventions.append(item);
      }
      children.push(textElement("h3", "learning-section-heading", "这份材料的记号与编号约定"), conventions);
    } else if (material.new_conventions.length) {
      const conventions = textElement("ul", "learning-view-conventions", "");
      for (const convention of material.new_conventions) {
        const item = textElement("li", "learning-convention", "");
        item.append(
          textElement("span", "learning-label is-convention", "新约定"),
          renderInlineMath(convention.text),
        );
        conventions.append(item);
      }
      children.push(textElement("h3", "learning-section-heading", "这份材料的约定"), conventions);
    }

    const debts = textElement("footer", "learning-view-debts", "");
    debts.append(textElement("strong", "", `未还清的账 ${material.open_debts.length}`));
    if (material.open_debts.length) {
      const list = textElement("ul", "learning-debt-list", "");
      for (const debt of material.open_debts) {
        const li = document.createElement("li");
        li.append(renderInlineMath(debt.obligation), textElement("small", "semantic-muted", `（${debtStatusLabel(debt.status)}）`));
        if (this.deps.focusObject) {
          li.className = "is-clickable";
          li.onclick = () => this.deps.focusObject?.(debt.debt_id);
        }
        list.append(li);
      }
      debts.append(list);
    }
    children.push(
      debts,
      textElement(
        "p",
        "semantic-muted",
        "「未读 / 在读 / 已掌握」保存在本地学习进度侧车表，不产生地图提交，也不改变任何对象的可信度轴。",
      ),
    );
    this.body.replaceChildren(...children);
  }
}
