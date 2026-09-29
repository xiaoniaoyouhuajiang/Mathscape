import { invoke } from "@tauri-apps/api/core";
import { definitionList, formatDuration, textElement } from "../core/dom";
import { errorMessage } from "../core/ipc";
import type {
  DesktopSnapshot,
  MarkerProfileChoice,
  MarkerRuntimeStatus,
  ParseRequest,
  PdfArtifactSummary,
  PdfImportResult,
  PdfParseResult,
} from "./types";

export interface ArtifactControllerElements {
  markerState: HTMLElement;
  markerVersion: HTMLElement;
  markerProblem: HTMLElement;
  lastImport: HTMLElement;
  lastParse: HTMLElement;
}

export interface ArtifactControllerDelegate {
  showToast: (message: string, kind: "success" | "error") => void;
  setBusy: (busy: boolean, message?: string) => void;
  onSnapshotChanged: (snapshot: DesktopSnapshot) => Promise<void> | void;
  getSnapshot: () => DesktopSnapshot | undefined;
  onParseStateChanged?: () => void;
}

export class ArtifactController {
  private activeParse: ParseRequest | null = null;
  private submittingParse = false;
  private pollingParse = false;
  private pollIntervalId: number | undefined;

  constructor(
    private readonly elements: ArtifactControllerElements,
    private readonly delegate: ArtifactControllerDelegate,
  ) {}

  isParseActive(): boolean {
    return this.activeParse !== null;
  }

  getActiveParse(): ParseRequest | null {
    return this.activeParse;
  }

  startPolling(intervalMs = 1000): () => void {
    this.pollIntervalId = window.setInterval(() => void this.syncParseState(), intervalMs);
    return () => {
      if (this.pollIntervalId !== undefined) {
        window.clearInterval(this.pollIntervalId);
        this.pollIntervalId = undefined;
      }
    };
  }

  async importPdf(): Promise<void> {
    this.delegate.setBusy(true, "正在检查并导入 PDF");
    try {
      const result = await invoke<PdfImportResult | null>("choose_and_import_pdf");
      if (result === null) {
        return;
      }
      await this.delegate.onSnapshotChanged(result.snapshot);
      this.renderImportResult(result);
      this.delegate.showToast(
        result.outcome === "inserted"
          ? `已导入 ${result.filename}`
          : `${result.filename} 已存在，复用了原 Artifact`,
        "success",
      );
    } catch (error: unknown) {
      this.delegate.showToast(errorMessage(error), "error");
    } finally {
      this.delegate.setBusy(false);
    }
  }

  renderMarkerStatus(marker: MarkerRuntimeStatus): void {
    const { markerState, markerVersion, markerProblem } = this.elements;
    markerState.textContent = marker.ready ? "可用" : marker.configured ? "异常" : "未配置";
    markerState.classList.toggle("is-ready", marker.ready);
    markerVersion.textContent = marker.version === null ? "—" : `marker-pdf ${marker.version}`;
    markerProblem.textContent = marker.ready
      ? marker.executable ?? "Marker 已通过版本探测"
      : marker.problem ?? "Marker 不可用";
  }

  renderImportResult(result: PdfImportResult): void {
    const { lastImport } = this.elements;
    lastImport.classList.remove("is-empty");
    lastImport.replaceChildren(
      textElement("span", "field-label", "最近一次导入"),
      textElement("strong", "import-name", result.filename),
      definitionList([
        ["结果", result.outcome === "inserted" ? "已写入" : "已去重"],
        ["页数", `${result.page_count}`],
        ["类型", this.contentKindLabel(result.content_kind)],
        ["抽样 OCR 页", `${result.sampled_ocr_page_count}`],
      ]),
    );
  }

  async syncParseState(): Promise<void> {
    if (this.submittingParse || this.pollingParse) return;
    this.pollingParse = true;
    try {
      const task = await invoke<ParseRequest | null>("get_active_parse");
      if (this.submittingParse) return;
      const finished = this.activeParse !== null && task === null;
      if (JSON.stringify(task) !== JSON.stringify(this.activeParse)) {
        this.activeParse = task;
        this.refreshParseState();
        if (finished) {
          this.elements.lastParse.replaceChildren(
            textElement("span", "field-label", "后台任务已结束"),
            textElement("p", "", "正在重新读取来源图；转换结果以目录记录为准。"),
          );
          const currentSnapshot = this.delegate.getSnapshot();
          if (currentSnapshot) {
            await this.delegate.onSnapshotChanged(currentSnapshot);
          }
        }
      }
    } catch {
      // A transient IPC error must not clear the last known running task.
    } finally {
      this.pollingParse = false;
    }
  }

  refreshParseState(): void {
    const { lastParse } = this.elements;
    const currentSnapshot = this.delegate.getSnapshot();
    if (this.activeParse !== null) {
      const task = this.activeParse;
      lastParse.replaceChildren(
        textElement("span", "field-label", "正在解析"),
        textElement(
          "strong",
          "import-name",
          currentSnapshot?.pdf_artifacts.find((pdf) => pdf.artifact_id === task.artifact_id)
            ?.filename ?? "PDF",
        ),
        textElement("p", "", `PDF 第 ${task.first_page}–${task.last_page} 页 · ${task.profile}`),
      );
      this.delegate.setBusy(true, `Marker 正在解析第 ${task.first_page}–${task.last_page} 页`);
    } else {
      this.delegate.setBusy(false);
    }
    this.delegate.onParseStateChanged?.();
  }

  createParseForm(pdf: PdfArtifactSummary, marker: MarkerRuntimeStatus): HTMLFormElement {
    const form = document.createElement("form");
    form.className = "parse-form";
    const heading = textElement("span", "field-label", "Marker 解析范围");
    const range = document.createElement("div");
    range.className = "range-fields";
    const firstPage = this.pageInput("起始页", 1, pdf.page_count);
    const lastPage = this.pageInput("结束页", pdf.page_count ?? 1, pdf.page_count);
    const running = this.activeParse?.artifact_id === pdf.artifact_id ? this.activeParse : null;
    if (running !== null) {
      firstPage.input.value = String(running.first_page);
      lastPage.input.value = String(running.last_page);
    }
    firstPage.input.disabled = this.activeParse !== null;
    lastPage.input.disabled = this.activeParse !== null;
    range.append(firstPage.label, lastPage.label);

    const profileLabel = document.createElement("label");
    profileLabel.className = "form-field profile-field";
    profileLabel.append(textElement("span", "", "识别策略"));
    const profile = document.createElement("select");
    profile.append(
      new Option("Math OCR · balanced", "math_ocr_v1"),
      new Option("Fast native", "fast_native_v1"),
    );
    profileLabel.append(profile);
    if (running !== null) profile.value = running.profile;
    profile.disabled = this.activeParse !== null;

    const submit = document.createElement("button");
    submit.type = "submit";
    submit.className = "parse-button";
    submit.textContent = marker.ready ? "解析选定页面" : "Marker 尚未就绪";
    submit.disabled = !marker.ready || pdf.page_count === null;
    if (this.activeParse !== null) {
      submit.disabled = true;
      submit.textContent = running !== null ? "正在解析…" : "请等待当前解析完成";
    }
    const note = textElement(
      "p",
      "form-note",
      pdf.page_count === null
        ? "该 Artifact 缺少页数，暂不能构造安全页码范围。"
        : `页码按 PDF 页面计算，共 ${pdf.page_count} 页；一次请求必须连续。`,
    );
    form.append(heading, range, profileLabel, submit, note);
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      const first = firstPage.input.valueAsNumber;
      const last = lastPage.input.valueAsNumber;
      if (
        !Number.isInteger(first) ||
        !Number.isInteger(last) ||
        first < 1 ||
        last < first ||
        pdf.page_count === null ||
        last > pdf.page_count
      ) {
        this.delegate.showToast("请输入 PDF 页数范围内的连续页码。", "error");
        return;
      }
      void this.parsePdf(pdf, first, last, profile.value as MarkerProfileChoice, submit);
    });
    return form;
  }

  async parsePdf(
    pdf: PdfArtifactSummary,
    firstPage: number,
    lastPage: number,
    profile: MarkerProfileChoice,
    submit: HTMLButtonElement,
  ): Promise<void> {
    if (this.activeParse !== null || this.submittingParse) return;
    this.submittingParse = true;
    this.activeParse = {
      artifact_id: pdf.artifact_id,
      first_page: firstPage,
      last_page: lastPage,
      profile,
    };
    this.refreshParseState();
    this.delegate.setBusy(true, `Marker 正在解析第 ${firstPage}–${lastPage} 页`);
    submit.disabled = true;
    submit.textContent = "正在解析…";
    try {
      const result = await invoke<PdfParseResult>("parse_pdf", {
        request: {
          artifact_id: pdf.artifact_id,
          first_page: firstPage,
          last_page: lastPage,
          profile,
        },
      });
      await this.delegate.onSnapshotChanged(result.snapshot);
      this.renderParseResult(result, pdf.filename);
      this.delegate.showToast(
        `已解析 ${pdf.filename} 第 ${firstPage}–${lastPage} 页，生成 ${result.outputs.length} 个产物`,
        "success",
      );
    } catch (error: unknown) {
      submit.disabled = false;
      submit.textContent = "解析选定页面";
      this.delegate.showToast(errorMessage(error), "error");
      this.elements.lastParse.replaceChildren(
        textElement("span", "field-label", "解析失败"),
        textElement("p", "", errorMessage(error)),
      );
    } finally {
      this.activeParse = null;
      this.submittingParse = false;
      this.refreshParseState();
      void this.syncParseState();
      this.delegate.setBusy(false);
    }
  }

  renderParseResult(result: PdfParseResult, filename: string): void {
    const { lastParse } = this.elements;
    lastParse.classList.remove("is-empty");
    lastParse.replaceChildren(
      textElement("span", "field-label", "最近一次解析"),
      textElement("strong", "import-name", filename),
      definitionList([
        ["页面", `${result.first_page}–${result.last_page}`],
        ["Profile", result.profile],
        ["产物", `${result.outputs.length}`],
        ["耗时", formatDuration(result.elapsed_ms)],
      ]),
    );
    if (result.cleanup_warning !== null) {
      lastParse.append(textElement("p", "runtime-problem", result.cleanup_warning));
    }
  }

  private pageInput(
    caption: string,
    value: number,
    maximum: number | null,
  ): { label: HTMLLabelElement; input: HTMLInputElement } {
    const label = document.createElement("label");
    label.className = "form-field";
    label.append(textElement("span", "", caption));
    const input = document.createElement("input");
    input.type = "number";
    input.min = "1";
    input.step = "1";
    input.value = value.toString();
    if (maximum !== null) {
      input.max = maximum.toString();
    }
    label.append(input);
    return { label, input };
  }

  private contentKindLabel(kind: PdfImportResult["content_kind"]): string {
    return {
      text_based: "文本型",
      scanned: "扫描型",
      image_based: "图像型",
      mixed: "混合型",
    }[kind];
  }
}
