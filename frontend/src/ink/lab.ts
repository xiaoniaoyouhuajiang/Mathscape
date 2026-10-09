import QRCode from "qrcode";
import { InkCanvas, type CanvasMetrics, type InkTool } from "./canvas";
import { appendInk, encodeInk, inkId, MAX_INK_BYTES, newDocument, parseInkDocument, projectInk, type CaptureSegment, type InkBrush, type InkDocument, type InkStroke, type InkPage, type InkTransform } from "./model";
import "./lab.css";

const root = document.querySelector<HTMLDivElement>("#ink-app")!;
const fragment = new URLSearchParams(location.hash.slice(1));
const isHost = location.pathname !== "/pad";
const credentialKey = isHost ? "mathscape-ink-host" : "mathscape-ink-pad";
let credential = fragment.get(isHost ? "host" : "session") ?? sessionStorage.getItem(credentialKey) ?? "";
if (credential) sessionStorage.setItem(credentialKey, credential);
// Keep the fragment in the address bar: it is never sent in an HTTP request, and
// a user copying the visible pad URL must be able to open it on another device.
if (credential) history.replaceState(null, "", `${location.pathname}#${isHost ? "host" : "session"}=${encodeURIComponent(credential)}`);

function errorText(error: unknown): string { return error instanceof Error ? error.message : String(error); }
async function api<T>(path: string, options: RequestInit = {}): Promise<T> {
  const response = await fetch(path, { ...options, headers: { Authorization: `Bearer ${credential}`, ...options.headers } });
  if (!response.ok) {
    const result = await response.json().catch(() => ({ error: `HTTP ${response.status}` }));
    throw new Error(result.error ?? `HTTP ${response.status}`);
  }
  return await response.json() as T;
}
async function connectPad(): Promise<void> {
  let lastError: unknown;
  for (let attempt = 0; attempt < 5; attempt++) {
    try { await api("/api/pad"); return; }
    catch (error) {
      lastError = error;
      // A tablet can load the static page just before the LAN route is ready.
      // Retry transport failures, while still surfacing a real 401 immediately.
      if (error instanceof Error && error.message.includes("链接已失效")) throw error;
      await new Promise(resolve => window.setTimeout(resolve, 250 * (attempt + 1)));
    }
  }
  throw lastError instanceof Error ? lastError : new Error("无法连接电脑 Ink Lab");
}
function download(bytes: Blob, name: string): void {
  const url = URL.createObjectURL(bytes), anchor = document.createElement("a");
  anchor.href = url; anchor.download = name; document.body.append(anchor); anchor.click(); anchor.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 60_000);
}
function element<T extends HTMLElement>(selector: string): T { return root.querySelector<T>(selector)!; }

interface Receipt { hash: string; filename: string; title: string; point_count: number; stroke_count: number; visible_count: number; saved_at: string }
interface LabInfo { pad_urls: string[]; output_directory: string; exports: Receipt[] }

async function host(): Promise<void> {
  root.innerHTML = `
    <main class="lab-host">
      <header class="lab-brand"><span class="lab-mark">M</span><div><strong>Mathscape</strong><span>手写实验 · P0</span></div></header>
      <section class="lab-intro"><span class="lab-eyebrow">INK LAB</span><h1>让草稿进入 Mathscape</h1><p>电脑与平板连接同一局域网，选择电脑地址，再用平板扫码打开画板。</p></section>
      <section class="host-grid">
        <article class="lab-card pairing-card"><h2>在平板上书写</h2><label for="lab-address">电脑的局域网地址</label><select id="lab-address"></select><canvas id="lab-qr" aria-label="画板二维码"></canvas><input id="lab-url" aria-label="平板画板地址" readonly><div class="lab-actions"><button id="lab-copy">复制地址</button><a id="lab-local" class="lab-primary" target="_blank" rel="noopener">本机试写</a></div><p class="lab-note">此实验通过 HTTP 提供画板。合并采样等能力以画板诊断为准；停止服务后链接失效。</p></article>
        <article class="lab-card"><h2>电脑收到的手稿</h2><p>在画板点击“保存到电脑”，这里会出现完整的 Ink JSON 文件。</p><div id="lab-files" class="lab-files"></div><label>实验文件目录</label><code id="lab-output"></code><p class="lab-note">文件保留采样和编辑历史，可在画板导入重现。刷新前请手动导出；自动保存与断线恢复属于下一阶段。</p></article>
      </section>
      <p id="lab-status" class="lab-status" role="status">正在连接实验服务…</p>
    </main>`;
  const status = element("#lab-status"), select = element<HTMLSelectElement>("#lab-address");
  let info: LabInfo | undefined, rendered = "";
  const updateQr = async () => {
    const url = select.value;
    element<HTMLInputElement>("#lab-url").value = url;
    await QRCode.toCanvas(element<HTMLCanvasElement>("#lab-qr"), url, { width: 256, margin: 3, errorCorrectionLevel: "M" });
  };
  select.addEventListener("change", () => void updateQr().catch(error => status.textContent = errorText(error)));
  element("#lab-copy").addEventListener("click", () => {
    void (async () => {
      try { await navigator.clipboard.writeText(select.value); status.textContent = "画板地址已复制。"; }
      catch { element<HTMLInputElement>("#lab-url").select(); status.textContent = "地址已选中，请手动复制。"; }
    })();
  });
  const refresh = async () => {
    try {
      const next = await api<LabInfo>("/api/lab");
      if (!info) {
        for (const url of next.pad_urls) {
          const option = document.createElement("option"); option.value = url;
          const hostname = new URL(url).hostname; option.textContent = hostname === "127.0.0.1" ? "127.0.0.1（仅本机）" : hostname;
          select.append(option);
        }
        element<HTMLAnchorElement>("#lab-local").href = next.pad_urls.at(-1)!;
        element("#lab-output").textContent = next.output_directory;
        await updateQr();
        status.textContent = next.pad_urls.length > 1 ? "实验服务已就绪。平板扫码后即可书写。" : "未发现局域网 IPv4 地址；本机仍可试写，请先检查电脑网络。";
      }
      info = next;
      const key = JSON.stringify(next.exports);
      if (key === rendered) return;
      rendered = key;
      const list = element("#lab-files"); list.replaceChildren();
      if (!next.exports.length) { const empty = document.createElement("p"); empty.className = "lab-empty"; empty.textContent = "尚未收到手稿"; list.append(empty); }
      for (const receipt of [...next.exports].reverse()) {
        const item = document.createElement("article"); item.className = "lab-file";
        const title = document.createElement("strong"); title.textContent = receipt.title;
        const detail = document.createElement("span"); detail.textContent = `${receipt.stroke_count} 笔 · ${receipt.point_count} 个采样 · ${new Date(receipt.saved_at).toLocaleTimeString()}`;
        const button = document.createElement("button"); button.textContent = "下载 Ink JSON";
        button.addEventListener("click", () => {
          void (async () => {
            try {
              const response = await fetch(`/api/files/${receipt.hash}`, { headers: { Authorization: `Bearer ${credential}` } });
              if (!response.ok) throw new Error("文件下载失败，请检查实验服务。");
              download(await response.blob(), receipt.filename); status.textContent = "文件已生成下载，请确认浏览器保存成功。";
            } catch (error) { status.textContent = errorText(error); }
          })();
        });
        item.append(title, detail, button); list.append(item);
      }
    } catch (error) { status.textContent = errorText(error); }
  };
  await refresh(); window.setInterval(() => void refresh(), 1500);
}

async function pad(): Promise<void> {
  root.innerHTML = `
    <main class="lab-pad">
      <header class="pad-header"><div class="pad-title"><span class="lab-mark">M</span><input id="ink-title" aria-label="手稿标题" value="数学手稿" maxlength="200"><span class="lab-badge">P0</span></div><span id="ink-connection" class="pad-connection">连接中…</span></header>
      <nav class="ink-toolbar" aria-label="书写工具">
        <div class="ink-tool-group"><button id="ink-pen" class="is-active" aria-pressed="true">笔</button><button id="ink-select" aria-pressed="false">套索选择</button><button id="ink-erase" aria-pressed="false">整笔擦除</button><button id="ink-partial_erase" aria-pressed="false">局部擦除</button><button id="ink-undo" disabled>撤销</button><button id="ink-redo" disabled>重做</button></div>
        <div class="ink-tool-group"><label class="ink-control">颜色<input id="ink-color" type="color" value="#193a66"></label><label class="ink-control">笔宽<input id="ink-width" type="range" min="1" max="16" step="0.5" value="4"></label><label class="ink-control"><input id="ink-finger" type="checkbox">手指书写</label></div>
        <div class="ink-tool-group"><select id="ink-page" aria-label="页面"></select><button id="ink-add-page">新增页面</button><button id="ink-zoom-out" aria-label="缩小">−</button><button id="ink-fit">整页</button><button id="ink-zoom-in" aria-label="放大">＋</button></div>
      </nav>
      <section id="ink-board" class="ink-board" aria-label="画板区域"></section>
      <footer class="pad-footer"><div class="ink-file-actions"><button id="ink-export" class="lab-primary">导出 Ink JSON</button><button id="ink-send">保存到电脑</button><button id="ink-png">导出 PNG</button><button id="ink-import">导入 Ink JSON</button><button id="ink-new">新建画板</button><button id="ink-retry" hidden>重试连接</button><input id="ink-file" type="file" accept="application/json,.json" hidden></div><p id="ink-status" class="lab-status" role="status">使用笔书写；触摸拖动或双指缩放。刷新前请手动导出文件。</p><details class="ink-diagnostics"><summary>输入诊断 <span id="ink-summary"></span></summary><div id="ink-diagnostics"></div></details></footer>
    </main>`;
  let documentState: InkDocument = newDocument(), currentPage = documentState.pages[0].id;
  let currentSegment: CaptureSegment | undefined, tool: InkTool = "pen", selectedStrokeIds: string[] = [];
  let revision = 0, lastExportRevision = 0, lastSavedRevision = -1, documentEpoch = 0;
  let lastSavedSeq = -1;
  const status = element("#ink-status"), connection = element("#ink-connection");
  const message = (text: string) => { status.textContent = text; };
  const segment = () => {
    if (!currentSegment) {
      if (documentState.capture_segments.length >= 1000) throw new Error("采集片段达到实验上限，请先导出并新建画板。");
      currentSegment = { id: inkId(), started_at: new Date().toISOString(), user_agent: navigator.userAgent,
        secure_context: window.isSecureContext, coalesced_events: typeof PointerEvent.prototype.getCoalescedEvents === "function",
        pointer_raw_update: "onpointerrawupdate" in window, device_pixel_ratio: window.devicePixelRatio || 1,
        time_origin_ms: performance.timeOrigin, pressure_capability: "unknown", tilt_capability: "unknown" };
      documentState.capture_segments.push(currentSegment);
    }
    return currentSegment.id;
  };
  const brush = (): InkBrush => ({ renderer: "perfect-freehand@1.2.3", color: element<HTMLInputElement>("#ink-color").value,
    width: Number(element<HTMLInputElement>("#ink-width").value), smoothing: 0.35, streamline: 0.15, thinning: 0.4, simulate_pressure: false });
  let metrics: CanvasMetrics = { pointerEvents: 0, coalescedSamples: 0, cancelled: 0, lastPointer: "—", frameMs: 0, activePoints: 0 };
  function update(): void {
    const projection = projectInk(documentState);
    element<HTMLButtonElement>("#ink-undo").disabled = !projection.undo.length;
    element<HTMLButtonElement>("#ink-redo").disabled = !projection.redo.length;
    element("#ink-summary").textContent = `${projection.visible.length} 笔 · ${projection.pointCount + metrics.activePoints} 点${selectedStrokeIds.length ? ` · 已选 ${selectedStrokeIds.length} 笔` : ""}`;
    const latest = projection.strokes.at(-1);
    const pressure = latest?.pointer_type === "pen" ? latest.points.filter(p => p.buttons !== 0 && p.pressure !== null).map(p => p.pressure!) : [];
    let minimum = 1, maximum = 0;
    for (const value of pressure) { minimum = Math.min(minimum, value); maximum = Math.max(maximum, value); }
    const pressureText = pressure.length ? `${minimum.toFixed(3)}–${maximum.toFixed(3)}（API 报告值，能力待实测）` : "未记录笔输入压感";
    element("#ink-diagnostics").textContent = `安全上下文：${window.isSecureContext ? "是" : "否（LAN HTTP）"} · 合并采样 API：${typeof PointerEvent.prototype.getCoalescedEvents === "function" ? "可用" : "不可用"}\n当前输入：${metrics.lastPointer} · 接收事件：${metrics.pointerEvents} · 合并采样：${metrics.coalescedSamples} · pointercancel：${metrics.cancelled}\n最近笔压：${pressureText} · 最近前景绘制：${metrics.frameMs.toFixed(2)} ms（不是笔尖物理延迟）\n历史：${documentState.operations.length} 个操作 · 桌面保存至：${lastSavedSeq < 0 ? "尚未保存" : `操作 ${lastSavedSeq}`}\n${navigator.userAgent}`;
  }
  const changed = () => { update(); };
  const board = new InkCanvas(element("#ink-board"), {
    document: () => documentState, page: () => documentState.pages.find(p => p.id === currentPage)!,
    segmentId: segment, brush, tool: () => tool, fingerWriting: () => element<HTMLInputElement>("#ink-finger").checked,
    onStroke: stroke => { appendInk(documentState, { kind: "stroke", stroke }); revision++; changed(); },
    onErase: stroke_ids => { appendInk(documentState, { kind: "erase", stroke_ids }); revision++; changed(); },
    onPartialErase: (source_stroke_id, fragments) => { if (fragments.length) appendInk(documentState, { kind: "partial_erase", source_stroke_id, fragments }); else appendInk(documentState, { kind: "erase", stroke_ids: [source_stroke_id] }); revision++; changed(); },
    onTransform: (stroke_ids, matrix) => { appendInk(documentState, { kind: "transform", stroke_ids, matrix }); revision++; changed(); },
    onSelection: ids => { selectedStrokeIds = ids; changed(); },
    onMessage: message, onMetrics: m => { metrics = m; },
  });
  const pages = () => {
    const select = element<HTMLSelectElement>("#ink-page"); select.replaceChildren();
    documentState.pages.forEach((page, index) => { const option = document.createElement("option"); option.value = page.id; option.textContent = `第 ${index + 1} 页`; select.append(option); });
    select.value = currentPage;
  };
  pages(); update();
  window.setInterval(update, 250);
  for (const type of ["pen", "select", "erase", "partial_erase"] as const) element(`#ink-${type}`).addEventListener("click", () => {
    board.finish(); tool = type;
    for (const value of ["pen", "select", "erase", "partial_erase"] as const) { element(`#ink-${value}`).classList.toggle("is-active", value === tool); element(`#ink-${value}`).setAttribute("aria-pressed", String(value === tool)); }
  });
  element("#ink-add-page").addEventListener("click", () => {
    board.finish(); tool = "pen"; for (const value of ["pen", "select", "erase", "partial_erase"] as const) { element(`#ink-${value}`).classList.toggle("is-active", value === tool); element(`#ink-${value}`).setAttribute("aria-pressed", String(value === tool)); } const page: InkPage = { id: inkId(), width: 1200, height: 1600 };
    documentState.pages.push(page); appendInk(documentState, { kind: "page_add", page }); currentPage = page.id; selectedStrokeIds = []; revision++; pages(); board.fit(); changed(); message("已新增页面。");
  });
  for (const kind of ["undo", "redo"] as const) element(`#ink-${kind}`).addEventListener("click", () => {
    try {
      board.finish(); const target = projectInk(documentState)[kind].at(-1);
      if (target) { appendInk(documentState, { kind, target_command_id: target }); revision++; board.refresh(); changed(); }
    } catch (error) { message(errorText(error)); }
  });
  element("#ink-fit").addEventListener("click", () => board.fit());
  element("#ink-zoom-in").addEventListener("click", () => board.zoom(1.3));
  element("#ink-zoom-out").addEventListener("click", () => board.zoom(1 / 1.3));
  element<HTMLSelectElement>("#ink-page").addEventListener("change", e => { board.finish(); currentPage = (e.target as HTMLSelectElement).value; board.fit(); });
  element<HTMLInputElement>("#ink-title").addEventListener("input", e => {
    const title = (e.target as HTMLInputElement).value.trim() || "数学手稿";
    if (title !== documentState.title) { documentState.title = title; revision++; }
  });
  element("#ink-export").addEventListener("click", () => {
    try {
      board.finish(); const json = encodeInk(documentState);
      download(new Blob([json], { type: "application/json" }), `mathscape-ink-${documentState.document_id}.json`);
      lastExportRevision = revision; message("Ink JSON 已生成下载，请确认浏览器保存成功。文件可重新导入继续书写。");
    } catch (error) { message(errorText(error)); }
  });
  element("#ink-png").addEventListener("click", () => {
    try { board.png().toBlob(blob => { if (blob) { download(blob, "mathscape-ink-preview.png"); message("PNG 预览已生成下载。笔迹与编辑历史请使用 Ink JSON 保存。"); } else message("PNG 生成失败，请使用 Ink JSON。"); }); }
    catch (error) { message(errorText(error)); }
  });
  element("#ink-send").addEventListener("click", () => {
    void (async () => {
      const button = element<HTMLButtonElement>("#ink-send");
      try {
        board.finish(); const seq = documentState.operations.length, epoch = documentEpoch, savedRevision = revision;
        const body = encodeInk(documentState); button.disabled = true; message("正在把当前内容保存到电脑…");
        const receipt = await api<Receipt>("/api/export", { method: "POST", headers: { "Content-Type": "application/json" }, body });
        if (documentEpoch === epoch) { lastSavedSeq = seq; lastSavedRevision = savedRevision; }
        message(`电脑已保存 ${receipt.stroke_count} 笔、${receipt.point_count} 个采样。${documentEpoch !== epoch || revision !== savedRevision ? "后续修改尚未发送。" : "可在桌面页面下载文件。"}`);
        connection.textContent = "电脑可连接"; changed();
      } catch (error) { connection.textContent = "连接失败"; message(`${errorText(error)}；当前内容仍在画板中，可先下载 Ink JSON。`); }
      finally { button.disabled = false; }
    })();
  });
  const file = element<HTMLInputElement>("#ink-file");
  element("#ink-import").addEventListener("click", () => file.click());
  file.addEventListener("change", () => {
    void (async () => {
      try {
        const chosen = file.files?.[0]; if (!chosen) return;
        if (chosen.size > MAX_INK_BYTES) throw new Error("文件超过 64 MiB 实验上限。");
        const imported = parseInkDocument(JSON.parse(await chosen.text()));
        board.finish();
        if (revision !== lastExportRevision && !confirm("当前手稿有尚未导出的修改。导入会替换画板，是否继续？")) return;
        documentState = imported; currentPage = imported.pages[0].id;
        currentSegment = undefined; lastSavedSeq = -1; revision = 0; lastExportRevision = 0; lastSavedRevision = -1; documentEpoch++; selectedStrokeIds = []; board.setSelection([]);
        element<HTMLInputElement>("#ink-title").value = imported.title; pages(); board.fit(); changed();
        message(`已导入 ${projectInk(imported).strokes.length} 笔及完整编辑历史，可以继续书写。`);
      } catch (error) { message(errorText(error)); }
      finally { file.value = ""; }
    })();
  });
  element("#ink-new").addEventListener("click", () => {
    board.finish();
    if (revision !== lastExportRevision && !confirm("当前手稿有尚未导出的修改。是否新建画板？")) return;
    documentState = newDocument(); currentPage = documentState.pages[0].id;
    currentSegment = undefined; lastSavedSeq = -1; revision = 0; lastExportRevision = 0; lastSavedRevision = -1; documentEpoch++; selectedStrokeIds = []; board.setSelection([]);
    element<HTMLInputElement>("#ink-title").value = documentState.title; pages(); board.fit(); changed(); message("已新建画板。刷新前请手动导出。");
  });
  window.addEventListener("beforeunload", e => { board.finish(); if (revision !== lastExportRevision && revision !== lastSavedRevision) { e.preventDefault(); e.returnValue = ""; } });
  const retry = element<HTMLButtonElement>("#ink-retry");
  const checkConnection = async () => {
    retry.disabled = true; connection.textContent = "连接中…";
    try { await connectPad(); connection.textContent = "电脑可连接"; retry.hidden = true; message("电脑已连接。手动保存会把完整 Ink JSON 写入电脑。"); }
    catch (error) {
      connection.textContent = "离线试写 · 可导出"; retry.hidden = false;
      message(`${errorText(error)}；可先导出 JSON，确认电脑后台运行后重试连接。`);
    } finally { retry.disabled = false; }
  };
  retry.addEventListener("click", () => void checkConnection());
  await checkConnection();
}

void (isHost ? host() : pad()).catch(error => { root.textContent = errorText(error); });
