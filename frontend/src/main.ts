import { invoke } from "@tauri-apps/api/core";
import type { Core, NodeSingular } from "cytoscape";

import { mountCytoscapeGraph } from "./graph/cytoscape";
import type { IncidenceGraph, IncidenceNode } from "./graph/projection";
import "./styles.css";

interface DesktopSnapshot {
  workspace_id: string;
  workspace_root: string;
  artifact_count: number;
  conversion_count: number;
  marker: MarkerRuntimeStatus;
  pdf_artifacts: PdfArtifactSummary[];
  graph: IncidenceGraph;
}

interface MarkerRuntimeStatus {
  configured: boolean;
  ready: boolean;
  executable: string | null;
  version: string | null;
  problem: string | null;
}

interface PdfArtifactSummary {
  artifact_id: string;
  filename: string;
  page_count: number | null;
}

type PdfImportOutcome = "inserted" | "deduplicated";

interface PdfImportResult {
  artifact_id: string;
  content_hash: string;
  byte_size: number;
  filename: string;
  page_count: number;
  document_title: string | null;
  content_kind: "text_based" | "scanned" | "image_based" | "mixed";
  classification_confidence: number;
  sampled_ocr_page_count: number;
  outcome: PdfImportOutcome;
  snapshot: DesktopSnapshot;
}

type MarkerProfileChoice = "fast_native_v1" | "math_ocr_v1";

interface ParsedArtifactOutput {
  artifact_id: string;
  role: string;
  relative_path: string;
  byte_size: number;
}

interface PdfParseResult {
  conversion_id: string;
  input_artifact_id: string;
  first_page: number;
  last_page: number;
  profile: MarkerProfileChoice;
  marker_version: string;
  elapsed_ms: number;
  outputs: ParsedArtifactOutput[];
  cleanup_warning: string | null;
  snapshot: DesktopSnapshot;
}

const app = document.querySelector<HTMLDivElement>("#app");
if (app === null) {
  throw new Error("application root is missing");
}

app.innerHTML = `
  <div class="shell">
    <header class="topbar">
      <div class="brand-block">
        <div class="brand-mark">M</div>
        <div>
          <div class="brand">Mathscape</div>
          <div class="tagline">数学景观 · 来源视图</div>
        </div>
      </div>
      <div class="topbar-actions">
        <span id="service-state" class="service-state">正在连接本地内核</span>
        <button id="import-pdf" class="primary-button" type="button">
          <span aria-hidden="true">＋</span> 导入 PDF
        </button>
      </div>
    </header>

    <main class="workspace-grid">
      <aside class="panel source-panel">
        <div class="panel-heading">
          <span class="eyebrow">SOURCE LAYER</span>
          <h1>来源目录</h1>
        </div>
        <div class="stat-grid">
          <article class="stat-card">
            <span>Artifacts</span>
            <strong id="artifact-count">—</strong>
          </article>
          <article class="stat-card">
            <span>Conversions</span>
            <strong id="conversion-count">—</strong>
          </article>
        </div>
        <section class="workspace-card">
          <span class="field-label">Workspace</span>
          <code id="workspace-id">正在初始化…</code>
          <span class="field-label">本地路径</span>
          <p id="workspace-root" class="path-value">—</p>
        </section>
        <section class="marker-card">
          <div class="runtime-heading">
            <span class="field-label">Marker Runtime</span>
            <span id="marker-state" class="runtime-state">检查中</span>
          </div>
          <strong id="marker-version" class="runtime-version">—</strong>
          <p id="marker-problem" class="runtime-problem">正在探测本地 Marker 环境。</p>
        </section>
        <section id="last-import" class="import-card is-empty">
          <span class="field-label">最近一次导入</span>
          <p>选择一份 PDF，将其复制到自包含 Artifact Pool。</p>
        </section>
        <section id="last-parse" class="import-card is-empty">
          <span class="field-label">最近一次解析</span>
          <p>选择图中的 PDF 节点以配置连续页码范围。</p>
        </section>
      </aside>

      <section class="canvas-panel" aria-label="来源关系图">
        <div class="canvas-toolbar">
          <div>
            <span class="eyebrow">GRAPH PROJECTION</span>
            <h2>Artifact / Conversion</h2>
          </div>
          <div class="legend" aria-label="图例">
            <span><i class="legend-dot entity"></i>Artifact</span>
            <span><i class="legend-dot relation"></i>Conversion</span>
          </div>
        </div>
        <div id="graph" class="graph-canvas"></div>
        <div id="empty-state" class="empty-state">
          <strong>景观尚未生成</strong>
          <span>导入第一份 PDF 后，来源节点会出现在这里。</span>
        </div>
      </section>

      <aside class="panel detail-panel">
        <div class="panel-heading">
          <span class="eyebrow">INSPECTOR</span>
          <h2>节点详情</h2>
        </div>
        <div id="node-detail" class="node-detail empty-detail">
          <div class="detail-symbol">⌁</div>
          <p>点击图中的节点以查看它在来源层中的身份和状态。</p>
        </div>
      </aside>
    </main>

    <div id="toast" class="toast" role="status" aria-live="polite"></div>
  </div>
`;

const importButton = requiredElement<HTMLButtonElement>("#import-pdf");
const serviceState = requiredElement<HTMLElement>("#service-state");
const artifactCount = requiredElement<HTMLElement>("#artifact-count");
const conversionCount = requiredElement<HTMLElement>("#conversion-count");
const workspaceId = requiredElement<HTMLElement>("#workspace-id");
const workspaceRoot = requiredElement<HTMLElement>("#workspace-root");
const graphContainer = requiredElement<HTMLElement>("#graph");
const emptyState = requiredElement<HTMLElement>("#empty-state");
const nodeDetail = requiredElement<HTMLElement>("#node-detail");
const lastImport = requiredElement<HTMLElement>("#last-import");
const lastParse = requiredElement<HTMLElement>("#last-parse");
const markerState = requiredElement<HTMLElement>("#marker-state");
const markerVersion = requiredElement<HTMLElement>("#marker-version");
const markerProblem = requiredElement<HTMLElement>("#marker-problem");
const toast = requiredElement<HTMLElement>("#toast");

let graphCore: Core | null = null;
let toastTimer: number | undefined;

importButton.addEventListener("click", () => void importPdf());
void loadSnapshot();

async function loadSnapshot(): Promise<void> {
  setBusy(true, "正在读取 Workspace");
  try {
    const snapshot = await invoke<DesktopSnapshot>("get_desktop_snapshot");
    renderSnapshot(snapshot);
    serviceState.textContent = "本地内核已连接";
    serviceState.classList.add("is-ready");
  } catch (error: unknown) {
    serviceState.textContent = "Workspace 初始化失败";
    showToast(errorMessage(error), "error");
  } finally {
    setBusy(false);
  }
}

async function importPdf(): Promise<void> {
  setBusy(true, "正在检查并导入 PDF");
  try {
    const result = await invoke<PdfImportResult | null>(
      "choose_and_import_pdf",
    );
    if (result === null) {
      return;
    }
    renderSnapshot(result.snapshot);
    renderImportResult(result);
    showToast(
      result.outcome === "inserted"
        ? `已导入 ${result.filename}`
        : `${result.filename} 已存在，复用了原 Artifact`,
      "success",
    );
  } catch (error: unknown) {
    showToast(errorMessage(error), "error");
  } finally {
    setBusy(false);
  }
}

function renderSnapshot(snapshot: DesktopSnapshot): void {
  artifactCount.textContent = snapshot.artifact_count.toString();
  conversionCount.textContent = snapshot.conversion_count.toString();
  workspaceId.textContent = snapshot.workspace_id;
  workspaceRoot.textContent = snapshot.workspace_root;
  renderMarkerStatus(snapshot.marker);

  graphCore?.destroy();
  graphCore = mountCytoscapeGraph(graphContainer, snapshot.graph);
  emptyState.classList.toggle("is-hidden", snapshot.graph.nodes.length > 0);
  graphCore.on("tap", "node", (event) => {
    const selected = event.target as NodeSingular;
    const model = snapshot.graph.nodes.find(
      (node) => node.id === selected.id(),
    );
    if (model !== undefined) {
      const pdf = snapshot.pdf_artifacts.find(
        (artifact) => artifact.artifact_id === model.domain_id,
      );
      renderNodeDetail(model, pdf, snapshot.marker);
    }
  });
}

function renderImportResult(result: PdfImportResult): void {
  lastImport.classList.remove("is-empty");
  lastImport.replaceChildren(
    textElement("span", "field-label", "最近一次导入"),
    textElement("strong", "import-name", result.filename),
    definitionList([
      ["结果", result.outcome === "inserted" ? "已写入" : "已去重"],
      ["页数", `${result.page_count}`],
      ["类型", contentKindLabel(result.content_kind)],
      ["抽样 OCR 页", `${result.sampled_ocr_page_count}`],
    ]),
  );
}

function renderNodeDetail(
  node: IncidenceNode,
  pdf: PdfArtifactSummary | undefined,
  marker: MarkerRuntimeStatus,
): void {
  nodeDetail.classList.remove("empty-detail");
  nodeDetail.replaceChildren(
    textElement("span", "kind-pill", node.element_kind),
    textElement("h3", "detail-title", node.label),
    definitionList([
      ["Domain ID", node.domain_id ?? "—"],
      ["类型", node.domain_kind ?? "—"],
      ["层级", node.layer],
      ["生命周期", node.lifecycle ?? "—"],
      ["摘要", node.summary ?? "—"],
    ]),
  );
  if (pdf !== undefined) {
    nodeDetail.append(createParseForm(pdf, marker));
  }
}

function createParseForm(
  pdf: PdfArtifactSummary,
  marker: MarkerRuntimeStatus,
): HTMLFormElement {
  const form = document.createElement("form");
  form.className = "parse-form";
  const heading = textElement("span", "field-label", "Marker 解析范围");
  const range = document.createElement("div");
  range.className = "range-fields";
  const firstPage = pageInput("起始页", 1, pdf.page_count);
  const lastPage = pageInput("结束页", pdf.page_count ?? 1, pdf.page_count);
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

  const submit = document.createElement("button");
  submit.type = "submit";
  submit.className = "parse-button";
  submit.textContent = marker.ready ? "解析选定页面" : "Marker 尚未就绪";
  submit.disabled = !marker.ready || pdf.page_count === null;
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
      showToast("请输入 PDF 页数范围内的连续页码。", "error");
      return;
    }
    void parsePdf(
      pdf,
      first,
      last,
      profile.value as MarkerProfileChoice,
      submit,
    );
  });
  return form;
}

function pageInput(
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

async function parsePdf(
  pdf: PdfArtifactSummary,
  firstPage: number,
  lastPage: number,
  profile: MarkerProfileChoice,
  submit: HTMLButtonElement,
): Promise<void> {
  setBusy(true, `Marker 正在解析第 ${firstPage}–${lastPage} 页`);
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
    renderSnapshot(result.snapshot);
    renderParseResult(result, pdf.filename);
    showToast(
      `已解析 ${pdf.filename} 第 ${firstPage}–${lastPage} 页，生成 ${result.outputs.length} 个产物`,
      "success",
    );
  } catch (error: unknown) {
    submit.disabled = false;
    submit.textContent = "解析选定页面";
    showToast(errorMessage(error), "error");
  } finally {
    setBusy(false);
  }
}

function renderMarkerStatus(marker: MarkerRuntimeStatus): void {
  markerState.textContent = marker.ready ? "可用" : marker.configured ? "异常" : "未配置";
  markerState.classList.toggle("is-ready", marker.ready);
  markerVersion.textContent = marker.version === null ? "—" : `marker-pdf ${marker.version}`;
  markerProblem.textContent = marker.ready
    ? marker.executable ?? "Marker 已通过版本探测"
    : marker.problem ?? "Marker 不可用";
}

function renderParseResult(result: PdfParseResult, filename: string): void {
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
    lastParse.append(
      textElement("p", "runtime-problem", result.cleanup_warning),
    );
  }
}

function definitionList(rows: ReadonlyArray<readonly [string, string]>): HTMLElement {
  const list = document.createElement("dl");
  list.className = "definition-list";
  for (const [term, value] of rows) {
    list.append(textElement("dt", "", term), textElement("dd", "", value));
  }
  return list;
}

function textElement<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className: string,
  value: string,
): HTMLElementTagNameMap[K] {
  const element = document.createElement(tag);
  element.className = className;
  element.textContent = value;
  return element;
}

function requiredElement<T extends Element>(selector: string): T {
  const element = document.querySelector<T>(selector);
  if (element === null) {
    throw new Error(`required element is missing: ${selector}`);
  }
  return element;
}

function setBusy(busy: boolean, message = ""): void {
  importButton.disabled = busy;
  importButton.classList.toggle("is-busy", busy);
  if (busy) {
    serviceState.textContent = message;
  } else if (!serviceState.textContent?.includes("失败")) {
    serviceState.textContent = "本地内核已连接";
  }
}

function showToast(message: string, kind: "success" | "error"): void {
  window.clearTimeout(toastTimer);
  toast.textContent = message;
  toast.dataset.kind = kind;
  toast.classList.add("is-visible");
  toastTimer = window.setTimeout(() => toast.classList.remove("is-visible"), 5000);
}

function errorMessage(error: unknown): string {
  return typeof error === "string" ? error : "发生了未知错误";
}

function contentKindLabel(kind: PdfImportResult["content_kind"]): string {
  return {
    text_based: "文本型",
    scanned: "扫描型",
    image_based: "图像型",
    mixed: "混合型",
  }[kind];
}

function formatDuration(milliseconds: number): string {
  if (milliseconds < 1_000) {
    return `${milliseconds} ms`;
  }
  const seconds = milliseconds / 1_000;
  return seconds < 60 ? `${seconds.toFixed(1)} s` : `${(seconds / 60).toFixed(1)} min`;
}
