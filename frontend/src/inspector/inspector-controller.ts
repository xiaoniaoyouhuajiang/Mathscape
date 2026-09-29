import type {
  DesktopSnapshot,
  MarkerRuntimeStatus,
  PdfArtifactSummary,
} from "../artifact/types";
import { definitionList, textElement } from "../core/dom";
import { errorMessage } from "../core/ipc";
import type { IncidenceNode } from "../graph/projection";
import { domainKindLabel, nodeTypeLabel } from "../graph/taxonomy";
import {
  commitMapChange,
  getEditContext,
  planMapChange,
  renderCommitError,
  renderCommitting,
  renderCommitSuccess,
  renderCreateRelationForm,
  renderCreateUnitForm,
  renderEditActions,
  renderEditForm,
  renderImpactPreview,
  renderRetireForm,
  type EditContext,
  type UserChangeRequest,
} from "../map-edit";
import { sourceNodeDetail } from "../reader/content";
import { markdownRenderer } from "../reader/markdown";
import type { ReaderFocus } from "../reader/reader";

export interface InspectorControllerDelegate {
  getSnapshot: () => DesktopSnapshot | undefined;
  showToast: (message: string, kind: "success" | "error") => void;
  copyIdentifier: (id: string) => Promise<void> | void;
  onFocusNode: (nodeId: string) => void;
  onOpenReader: (artifactId: string, label: string) => void;
  onOpenReaderWithFocus: (artifactId: string, title: string, focus: ReaderFocus) => void;
  createParseForm: (pdf: PdfArtifactSummary, marker: MarkerRuntimeStatus) => HTMLFormElement;
  refreshSnapshot: () => Promise<void>;
  canFocus: (node: IncidenceNode) => boolean;
  isFocusRoot: (nodeId: string) => boolean;
}

export class InspectorController {
  private selectedNode?: IncidenceNode;
  private inspectorLocked = false;

  constructor(
    private readonly nodeDetail: HTMLElement,
    private readonly delegate: InspectorControllerDelegate,
  ) {}

  getSelectedNode(): IncidenceNode | undefined {
    return this.selectedNode;
  }

  isLocked(): boolean {
    return this.inspectorLocked;
  }

  setLocked(locked: boolean): void {
    this.inspectorLocked = locked;
  }

  updateSelectedNodeFromSnapshot(snapshot: DesktopSnapshot): void {
    if (this.selectedNode !== undefined) {
      this.selectedNode = snapshot.landscape_graph.nodes.find(
        (node) => node.id === this.selectedNode?.id,
      );
    }
  }

  selectNodeById(nodeId: string): void {
    const snapshot = this.delegate.getSnapshot();
    if (snapshot === undefined) return;
    const model = snapshot.landscape_graph.nodes.find((node) => node.id === nodeId);
    if (model === undefined) return;
    this.inspectorLocked = false;
    this.selectedNode = model;
    const pdf = snapshot.pdf_artifacts.find(
      (artifact) => artifact.artifact_id === model.domain_id,
    );
    this.renderNodeDetail(model, pdf, snapshot.marker);
  }

  refreshIfUnlocked(): void {
    if (this.inspectorLocked || this.selectedNode === undefined) return;
    const snapshot = this.delegate.getSnapshot();
    if (snapshot === undefined) return;
    const pdf = snapshot.pdf_artifacts.find(
      (artifact) => artifact.artifact_id === this.selectedNode?.domain_id,
    );
    this.renderNodeDetail(this.selectedNode, pdf, snapshot.marker);
  }

  renderNodeDetail(
    node: IncidenceNode,
    pdf: PdfArtifactSummary | undefined,
    marker: MarkerRuntimeStatus | undefined,
  ): void {
    this.nodeDetail.classList.remove("empty-detail");
    this.nodeDetail.replaceChildren(
      textElement("span", "kind-pill", nodeTypeLabel(node)),
      textElement("h3", "detail-title", node.label),
      definitionList([
        ["类型", domainKindLabel(node.domain_kind)],
        ["层级", this.displayLayer(node.layer)],
        ["生命周期", this.displayLifecycle(node.lifecycle)],
        ["摘要", node.summary ?? "—"],
      ]),
    );

    const sourceSlot = textElement("div", "detail-source", "");
    const actions = textElement("div", "detail-actions", "");
    const editSlot = textElement("div", "detail-edit-slot", "");
    this.nodeDetail.append(sourceSlot, actions, editSlot);

    if (
      node.domain_id !== null &&
      (node.domain_kind === "SourceSemanticUnit" || node.domain_kind === "SourceSpan")
    ) {
      void this.renderSourceDetail(node, sourceSlot, actions);
    }

    if (this.delegate.canFocus(node)) {
      const focused = this.delegate.isFocusRoot(node.id);
      const focusButton = textElement(
        "button",
        "parse-button",
        focused ? "已以此为中心" : "以此为中心聚焦",
      );
      focusButton.type = "button";
      focusButton.disabled = focused;
      focusButton.onclick = () => this.delegate.onFocusNode(node.id);
      actions.append(focusButton);
    }

    if (node.domain_kind === "SourceArtifact" && node.domain_id !== null) {
      const read = textElement("button", "parse-button", "阅读文件");
      read.type = "button";
      read.onclick = () => this.delegate.onOpenReader(node.domain_id!, node.label);
      actions.append(read);
    }

    if (pdf !== undefined && marker !== undefined) {
      this.nodeDetail.append(this.delegate.createParseForm(pdf, marker));
    }

    if (
      node.domain_id !== null &&
      (node.domain_kind === "SourceSemanticUnit" || node.domain_kind === "SourceUnitRelation")
    ) {
      void this.mountEditActions(node.domain_id, editSlot);
    }

    const technical = document.createElement("details");
    technical.className = "technical-details";
    technical.append(textElement("summary", "", "技术详情"));
    technical.append(textElement("code", "identifier-value", node.domain_id ?? "—"));
    if (node.domain_id !== null) {
      const copy = textElement("button", "copy-button", "复制 ID");
      copy.type = "button";
      copy.addEventListener("click", () => void this.delegate.copyIdentifier(node.domain_id!));
      technical.append(copy);
    }
    this.nodeDetail.append(technical);
  }

  restoreNodeDetail(): void {
    this.inspectorLocked = false;
    const snapshot = this.delegate.getSnapshot();
    if (this.selectedNode !== undefined && snapshot !== undefined) {
      this.renderNodeDetail(
        this.selectedNode,
        snapshot.pdf_artifacts.find((pdf) => pdf.artifact_id === this.selectedNode?.domain_id),
        snapshot.marker,
      );
    } else {
      this.nodeDetail.classList.add("empty-detail");
      this.nodeDetail.replaceChildren(
        textElement("div", "detail-symbol", "⌁"),
        textElement("p", "", "点击图中的节点以查看它在来源层中的身份和状态。"),
      );
    }
  }

  private async renderSourceDetail(
    node: IncidenceNode,
    host: HTMLElement,
    actions: HTMLElement,
  ): Promise<void> {
    try {
      const detail = await sourceNodeDetail(node.domain_id!);
      if (detail.excerpt) {
        const preview = document.createElement("div");
        preview.className = "source-preview markdown-body";
        preview.innerHTML = markdownRenderer().render(detail.excerpt);
        host.append(preview);
      }
      if (detail.artifact_id) {
        const button = textElement("button", "parse-button", "打开并定位原文");
        button.type = "button";
        button.onclick = () => {
          this.delegate.onOpenReaderWithFocus(
            detail.artifact_id!,
            detail.display_title ?? node.label,
            detail.byte_range
              ? { ...detail.byte_range, blockIds: detail.markdown_block_ids }
              : undefined!,
          );
        };
        actions.prepend(button);
      }
    } catch (error) {
      host.append(
        textElement(
          "p",
          "detail-error",
          `来源详情读取失败：${error instanceof Error ? error.message : String(error)}`,
        ),
      );
    }
  }

  private async mountEditActions(objectId: string, host: HTMLElement): Promise<void> {
    try {
      const editCtx = await getEditContext(objectId);
      if (this.inspectorLocked || this.selectedNode?.domain_id !== objectId) return;
      renderEditActions(
        host,
        editCtx,
        () => this.enterEditMode(editCtx),
        () => this.enterRetireMode(editCtx),
      );
    } catch (error) {
      const message = errorMessage(error);
      if (message.includes("对象不存在")) return;
      host.append(textElement("p", "detail-error", `编辑上下文读取失败：${message}`));
    }
  }

  enterEditMode(editCtx: EditContext): void {
    this.inspectorLocked = true;
    this.nodeDetail.classList.remove("empty-detail");
    this.nodeDetail.replaceChildren();
    let draftForm: HTMLFormElement | null = null;
    const { form } = renderEditForm(
      editCtx,
      (request) =>
        void this.enterPreviewMode(request, () => {
          if (draftForm !== null) {
            this.nodeDetail.replaceChildren(draftForm);
            this.inspectorLocked = true;
          }
        }),
      () => this.restoreNodeDetail(),
    );
    draftForm = form;
    this.nodeDetail.append(form);
  }

  enterRetireMode(editCtx: EditContext): void {
    this.inspectorLocked = true;
    this.nodeDetail.classList.remove("empty-detail");
    this.nodeDetail.replaceChildren();
    let draftForm: HTMLFormElement | null = null;
    draftForm = renderRetireForm(
      editCtx,
      (request) =>
        void this.enterPreviewMode(request, () => {
          if (draftForm !== null) {
            this.nodeDetail.replaceChildren(draftForm);
            this.inspectorLocked = true;
          }
        }),
      () => this.restoreNodeDetail(),
    );
    this.nodeDetail.append(draftForm);
  }

  async enterCreateUnitMode(parentId: string): Promise<void> {
    try {
      const context = await getEditContext(parentId);
      if (!context.available_parents.some((option) => option.object_id === parentId)) {
        context.available_parents.unshift({
          object_id: parentId,
          label: this.selectedNode?.label ?? parentId,
        });
      }
      this.inspectorLocked = true;
      this.nodeDetail.replaceChildren();
      let draftForm: HTMLFormElement | null = null;
      draftForm = renderCreateUnitForm(
        context,
        parentId,
        (request) =>
          void this.enterPreviewMode(request, () => {
            if (draftForm !== null) {
              this.nodeDetail.replaceChildren(draftForm);
              this.inspectorLocked = true;
            }
          }),
        () => this.restoreNodeDetail(),
      );
      this.nodeDetail.append(draftForm);
    } catch (error) {
      this.delegate.showToast(`读取创建上下文失败：${errorMessage(error)}`, "error");
    }
  }

  async enterCreateRelationMode(sourceId: string, targetId: string): Promise<void> {
    try {
      const context = await getEditContext(sourceId);
      this.inspectorLocked = true;
      this.nodeDetail.replaceChildren();
      let draftForm: HTMLFormElement | null = null;
      draftForm = renderCreateRelationForm(
        context,
        sourceId,
        targetId,
        (request) =>
          void this.enterPreviewMode(request, () => {
            if (draftForm !== null) {
              this.nodeDetail.replaceChildren(draftForm);
              this.inspectorLocked = true;
            }
          }),
        () => this.restoreNodeDetail(),
      );
      this.nodeDetail.append(draftForm);
    } catch (error) {
      this.delegate.showToast(`读取关系创建上下文失败：${errorMessage(error)}`, "error");
    }
  }

  private async enterPreviewMode(
    request: UserChangeRequest,
    onBack: () => void,
  ): Promise<void> {
    this.inspectorLocked = true;
    this.nodeDetail.replaceChildren();
    this.nodeDetail.append(textElement("p", "", "正在分析变更影响…"));
    try {
      const plan = await planMapChange(request);
      this.nodeDetail.replaceChildren();
      renderImpactPreview(
        this.nodeDetail,
        plan,
        () => void this.executeCommit(request, plan.plan_fingerprint, onBack),
        onBack,
      );
    } catch (error) {
      renderCommitError(
        this.nodeDetail,
        errorMessage(error),
        () => void this.enterPreviewMode(request, onBack),
        () => this.restoreNodeDetail(),
      );
    }
  }

  private async executeCommit(
    request: UserChangeRequest,
    fingerprint: string,
    onBack: () => void,
  ): Promise<void> {
    this.inspectorLocked = true;
    renderCommitting(this.nodeDetail);
    try {
      const receipt = await commitMapChange(request, fingerprint);
      try {
        await this.delegate.refreshSnapshot();
      } catch {
        // The commit already succeeded; a snapshot refresh failure must not hide the receipt.
      }
      renderCommitSuccess(this.nodeDetail, receipt, () => this.restoreNodeDetail());
      this.delegate.showToast("变更已提交", "success");
    } catch (error) {
      renderCommitError(
        this.nodeDetail,
        errorMessage(error),
        () => void this.enterPreviewMode(request, onBack),
        () => this.restoreNodeDetail(),
      );
    }
  }

  private displayLayer(layer: string): string {
    return (
      (
        {
          source: "来源层",
          local_semantic: "局部语义层",
          global_semantic: "全局语义层",
          alignment: "对齐层",
          rdtx: "RDTX 层",
          formalization: "形式化层",
        } as Record<string, string>
      )[layer] ?? layer
    );
  }

  private displayLifecycle(lifecycle: string | null): string {
    if (lifecycle === null) return "—";
    return (
      (
        {
          active: "活动",
          deprecated: "已退役",
          superseded: "已被替代",
          rejected: "已拒绝",
          deleted: "已删除",
        } as Record<string, string>
      )[lifecycle] ?? lifecycle
    );
  }
}
