/**
 * Sandbox panel (docs/sandbox-flow.md §5 S0/S1): register checkpoints, open
 * sandbox sessions (full-copy isolation), discard and export them (S0); open a
 * session in this window and talk to the local Codex over it (S1).
 *
 * "Sandbox session" is the product concept; it has nothing to do with Codex
 * `SandboxMode`. There is still no write path back to the checkpoint.
 */
import { reviewBeforeLeaving } from "../review/change-sets";
import { invoke } from "@tauri-apps/api/core";

import { errorMessage } from "../core/ipc";
import { enterSandbox, returnToMother, type WorkspaceStatus } from "./chrome";
import { fullTimestamp, sessionOpenByDefault, shortTimestamp, STATE_LABEL, type SessionState } from "./format";
import "./panel.css";

interface CheckpointRecord {
  checkpoint_id: string;
  label: string;
  root: string;
  registered_at: string;
}

interface SessionView {
  sandbox_session_id: string;
  state: SessionState;
  checkpoint_id: string;
  checkpoint_digest: string | null;
  created_at: string;
  last_exported_at: string | null;
  export_count: number;
  discarded_at: string | null;
  workspace_root: string | null;
}

interface SandboxOverview {
  sandboxRoot: string;
  defaultExportDir: string;
  currentWorkspaceRoot: string;
  workspace: WorkspaceStatus;
  checkpoints: CheckpointRecord[];
  sessions: SessionView[];
}

interface ExportOutcome {
  export_dir: string;
  session: SessionView;
}

type CheckpointSource =
  | { kind: "current_workspace" }
  | { kind: "pick_folder" }
  | { kind: "path"; path: string };

type Toast = (message: string, kind: "success" | "error") => void;

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className = "",
  text = "",
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text) node.textContent = text;
  return node;
}

type ButtonTone = "primary" | "secondary" | "quiet" | "danger";

function button(label: string, onClick: () => void, tone: ButtonTone = "secondary"): HTMLButtonElement {
  const node = el("button", `sandbox-btn sandbox-btn-${tone}`, label);
  node.type = "button";
  node.onclick = onClick;
  return node;
}

/** Monospace path, truncated from the left (keeps the tail visible); full text on hover. */
function pathText(path: string, className = ""): HTMLElement {
  const node = el("span", `sandbox-path ${className}`.trim());
  node.title = path;
  const inner = el("bdi", "", path);
  inner.dir = "ltr";
  node.append(inner);
  return node;
}

/** `label：<path>` line used inside cards and section hints. */
function labeledPath(label: string, path: string): HTMLElement {
  const row = el("div", "sandbox-field");
  row.append(el("span", "sandbox-field-label", label), pathText(path));
  return row;
}

export class SandboxPanelController {
  private readonly overlay: HTMLElement;
  private readonly body: HTMLElement;
  private busy = false;
  /** User-toggled open/closed state per session, kept across re-renders. */
  private readonly sessionOpen = new Map<string, boolean>();

  constructor(
    host: HTMLElement,
    trigger: HTMLButtonElement,
    private readonly toast: Toast,
    private readonly dialogue: { open: (sessionId: string) => void; forget: (sessionId: string) => void },
  ) {
    this.overlay = el("div", "settings-overlay sandbox-overlay");
    this.overlay.hidden = true;
    const dialog = el("section", "settings-dialog sandbox-dialog");
    dialog.setAttribute("role", "dialog");
    dialog.setAttribute("aria-modal", "true");
    dialog.setAttribute("aria-label", "沙箱");
    const header = el("header", "settings-header");
    const heading = el("div");
    heading.append(
      el("span", "settings-eyebrow", "SANDBOX · S1"),
      el("h2", "", "沙箱"),
      el("p", "", "从检查点完整复制出一个隔离工作区，可在本窗口打开并选择本机执行器对话；丢弃即物理删除，检查点永不被修改。"),
    );
    const close = el("button", "settings-close", "×");
    close.type = "button";
    close.setAttribute("aria-label", "关闭");
    close.onclick = () => this.close();
    header.append(heading, close);
    this.body = el("div", "sandbox-body");
    dialog.append(header, this.body);
    this.overlay.append(dialog);
    host.append(this.overlay);
    this.overlay.addEventListener("click", (event) => {
      if (event.target === this.overlay) this.close();
    });
    document.addEventListener("keydown", (event) => {
      if (event.key === "Escape" && !this.overlay.hidden) this.close();
    });
    trigger.onclick = () => void this.open();
  }

  async open(): Promise<void> {
    this.overlay.hidden = false;
    await this.refresh();
  }

  close(): void {
    this.overlay.hidden = true;
  }

  private async refresh(): Promise<void> {
    try {
      this.render(await invoke<SandboxOverview>("sandbox_overview"));
    } catch (error) {
      this.body.replaceChildren(el("p", "settings-result sandbox-error", errorMessage(error)));
    }
  }

  private async act<T>(work: () => Promise<T>, success: (value: T) => string | null): Promise<void> {
    if (this.busy) return;
    this.busy = true;
    this.body.classList.add("is-busy");
    try {
      const value = await work();
      const message = success(value);
      if (message) this.toast(message, "success");
    } catch (error) {
      this.toast(errorMessage(error), "error");
    } finally {
      this.busy = false;
      this.body.classList.remove("is-busy");
      await this.refresh();
    }
  }

  private register(source: CheckpointSource, label: string | null): void {
    void this.act(
      () =>
        invoke<CheckpointRecord | null>("sandbox_register_checkpoint", {
          request: { source, label },
        }),
      (record) => (record ? `已登记检查点「${record.label}」（${record.checkpoint_id}）` : null),
    );
  }

  private render(overview: SandboxOverview): void {
    this.body.replaceChildren(this.renderCheckpoints(overview), this.renderSessions(overview));
  }

  private renderCheckpoints(overview: SandboxOverview): HTMLElement {
    const section = el("section", "settings-section sandbox-section");
    const heading = el("div", "settings-section-heading");
    const title = el("div");
    title.append(el("span", "settings-eyebrow", "CHECKPOINTS"), el("h3", "", "检查点"));
    heading.append(title, el("span", "settings-scope", "只读"));
    section.append(heading);

    const label = el("input", "sandbox-input");
    label.placeholder = "可选：检查点名称";
    label.maxLength = 80;
    const pathInput = el("input", "sandbox-input");
    pathInput.placeholder = "或粘贴工作区目录路径（含 mathscape.sqlite3）";
    const labelValue = () => label.value.trim() || null;
    const actions = el("div", "sandbox-actions");
    actions.append(
      button("登记当前工作区", () => this.register({ kind: "current_workspace" }, labelValue())),
      button("选择文件夹…", () => this.register({ kind: "pick_folder" }, labelValue())),
      button("按路径登记", () => {
        const path = pathInput.value.trim();
        if (!path) {
          this.toast("请先填写工作区目录路径", "error");
          return;
        }
        this.register({ kind: "path", path }, labelValue());
      }),
    );
    section.append(label, pathInput, actions);
    section.append(labeledPath("当前工作区", overview.currentWorkspaceRoot));

    const list = el("ul", "sandbox-list");
    if (overview.checkpoints.length === 0) {
      list.append(el("li", "sandbox-empty", "还没有检查点。"));
    }
    for (const checkpoint of overview.checkpoints) {
      const item = el("li", "sandbox-item");
      const meta = el("div", "sandbox-meta");
      meta.append(
        el("strong", "", checkpoint.label),
        el("code", "sandbox-id", checkpoint.checkpoint_id),
        pathText(checkpoint.root),
      );
      const row = el("div", "sandbox-item-actions");
      row.append(
        button(
          "开沙箱",
          () =>
            void this.act(
              () => invoke<SessionView>("sandbox_start_session", { checkpointId: checkpoint.checkpoint_id }),
              (session) => `已开沙箱 ${session.sandbox_session_id}`,
            ),
          "primary",
        ),
        button("移除登记", () => {
          if (!window.confirm(`只移除别名「${checkpoint.label}」，不会删除目录或已有沙箱。继续？`)) return;
          void this.act(
            () => invoke("sandbox_unregister_checkpoint", { checkpointId: checkpoint.checkpoint_id }),
            () => "已移除登记",
          );
        }, "quiet"),
      );
      item.append(meta, row);
      list.append(item);
    }
    section.append(list);
    return section;
  }

  private renderSessions(overview: SandboxOverview): HTMLElement {
    const section = el("section", "settings-section sandbox-section");
    const heading = el("div", "settings-section-heading");
    const title = el("div");
    title.append(el("span", "settings-eyebrow", "SESSIONS"), el("h3", "", "沙箱列表"));
    heading.append(title, el("span", "settings-scope", "完整复制"));
    section.append(heading);
    section.append(labeledPath("默认导出目录", overview.defaultExportDir));

    const list = el("ul", "sandbox-list");
    if (overview.sessions.length === 0) {
      list.append(el("li", "sandbox-empty", "还没有沙箱。"));
    }
    for (const session of overview.sessions) {
      list.append(this.renderSession(session, overview.workspace));
    }
    section.append(list);
    return section;
  }

  private renderSession(session: SessionView, workspace: WorkspaceStatus): HTMLElement {
    const id = session.sandbox_session_id;
    const item = el("li", `sandbox-item sandbox-session state-${session.state}`);
    const details = el("details", "sandbox-details");
    details.open = this.sessionOpen.get(id) ?? sessionOpenByDefault(session.state);
    details.addEventListener("toggle", () => this.sessionOpen.set(id, details.open));

    const summary = el("summary", "sandbox-summary");
    const time = el("time", "sandbox-summary-time", shortTimestamp(session.created_at));
    time.dateTime = session.created_at;
    time.title = `创建于 ${fullTimestamp(session.created_at)}`;
    const inWindow = workspace.sandbox?.sandboxSessionId === id;
    summary.append(
      el("code", "sandbox-id", id),
      el("span", `sandbox-state state-${session.state}`, STATE_LABEL[session.state]),
      time,
    );
    if (inWindow) summary.append(el("span", "sandbox-current-badge", "本窗口"));
    details.append(summary);

    const meta = el("div", "sandbox-meta");
    const facts = el("dl", "sandbox-facts");
    const fact = (label: string, value: string | HTMLElement) => {
      const dd = el("dd");
      if (typeof value === "string") dd.textContent = value;
      else dd.append(value);
      facts.append(el("dt", "", label), dd);
    };
    fact("检查点", session.checkpoint_id);
    fact("创建于", fullTimestamp(session.created_at));
    if (session.checkpoint_digest) {
      const digest = el("code", "sandbox-digest", `${session.checkpoint_digest.slice(0, 23)}…`);
      digest.title = session.checkpoint_digest;
      fact("指纹", digest);
    }
    if (session.last_exported_at) {
      fact("导出", `${session.export_count} 次，最近 ${fullTimestamp(session.last_exported_at)}`);
    }
    if (session.discarded_at) {
      fact("丢弃于", `${fullTimestamp(session.discarded_at)}（隔离目录已删除）`);
    }
    if (session.workspace_root) {
      fact("隔离工作区", pathText(session.workspace_root));
    }
    meta.append(facts);
    details.append(meta);

    if (session.state !== "discarded" && session.workspace_root) {
      const root = session.workspace_root;
      const row = el("div", "sandbox-item-actions");
      row.append(
        inWindow
          ? button("返回母工作区", () => void returnToMother(workspace.motherWorkspaceRoot, this.toast, id), "primary")
          : button("在本窗口打开…", () => void enterSandbox(id, root, this.toast, workspace.sandbox?.sandboxSessionId ?? null), "primary"),
        button("对话", () => {
          this.close();
          void this.dialogue.open(id);
        }, "secondary"),
      );
      details.append(row);
    }

    if (session.state !== "discarded") {
      const notes = el("input", "sandbox-input");
      notes.placeholder = "可选：导出备注";
      notes.maxLength = 2000;
      const exportWith = async (chooseDestination: boolean) => {
        if (!await reviewBeforeLeaving(id, "export", this.toast)) return;
        void this.act(
          () =>
            invoke<ExportOutcome | null>("sandbox_export_session", {
              request: {
                sandboxSessionId: id,
                chooseDestination,
                notes: notes.value.trim() || null,
              },
            }),
          (outcome) => (outcome ? `已导出到 ${outcome.export_dir}` : null),
        );
      };
      const row = el("div", "sandbox-item-actions");
      row.append(
        button("导出", () => void exportWith(false), "secondary"),
        button("导出到…", () => void exportWith(true), "quiet"),
        el("span", "sandbox-actions-spacer"),
        button("丢弃", async () => {
          if (!await reviewBeforeLeaving(id, "discard", this.toast)) return;
          if (!window.confirm(`丢弃会物理删除 ${id} 的整个隔离目录，且不可恢复（已导出的包保留）。继续？`)) return;
          void this.act(
            () => invoke<SessionView>("sandbox_discard_session", { sandboxSessionId: id }),
            () => {
              this.dialogue.forget(id);
              return "已丢弃，隔离目录已删除";
            },
          );
        }, "danger"),
      );
      details.append(notes, row);
    }
    item.append(details);
    return item;
  }
}
