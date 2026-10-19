/**
 * N3c: left-pane empty state when 并排阅读 has no reader document yet.
 * Offers opening the note's primary link (原文), picking a source artifact,
 * or importing a PDF — all without leaving split mode.
 */
export interface SplitSourceItem {
  artifactId: string;
  label: string;
}

export interface SplitEmptyOptions {
  primaryLabel?: string;
  /** Primary can be opened in the reader (markdown / readable artifact). */
  primaryOpenable: boolean;
  sources: SplitSourceItem[];
  onOpenPrimary: () => void;
  onOpenSource: (item: SplitSourceItem) => void;
  onImportPdf: () => void;
  onBackToMap: () => void;
}

export function ensureSplitEmptyHost(parent: HTMLElement): HTMLElement {
  let host = parent.querySelector<HTMLElement>("#ink-split-empty");
  if (!host) {
    host = document.createElement("div");
    host.id = "ink-split-empty";
    host.className = "ink-split-empty";
    host.hidden = true;
    host.setAttribute("aria-label", "并排阅读选原文");
    parent.prepend(host);
  }
  return host;
}

export function showSplitEmpty(host: HTMLElement, options: SplitEmptyOptions): void {
  host.hidden = false;
  host.replaceChildren();
  const card = document.createElement("div");
  card.className = "ink-split-empty-card";
  const title = document.createElement("h3");
  title.textContent = "并排阅读";
  const blurb = document.createElement("p");
  blurb.textContent = "左侧显示原文，右侧继续写手稿。当前还没有打开的阅读文档。";
  card.append(title, blurb);

  if (options.primaryLabel) {
    const primary = document.createElement("button");
    primary.type = "button";
    primary.className = "primary-button";
    primary.dataset.action = "primary";
    primary.textContent = options.primaryOpenable
      ? `打开主关联「${options.primaryLabel}」`
      : `主关联「${options.primaryLabel}」（请从下方选择原文）`;
    primary.disabled = !options.primaryOpenable;
    if (options.primaryOpenable) primary.onclick = () => options.onOpenPrimary();
    card.append(primary);
  }

  if (options.sources.length) {
    const heading = document.createElement("h4");
    heading.textContent = "来源材料";
    const list = document.createElement("ul");
    list.className = "ink-split-empty-sources";
    for (const item of options.sources) {
      const li = document.createElement("li");
      const button = document.createElement("button");
      button.type = "button";
      button.className = "secondary-button";
      button.textContent = item.label;
      button.onclick = () => options.onOpenSource(item);
      li.append(button);
      list.append(li);
    }
    card.append(heading, list);
  } else {
    const none = document.createElement("p");
    none.className = "ink-split-empty-hint";
    none.textContent = "工作区里还没有可打开的 PDF / Markdown。可以导入，或回地图从节点打开。";
    card.append(none);
  }

  const actions = document.createElement("div");
  actions.className = "ink-split-empty-actions";
  const importBtn = document.createElement("button");
  importBtn.type = "button";
  importBtn.className = "secondary-button";
  importBtn.textContent = "导入 PDF";
  importBtn.onclick = () => options.onImportPdf();
  const mapBtn = document.createElement("button");
  mapBtn.type = "button";
  mapBtn.className = "secondary-button";
  mapBtn.textContent = "回地图选材料";
  mapBtn.onclick = () => options.onBackToMap();
  actions.append(importBtn, mapBtn);
  card.append(actions);
  host.append(card);
}

export function hideSplitEmpty(host: HTMLElement | null | undefined): void {
  if (!host) return;
  host.hidden = true;
  host.replaceChildren();
}
