/**
 * PDF 透明文本层（PDF.js `TextLayer`）、跨页全文查找器与反查落点高亮控制器。
 */
import {
  setLayerDimensions,
  TextLayer,
  type PDFDocumentProxy,
  type PDFPageProxy,
} from "pdfjs-dist/legacy/build/pdf.mjs";

export type PageViewport = ReturnType<PDFPageProxy["getViewport"]>;

/** Normalizes text for case-insensitive, whitespace-tolerant PDF matching. */
export function normalizeSearchText(text: string): string {
  return text.normalize("NFKC").replace(/\s+/g, "").toLowerCase();
}

/**
 * Extracts a distinctive plain-text phrase (3..22 chars) from a Markdown source
 * excerpt so jumping to a PDF page can highlight the corresponding line in `.textLayer`.
 */
export function extractAnchorPhrase(markdownExcerpt: string | undefined | null): string | null {
  if (!markdownExcerpt) return null;
  const cleaned = markdownExcerpt
    .replace(/^\s*\{\d+\}-{8,}\s*$/gm, " ")
    .replace(/\$\$[\s\S]*?\$\$/g, " ")
    .replace(/\\\[[\s\S]*?\\\]/g, " ")
    .replace(/\$[^$\n]+\$/g, " ")
    .replace(/\\\([^\n]*?\\\)/g, " ")
    .replace(/!\[[^\]]*\]\([^)]*\)/g, " ")
    .replace(/[#>*_`~[\]]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  if (!cleaned) return null;

  // Prefer the first meaningful clause before punctuation, at least 3 non-space chars.
  const clauses = cleaned
    .split(/[。；！？\n|]+/)
    .map((s) => s.trim())
    .filter((s) => normalizeSearchText(s).length >= 3);
  const candidate = clauses[0] ?? cleaned;
  const compact = [...candidate].slice(0, 20).join("").trim();
  return normalizeSearchText(compact).length >= 3 ? compact : null;
}

/**
 * Reads a page's text-run strings with a stream reader. `PDFPageProxy.getTextContent`
 * iterates a `ReadableStream` with `for await`, which WebKitGTK (the Linux Tauri
 * WebView) does not support.
 */
export async function readPageTextStrings(page: Pick<PDFPageProxy, "streamTextContent">): Promise<string[]> {
  const reader = page.streamTextContent().getReader();
  const strs: string[] = [];
  for (;;) {
    const { value, done } = await reader.read();
    if (done) return strs;
    for (const item of value.items) {
      strs.push("str" in item && typeof item.str === "string" ? item.str : "");
    }
  }
}

export interface PageSpanMatch {
  spanIndices: number[];
}

/**
 * Finds all occurrences of `query` across a page's `spanStrings` even when a
 * phrase is split across adjacent PDF text-run spans.
 */
export function findSpanMatchesInPage(
  spanStrings: readonly string[],
  query: string,
): PageSpanMatch[] {
  const needle = normalizeSearchText(query);
  if (!needle) return [];

  let joined = "";
  const ownerAtChar: number[] = [];
  for (let spanIdx = 0; spanIdx < spanStrings.length; spanIdx++) {
    const norm = normalizeSearchText(spanStrings[spanIdx] ?? "");
    if (!norm) continue;
    for (let k = 0; k < norm.length; k++) {
      ownerAtChar.push(spanIdx);
    }
    joined += norm;
  }
  if (!joined || joined.length < needle.length) return [];

  const matches: PageSpanMatch[] = [];
  let from = 0;
  while (from <= joined.length - needle.length) {
    const idx = joined.indexOf(needle, from);
    if (idx < 0) break;
    const spanSet = new Set<number>();
    for (let c = idx; c < idx + needle.length; c++) {
      spanSet.add(ownerAtChar[c]);
    }
    matches.push({ spanIndices: [...spanSet] });
    from = idx + Math.max(1, needle.length);
  }
  return matches;
}

export interface MountedPdfTextLayer {
  readonly container: HTMLElement;
  updateViewport(viewport: PageViewport): void;
  applyHighlights(opts: {
    searchQuery?: string;
    activeMatchIndexOnPage?: number | null;
    backlinkPhrase?: string | null;
  }): HTMLElement | null;
  dispose(): void;
}

export async function mountPdfTextLayer(
  page: PDFPageProxy,
  viewport: PageViewport,
  shell: HTMLElement,
  isCancelled: () => boolean,
): Promise<MountedPdfTextLayer | null> {
  const container = document.createElement("div");
  container.className = "textLayer";
  shell.style.setProperty("--total-scale-factor", String(viewport.scale));
  shell.style.setProperty("--scale-round-x", "1px");
  shell.style.setProperty("--scale-round-y", "1px");
  setLayerDimensions(container, viewport);
  shell.append(container);

  const layer = new TextLayer({
    textContentSource: page.streamTextContent(),
    container,
    viewport,
  });

  try {
    await layer.render();
  } catch (err) {
    if (err instanceof Error && (err.name === "AbortException" || err.name === "RenderingCancelledException")) {
      container.remove();
      return null;
    }
    container.remove();
    return null;
  }

  if (isCancelled()) {
    layer.cancel();
    container.remove();
    return null;
  }

  return {
    container,
    updateViewport(nextViewport: PageViewport) {
      shell.style.setProperty("--total-scale-factor", String(nextViewport.scale));
      setLayerDimensions(container, nextViewport);
      layer.update({ viewport: nextViewport });
    },
    applyHighlights({ searchQuery, activeMatchIndexOnPage, backlinkPhrase }) {
      const divs = layer.textDivs;
      const strs = layer.textContentItemsStr;
      for (const div of divs) {
        div.classList.remove("pdf-find-hit", "pdf-find-hit-active", "pdf-backlink-text-hit");
      }
      let focusSpan: HTMLElement | null = null;

      if (backlinkPhrase) {
        const backHits = findSpanMatchesInPage(strs, backlinkPhrase);
        if (backHits.length > 0) {
          for (const idx of backHits[0].spanIndices) {
            const el = divs[idx];
            if (el) {
              el.classList.add("pdf-backlink-text-hit");
              if (!focusSpan) focusSpan = el;
            }
          }
        }
      }

      if (searchQuery && normalizeSearchText(searchQuery)) {
        const hits = findSpanMatchesInPage(strs, searchQuery);
        hits.forEach((hit, matchIdx) => {
          const isActive = activeMatchIndexOnPage === matchIdx;
          for (const idx of hit.spanIndices) {
            const el = divs[idx];
            if (!el) continue;
            el.classList.add("pdf-find-hit");
            if (isActive) {
              el.classList.add("pdf-find-hit-active");
              focusSpan = el;
            }
          }
        });
      }
      return focusSpan;
    },
    dispose() {
      layer.cancel();
      container.remove();
    },
  };
}

export interface GlobalPdfMatch {
  pageNumber: number;
  matchIndexOnPage: number;
}

export class PdfFindController {
  readonly bar = document.createElement("div");
  readonly toggleButton = document.createElement("button");
  private readonly input = document.createElement("input");
  private readonly countLabel = document.createElement("span");
  private readonly prevBtn = document.createElement("button");
  private readonly nextBtn = document.createElement("button");
  private readonly closeBtn = document.createElement("button");
  private pageTextCache = new Map<number, string[]>();
  private indexingPromise: Promise<void> | null = null;
  private matches: GlobalPdfMatch[] = [];
  private activeIdx = -1;
  private query = "";
  private searchVersion = 0;
  private disposed = false;

  constructor(
    private readonly pdf: PDFDocumentProxy,
    private readonly onNavigateToMatch: (match: GlobalPdfMatch | null) => void,
    private readonly onRefreshVisibleHighlights: () => void,
  ) {
    this.toggleButton.type = "button";
    this.toggleButton.textContent = "查找";
    this.toggleButton.title = "在 PDF 中查找文字（Ctrl/⌘ + F）";
    this.toggleButton.setAttribute("aria-pressed", "false");
    this.toggleButton.onclick = () => {
      if (this.bar.hidden) this.open();
      else this.close();
    };

    this.bar.className = "pdf-find-bar";
    this.bar.hidden = true;
    this.bar.setAttribute("role", "search");
    this.bar.setAttribute("aria-label", "PDF 页内查找");

    this.input.type = "search";
    this.input.className = "pdf-find-input";
    this.input.placeholder = "查找原书文字…";
    this.input.setAttribute("aria-label", "查找 PDF 文字");

    this.countLabel.className = "pdf-find-count";
    this.countLabel.setAttribute("role", "status");
    this.countLabel.textContent = "";

    this.prevBtn.type = "button";
    this.prevBtn.textContent = "上一个";
    this.prevBtn.disabled = true;
    this.prevBtn.onclick = () => this.step(-1);

    this.nextBtn.type = "button";
    this.nextBtn.textContent = "下一个";
    this.nextBtn.disabled = true;
    this.nextBtn.onclick = () => this.step(1);

    this.closeBtn.type = "button";
    this.closeBtn.textContent = "✕";
    this.closeBtn.title = "关闭查找栏（Esc）";
    this.closeBtn.setAttribute("aria-label", "关闭查找栏");
    this.closeBtn.onclick = () => this.close();

    this.input.addEventListener("input", () => void this.runSearch(this.input.value));
    this.input.addEventListener("keydown", (event) => {
      if (event.key === "Enter") {
        event.preventDefault();
        this.step(event.shiftKey ? -1 : 1);
      } else if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        this.close();
      }
    });

    this.bar.append(this.input, this.countLabel, this.prevBtn, this.nextBtn, this.closeBtn);
  }

  isOpen(): boolean {
    return !this.bar.hidden;
  }

  currentQuery(): string {
    return this.bar.hidden ? "" : this.query;
  }

  activeMatchForPage(pageNumber: number): number | null {
    if (this.bar.hidden || this.activeIdx < 0 || this.activeIdx >= this.matches.length) return null;
    const active = this.matches[this.activeIdx];
    return active.pageNumber === pageNumber ? active.matchIndexOnPage : null;
  }

  open(): void {
    this.bar.hidden = false;
    this.toggleButton.setAttribute("aria-pressed", "true");
    this.input.focus();
    this.input.select();
    if (this.input.value.trim()) {
      void this.runSearch(this.input.value);
    }
  }

  close(): void {
    if (this.bar.hidden) return;
    this.bar.hidden = true;
    this.toggleButton.setAttribute("aria-pressed", "false");
    this.query = "";
    this.matches = [];
    this.activeIdx = -1;
    this.countLabel.textContent = "";
    this.prevBtn.disabled = this.nextBtn.disabled = true;
    this.onRefreshVisibleHighlights();
  }

  dispose(): void {
    this.disposed = true;
    this.searchVersion++;
    this.bar.remove();
    this.toggleButton.remove();
  }

  private async ensureIndexed(): Promise<void> {
    if (this.pageTextCache.size === this.pdf.numPages) return;
    if (this.indexingPromise) return this.indexingPromise;
    this.indexingPromise = (async () => {
      for (let p = 1; p <= this.pdf.numPages; p++) {
        if (this.disposed) return;
        if (this.pageTextCache.has(p)) continue;
        try {
          const page = await this.pdf.getPage(p);
          if (this.disposed) return;
          this.pageTextCache.set(p, await readPageTextStrings(page));
        } catch {
          this.pageTextCache.set(p, []);
        }
      }
    })().finally(() => {
      this.indexingPromise = null;
    });
    return this.indexingPromise;
  }

  private async runSearch(rawQuery: string): Promise<void> {
    const version = ++this.searchVersion;
    this.query = rawQuery;
    if (!normalizeSearchText(rawQuery)) {
      this.matches = [];
      this.activeIdx = -1;
      this.countLabel.textContent = "";
      this.prevBtn.disabled = this.nextBtn.disabled = true;
      this.onRefreshVisibleHighlights();
      return;
    }

    this.countLabel.textContent = "正在检索…";
    await this.ensureIndexed();
    if (this.disposed || version !== this.searchVersion) return;

    const found: GlobalPdfMatch[] = [];
    for (let p = 1; p <= this.pdf.numPages; p++) {
      const strs = this.pageTextCache.get(p) ?? [];
      const pageHits = findSpanMatchesInPage(strs, rawQuery);
      for (let m = 0; m < pageHits.length; m++) {
        found.push({ pageNumber: p, matchIndexOnPage: m });
      }
    }
    this.matches = found;
    if (found.length === 0) {
      this.activeIdx = -1;
      this.countLabel.textContent = "无匹配";
      this.prevBtn.disabled = this.nextBtn.disabled = true;
      this.onRefreshVisibleHighlights();
      return;
    }
    this.activeIdx = 0;
    this.prevBtn.disabled = this.nextBtn.disabled = found.length <= 1;
    this.updateCountLabel();
    this.onNavigateToMatch(found[0]);
  }

  private step(delta: 1 | -1): void {
    if (!this.matches.length) return;
    this.activeIdx = (this.activeIdx + delta + this.matches.length) % this.matches.length;
    this.updateCountLabel();
    this.onNavigateToMatch(this.matches[this.activeIdx]);
  }

  private updateCountLabel(): void {
    if (!this.matches.length || this.activeIdx < 0) {
      this.countLabel.textContent = "无匹配";
      return;
    }
    this.countLabel.textContent = `${this.activeIdx + 1} / ${this.matches.length}`;
  }
}
