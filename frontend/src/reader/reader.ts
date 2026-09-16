import { getDocument, GlobalWorkerOptions, type PDFDocumentProxy } from "pdfjs-dist/legacy/build/pdf.mjs";
import { ArtifactPdfTransport } from "./pdf-transport";
import workerUrl from "pdfjs-dist/legacy/build/pdf.worker.min.mjs?url";
import { contentInfo, readRange, readSmallFile, resolveProductPath, type ContentInfo } from "./content";
import { markdownRenderer } from "./markdown";
import { markdownDocument, MarkdownSelection } from "./selection";
import "./reader.css";

GlobalWorkerOptions.workerSrc = workerUrl;

function element<K extends keyof HTMLElementTagNameMap>(tag: K, text = ""): HTMLElementTagNameMap[K] {
  const result = document.createElement(tag);
  result.textContent = text;
  return result;
}

export class ArtifactReader {
  private generation = 0;
  private disposeDocument: (() => void) | undefined;
  private urls: string[] = [];
  constructor(private host: HTMLElement) {}

  private revokeImages(): void {
    this.urls.forEach(url => URL.revokeObjectURL(url));
    this.urls = [];
  }

  close(): void {
    this.generation++;
    this.disposeDocument?.();
    this.disposeDocument = undefined;
    this.revokeImages();
    this.host.replaceChildren();
  }

  async open(id: string, title: string, focus?: { start: number; end: number; blockIds?: string[] }): Promise<void> {
    this.close();
    const generation = this.generation;
    const heading = element("h2", title);
    const status = element("p", "正在读取…");
    this.host.append(heading, status);
    try {
      const info = await contentInfo(id);
      if (generation !== this.generation) return;
      if (info.media_type === "application/pdf") {
        await this.pdf(info, generation);
      } else if (info.media_type === "text/markdown") {
        await this.markdown(info, generation, focus);
      } else if (info.media_type === "application/json" || info.media_type.startsWith("text/")) {
        const bytes = await readSmallFile(info);
        if (generation !== this.generation) return;
        const text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
        const pre = element("pre", info.media_type === "application/json" ? JSON.stringify(JSON.parse(text), null, 2) : text);
        pre.className = "reader-text";
        this.host.append(pre);
      } else if (["image/png", "image/jpeg", "image/webp", "image/gif"].includes(info.media_type)) {
        const bytes = await readSmallFile(info);
        if (generation !== this.generation) return;
        const img = element("img");
        img.src = this.blobUrl(bytes, info.media_type);
        img.alt = title;
        this.host.append(img);
      } else {
        throw new Error("当前不支持此文件类型：" + info.media_type);
      }
      status.remove();
    } catch (error) {
      if (generation === this.generation) status.textContent = error instanceof Error ? error.message : String(error);
    }
  }

  private blobUrl(bytes: Uint8Array, media: string): string {
    const url = URL.createObjectURL(new Blob([bytes.slice().buffer], { type: media }));
    this.urls.push(url);
    return url;
  }

  private async pdf(info: ContentInfo, generation: number): Promise<void> {
    const initial = await readRange(info.artifact_id, 0, Math.min(65536, info.byte_size));
    if (generation !== this.generation) return;
    const failure = element("p");
    let firstFailure: Error | undefined;
    let rejectRead!: (error: Error) => void;
    const readFailure = new Promise<never>((_, reject) => { rejectRead = reject; });
    // Failures can also occur after loading.promise has resolved.
    void readFailure.catch(() => {});
    const transport = new ArtifactPdfTransport(info.byte_size, initial,
      (begin, length) => readRange(info.artifact_id, begin, length),
      error => {
        if (generation !== this.generation || firstFailure) return;
        firstFailure = error;
        failure.textContent = error.message;
        console.error("PDF range read failed", { artifactId: info.artifact_id, error });
        rejectRead(error);
        dispose();
      });
    const loading = getDocument({
      range: transport, disableAutoFetch: true, disableStream: true,
      cMapUrl: "/pdf-assets/cmaps/", cMapPacked: true,
      standardFontDataUrl: "/pdf-assets/standard_fonts/",
      wasmUrl: "/pdf-assets/wasm/",
    });
    let destroyed = false;
    const dispose = () => {
      transport.abort();
      if (destroyed) return;
      destroyed = true;
      void loading.destroy().catch(error => console.warn("PDF cleanup failed", error));
    };
    this.disposeDocument = dispose;
    this.host.append(failure);
    let pdf: PDFDocumentProxy;
    try {
      pdf = await Promise.race([loading.promise, readFailure]);
    } catch (error) {
      dispose();
      if (!firstFailure) throw error;
      return; // The detailed first read failure is already displayed.
    }
    if (generation !== this.generation || transport.stopped) return;
    const controls = element("div");
    controls.className = "reader-controls";
    const previous = element("button", "上一页");
    const next = element("button", "下一页");
    const pageInput = element("input");
    pageInput.type = "number"; pageInput.min = "1"; pageInput.max = String(pdf.numPages); pageInput.value = "1";
    pageInput.setAttribute("aria-label", "PDF 页码");
    const zoom = element("select");
    zoom.setAttribute("aria-label", "缩放");
    for (const scale of ["0.75", "1", "1.5", "2"]) zoom.add(new Option(Number(scale) * 100 + "%", scale));
    zoom.value = "1";
    controls.append(previous, pageInput, element("span", "/ " + pdf.numPages + " 页"), next, zoom);
    const layout = element("div"); layout.className = "pdf-layout";
    const outline = element("nav"); outline.className = "pdf-outline"; outline.setAttribute("aria-label", "PDF 目录");
    const viewport = element("div"); viewport.className = "pdf-viewport";
    layout.append(outline, viewport);
    this.host.append(controls, layout);
    let pageNumber = 1, renderVersion = 0;
    let renderTask: ReturnType<Awaited<ReturnType<PDFDocumentProxy["getPage"]>>["render"]> | undefined;
    const oldDispose = this.disposeDocument;
    this.disposeDocument = () => { renderVersion++; renderTask?.cancel(); oldDispose?.(); };
    const show = async (number: number): Promise<void> => {
      if (transport.stopped) return;
      if (!Number.isInteger(number) || number < 1 || number > pdf.numPages) { pageInput.value = String(pageNumber); return; }
      pageNumber = number; pageInput.value = String(number);
      previous.disabled = number === 1; next.disabled = number === pdf.numPages;
      const version = ++renderVersion;
      renderTask?.cancel();
      try {
        const page = await pdf.getPage(number);
        if (transport.stopped || generation !== this.generation || version !== renderVersion) return;
        const view = page.getViewport({ scale: Number(zoom.value) });
        const canvas = element("canvas");
        const ratio = window.devicePixelRatio || 1;
        canvas.width = Math.ceil(view.width * ratio); canvas.height = Math.ceil(view.height * ratio);
        canvas.style.width = view.width + "px"; canvas.style.height = view.height + "px";
        viewport.replaceChildren(canvas);
        renderTask = page.render({ canvas, viewport: view, transform: [ratio, 0, 0, ratio, 0, 0] });
        await renderTask.promise;
      } catch (error) {
        if (!firstFailure && !transport.stopped && version === renderVersion && generation === this.generation) failure.textContent = String(error);
      }
    };
    previous.onclick = () => void show(pageNumber - 1);
    next.onclick = () => void show(pageNumber + 1);
    pageInput.onchange = () => void show(pageInput.valueAsNumber);
    zoom.onchange = () => void show(pageNumber);
    await show(1);
    if (transport.stopped) return;
    const items = await pdf.getOutline().catch(error => {
      if (firstFailure || transport.stopped) return null;
      throw error;
    });
    if (generation !== this.generation) return;
    if (!items?.length) outline.append(element("p", "此 PDF 没有内置书签目录，可输入页码跳转。"));
    const add = (entries: NonNullable<typeof items>, parent: HTMLElement): void => {
      for (const item of entries) {
        const button = element("button", item.title);
        button.disabled = !item.dest;
        button.onclick = () => void (async () => {
          const destination = typeof item.dest === "string" ? await pdf.getDestination(item.dest) : item.dest;
          if (!destination) return;
          const index = typeof destination[0] === "number" ? destination[0] : await pdf.getPageIndex(destination[0]);
          if (generation === this.generation) await show(index + 1);
        })().catch(error => { if (!firstFailure && !transport.stopped && generation === this.generation) failure.textContent = String(error); });
        parent.append(button);
        if (item.items.length) { const child = element("div"); parent.append(child); add(item.items, child); }
      }
    };
    if (items) add(items, outline);
  }

  private async markdown(info: ContentInfo, generation: number, focus?: { start: number; end: number; blockIds?: string[] }): Promise<void> {
    let document = await markdownDocument(info.artifact_id, info.contexts[0]?.conversion_id ?? null);
    if (generation !== this.generation) return;
    const controls = element("div"); controls.className = "reader-controls";
    const raw = element("button", "查看原文");
    const contextSelect = element("select");
    contextSelect.setAttribute("aria-label", "转换来源");
    info.contexts.forEach((context, i) => contextSelect.add(new Option("转换来源 " + (i + 1), String(i))));
    controls.append(raw);
    if (info.contexts.length > 1) controls.append(contextSelect);
    const article = element("article"); article.className = "markdown-body";
    this.host.append(controls, article);
    let selection = new MarkdownSelection(document, article);
    controls.after(selection.panel);
    this.disposeDocument = () => selection.dispose();
    let rawMode = false, version = 0;
    const render = async (): Promise<void> => {
      const current = ++version;
      this.revokeImages();
      if (rawMode) { article.replaceChildren(element("pre", document.source)); return; }
      const md = markdownRenderer();
      const environment = {};
      // Parse the full source once to retain reference-style link/image definitions.
      md.parse(document.source, environment);
      const sourceBytes = new TextEncoder().encode(document.source);
      const decoder = new TextDecoder("utf-8", { fatal: true });
      article.replaceChildren();
      document.blocks.forEach((block, i) => {
        const wrapper = element("div");
        wrapper.className = "source-block";
        wrapper.dataset.sourceBlock = block.id;
        if (focus?.blockIds?.includes(block.id)) wrapper.classList.add("source-block-focus");
        wrapper.tabIndex = 0;
        wrapper.setAttribute("aria-label", `原文块 ${i + 1}`);
        wrapper.title = `第 ${i + 1} 块 · ${block.kind} · UTF-8 [${block.range.start}, ${block.range.end})`;
        const text = decoder.decode(sourceBytes.subarray(block.range.start, block.range.end));
        wrapper.innerHTML = md.render(text, environment);
        if (!wrapper.innerHTML.trim()) wrapper.append(element("pre", text));
        article.append(wrapper);
      });
      if (focus) (article.querySelector<HTMLElement>(".source-block-focus") as HTMLElement | null)?.scrollIntoView({ behavior: "smooth", block: "center" });
      selection.refresh();
      article.querySelectorAll("a").forEach(link => {
        link.removeAttribute("href");
        link.title = "链接导航尚未启用";
      });
      const context = info.contexts[Number(contextSelect.value) || 0];
      const bases = context?.paths.filter(p => p.artifact_id === info.artifact_id) ?? [];
      const placeholders = article.querySelectorAll<HTMLElement>("[data-image-ref]");
      for (const placeholder of placeholders) {
        if (generation !== this.generation || current !== version) return;
        try {
          const candidates = new Set<string>();
          for (const base of bases) {
            const path = resolveProductPath(base.path, placeholder.dataset.imageRef ?? "");
            context?.paths.filter(p => p.path === path).forEach(p => candidates.add(p.artifact_id));
          }
          if (candidates.size !== 1) throw new Error("图片映射缺失或不明确");
          const imageInfo = await contentInfo([...candidates][0]);
          if (!["image/png", "image/jpeg", "image/webp", "image/gif"].includes(imageInfo.media_type)) throw new Error("不支持的图片类型");
          const data = await readSmallFile(imageInfo);
          if (generation !== this.generation || current !== version) return;
          const img = element("img"); img.alt = placeholder.textContent ?? "";
          img.src = this.blobUrl(data, imageInfo.media_type);
          placeholder.replaceWith(img);
        } catch (error) {
          placeholder.textContent = "[" + String(error) + "]";
        }
      }
    };
    raw.onclick = () => { rawMode = !rawMode; raw.textContent = rawMode ? "查看渲染" : "查看原文"; void render(); };
    contextSelect.onchange = () => {
      const current = ++version;
      selection.dispose();
      this.revokeImages();
      article.textContent = "正在切换转换来源…";
      raw.disabled = true;
      void markdownDocument(info.artifact_id, info.contexts[Number(contextSelect.value)]?.conversion_id ?? null).then(data => {
        if (generation !== this.generation || current !== version) return;
        document = data;
        selection = new MarkdownSelection(document, article);
        controls.after(selection.panel);
        raw.disabled = false;
        return render();
      }).catch(error => {
        if (generation === this.generation && current === version) article.textContent = String(error);
      });
    };
    await render();
  }
}
