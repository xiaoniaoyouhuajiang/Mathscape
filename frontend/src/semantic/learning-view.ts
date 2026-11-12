/**
 * 学习视图（契约 S9.1，教程镜 11）：选一份材料，看它的每条内容是「新的」还是
 * 「别的材料里见过」，以及它带来的「新约定」和还没还清的账。
 *
 * 只读：全部由后端从已接受的对齐线计算，不写入任何对象。「读过」标记不在本期。
 */
import { textElement } from "../core/dom";
import { errorMessage } from "../core/ipc";
import type { LearningMaterial, LearningView } from "./api";
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

/** 纯函数：一份材料的汇总行，便于测试。 */
export function materialSummary(material: LearningMaterial): string {
  const seen = material.units.filter(unit => unit.label === "别的材料里见过").length;
  const fresh = material.units.length - seen;
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
    for (const unit of material.units) {
      const item = textElement("li", `learning-unit is-${unit.label === "新的" ? "new" : "seen"}`, "");
      item.append(
        textElement("span", "learning-label", unit.label),
        textElement("strong", "", unit.unit_title),
        textElement("span", "learning-relation", ` ${relationLabel(unit.relation)} `),
        textElement("span", "learning-entity", unit.entity_label),
      );
      if (unit.seen_in.length) {
        item.append(textElement("small", "learning-seen-in", ` · 见于 ${unit.seen_in.map(id => this.deps.materialName(id)).join("、")}`));
      }
      if (unit.only_differs_by.length) item.append(textElement("small", "learning-diff", ` · 只差：${unit.only_differs_by.join("；")}`));
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
