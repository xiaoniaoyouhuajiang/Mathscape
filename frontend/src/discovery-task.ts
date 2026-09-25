import { Channel, invoke } from "@tauri-apps/api/core";
import "./discovery-task.css";

let running = false;
let panel: HTMLElement | null = null;
let status: HTMLElement;
let result: HTMLElement;
let launcher: HTMLButtonElement;
let cancel: HTMLButtonElement;
let timeline: HTMLOListElement;
const attemptOutputs = new Map<number, { reasoning: HTMLElement; output: HTMLElement }>();

type WorkflowStatus = "running" | "awaiting_patch_review" | "completed" | "partial" | "needs_attention" | "failed" | "cancelled";
type WorkflowEvent = {
  patches?: number;
  applied?: boolean;
  rebuild_units?: boolean;
  kind: string;
  attempt?: number;
  stage?: string;
  batch_index?: number | null;
  batch_total?: number | null;
  repair?: boolean;
  units?: number;
  batches?: number;
  tasks?: number;
  completed_tasks?: number;
  total_tasks?: number;
  status?: WorkflowStatus;
};
type DiscoveryProgress =
  | { kind: "started"; profile: string; model: string }
  | { kind: "workflow_event"; event: WorkflowEvent }
  | {
      kind: "stream_delta" | "response_completed";
      attempt: number;
      stage: string;
      batch_index: number | null;
      batch_total: number | null;
      stream?: string;
      text?: string;
    };

export function mountDiscoveryTask(): void {
  if (panel) return;
  launcher = document.createElement("button");
  launcher.className = "task-launcher";
  launcher.textContent = "发现任务";
  launcher.onclick = () => { if (panel) panel.hidden = false; };
  panel = document.createElement("section");
  panel.className = "discovery-task-window";
  panel.setAttribute("aria-label", "内容发现任务");
  panel.innerHTML = `<header><strong>内容发现任务</strong><span><button class="task-cancel" type="button">取消</button><button class="task-minimize" type="button" aria-label="最小化任务">—</button></span></header>
    <p class="task-status" role="status">尚未启动任务</p>
    <ol class="task-timeline" aria-label="发现阶段"></ol>
    <section class="task-attempts"></section>
    <section class="task-result"></section>`;
  panel.querySelector<HTMLButtonElement>(".task-minimize")!.onclick = () => { panel!.hidden = true; };
  cancel = panel.querySelector<HTMLButtonElement>(".task-cancel")!;
  cancel.disabled = true;
  cancel.onclick = () => void cancelDiscovery();
  status = panel.querySelector(".task-status")!;
  timeline = panel.querySelector(".task-timeline")!;
  result = panel.querySelector(".task-result")!;
  panel.hidden = true;
  document.body.append(launcher, panel);
}

/** Application lifetime owns the promise and output, independent of readers. */
export function runDiscoveryTask<T>(request: unknown): Promise<T> {
  return executeDiscoveryTask<T>("run_discovery", { request }, false);
}

/** Continues a run paused for Markdown patch review after its decision is recorded. */
export function resumeDiscoveryTask<T>(runId: string): Promise<T> {
  return executeDiscoveryTask<T>("resume_discovery_after_patch", { request: { run_id: runId } }, true);
}

async function executeDiscoveryTask<T>(command: string, args: Record<string, unknown>, resume: boolean): Promise<T> {
  mountDiscoveryTask();
  if (running) throw new Error("已有内容发现任务正在运行");
  running = true;
  let completed = false;
  panel!.hidden = false;
  panel!.querySelector("header strong")!.textContent = "内容发现任务";
  status.textContent = resume ? "正在应用 Markdown 修订决定并恢复检查点…" : "等待模型响应…";
  launcher.textContent = "发现任务 · 运行中";
  if (resume) {
    // Earlier attempt numbers continue after the pause; keep their output.
    timelineStep("恢复暂停的内容发现");
  } else {
    panel!.querySelector(".task-attempts")!.replaceChildren();
    timeline.replaceChildren();
    attemptOutputs.clear();
  }
  result.replaceChildren();
  cancel.disabled = false;
  const channel = new Channel<DiscoveryProgress>();
  channel.onmessage = event => {
    if (completed) return;
    if (event.kind === "started") {
      panel!.querySelector("header strong")!.textContent = `内容发现 · ${event.profile} · ${event.model}`;
      return;
    }
    if (event.kind === "workflow_event") {
      applyWorkflowEvent(event.event);
      return;
    }
    const attempt = ensureAttempt(event.attempt, event.stage, event.batch_index, event.batch_total, false);
    if (event.kind === "response_completed") {
      status.textContent = `${stageLabel(event.stage)}响应接收完成，正在解码…`;
      return;
    }
    status.textContent = event.stream === "reasoning"
      ? `${stageLabel(event.stage)}正在接收推理摘要…`
      : `${stageLabel(event.stage)}正在接收输出…`;
    const target = event.stream === "reasoning" ? attempt.reasoning : attempt.output;
    const remaining = Math.max(0, 200_000 - (target.textContent?.length ?? 0));
    if (remaining) target.append(document.createTextNode((event.text ?? "").slice(0, remaining)));
    target.scrollTop = target.scrollHeight;
  };
  try {
    const outcome = await invoke<T>(command, { ...args, onProgress: channel });
    completed = true;
    const value = outcome as { outcome?: WorkflowStatus; view?: { units?: unknown[]; relations?: unknown[]; references?: unknown[]; pending_tasks?: unknown[] }; run?: { requested_model?: string } };
    const units = value.view?.units?.length ?? 0;
    const relations = value.view?.relations?.length ?? 0;
    const references = value.view?.references?.length ?? 0;
    status.textContent = value.outcome === "completed"
      ? `两阶段验证完成 · ${value.run?.requested_model ?? ""} · ${units} 个单元、${relations} 条关系；尚未接受或入图。`
      : value.outcome === "awaiting_patch_review"
        ? `已暂停：${units} 个单元已固定，等待审核 Markdown 修订；请在阅读器的发现面板中处理，检查点已保存。`
        : `工作流结束：${workflowStatusLabel(value.outcome)}；检查点已保存。`;
    const summaryCard = document.createElement("p");
    summaryCard.className = "task-result-summary";
    summaryCard.textContent = `${units} 个单元 · ${relations} 条关系 · ${references} 个引用 · ${value.view?.pending_tasks?.length ?? 0} 个待完成任务`;
    const details = document.createElement("details");
    const summary = document.createElement("summary"); summary.textContent = "查看完整结果与验证诊断";
    const pre = document.createElement("pre"); pre.textContent = JSON.stringify(outcome, null, 2);
    details.append(summary, pre); result.append(summaryCard, details);
    return outcome;
  } catch (error) {
    completed = true;
    status.textContent = `任务失败：${String(error)}`;
    throw error;
  } finally {
    running = false;
    cancel.disabled = true;
    launcher.textContent = status.textContent?.startsWith("已暂停") ? "发现任务 · 等待审核修订" : "发现任务 · 查看结果";
  }
}

function stageLabel(stage: string): string {
  if (stage === "unit-discovery") return "单元识别";
  if (stage === "source-facts") return "来源事实发现";
  if (stage === "discovery-repair") return "有界修订";
  if (stage === "markdown-patch") return "Markdown 修订建议";
  return stage;
}

function workflowStatusLabel(value: WorkflowStatus | undefined): string {
  return ({ completed: "已完成", partial: "部分完成", needs_attention: "需要处理", failed: "失败", cancelled: "已取消", running: "运行中", awaiting_patch_review: "等待审核 Markdown 修订" } as Record<string, string>)[value ?? ""] ?? String(value ?? "未知");
}

function ensureAttempt(
  attempt: number,
  stage: string,
  batchIndex: number | null | undefined,
  batchTotal: number | null | undefined,
  repair: boolean,
): { reasoning: HTMLElement; output: HTMLElement } {
  const existing = attemptOutputs.get(attempt);
  if (existing) return existing;
  const card = document.createElement("article");
  card.className = "task-attempt";
  const suffix = batchIndex && batchTotal ? ` · 批次 ${batchIndex}/${batchTotal}` : "";
  card.append(Object.assign(document.createElement("strong"), {
    textContent: `调用 ${attempt} · ${stageLabel(stage)}${suffix}${repair ? " · 修订" : ""}`,
  }));
  const reasoningDetails = document.createElement("details");
  reasoningDetails.className = "task-reasoning";
  reasoningDetails.innerHTML = "<summary>推理摘要</summary><pre></pre>";
  const outputDetails = document.createElement("details");
  outputDetails.open = true;
  outputDetails.innerHTML = "<summary>实时输出（尚未验证）</summary><pre class=\"task-output\"></pre>";
  card.append(reasoningDetails, outputDetails);
  panel!.querySelector(".task-attempts")!.append(card);
  const entry = {
    reasoning: reasoningDetails.querySelector("pre")!,
    output: outputDetails.querySelector("pre")!,
  };
  attemptOutputs.set(attempt, entry);
  return entry;
}

function applyWorkflowEvent(event: WorkflowEvent): void {
  if (event.kind === "attempt_started") {
    ensureAttempt(event.attempt ?? 0, event.stage ?? "", event.batch_index, event.batch_total, event.repair ?? false);
    status.textContent = `${stageLabel(event.stage ?? "")}已开始${event.batch_index && event.batch_total ? ` · 批次 ${event.batch_index}/${event.batch_total}` : ""}`;
    timelineStep(`调用 ${event.attempt ?? 0}：${stageLabel(event.stage ?? "")}${event.repair ? "修订" : ""}`);
  } else if (event.kind === "validation_started") {
    status.textContent = `正在验证${stageLabel(event.stage ?? "")}结果…`;
    timelineStep(`验证${stageLabel(event.stage ?? "")}结果`);
  } else if (event.kind === "unit_checkpoint_created") {
    status.textContent = `单元检查点已建立 · ${event.units ?? 0} 个单元；正在规划来源事实任务…`;
    timelineStep(`固定单元表：${event.units ?? 0} 个单元`);
  } else if (event.kind === "fact_plan_created") {
    status.textContent = `事实任务已规划 · ${event.tasks ?? 0} 个任务、${event.batches ?? 0} 个批次`;
    timelineStep(`规划事实任务：${event.tasks ?? 0} 个任务、${event.batches ?? 0} 个批次`);
  } else if (event.kind === "fact_checkpoint_updated") {
    status.textContent = `来源事实检查点 · ${event.completed_tasks ?? 0}/${event.total_tasks ?? 0} 个任务完成`;
    timelineStep(`事实检查点：${event.completed_tasks ?? 0}/${event.total_tasks ?? 0}`);
  } else if (event.kind === "patch_review_requested") {
    status.textContent = `模型提出 ${event.patches ?? 0} 条 Markdown 修订建议，等待人工审核…`;
    timelineStep(`等待审核 Markdown 修订：${event.patches ?? 0} 条建议`);
  } else if (event.kind === "patch_review_resolved") {
    const text = !event.applied
      ? "修订全部拒绝，沿用原 Markdown 继续"
      : event.rebuild_units
        ? "修订已应用，修订触及单元边界，重新识别一次单元"
        : "修订已应用，单元已重新绑定到修订后原文";
    status.textContent = `${text}…`;
    timelineStep(text);
  } else if (event.kind === "assembly_started") {
    status.textContent = "正在组装两阶段发现结果…";
    timelineStep("组装来源发现结果");
  } else if (event.kind === "workflow_finished") {
    status.textContent = `工作流${workflowStatusLabel(event.status)}`;
    timelineStep(`工作流${workflowStatusLabel(event.status)}`, `status-${event.status ?? "unknown"}`);
  }
}

function timelineStep(text: string, className = ""): void {
  const item = document.createElement("li");
  item.textContent = text;
  item.className = className;
  timeline.append(item);
}

async function cancelDiscovery(): Promise<void> {
  if (!running || cancel.disabled) return;
  cancel.disabled = true;
  status.textContent = "正在取消当前模型调用并保留内存检查点…";
  try {
    const accepted = await invoke<boolean>("cancel_discovery");
    if (!accepted) status.textContent = "任务已经结束，无需取消。";
  } catch (error) {
    status.textContent = `取消失败：${String(error)}`;
    cancel.disabled = false;
  }
}

export function askAdmission(): Promise<"review_before_publish" | "automatic_candidate" | null> {
  return new Promise(resolve => {
    const dialog = document.createElement("dialog");
    dialog.className = "admission-dialog";
    const title = document.createElement("p"); title.textContent = "本次结果采用哪种审核方式？";
    dialog.append(title);
    const finish = (value: "review_before_publish" | "automatic_candidate" | null) => { dialog.close(); dialog.remove(); resolve(value); };
    for (const [label, value] of [["人工审核", "review_before_publish"], ["自动候选", "automatic_candidate"], ["取消", null]] as const) {
      const button = document.createElement("button"); button.textContent = label; button.onclick = () => finish(value); dialog.append(button);
    }
    dialog.oncancel = event => { event.preventDefault(); finish(null); };
    document.body.append(dialog); dialog.showModal();
  });
}
