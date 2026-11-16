/**
 * 学习视图（契约 S9.1，教程镜 13）：选一份材料，看它的每条内容是「新的」还是
 * 「别的材料里见过」，以及它带来的「新约定」和还没还清的账。同一内容单元若有
 * 多条对齐线，合并成一行。
 *
 * 只读：全部由后端从已接受的对齐线计算，不写入任何对象。「读过」标记不在本期。
 */
import { textElement } from "../core/dom";
import { errorMessage } from "../core/ipc";
import type { LearningMaterial, LearningUnit, LearningView } from "./api";

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
  const parts = unit.targets.map((target) => {
    const name = entityName(target.entity_label);
    const diff = target.only_differs_by.length ? `（只差：${target.only_differs_by.join("；")}）` : "";
    return `${name}${diff}`;
  }).filter(Boolean);
  if (!parts.length) return "";
  return unit.label === "别的材料里见过" ? `见过：${parts.join("；")}` : parts.join("；");
}

import { getLearningView, recheckMathlibBaselines, recheckSummary } from "./api";
import { debtStatusLabel, relationLabel } from "./labels";

export interface LearningViewDeps {
  /** 材料 id → 显示名。 */
  materialName: (artifactId: string) => string;
  showToast: (message: string, kind: "success" | "error") => void;
  /** 点击一条内容时在地图上选中它。 */
  focusObject?: (objectId: string) => void;
  /** 测试注入。 */
  load?: () => Promise<LearningView>;
}

/** 纯函数：一份材料的汇总行，按合并后的单元计数，便于测试。 */
export function materialSummary(material: LearningMaterial): string {
  const units = mergeLearningUnits(material.units);
  const seen = units.filter(unit => unit.label === "别的材料里见过").length;
  const fresh = units.filter(unit => unit.label === "新的").length;
  return `新的 ${fresh} · 别的材料里见过 ${seen} · 新约定 ${material.new_conventions.length} · 未还清的账 ${material.open_debts.length}`;
}

export class LearningViewPanel {
  private readonly root: HTMLElement;
  private readonly body: HTMLElement;
  private readonly picker: HTMLSelectElement;
  private view: LearningView = { materials: [] };

  constructor(host: HTMLElement, private readonly deps: LearningViewDeps) {
    this.root = textElement("section", "learning-view", "");
    this.root.hidden = true;
    this.root.setAttribute("role", "dialog");
    this.root.setAttribute("aria-label", "学习视图");
    const header = textElement("header", "learning-view-header", "");
    header.append(textElement("h2", "", "学习视图"));
    this.picker = document.createElement("select");
    this.picker.className = "learning-view-picker";
    this.picker.setAttribute("aria-label", "选择材料");
    this.picker.onchange = () => this.renderMaterial();
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
    header.append(this.picker, recheck, close);
    this.body = textElement("div", "learning-view-body", "");
    this.root.append(header, this.body);
    this.root.addEventListener("keydown", (event) => { if (event.key === "Escape") this.close(); });
    host.append(this.root);
  }

  get isOpen(): boolean {
    return !this.root.hidden;
  }

  async open(): Promise<void> {
    this.root.hidden = false;
    this.body.replaceChildren(textElement("p", "semantic-muted", "正在计算…"));
    try {
      this.view = await (this.deps.load ?? getLearningView)();
    } catch (error) {
      this.body.replaceChildren(textElement("p", "runtime-problem", errorMessage(error)));
      return;
    }
    const previous = this.picker.value;
    this.picker.replaceChildren(...this.view.materials.map(material => {
      const option = document.createElement("option");
      option.value = material.artifact_id;
      option.textContent = this.deps.materialName(material.artifact_id);
      return option;
    }));
    if (this.view.materials.some(material => material.artifact_id === previous)) this.picker.value = previous;
    this.renderMaterial();
    this.picker.focus();
  }

  close(): void {
    this.root.hidden = true;
  }

  private renderMaterial(): void {
    const material = this.view.materials.find(item => item.artifact_id === this.picker.value) ?? this.view.materials[0];
    if (!material) {
      this.body.replaceChildren(textElement("p", "semantic-muted",
        "地图上还没有对齐线。接受含共享对象和对齐线的提案后，这里会显示每份材料的哪些内容是新的、哪些在别的材料里见过。"));
      return;
    }
    const children: HTMLElement[] = [textElement("p", "learning-view-summary", materialSummary(material))];
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
          textElement("span", "learning-entity", target.entity_label),
        );
        if (target.only_differs_by.length) {
          line.append(textElement("small", "learning-diff", ` · 只差：${target.only_differs_by.join("；")}`));
        }
        item.append(line);
      }
      if (unit.seen_in.length) {
        item.append(textElement("small", "learning-seen-in", ` · 见于 ${unit.seen_in.map(id => this.deps.materialName(id)).join("、")}`));
      }
      if (this.deps.focusObject) {
        item.tabIndex = 0;
        item.onclick = () => this.deps.focusObject?.(unit.unit_id);
      }
      units.append(item);
    }
    children.push(units);
    if (material.new_conventions.length) {
      const conventions = textElement("ul", "learning-view-conventions", "");
      for (const convention of material.new_conventions) {
        const item = textElement("li", "learning-convention", "");
        item.append(textElement("span", "learning-label is-convention", "新约定"), textElement("span", "", convention.text));
        conventions.append(item);
      }
      children.push(textElement("h3", "", "这份材料的约定"), conventions);
    }
    const debts = textElement("footer", "learning-view-debts", "");
    debts.append(textElement("strong", "", `未还清的账 ${material.open_debts.length}`));
    if (material.open_debts.length) {
      const list = textElement("ul", "", "");
      for (const debt of material.open_debts) list.append(textElement("li", "", `${debt.obligation}（${debtStatusLabel(debt.status)}）`));
      debts.append(list);
    }
    children.push(debts, textElement("p", "semantic-muted", "这些标签由对齐线自动计算，不写入地图；「读过」标记以后再做。"));
    this.body.replaceChildren(...children);
  }
}
