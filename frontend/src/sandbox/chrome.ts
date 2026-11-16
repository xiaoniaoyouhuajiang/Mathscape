/**
 * S1 in-window workspace switch (docs/sandbox-flow.md §5 S1): the window can
 * work in a sandbox session's isolated copy and come back to the mother
 * workspace. Both directions ask first; while in a sandbox the top bar carries
 * a permanent banner so it is never mistaken for the real workspace.
 *
 * The switch is not persisted: restarting the app always opens the mother.
 */
import { reviewBeforeLeaving } from "../review/change-sets";
import { invoke } from "@tauri-apps/api/core";

import { errorMessage } from "../core/ipc";
import { enterConfirmMessage, returnConfirmMessage } from "./format";

export interface InWindowSandbox {
  sandboxSessionId: string;
  checkpointId: string;
  workspaceRoot: string;
}

export interface WorkspaceStatus {
  motherWorkspaceRoot: string;
  currentWorkspaceRoot: string;
  sandbox: InWindowSandbox | null;
}

type Toast = (message: string, kind: "success" | "error") => void;

/** Reload so every view re-reads the newly bound workspace from scratch. */
function reloadWindow(): void {
  window.location.reload();
}

/** Ask, then bind this window to the session copy. Returns false if declined or refused. */
export async function enterSandbox(sessionId: string, workspaceRoot: string, toast: Toast, leavingSessionId: string | null = null): Promise<boolean> {
  // Switching this window away from a sandbox session reviews its proposals first.
  if (leavingSessionId && leavingSessionId !== sessionId && !await reviewBeforeLeaving(leavingSessionId, "switch_session", toast)) return false;
  if (!window.confirm(enterConfirmMessage(sessionId, workspaceRoot))) return false;
  try {
    await invoke<WorkspaceStatus>("sandbox_enter_session", { sandboxSessionId: sessionId });
  } catch (error) {
    toast(errorMessage(error), "error");
    return false;
  }
  reloadWindow();
  return true;
}

/** Ask, then rebind this window to the mother workspace. */
export async function returnToMother(motherRoot: string, toast: Toast, leavingSessionId: string | null = null): Promise<boolean> {
  if (!await reviewBeforeLeaving(leavingSessionId, "switch_session", toast)) return false;
  if (!window.confirm(returnConfirmMessage(motherRoot))) return false;
  try {
    await invoke<WorkspaceStatus>("sandbox_return_to_mother");
  } catch (error) {
    toast(errorMessage(error), "error");
    return false;
  }
  reloadWindow();
  return true;
}

/** 顶栏里显示的副本 id：超过 14 个字符时只留开头。 */
export function shortSandboxId(id: string): string {
  return id.length > 14 ? `${id.slice(0, 12)}…` : id;
}

/**
 * Read the binding once at startup and, if the window is in a sandbox, mark
 * the shell and put the banner into the top bar.
 */
export async function mountSandboxChrome(
  shell: HTMLElement,
  options: { toast: Toast; openDialogue: (sessionId: string) => void },
): Promise<WorkspaceStatus | null> {
  let status: WorkspaceStatus;
  try {
    status = await invoke<WorkspaceStatus>("sandbox_workspace_status");
  } catch {
    return null;
  }
  const sandbox = status.sandbox;
  shell.classList.toggle("is-sandbox", sandbox !== null);
  shell.querySelector(".sandbox-banner")?.remove();
  if (!sandbox) return status;

  const banner = document.createElement("div");
  banner.className = "sandbox-banner";
  banner.setAttribute("role", "status");
  const tag = document.createElement("span");
  tag.className = "sandbox-banner-tag";
  tag.textContent = "沙箱";
  const text = document.createElement("span");
  text.className = "sandbox-banner-text";
  // 顶栏放不下完整的副本 id：只显示开头，完整说明和路径放在悬停提示里。
  const full = `${sandbox.sandboxSessionId} · 改动只写入隔离副本，检查点 ${sandbox.checkpointId} 不受影响`;
  text.textContent = `${shortSandboxId(sandbox.sandboxSessionId)} · 改动只写入隔离副本`;
  text.title = `${full}\n当前工作区：${sandbox.workspaceRoot}\n母工作区：${status.motherWorkspaceRoot}`;
  banner.title = text.title;
  const talk = document.createElement("button");
  talk.type = "button";
  talk.className = "sandbox-btn sandbox-btn-secondary";
  talk.textContent = "对话";
  talk.onclick = () => options.openDialogue(sandbox.sandboxSessionId);
  const back = document.createElement("button");
  back.type = "button";
  back.className = "sandbox-btn sandbox-btn-primary";
  back.textContent = "返回母工作区";
  back.onclick = () => void returnToMother(status.motherWorkspaceRoot, options.toast);
  banner.append(tag, text, talk, back);
  shell.querySelector(".brand-block")?.after(banner);
  return status;
}
