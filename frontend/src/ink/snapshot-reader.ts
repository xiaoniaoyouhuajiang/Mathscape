import { eventBus } from "../core/events";
import { InkCanvas } from "./canvas";
import { InkEvidencePanel } from "./evidence-panel";
import { parseInkSnapshotManifest, projectInk } from "./model";
import type { InkSelection, StoredInkSnapshot } from "./persistence";

export interface InkReaderFocus { page_id: string; stroke_ids: string[] }

export function mountInkSnapshotReader(host: HTMLElement, snapshot: StoredInkSnapshot, focus?: InkReaderFocus): () => void {
  const manifest = parseInkSnapshotManifest(JSON.parse(snapshot.manifest_json));
  let pageId = focus?.page_id ?? manifest.document.pages[0].id;
  let ids = focus?.stroke_ids ?? [];
  let selectionMode: InkSelection["selection_mode"] = "lasso";
  if (!manifest.document.pages.some(page => page.id === pageId)) throw new Error("来源选区的页面不存在");
  const visible = projectInk(manifest.document).visible;
  if (ids.some(id => !visible.some(stroke => stroke.id === id && stroke.page_id === pageId))) throw new Error("来源选区包含快照中不可见或不属于该页的笔迹");
  const controls = document.createElement("div"); controls.className = "reader-controls ink-reader-controls";
  const label = document.createElement("span"); label.textContent = `历史快照 · 只读 · 截止 head ${snapshot.through_seq}`;
  const select = document.createElement("select"); select.setAttribute("aria-label", "快照页面");
  manifest.document.pages.forEach((page,index) => select.add(new Option(`第 ${index + 1} 页`, page.id)));
  select.value = pageId;
  const fit = document.createElement("button"); fit.textContent = "整页"; fit.type = "button";
  const zoomIn = document.createElement("button"); zoomIn.textContent = "放大"; zoomIn.type = "button";
  const zoomOut = document.createElement("button"); zoomOut.textContent = "缩小"; zoomOut.type = "button";
  const resume = document.createElement("button"); resume.textContent = "继续编辑当前手稿"; resume.type = "button";
  resume.onclick = () => eventBus.emit("open-ink-document", { documentId: snapshot.document_id });
  controls.append(label,select,zoomOut,fit,zoomIn,resume);
  const evidence = document.createElement("div");
  const boardHost = document.createElement("div"); boardHost.className = "ink-desktop-board ink-reader-board";
  boardHost.setAttribute("aria-label", "只读手稿快照");
  const message = document.createElement("p"); message.className = "ink-reader-message"; message.setAttribute("role", "status");
  host.append(controls,evidence,boardHost,message);
  let panel: InkEvidencePanel;
  const board = new InkCanvas(boardHost, {
    document: () => manifest.document,
    page: () => manifest.document.pages.find(page => page.id === pageId)!,
    segmentId: () => { throw new Error("只读快照不能采集笔迹"); },
    brush: () => ({ renderer: "perfect-freehand@1.2.3",color:"#193a66",width:4,smoothing:0.35,streamline:0.15,thinning:0.4,simulate_pressure:false }),
    tool: () => "select", fingerWriting: () => false, readOnly: true,
    onStroke: () => {}, onErase: () => {}, onPartialErase: () => {}, onTransform: () => {},
    onSelection: selected => { ids = selected; selectionMode = "lasso"; panel?.update(); },
    onMessage: text => { message.textContent = text; }, onMetrics: () => {},
  });
  const choose = (selection: InkSelection) => {
    board.resetTransientInteraction(); pageId = selection.page_id; select.value = pageId;
    board.fit(); board.setSelection(selection.stroke_ids); selectionMode = selection.selection_mode; panel?.update();
  };
  panel = new InkEvidencePanel(evidence,snapshot,() => ({page_id:pageId,stroke_ids:ids,selection_mode:selectionMode}),choose);
  select.onchange = () => choose({page_id:select.value,stroke_ids:[],selection_mode:"lasso"});
  fit.onclick = () => board.fit(); zoomIn.onclick = () => board.zoom(1.3); zoomOut.onclick = () => board.zoom(1/1.3);
  board.setSelection(ids); board.refresh();
  return () => { panel.dispose(); board.dispose(); };
}
