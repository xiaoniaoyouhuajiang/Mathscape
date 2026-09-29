import { invoke } from "@tauri-apps/api/core";
import type { DesktopSnapshot } from "../artifact/types";
import { textElement } from "../core/dom";
import { eventBus } from "../core/events";
import { errorMessage } from "../core/ipc";
import { openDiscoveryPatchReview } from "../reader/selection";
import { openReviewWorkspace } from "../review/workspace";
import {
  discoveryRunStatusLabel,
  discoveryStageLabel,
  discoveryWorkflowStatusLabel,
  shortRunId,
} from "./labels";
import type {
  DiscoveryReviewEnvelopeView,
  DiscoveryRunDiagnosticsView,
  DiscoveryRunSummaryView,
} from "./types";

export interface DiscoveryLedgerDelegate {
  getSnapshot: () => DesktopSnapshot | undefined;
  showToast: (message: string, kind: "success" | "error") => void;
}

export class DiscoveryLedgerController {
  private static readonly PREVIEW_LIMIT = 20;
  private showAllRuns = false;
  private unsubscribeEvent?: () => void;

  constructor(
    private readonly listContainer: HTMLElement,
    private readonly refreshButton: HTMLButtonElement,
    private readonly delegate: DiscoveryLedgerDelegate,
  ) {
    this.refreshButton.addEventListener("click", () => void this.loadDiscoveryRuns());
    this.unsubscribeEvent = eventBus.on("discovery-runs-changed", () => {
      void this.loadDiscoveryRuns();
    });
  }

  destroy(): void {
    this.unsubscribeEvent?.();
  }

  getArtifactDisplayName(artifactId: string, fallbackName?: string | null): string {
    const snapshot = this.delegate.getSnapshot();
    const node = snapshot?.landscape_graph.nodes.find(
      (n) => n.domain_kind === "SourceArtifact" && n.domain_id === artifactId,
    );
    if (node?.label) {
      return node.label;
    }
    const pdf = snapshot?.pdf_artifacts.find((p) => p.artifact_id === artifactId);
    if (pdf?.filename) {
      return pdf.filename;
    }
    if (fallbackName && fallbackName !== "document.md") {
      return fallbackName;
    }
    return fallbackName || artifactId;
  }

  async loadDiscoveryRuns(): Promise<void> {
    this.refreshButton.disabled = true;
    try {
      const runs = await invoke<DiscoveryRunSummaryView[]>("list_discovery_runs", {
        request: { artifact_id: null },
      });
      this.listContainer.replaceChildren();
      if (runs.length === 0) {
        this.listContainer.append(textElement("p", "ledger-empty", "尚无已保存的内容发现记录。"));
        return;
      }
      const visible = this.showAllRuns
        ? runs
        : runs.slice(0, DiscoveryLedgerController.PREVIEW_LIMIT);
      visible.forEach((run) => this.renderDiscoveryRunItem(run));
      if (runs.length > DiscoveryLedgerController.PREVIEW_LIMIT) {
        const toggle = textElement(
          "button",
          "ledger-more",
          this.showAllRuns ? "只显示最近记录" : `显示全部 ${runs.length} 条记录`,
        );
        toggle.type = "button";
        toggle.onclick = () => {
          this.showAllRuns = !this.showAllRuns;
          void this.loadDiscoveryRuns();
        };
        this.listContainer.append(toggle);
      }
    } catch (error) {
      this.listContainer.replaceChildren(
        textElement("p", "ledger-error", `读取失败：${errorMessage(error)}`),
      );
    } finally {
      this.refreshButton.disabled = false;
    }
  }

  private renderDiscoveryRunItem(run: DiscoveryRunSummaryView): void {
    const item = textElement("article", `discovery-run-item state-${run.review_status}`, "");
    const heading = textElement("div", "discovery-run-item-heading", "");
    const displayName = this.getArtifactDisplayName(run.artifact_id, run.artifact_name);
    heading.append(
      textElement("strong", "", displayName),
      textElement("span", "discovery-run-state", discoveryRunStatusLabel(run.review_status)),
    );
    const counts = textElement(
      "span",
      "discovery-run-counts",
      `${discoveryWorkflowStatusLabel(run.status)} · ${run.unit_count} 单元 · ${run.relation_count} 关系 · ${run.unresolved_reference_count} 未解析引用`,
    );
    const actions = textElement("div", "discovery-run-actions", "");
    const recoverable =
      run.result_fingerprint !== null &&
      ["completed", "partial", "needs_attention"].includes(run.status);
    const open = textElement(
      "button",
      "",
      run.status === "awaiting_patch_review"
        ? "打开修订审核"
        : recoverable
          ? "打开审核结果"
          : "查看运行诊断",
    );
    open.type = "button";
    if (run.status === "running") {
      open.title = "运行仍在进行中，将显示最近一次已保存的检查点";
    }
    open.onclick = () => void this.openPersistedDiscoveryRun(run, open);
    actions.append(open);
    if (run.review_status === "reviewable" && run.result_fingerprint !== null) {
      const discard = textElement("button", "", "放弃结果");
      discard.type = "button";
      discard.onclick = () => void this.discardPersistedDiscoveryRun(run, discard);
      actions.append(discard);
    }
    item.append(
      heading,
      counts,
      textElement("span", "discovery-run-id", shortRunId(run.run_id)),
      actions,
    );
    this.listContainer.append(item);
  }

  private async openPersistedDiscoveryRun(
    run: DiscoveryRunSummaryView,
    button: HTMLButtonElement,
  ): Promise<void> {
    button.disabled = true;
    const displayName = this.getArtifactDisplayName(run.artifact_id, run.artifact_name);
    try {
      if (run.status === "awaiting_patch_review") {
        openDiscoveryPatchReview(run.run_id, run.artifact_id, run.conversion_id, displayName);
        return;
      }
      const recoverable =
        run.result_fingerprint !== null &&
        ["completed", "partial", "needs_attention"].includes(run.status);
      if (!recoverable) {
        const diagnostics = await invoke<DiscoveryRunDiagnosticsView>(
          "get_discovery_diagnostics",
          {
            runId: run.run_id,
          },
        );
        this.showDiscoveryDiagnostics(run, diagnostics, displayName);
        return;
      }
      const envelope = await invoke<DiscoveryReviewEnvelopeView>("get_discovery_review", {
        runId: run.run_id,
      });
      openReviewWorkspace(envelope.outcome, {
        title: displayName,
        review_status: envelope.review_status,
        created_at: envelope.created_at,
        updated_at: envelope.updated_at,
      });
    } catch (error) {
      this.delegate.showToast(`恢复内容发现结果失败：${errorMessage(error)}`, "error");
    } finally {
      button.disabled = false;
    }
  }

  showDiscoveryDiagnostics(
    run: DiscoveryRunSummaryView,
    diagnostics: DiscoveryRunDiagnosticsView,
    displayName = this.getArtifactDisplayName(run.artifact_id, run.artifact_name),
  ): void {
    const dialog = document.createElement("dialog");
    dialog.className = "discovery-diagnostics-dialog";
    dialog.setAttribute("aria-labelledby", "discovery-diagnostics-title");
    const title = textElement("h2", "", "内容发现运行诊断");
    title.id = "discovery-diagnostics-title";
    const meta = textElement(
      "p",
      "discovery-diagnostics-meta",
      `${displayName} · ${discoveryWorkflowStatusLabel(run.status)} · ${shortRunId(run.run_id)}`,
    );
    const message = textElement("p", "discovery-diagnostics-message", diagnostics.message);
    const attempts = textElement("section", "discovery-diagnostics-section", "");
    attempts.append(textElement("h3", "", "调用记录"));
    if (diagnostics.attempts.length === 0) {
      attempts.append(textElement("p", "ledger-empty", "没有已保存的模型调用记录。"));
    } else {
      const list = document.createElement("ol");
      list.className = "discovery-diagnostics-list";
      diagnostics.attempts.forEach((attempt) => {
        const item = document.createElement("li");
        const response = attempt.response_received
          ? `已收到响应${attempt.elapsed_ms === null ? "" : ` · ${attempt.elapsed_ms} ms`}`
          : "没有完整响应";
        item.append(
          textElement(
            "strong",
            "",
            `调用 ${attempt.ordinal} · ${discoveryStageLabel(attempt.stage)}${attempt.repair ? " · 修订" : ""}`,
          ),
          textElement("span", "", `${attempt.status} · ${response}`),
        );
        if (attempt.error) {
          item.append(textElement("p", "discovery-diagnostics-error", attempt.error));
        }
        list.append(item);
      });
      attempts.append(list);
    }
    const validation = textElement("section", "discovery-diagnostics-section", "");
    validation.append(textElement("h3", "", "验证诊断"));
    if (diagnostics.diagnostics.length === 0) {
      validation.append(textElement("p", "ledger-empty", "没有已保存的验证诊断。"));
    } else {
      const list = document.createElement("ul");
      list.className = "discovery-diagnostics-list";
      diagnostics.diagnostics.forEach((item) => {
        const row = document.createElement("li");
        row.append(
          textElement("strong", "", `${item.code} · 报告 ${item.report}`),
          textElement("span", "", `${item.path} · ${item.message}`),
        );
        list.append(row);
      });
      validation.append(list);
    }
    const events = textElement("details", "discovery-diagnostics-section", "");
    events.append(textElement("summary", "", "工作流事件"));
    const eventList = document.createElement("ol");
    eventList.className = "discovery-diagnostics-list";
    diagnostics.events.forEach((event) => eventList.append(textElement("li", "", event)));
    events.append(eventList);
    const close = textElement("button", "", "关闭");
    close.type = "button";
    close.onclick = () => dialog.close();
    dialog.append(title, meta, message, attempts, validation, events, close);
    dialog.addEventListener("close", () => dialog.remove(), { once: true });
    document.body.append(dialog);
    dialog.showModal();
  }

  private async discardPersistedDiscoveryRun(
    run: DiscoveryRunSummaryView,
    button: HTMLButtonElement,
  ): Promise<void> {
    if (run.result_fingerprint === null) return;
    const reason = "用户从内容发现记录中放弃本次未登记结果";
    button.disabled = true;
    try {
      await invoke("discard_discovery_run", {
        request: {
          run_id: run.run_id,
          request_id: `mathscape:discovery-disposition/${crypto.randomUUID()}`,
          expected_result_fingerprint: run.result_fingerprint,
          reason,
        },
      });
      await this.loadDiscoveryRuns();
      this.delegate.showToast("已放弃本次内容发现结果；原始检查点仍保留。", "success");
    } catch (error) {
      this.delegate.showToast(`放弃内容发现结果失败：${errorMessage(error)}`, "error");
      button.disabled = false;
    }
  }
}
