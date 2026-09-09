import { Channel, invoke } from "@tauri-apps/api/core";
import "./discovery-task.css";

let running = false;
let panel: HTMLElement | null = null;
let status: HTMLElement;
let output: HTMLElement;
let reasoning: HTMLElement;
let result: HTMLElement;
let launcher: HTMLButtonElement;

export function mountDiscoveryTask(): void {
  if (panel) return;
  launcher = document.createElement("button");
  launcher.className = "task-launcher";
  launcher.textContent = "发现任务";
  launcher.onclick = () => { if (panel) panel.hidden = false; };
  panel = document.createElement("section");
  panel.className = "discovery-task-window";
  panel.setAttribute("aria-label", "内容发现任务");
  panel.innerHTML = `<header><strong>内容发现任务</strong><button type="button" aria-label="最小化任务">—</button></header>
    <p class="task-status" role="status">尚未启动任务</p>
    <details class="task-reasoning"><summary>模型返回的推理摘要</summary><pre></pre></details>
    <details open><summary>实时输出（尚未验证）</summary><pre class="task-output"></pre></details>
    <section class="task-result"></section>`;
  panel.querySelector("button")!.onclick = () => { panel!.hidden = true; };
  status = panel.querySelector(".task-status")!;
  output = panel.querySelector(".task-output")!;
  reasoning = panel.querySelector(".task-reasoning pre")!;
  result = panel.querySelector(".task-result")!;
  panel.hidden = true;
  document.body.append(launcher, panel);
}

/** Application lifetime owns the promise and output, independent of readers. */
export async function runDiscoveryTask<T>(request: unknown): Promise<T> {
  mountDiscoveryTask();
  if (running) throw new Error("已有内容发现任务正在运行");
  running = true;
  let completed = false;
  panel!.hidden = false;
  panel!.querySelector("header strong")!.textContent = "内容发现任务";
  status.textContent = "等待模型响应…";
  launcher.textContent = "发现任务 · 运行中";
  output.textContent = reasoning.textContent = "";
  result.replaceChildren();
  const channel = new Channel<{ kind: string; text: string }>();
  channel.onmessage = event => {
    if (completed) return;
    if (event.kind === "started") { panel!.querySelector("header strong")!.textContent = `内容发现 · ${event.text}`; return; }
    if (event.kind === "validating") { status.textContent = "接收完成，正在验证提案…"; return; }
    status.textContent = event.kind === "reasoning" ? "正在接收推理摘要…" : "正在接收输出；完成后验证…";
    const target = event.kind === "reasoning" ? reasoning : output;
    // Bound the visible transcript. Complete output remains in the backend validator.
    const remaining = Math.max(0, 200_000 - (target.textContent?.length ?? 0));
    if (remaining) target.append(document.createTextNode(event.text.slice(0, remaining)));
    target.scrollTop = target.scrollHeight;
  };
  try {
    const outcome = await invoke<T>("run_discovery", { request, onProgress: channel });
    completed = true;
    const value = outcome as { outcome?: string; message?: string; proposal?: { units?: unknown[]; relations?: unknown[] }; run?: { requested_model?: string } };
    status.textContent = value.outcome === "valid"
      ? `验证通过 · ${value.run?.requested_model ?? ""} · ${value.proposal?.units?.length ?? 0} 个单元、${value.proposal?.relations?.length ?? 0} 条关系；尚未接受或入图。`
      : `任务未完成：${value.message ?? value.outcome}`;
    const details = document.createElement("details");
    const summary = document.createElement("summary"); summary.textContent = "查看完整结果与验证诊断";
    const pre = document.createElement("pre"); pre.textContent = JSON.stringify(outcome, null, 2);
    details.append(summary, pre); result.append(details);
    return outcome;
  } catch (error) {
    completed = true;
    status.textContent = `任务失败：${String(error)}`;
    throw error;
  } finally {
    running = false;
    launcher.textContent = "发现任务 · 查看结果";
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
