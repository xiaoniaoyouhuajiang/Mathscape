/**
 * S0 sandbox panel (docs/sandbox-flow.md §5 S0): register checkpoints, open
 * sandbox sessions (full-copy isolation), discard and export them.
 *
 * "Sandbox session" is the product concept; it has nothing to do with Codex
 * `SandboxMode`. S0 has no dialogue and no write path back to the checkpoint.
 */
import { invoke } from "@tauri-apps/api/core";

import { errorMessage } from "../core/ipc";
import "./panel.css";

type SessionState = "active" | "exported" | "discarded";

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

const STATE_LABEL: Record<SessionState, string> = {
  active: "进行中",
  exported: "已导出",
  discarded: "已丢弃",
};

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

function button(label: string, onClick: () => void, className = "settings-secondary"): HTMLButtonElement {
  const node = el("button", className, label);
  node.type = "button";
  node.onclick = onClick;
  return node;
}

export class SandboxPanelController {
  private readonly overlay: HTMLElement;
  private readonly body: HTMLElement;
  private busy = false;

  constructor(
    host: HTMLElement,
    trigger: HTMLButtonElement,
    private readonly toast: Toast,
  ) {
    this.overlay = el("div", "settings-overlay sandbox-overlay");
    this.overlay.hidden = true;
    const dialog = el("section", "settings-dialog sandbox-dialog");
    dialog.setAttribute("role", "dialog");
    dialog.setAttribute("aria-modal", "true");
    dialog.setAttribute("aria-label", "沙箱会话");
    const header = el("header", "settings-header");
    const heading = el("div");
    heading.append(
      el("span", "settings-eyebrow", "SANDBOX · S0"),
      el("h2", "", "沙箱会话"),
      el("p", "", "从 checkpoint 完整复制出一个隔离工作区；丢弃即物理删除，checkpoint 永不被修改。"),
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
      (record) => (record ? `已登记 checkpoint「${record.label}」（${record.checkpoint_id}）` : null),
    );
  }

  private render(overview: SandboxOverview): void {
    this.body.replaceChildren(this.renderCheckpoints(overview), this.renderSessions(overview));
  }

  private renderCheckpoints(overview: SandboxOverview): HTMLElement {
    const section = el("section", "settings-section sandbox-section");
    const heading = el("div", "settings-section-heading");
    const title = el("div");
    title.append(el("span", "settings-eyebrow", "CHECKPOINTS"), el("h3", "", "Checkpoint（只读基线）"));
    heading.append(title, el("span", "settings-scope", "只读"));
    section.append(heading);

    const label = el("input", "sandbox-input");
    label.placeholder = "可选：checkpoint 名称";
    label.maxLength = 80;
    const pathInput = el("input", "sandbox-input");
    pathInput.placeholder = "或粘贴工作区目录路径（含 mathscape.sqlite3）";
    const labelValue = () => label.value.trim() || null;
    const actions = el("div", "settings-actions sandbox-actions");
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
    section.append(el("small", "sandbox-hint", `当前工作区：${overview.currentWorkspaceRoot}`));

    const list = el("ul", "sandbox-list");
    if (overview.checkpoints.length === 0) {
      list.append(el("li", "sandbox-empty", "还没有 checkpoint。"));
    }
    for (const checkpoint of overview.checkpoints) {
      const item = el("li", "sandbox-item");
      const meta = el("div", "sandbox-meta");
      meta.append(
        el("strong", "", checkpoint.label),
        el("code", "", checkpoint.checkpoint_id),
        el("small", "", checkpoint.root),
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
          "settings-primary",
        ),
        button("移除登记", () => {
          if (!window.confirm(`只移除别名「${checkpoint.label}」，不会删除目录或已有沙箱。继续？`)) return;
          void this.act(
            () => invoke("sandbox_unregister_checkpoint", { checkpointId: checkpoint.checkpoint_id }),
            () => "已移除登记",
          );
        }),
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
    title.append(el("span", "settings-eyebrow", "SESSIONS"), el("h3", "", "沙箱会话"));
    heading.append(title, el("span", "settings-scope", "完整复制"));
    section.append(heading);
    section.append(el("small", "sandbox-hint", `默认导出目录：${overview.defaultExportDir}`));

    const list = el("ul", "sandbox-list");
    if (overview.sessions.length === 0) {
      list.append(el("li", "sandbox-empty", "还没有沙箱会话。"));
    }
    for (const session of overview.sessions) {
      const item = el("li", `sandbox-item state-${session.state}`);
      const meta = el("div", "sandbox-meta");
      const head = el("div", "sandbox-session-head");
      head.append(
        el("code", "", session.sandbox_session_id),
        el("span", `sandbox-state state-${session.state}`, STATE_LABEL[session.state]),
      );
      meta.append(head, el("small", "", `checkpoint：${session.checkpoint_id} · 创建于 ${session.created_at}`));
      if (session.checkpoint_digest) {
        meta.append(el("small", "", `指纹：${session.checkpoint_digest.slice(0, 23)}…`));
      }
      if (session.last_exported_at) {
        meta.append(el("small", "", `已导出 ${session.export_count} 次，最近 ${session.last_exported_at}`));
      }
      if (session.discarded_at) {
        meta.append(el("small", "", `丢弃于 ${session.discarded_at}（隔离目录已删除）`));
      }
      if (session.workspace_root) {
        meta.append(
          el("small", "", `隔离工作区：${session.workspace_root}`),
          el("small", "sandbox-hint", `S0 不切换当前窗口；可用 MATHSCAPE_WORKSPACE_ROOT="${session.workspace_root}" 另开一个 Mathscape 查看。`),
        );
      }
      item.append(meta);
      if (session.state !== "discarded") {
        const notes = el("input", "sandbox-input");
        notes.placeholder = "可选：导出备注";
        notes.maxLength = 2000;
        const exportWith = (chooseDestination: boolean) =>
          void this.act(
            () =>
              invoke<ExportOutcome | null>("sandbox_export_session", {
                request: {
                  sandboxSessionId: session.sandbox_session_id,
                  chooseDestination,
                  notes: notes.value.trim() || null,
                },
              }),
            (outcome) => (outcome ? `已导出到 ${outcome.export_dir}` : null),
          );
        const row = el("div", "sandbox-item-actions");
        row.append(
          button("导出", () => exportWith(false), "settings-primary"),
          button("导出到…", () => exportWith(true)),
          button("丢弃", () => {
            if (!window.confirm(`丢弃会物理删除 ${session.sandbox_session_id} 的整个隔离目录，且不可恢复（已导出的包保留）。继续？`)) return;
            void this.act(
              () => invoke<SessionView>("sandbox_discard_session", { sandboxSessionId: session.sandbox_session_id }),
              () => "已丢弃，隔离目录已删除",
            );
          }, "settings-secondary sandbox-danger"),
        );
        item.append(notes, row);
      }
      list.append(item);
    }
    section.append(list);
    return section;
  }
}
