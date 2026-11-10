import { stagedConflict, stagedMark } from "../graph/staged-overlay";
import type {
  DesktopSnapshot,
  MarkerRuntimeStatus,
  PdfArtifactSummary,
} from "../artifact/types";
import { definitionList, textElement } from "../core/dom";
import { errorMessage } from "../core/ipc";
import type { IncidenceNode } from "../graph/projection";
import { isRetired } from "../graph/lifecycle";
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
import { readerTargetId, tryOpenableArtifactId } from "../reader/openable";
import { markdownRenderer } from "../reader/markdown";
import type { ReaderFocus } from "../reader/reader";
import type { InkReaderFocus } from "../ink/snapshot-reader";
import { eventBus } from "../core/events";
import { listInkLinksToTarget, roleLabel, startInkCreateFromTarget } from "../ink/links";

export interface InspectorControllerDelegate {
  getSnapshot: () => DesktopSnapshot | undefined;
  showToast: (message: string, kind: "success" | "error") => void;
  copyIdentifier: (id: string) => Promise<void> | void;
  onFocusNode: (nodeId: string) => void;
  onOpenReader: (artifactId: string, label: string, inkFocus?: InkReaderFocus) => void;
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
      if(this.selectedNode&&isRetired(this.selectedNode.lifecycle))this.inspectorLocked=false;
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
    // Phase2: a staged (「未审批」) object is not in the accepted head; the
    // formal edit entry points and head-based source detail do not apply.
    const staged = stagedMark(this.delegate.getSnapshot()?.investigation, node.domain_id);
    if (staged) {
      sourceSlot.append(textElement(
        "p",
        "detail-staged-badge",
        `未审批提案 · ${staged.change_set_id.slice(-12)}@v${staged.change_set_version}${staged.replaces_revision ? "（替换已接受版本）" : ""}。在「审核提案」中接受后才进入正式地图。`,
      ));
      // A staged edit is shown as a replacement; the accepted version is
      // still the head and can be read on demand.
      if (staged.replaces_revision && node.domain_kind === "SourceSemanticUnit") {
        const showAccepted = textElement("button", "parse-button", "查看已接受版本");
        showAccepted.type = "button";
        showAccepted.onclick = () => {
          showAccepted.disabled = true;
          const accepted = textElement("section", "detail-accepted-version", "");
          accepted.append(textElement("h4", "", "已接受版本"));
          sourceSlot.append(accepted);
          void this.renderSourceDetail(node, accepted, actions);
        };
        actions.append(showAccepted);
      }
    }
    const conflict = stagedConflict(this.delegate.getSnapshot()?.investigation, node.domain_id);
    if (!staged && conflict) {
      sourceSlot.append(textElement(
        "p",
        "detail-staged-badge is-conflict",
        `提案冲突：${conflict.length} 个未审批提案同时修改此对象（${conflict.join("、")}），地图显示已接受版本。请在「审核提案」中决定保留哪一个。`,
      ));
    }

    if (
      !staged &&
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
      // A folded canonical node opens its unified, page-ordered read-only view.
      read.onclick = () => {
        let artifactId: string;
        try { artifactId = readerTargetId(node); } catch (error) { this.delegate.showToast(errorMessage(error), "error"); return; }
        this.delegate.onOpenReader(artifactId, node.label);
      };
      actions.append(read);
    }

    if (pdf !== undefined && marker !== undefined) {
      this.nodeDetail.append(this.delegate.createParseForm(pdf, marker));
    }

    if (node.domain_id !== null && (node.domain_kind === "SourceSemanticUnit" || node.domain_kind === "SourceArtifact")) {
      const inkSlot = textElement("section", "detail-ink-notes", "");
      inkSlot.setAttribute("aria-label", "相关手稿");
      this.nodeDetail.insertBefore(inkSlot, editSlot.nextSibling);
      void this.renderInkNotes(node, inkSlot, actions);
    }

    if (
      !staged &&
      node.domain_id !== null &&
      (node.domain_kind === "SourceSemanticUnit" || node.domain_kind === "SourceUnitRelation" || node.domain_kind === "SourceSpan")
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

  /** "相关手稿" (§6.3): notes linked to this object (or, for a Markdown file,
   * to its blocks), plus "新建手稿" for source units. Trashed notes are never
   * listed. */
  private async renderInkNotes(node: IncidenceNode, host: HTMLElement, actions: HTMLElement): Promise<void> {
    const objectId = node.domain_id!;
    const unit = node.domain_kind === "SourceSemanticUnit";
    if (unit && !isRetired(node.lifecycle)) {
      const create = textElement("button", "parse-button", "新建手稿");
      create.type = "button";
      create.onclick = () => {
        const bounds = create.getBoundingClientRect();
        startInkCreateFromTarget({
          x: bounds.left,
          y: bounds.bottom + 4,
          target: { kind: "source_object", object_id: objectId },
          targetLabel: node.label,
          excerpt: node.summary ?? undefined,
        });
      };
      actions.append(create);
    }
    try {
      // Markdown ink links point at the physical snapshot (head for a canonical node).
      const target = unit ? objectId : tryOpenableArtifactId(node);
      if (target === undefined) { host.remove(); return; }
      const notes = await listInkLinksToTarget(unit ? "source_object" : "markdown", target);
      if (this.selectedNode?.domain_id !== objectId || !host.isConnected) return;
      if (!notes.length && !unit) { host.remove(); return; }
      const list = document.createElement("ul");
      const seen = new Set<string>();
      for (const { note, link } of notes) {
        if (seen.has(note.document_id)) continue;
        seen.add(note.document_id);
        const item = document.createElement("li");
        const open = textElement("button", "link-button", note.title);
        open.type = "button";
        open.onclick = () => eventBus.emit("open-ink-document", { documentId: note.document_id });
        item.append(open, textElement("span", "ink-related-meta", roleLabel(link.role) + (link.is_primary ? " · 主关联" : "")));
        list.append(item);
      }
      host.replaceChildren(textElement("h4", "", `相关手稿（${seen.size}）`));
      if (seen.size) host.append(list);
      else host.append(textElement("p", "detail-muted", "还没有相关手稿。"));
    } catch (error) {
      host.replaceChildren(textElement("p", "detail-error", `相关手稿读取失败：${errorMessage(error)}`));
    }
  }

  private async renderSourceDetail(
    node: IncidenceNode,
    host: HTMLElement,
    actions: HTMLElement,
  ): Promise<void> {
    try {
      const detail = await sourceNodeDetail(node.domain_id!);
      if (this.selectedNode?.domain_id !== node.domain_id) return;
      if (detail.excerpt) {
        const preview = document.createElement("div");
        preview.className = "source-preview markdown-body";
        preview.innerHTML = markdownRenderer().render(detail.excerpt);
        host.append(preview);
      }
      if (detail.artifact_id) {
        const button = textElement("button", "parse-button", detail.ink ? "打开并定位手稿证据" : "打开并定位原文");
        button.type = "button";
        button.onclick = () => {
          if (detail.ink) {
            this.delegate.onOpenReader(detail.artifact_id!, detail.display_title ?? node.label, detail.ink);
            return;
          }
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
