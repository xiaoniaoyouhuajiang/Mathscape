import { eventBus } from "../core/events";
import { errorMessage } from "../core/ipc";
import { isMapTarget, type SearchHit, type SearchQuery, type SearchResponse } from "./api";
import "./search.css";

export type SearchAction = "open" | "locate";

export interface SearchPanelOptions {
  search: (query: SearchQuery) => Promise<SearchResponse>;
  /** Navigates to a result. The panel is already closed when this runs. */
  activate: (hit: SearchHit, action: SearchAction) => void | Promise<void>;
  /** Debounce before a query is sent (ms). */
  debounceMs?: number;
}

const OPTION_ID = "workspace-search-option-";

/** ⌘K / Ctrl+K workspace search (N3d). Results are grouped by type; the
 * panel only renders and forwards — scoring lives in `mathscape-query`. */
export class WorkspaceSearchPanel {
  readonly root: HTMLElement;
  private readonly input: HTMLInputElement;
  private readonly trash: HTMLInputElement;
  private readonly results: HTMLElement;
  private readonly statusLine: HTMLElement;
  private hits: SearchHit[] = [];
  private active = -1;
  private request = 0;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private returnFocus: HTMLElement | null = null;
  /** Enter pressed while a query is still in flight: act once it lands. */
  private pendingAction: SearchAction | undefined;
  private inFlight = false;

  constructor(private readonly options: SearchPanelOptions) {
    this.root = document.createElement("div");
    this.root.id = "workspace-search";
    this.root.className = "workspace-search";
    this.root.hidden = true;
    this.root.innerHTML =
      '<div class="workspace-search-box" role="dialog" aria-modal="true" aria-label="搜索工作区">' +
      '<div class="workspace-search-head"><input id="workspace-search-input" class="workspace-search-input" type="search" role="combobox" aria-autocomplete="list" aria-expanded="false" aria-controls="workspace-search-results" autocomplete="off" spellcheck="false" placeholder="搜索手稿、材料、来源单元、理论、共享对象、命名快照…">' +
      '<label class="workspace-search-trash"><input id="workspace-search-trash" type="checkbox"> 包含回收站</label></div>' +
      '<p id="workspace-search-status" class="workspace-search-status" role="status" aria-live="polite"></p>' +
      '<div id="workspace-search-results" class="workspace-search-results" role="listbox" aria-label="搜索结果"></div>' +
      '<p class="workspace-search-hint">↑↓ 选择 · Enter 打开 · Alt+Enter 在地图定位 · Esc 关闭</p></div>';
    this.input = this.root.querySelector<HTMLInputElement>("#workspace-search-input")!;
    this.trash = this.root.querySelector<HTMLInputElement>("#workspace-search-trash")!;
    this.results = this.root.querySelector<HTMLElement>("#workspace-search-results")!;
    this.statusLine = this.root.querySelector<HTMLElement>("#workspace-search-status")!;
    this.input.addEventListener("input", () => this.schedule());
    this.trash.addEventListener("change", () => this.schedule(0));
    this.root.addEventListener("keydown", event => this.onKey(event));
    // Clicking the backdrop (not the box) closes.
    this.root.addEventListener("pointerdown", event => { if (event.target === this.root) this.close(); });
    document.body.append(this.root);
  }

  isOpen(): boolean { return !this.root.hidden; }

  open(query?: string): void {
    if (!this.isOpen()) this.returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    this.root.hidden = false;
    if (query !== undefined) this.input.value = query;
    this.input.focus();
    this.input.select();
    if (this.input.value.trim()) this.schedule(0); else this.render(undefined);
  }

  close(restoreFocus = true): void {
    if (!this.isOpen()) return;
    this.request++;
    if (this.timer) clearTimeout(this.timer);
    this.inFlight = false;
    this.pendingAction = undefined;
    this.root.hidden = true;
    this.input.setAttribute("aria-expanded", "false");
    if (restoreFocus && this.returnFocus?.isConnected) this.returnFocus.focus();
    this.returnFocus = null;
  }

  toggle(): void { if (this.isOpen()) this.close(); else this.open(); }

  private schedule(delay = this.options.debounceMs ?? 120): void {
    if (this.timer) clearTimeout(this.timer);
    const request = ++this.request;
    const text = this.input.value;
    this.pendingAction = undefined;
    if (!text.trim()) { this.inFlight = false; this.render(undefined); return; }
    this.inFlight = true;
    this.timer = setTimeout(() => void this.run(request, text), delay);
  }

  private async run(request: number, text: string): Promise<void> {
    // Only show "searching" when the answer is slow; keep old results meanwhile.
    const slow = setTimeout(() => { if (request === this.request) this.status("正在搜索…"); }, 150);
    try {
      const response = await this.options.search({ text, include_trashed: this.trash.checked });
      if (request !== this.request || !this.isOpen()) return; // A newer query or a close won.
      this.inFlight = false;
      this.render(response);
      const action = this.pendingAction;
      this.pendingAction = undefined;
      if (action && this.hits.length) this.activate(this.active, action);
    } catch (error) {
      if (request !== this.request || !this.isOpen()) return;
      this.inFlight = false;
      this.pendingAction = undefined;
      this.hits = []; this.active = -1;
      this.results.replaceChildren();
      this.status("搜索失败：" + errorMessage(error), true);
    } finally {
      clearTimeout(slow);
    }
  }

  private status(text: string, error = false): void {
    this.statusLine.textContent = text;
    this.statusLine.classList.toggle("is-error", error);
  }

  private render(response: SearchResponse | undefined): void {
    this.results.replaceChildren();
    this.hits = [];
    this.active = -1;
    if (!response) {
      this.status("输入名称查找：手稿标题、材料文件名、来源单元、理论与共享对象、命名快照。");
      this.input.setAttribute("aria-expanded", "false");
      this.input.removeAttribute("aria-activedescendant");
      return;
    }
    if (!response.total) {
      this.status(`没有找到“${response.query.trim()}”。` + (this.trash.checked ? "" : "可勾选“包含回收站”。"));
      this.input.setAttribute("aria-expanded", "false");
      this.input.removeAttribute("aria-activedescendant");
      return;
    }
    this.status(`共 ${response.total} 个结果`);
    for (const group of response.groups) {
      const section = document.createElement("div");
      section.className = "workspace-search-group";
      section.setAttribute("role", "group");
      section.dataset.kind = group.kind;
      const heading = document.createElement("div");
      heading.className = "workspace-search-group-title";
      heading.id = `workspace-search-group-${group.kind}`;
      heading.textContent = group.total > group.hits.length ? `${group.label} · 前 ${group.hits.length} / ${group.total}` : `${group.label} · ${group.total}`;
      section.setAttribute("aria-labelledby", heading.id);
      section.append(heading);
      for (const hit of group.hits) section.append(this.option(hit, this.hits.push(hit) - 1));
      this.results.append(section);
    }
    this.input.setAttribute("aria-expanded", "true");
    this.select(0);
  }

  private option(hit: SearchHit, index: number): HTMLElement {
    const row = document.createElement("div");
    row.id = OPTION_ID + index;
    row.className = "workspace-search-option";
    row.setAttribute("role", "option");
    row.setAttribute("aria-selected", "false");
    row.dataset.kind = hit.kind;
    row.dataset.id = hit.id;
    row.classList.toggle("is-trashed", hit.trashed);
    const text = document.createElement("div");
    text.className = "workspace-search-text";
    const title = document.createElement("span");
    title.className = "workspace-search-title";
    let cursor = 0;
    for (const [start, end] of hit.highlights) {
      if (start < cursor || end > hit.title.length) continue;
      title.append(hit.title.slice(cursor, start));
      const mark = document.createElement("mark");
      mark.textContent = hit.title.slice(start, end);
      title.append(mark);
      cursor = end;
    }
    title.append(hit.title.slice(cursor));
    const subtitle = document.createElement("span");
    subtitle.className = "workspace-search-subtitle";
    subtitle.textContent = [hit.subtitle, hit.matched_alias ? `匹配：${hit.matched_alias}` : null].filter(Boolean).join(" · ");
    text.append(title, subtitle);
    row.append(text);
    if (isMapTarget(hit.target)) {
      const locate = document.createElement("button");
      locate.type = "button";
      locate.tabIndex = -1;
      locate.className = "workspace-search-locate";
      locate.textContent = "定位";
      locate.title = "在地图中定位（Alt+Enter）";
      locate.addEventListener("click", event => { event.stopPropagation(); this.activate(index, "locate"); });
      row.append(locate);
    }
    row.addEventListener("pointermove", () => { if (this.active !== index) this.select(index, false); });
    row.addEventListener("click", () => this.activate(index, "open"));
    return row;
  }

  private select(index: number, scroll = true): void {
    if (!this.hits.length) return;
    const next = (index + this.hits.length) % this.hits.length;
    this.results.querySelector(`#${OPTION_ID}${this.active}`)?.setAttribute("aria-selected", "false");
    this.active = next;
    const row = this.results.querySelector<HTMLElement>(`#${OPTION_ID}${next}`);
    row?.setAttribute("aria-selected", "true");
    this.input.setAttribute("aria-activedescendant", OPTION_ID + next);
    if (scroll) row?.scrollIntoView({ block: "nearest" });
  }

  private activate(index: number, action: SearchAction): void {
    const hit = this.hits[index];
    if (!hit) return;
    if (action === "locate" && !isMapTarget(hit.target)) action = "open";
    // Navigation moves focus elsewhere; do not restore it to the trigger.
    this.close(false);
    void Promise.resolve(this.options.activate(hit, action)).catch(() => undefined);
  }

  private onKey(event: KeyboardEvent): void {
    if (event.isComposing) return;
    if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); this.close(); return; }
    if (event.key === "ArrowDown") { event.preventDefault(); this.select(this.active + 1); return; }
    if (event.key === "ArrowUp") { event.preventDefault(); this.select(this.active - 1); return; }
    if (event.key === "Enter" && event.target === this.input) {
      event.preventDefault();
      const action: SearchAction = event.altKey ? "locate" : "open";
      // Never act on results of an older query.
      if (this.inFlight) { this.pendingAction = action; return; }
      this.activate(this.active, action);
      return;
    }
    if (event.key === "Tab") {
      // Keep focus inside the dialog: input ↔ trash toggle.
      event.preventDefault();
      (document.activeElement === this.input ? this.trash : this.input).focus();
    }
  }
}

/** ⌘K / Ctrl+K anywhere toggles the panel; the 搜索 buttons open it. */
export function installWorkspaceSearch(panel: WorkspaceSearchPanel): () => void {
  const onKey = (event: KeyboardEvent) => {
    if (event.isComposing || event.altKey || event.shiftKey) return;
    if (!(event.metaKey || event.ctrlKey) || event.key.toLowerCase() !== "k") return;
    event.preventDefault();
    panel.toggle();
  };
  window.addEventListener("keydown", onKey, true);
  const off = eventBus.on("open-workspace-search", detail => {
    panel.open(detail && typeof detail === "object" ? detail.query : undefined);
  });
  return () => { window.removeEventListener("keydown", onKey, true); off(); };
}
