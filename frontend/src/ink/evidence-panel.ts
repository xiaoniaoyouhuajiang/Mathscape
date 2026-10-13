import { eventBus } from "../core/events";
import { inkId, parseInkSnapshotManifest, projectInk } from "./model";
import { listInkEvidence, registerInkEvidence, type InkEvidenceRequest, type InkSelection, type StoredInkSnapshot } from "./persistence";

/** Shared controls for immutable snapshot evidence in the workspace and reader. */
export class InkEvidencePanel {
  private readonly summary = document.createElement("span");
  private readonly register = document.createElement("button");
  private readonly list = document.createElement("div");
  private readonly message = document.createElement("span");
  private request: InkEvidenceRequest | undefined;
  private submitting = false;
  private disposed = false;
  private generation = 0;

  constructor(
    private readonly host: HTMLElement,
    private readonly snapshot: StoredInkSnapshot,
    private readonly selection: () => InkSelection,
    private readonly choose: (selection: InkSelection) => void,
  ) {
    host.className = "ink-evidence-panel";
    const all = document.createElement("button");
    all.type = "button"; all.textContent = "选择本页全部笔迹";
    all.onclick = () => {
      const manifest = parseInkSnapshotManifest(JSON.parse(snapshot.manifest_json));
      const page_id = selection().page_id;
      choose({ page_id, stroke_ids: projectInk(manifest.document).visible.filter(stroke => stroke.page_id === page_id).map(stroke => stroke.id), selection_mode: "whole_strokes" });
    };
    const clear = document.createElement("button");
    clear.type = "button"; clear.textContent = "清除选区";
    clear.onclick = () => choose({ ...selection(), stroke_ids: [] });
    this.register.type = "button"; this.register.textContent = "登记选区为来源证据";
    this.register.onclick = () => void this.submit();
    this.summary.className = "ink-selection-summary";
    this.message.className = "ink-evidence-message";
    this.message.setAttribute("role", "status");
    const controls = document.createElement("div");
    controls.className = "ink-evidence-controls";
    controls.append(this.summary, all, clear, this.register, this.message);
    const history = document.createElement("details");
    const label = document.createElement("summary");
    label.textContent = "此快照已登记的来源证据";
    history.append(label, this.list);
    host.replaceChildren(controls, history);
    this.update();
    void this.refresh();
  }

  update(): void {
    const selected = this.selection();
    this.summary.textContent = `已选 ${selected.stroke_ids.length} 笔 · 只读选区`;
    this.host.dataset.selectedStrokes = selected.stroke_ids.join(",");
    this.register.disabled = this.submitting || !selected.stroke_ids.length;
  }

  dispose(): void { this.disposed = true; this.generation++; }

  private async submit(): Promise<void> {
    const selected = this.selection();
    if (this.submitting || !selected.stroke_ids.length) return;
    const next = { ...selected, stroke_ids: [...new Set(selected.stroke_ids)].sort(), snapshot_id: this.snapshot.snapshot_id, expected_artifact_id: this.snapshot.artifact_id, expected_content_hash: this.snapshot.content_hash };
    // Keep the same request after an unknown outcome; changing the selection
    // explicitly begins a new authorized registration request.
    if (!this.request || JSON.stringify({ ...this.request, request_id: undefined }) !== JSON.stringify(next)) this.request = { ...next, request_id: inkId() };
    this.submitting = true; this.update();
    this.message.textContent = "正在登记…";
    try {
      const receipt = await registerInkEvidence(this.request);
      eventBus.emit("source-map-changed", { reason: "ink-evidence" });
      if (this.disposed) return;
      this.message.textContent = receipt.outcome === "created" ? "来源证据已登记，可从地图打开原快照选区。" : "已复用相同选区的来源证据。";
      await this.refresh();
    } catch (error) {
      if (!this.disposed) this.message.textContent = "登记失败，可重试：" + (error instanceof Error ? error.message : String(error));
    } finally { this.submitting = false; if (!this.disposed) this.update(); }
  }

  private async refresh(): Promise<void> {
    const generation = ++this.generation;
    try {
      const receipts = await listInkEvidence(this.snapshot.snapshot_id);
      if (this.disposed || generation !== this.generation) return;
      this.list.replaceChildren();
      if (!receipts.length) { this.list.textContent = "此快照尚未登记来源证据。"; return; }
      for (const [index,receipt] of receipts.entries()) {
        const button = document.createElement("button");
        button.type = "button";
        button.textContent = `证据 ${index + 1} · ${receipt.stroke_ids.length} 笔`;
        button.title = receipt.span_id;
        button.onclick = () => this.choose(receipt);
        this.list.append(button);
      }
    } catch (error) {
      if (!this.disposed && generation === this.generation) this.list.textContent = "来源证据列表读取失败：" + String(error);
    }
  }
}
