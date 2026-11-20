/**
 * N2 NoteLinks (proposal §4.2, §6, §14): command wrappers, labels and the
 * small popovers shared by the reader, the inspector and the note page.
 */
import { errorCode, errorMessage, safeInvoke } from "../core/ipc";
import { eventBus } from "../core/events";
import { autoInkTitle, inkId, newDocument, type InkDocument } from "./model";
import { listInkDocuments, type InkNoteState, type StoredInkSession } from "./persistence";

export type InkLinkRole = "attempt" | "alt_proof" | "check" | "study" | "question" | "reference";

export const INK_LINK_ROLES: ReadonlyArray<{ role: InkLinkRole; label: string }> = [
  { role: "attempt", label: "尝试" },
  { role: "alt_proof", label: "另证" },
  { role: "check", label: "验算" },
  { role: "study", label: "学习笔记" },
  { role: "question", label: "疑问" },
  { role: "reference", label: "参考" },
];

export function roleLabel(role: InkLinkRole): string {
  return INK_LINK_ROLES.find(item => item.role === role)?.label ?? role;
}

/** A Markdown block range as the reader knows it (§14.3). */
export interface InkMarkdownTargetRef {
  artifact_id: string;
  content_hash: string;
  first_block: string;
  last_block: string;
}

export type InkLinkTargetRef =
  | { kind: "source_object"; object_id: string }
  | ({ kind: "markdown" } & InkMarkdownTargetRef)
  | { kind: "ink_note"; document_id: string };

export interface InkStoredMarkdownTarget extends InkMarkdownTargetRef {
  block_ids: string[];
  start: number;
  end: number;
  block_hash: string;
  excerpt: string;
  label: string;
}

export type InkStoredTarget =
  | { kind: "source_object"; object_id: string }
  | ({ kind: "markdown" } & InkStoredMarkdownTarget)
  | { kind: "ink_note"; document_id: string };

export type InkLinkTargetStatus = "active" | "retired" | "trashed" | "missing";

export interface InkNoteLink {
  link_id: string;
  document_id: string;
  target_kind: InkStoredTarget["kind"];
  target_key: string;
  target_id: string;
  target: InkStoredTarget;
  target_revision: string | null;
  target_label: string;
  role: InkLinkRole;
  is_primary: boolean;
  state: "active" | "removed";
  created_at: number;
  removed_at: number | null;
  target_status: InkLinkTargetStatus;
}

export interface InkLinkedNote { note: InkNoteState; link: InkNoteLink }

export interface InkLinkReceipt {
  request_id: string;
  kind: "link" | "unlink" | "set_primary";
  note: InkNoteState;
  link: InkNoteLink | null;
  /** The (target, role) link already existed; `link` is that link. */
  existing: boolean;
  /** D12 merge: the replaced link was folded into the existing one. */
  merged?: boolean;
  primary_changed?: boolean;
  title_changed: boolean;
  replayed: boolean;
}

export interface InkContextCreateReceipt extends StoredInkSession {
  note: InkNoteState;
  link: InkNoteLink;
  request_id: string;
  replayed: boolean;
}

export interface InkOverlayLink {
  document_id: string;
  title: string;
  last_used_at: number;
  target_kind: "source_object" | "markdown";
  target_id: string;
  role: InkLinkRole;
  is_primary: boolean;
}

export interface InkLinkOverlay {
  counts: Array<{ target_kind: "source_object" | "markdown"; target_id: string; note_count: number }>;
  links: InkOverlayLink[];
}

export interface InkMarkdownLinks {
  artifact_id: string;
  links: InkLinkedNote[];
  unit_links?: InkLinkedNote[];
  original_note_count: number;
}

/** A new note waiting to be created; reused on retry so a lost reply never
 * creates a second note. */
export interface InkCreateDraft { requestId: string; sessionId: string; document: InkDocument }

export function newInkCreateDraft(): InkCreateDraft {
  const document = newDocument();
  document.title = autoInkTitle(document.created_at);
  return { requestId: inkId(), sessionId: inkId(), document };
}

export function createInkNoteFromContext(draft: InkCreateDraft, target: InkLinkTargetRef, role: InkLinkRole, title: string): Promise<InkContextCreateReceipt> {
  return safeInvoke("create_ink_note_from_context", {
    request: {
      request_id: draft.requestId,
      document: structuredClone(draft.document),
      session_id: draft.sessionId,
      writer_id: "desktop-local",
      device: { source: "desktop" },
      target,
      role,
      title: title.trim() || null,
    },
  });
}

export function linkInkNote(requestId: string, documentId: string, target: InkLinkTargetRef, role: InkLinkRole, primary: boolean, replacesLinkId?: string): Promise<InkLinkReceipt> {
  return safeInvoke("link_ink_note", {
    request: { request_id: requestId, document_id: documentId, target, role, primary, replaces_link_id: replacesLinkId ?? null },
  });
}

export function unlinkInkNote(requestId: string, documentId: string, linkId: string): Promise<InkLinkReceipt> {
  return safeInvoke("unlink_ink_note", { request: { request_id: requestId, document_id: documentId, link_id: linkId } });
}

export function setInkPrimaryLink(requestId: string, documentId: string, linkId: string | null): Promise<InkLinkReceipt> {
  return safeInvoke("set_ink_note_primary_link", { request: { request_id: requestId, document_id: documentId, link_id: linkId } });
}

export function listInkNoteLinks(documentId: string): Promise<InkNoteLink[]> {
  return safeInvoke("list_ink_note_links", { documentId });
}

export function listInkLinksToTarget(targetKind: InkStoredTarget["kind"], targetId: string): Promise<InkLinkedNote[]> {
  return safeInvoke("list_ink_links_to_target", { targetKind, targetId });
}

export function getInkMarkdownLinks(artifactId: string): Promise<InkMarkdownLinks> {
  return safeInvoke("get_ink_markdown_links", { artifactId });
}

export function getInkLinkOverlay(): Promise<InkLinkOverlay> {
  return safeInvoke("get_ink_link_overlay");
}

export function exportInkUnsavedWriting(fileName: string, contents: string): Promise<string | null> {
  return safeInvoke("export_ink_unsaved_writing", { fileName, contents });
}

export function exportInkFile(fileName: string, format: "json" | "png", contents: Uint8Array): Promise<string | null> {
  return safeInvoke("export_ink_file", { fileName, format, contents: Array.from(contents) });
}

/** Notes whose Markdown range overlaps any of `blockIds` (§6.5), one entry
 * per note (primary link first), so the count matches the block markers. */
export function overlappingNotes(links: InkLinkedNote[], blockIds: readonly string[]): InkLinkedNote[] {
  const wanted = new Set(blockIds);
  const byNote = new Map<string, InkLinkedNote>();
  for (const entry of links) {
    const { link } = entry;
    if (link.target.kind !== "markdown" || !link.target.block_ids.some(id => wanted.has(id))) continue;
    const previous = byNote.get(link.document_id);
    if (!previous || (link.is_primary && !previous.link.is_primary)) byNote.set(link.document_id, entry);
  }
  return [...byNote.values()];
}

/**
 * Combines block-level Markdown links (`target.kind === "markdown"`) and
 * `SourceSemanticUnit` links (`target.kind === "source_object"`) anchored on
 * `blockIds`, deduplicated per `document_id` (preferring primary links).
 */
export function overlappingNotesForBlocks(
  links: readonly InkLinkedNote[],
  unitLinks: readonly InkLinkedNote[],
  blockIds: readonly string[],
  unitIdsByBlock?: ReadonlyMap<string, readonly string[]>,
  expectedContentHash?: string,
): InkLinkedNote[] {
  const wantedBlocks = new Set(blockIds);
  const wantedUnits = new Set<string>();
  if (unitIdsByBlock) {
    for (const bid of blockIds) {
      for (const uid of unitIdsByBlock.get(bid) ?? []) wantedUnits.add(uid);
    }
  }
  const byNote = new Map<string, InkLinkedNote>();
  const record = (entry: InkLinkedNote) => {
    const previous = byNote.get(entry.link.document_id);
    if (!previous || (entry.link.is_primary && !previous.link.is_primary)) {
      byNote.set(entry.link.document_id, entry);
    }
  };
  for (const entry of links) {
    const { link } = entry;
    if (link.target.kind !== "markdown") continue;
    if (expectedContentHash && link.target.content_hash !== expectedContentHash) continue;
    if (!link.target.block_ids.some((id) => wantedBlocks.has(id))) continue;
    record(entry);
  }
  if (wantedUnits.size > 0) {
    for (const entry of unitLinks) {
      const { link } = entry;
      if (link.target.kind !== "source_object") continue;
      if (!wantedUnits.has(link.target.object_id)) continue;
      record(entry);
    }
  }
  return [...byNote.values()];
}

/** Display-only short name of a Markdown excerpt; the desktop computes the
 * stored one with the same rules (§5.2). */
export function markdownPreviewLabel(source: string): string {
  const text = source.split("\n").map(line => line.replace(/^\s*(?:#+|>|[-*+]\s|\d+[.)]\s)\s*/, "")).join(" ")
    .replace(/\*\*|__|`/g, "").replace(/\s+/g, " ").trim();
  const numbered = /^(定理|引理|命题|推论|定义|例题|习题|练习|问题|例|注|Theorem|Lemma|Proposition|Corollary|Definition|Example|Exercise|Problem)\s*(\d[\d.-]*)/.exec(text);
  if (numbered) return `${numbered[1]} ${numbered[2].replace(/[.-]+$/, "")}`;
  return [...text].slice(0, 24).join("").trim() || "Markdown 片段";
}

export function targetStatusLabel(status: InkLinkTargetStatus): string {
  return ({ active: "", retired: "目标已退役", trashed: "目标在回收站", missing: "目标已不存在" } as const)[status];
}

/** Opens the target of a link from the note page (§6.2). */
export function openInkLinkTarget(link: InkNoteLink): void {
  eventBus.emit("open-ink-link-target", { link });
}

// ------------------------------------------------------------------ popovers

let openPopover: { element: HTMLElement; close: () => void } | undefined;

export function closeInkPopover(): void {
  openPopover?.close();
}

function popover(x: number, y: number, label: string, onClose: () => void): HTMLElement {
  closeInkPopover();
  const element = document.createElement("section");
  element.className = "ink-link-popover";
  element.setAttribute("role", "dialog");
  element.setAttribute("aria-label", label);
  element.style.left = "-9999px";
  element.style.top = "-9999px";
  document.body.append(element);
  const outside = (event: PointerEvent) => { if (!element.contains(event.target as Node)) close(); };
  const key = (event: KeyboardEvent) => { if (event.key === "Escape") { event.preventDefault(); close(); } };
  const close = () => {
    if (openPopover?.element !== element) return;
    openPopover = undefined;
    element.remove();
    document.removeEventListener("pointerdown", outside, true);
    document.removeEventListener("keydown", key, true);
    onClose();
  };
  openPopover = { element, close };
  requestAnimationFrame(() => {
    const bounds = element.getBoundingClientRect();
    element.style.left = `${Math.max(8, Math.min(x, window.innerWidth - bounds.width - 8))}px`;
    element.style.top = `${Math.max(8, Math.min(y, window.innerHeight - bounds.height - 8))}px`;
  });
  document.addEventListener("pointerdown", outside, true);
  document.addEventListener("keydown", key, true);
  return element;
}

function node<K extends keyof HTMLElementTagNameMap>(tag: K, text = "", className = ""): HTMLElementTagNameMap[K] {
  const element = document.createElement(tag);
  element.textContent = text;
  if (className) element.className = className;
  return element;
}

function roleSelect(value: InkLinkRole): HTMLSelectElement {
  const select = node("select");
  select.setAttribute("aria-label", "用途");
  for (const item of INK_LINK_ROLES) select.add(new Option(item.label, item.role));
  select.value = value;
  return select;
}

export interface InkCreateChoice { role: InkLinkRole; title: string; mode: "desktop" | "tablet" }

/**
 * "新建关联手稿" (§6.5): purpose, title and where to write. `submit` performs
 * the creation; on failure the popover stays open with the user's input.
 */
export function openCreateInkPopover(options: {
  x: number; y: number;
  targetLabel: string;
  excerpt?: string;
  defaultRole?: InkLinkRole;
  submit: (choice: InkCreateChoice) => Promise<void>;
  onClose?: () => void;
}): void {
  const root = popover(options.x, options.y, "新建关联手稿", options.onClose ?? (() => {}));
  root.classList.add("ink-create-popover");
  const role = roleSelect(options.defaultRole ?? "attempt");
  const title = node("input");
  title.type = "text"; title.maxLength = 100; title.setAttribute("aria-label", "标题");
  const placeholder = () => { title.placeholder = `留空自动命名：${roleLabel(role.value as InkLinkRole)} · ${options.targetLabel}`; };
  placeholder();
  role.onchange = placeholder;
  const message = node("p", "", "ink-link-message");
  message.setAttribute("role", "status");
  const actions = node("div", "", "ink-link-actions");
  const desktop = node("button", "在电脑上书写", "primary-button");
  const tablet = node("button", "用平板书写", "secondary-button");
  const cancel = node("button", "取消", "secondary-button");
  for (const button of [desktop, tablet, cancel]) button.type = "button";
  const tabletHint = node("p", "", "ink-tablet-hint");
  tabletHint.hidden = true;
  void safeInvoke<{ running?: boolean; online_tablets?: number }>("get_ink_lan_status")
    .then((status) => {
      if (!root.isConnected) return;
      if (status?.running && (status.online_tablets ?? 0) > 0) {
        tablet.textContent = "推送到平板（已在线）";
        tablet.classList.add("is-tablet-online");
        tabletHint.textContent = "检测到平板已连接，创建后将直接推送切换，无需重新扫码。";
        tabletHint.hidden = false;
      } else {
        tabletHint.textContent = "当前未连接平板，点击「用平板书写」后将打开配对二维码。";
        tabletHint.hidden = false;
      }
    })
    .catch(() => { /* Optional hint when LAN IPC is unavailable. */ });
  const go = async (mode: InkCreateChoice["mode"]) => {
    desktop.disabled = tablet.disabled = true;
    message.textContent = "正在创建手稿…";
    try {
      await options.submit({ role: role.value as InkLinkRole, title: title.value, mode });
      closeInkPopover();
    } catch (error) {
      message.textContent = errorCode(error) === "link_target_invalid" || errorCode(error) === "link_target_retired"
        ? `${errorMessage(error)}请关闭后重新选择。`
        : `创建失败，可重试：${errorMessage(error)}`;
      desktop.disabled = tablet.disabled = false;
    }
  };
  desktop.onclick = () => void go("desktop");
  tablet.onclick = () => void go("tablet");
  cancel.onclick = () => closeInkPopover();
  actions.append(desktop, tablet, cancel);
  const target = node("p", `关联到：${options.targetLabel}`, "ink-link-target");
  const fields = node("div", "", "ink-link-fields");
  const roleLabelEl = node("label", "用途 "); roleLabelEl.append(role);
  const titleLabelEl = node("label", "标题 "); titleLabelEl.append(title);
  fields.append(roleLabelEl, titleLabelEl);
  root.append(node("h3", "新建关联手稿"), target);
  if (options.excerpt) root.append(node("blockquote", options.excerpt, "ink-link-excerpt"));
  root.append(fields, message, actions, tabletHint);
  desktop.focus();
}

/**
 * "关联到已有手稿" (§6.5, D12): candidates by last use, a duplicate returns the
 * existing link, and "设为主关联" defaults to on only when the chosen note has
 * no primary link yet.
 */
export function openLinkExistingPopover(options: {
  x: number; y: number;
  target: InkLinkTargetRef;
  targetLabel: string;
  excludeDocumentId?: string;
  onClose?: () => void;
}): void {
  const root = popover(options.x, options.y, "关联到已有手稿", options.onClose ?? (() => {}));
  root.classList.add("ink-existing-popover");
  const search = node("input");
  search.type = "search"; search.placeholder = "搜索手稿标题"; search.setAttribute("aria-label", "搜索手稿");
  const list = node("div", "", "ink-link-candidates");
  list.setAttribute("role", "listbox");
  list.setAttribute("aria-label", "已有手稿");
  const role = roleSelect("attempt");
  const primary = node("input"); primary.type = "checkbox";
  const primaryLabel = node("label", "", "ink-check"); primaryLabel.append(primary, document.createTextNode(" 设为主关联"));
  const hint = node("p", "", "ink-link-hint");
  const message = node("p", "", "ink-link-message"); message.setAttribute("role", "status");
  const submit = node("button", "关联", "primary-button"); submit.type = "button"; submit.disabled = true;
  const cancel = node("button", "取消", "secondary-button"); cancel.type = "button";
  const actions = node("div", "", "ink-link-actions"); actions.append(submit, cancel);
  const roleLabelEl = node("label", "用途 "); roleLabelEl.append(role);
  root.append(node("h3", "关联到已有手稿"), node("p", `目标：${options.targetLabel}`, "ink-link-target"), search, list, roleLabelEl, primaryLabel, hint, message, actions);
  let selected: string | undefined;
  let currentPrimary: InkNoteLink | undefined;
  let request: { id: string; key: string } | undefined;
  let candidates: Array<{ document_id: string; title: string }> = [];
  const updateHint = () => {
    hint.textContent = primary.checked && currentPrimary ? `将替换现有主关联「${currentPrimary.target_label}」。` : "";
  };
  primary.onchange = updateHint;
  const choose = async (documentId: string) => {
    selected = documentId;
    list.querySelectorAll<HTMLButtonElement>("button").forEach(button => button.setAttribute("aria-selected", String(button.dataset.documentId === documentId)));
    submit.disabled = true;
    try {
      const links = await listInkNoteLinks(documentId);
      if (selected !== documentId) return;
      currentPrimary = links.find(link => link.is_primary);
      primary.checked = !currentPrimary;
      updateHint();
    } catch {
      currentPrimary = undefined; primary.checked = false; updateHint();
    }
    submit.disabled = false;
  };
  const render = () => {
    const query = search.value.trim().toLowerCase();
    list.replaceChildren();
    const shown = candidates.filter(item => !query || item.title.toLowerCase().includes(query));
    if (!shown.length) { list.append(node("p", candidates.length ? "没有匹配的手稿。" : "还没有其他手稿。")); return; }
    for (const item of shown) {
      const button = node("button", item.title, "ink-link-candidate");
      button.type = "button";
      button.setAttribute("role", "option");
      button.dataset.documentId = item.document_id;
      button.setAttribute("aria-selected", String(item.document_id === selected));
      button.onclick = () => void choose(item.document_id);
      list.append(button);
    }
  };
  search.oninput = render;
  cancel.onclick = () => closeInkPopover();
  submit.onclick = () => void (async () => {
    if (!selected) return;
    const key = JSON.stringify([selected, options.target, role.value, primary.checked]);
    if (request?.key !== key) request = { id: inkId(), key };
    submit.disabled = true;
    message.textContent = "正在关联…";
    try {
      const receipt = await linkInkNote(request.id, selected, options.target, role.value as InkLinkRole, primary.checked);
      request = undefined;
      const documentId = selected;
      message.replaceChildren(document.createTextNode(!receipt.existing ? `已关联到「${receipt.note.title}」 · ` : receipt.primary_changed ? "已关联，已设为主关联 · " : "已关联 · "));
      const open = node("button", "打开手稿", "link-button"); open.type = "button";
      open.onclick = () => { closeInkPopover(); eventBus.emit("open-ink-document", { documentId }); };
      message.append(open);
      if (!receipt.existing || receipt.primary_changed) eventBus.emit("ink-links-changed");
    } catch (error) {
      message.textContent = "关联失败：" + errorMessage(error);
      submit.disabled = false;
    }
  })();
  list.append(node("p", "正在读取手稿…"));
  void listInkDocuments().then(documents => {
    candidates = documents
      .filter(item => item.lifecycle !== "trashed" && item.document_id !== options.excludeDocumentId)
      .sort((a, b) => (b.last_used_at ?? 0) - (a.last_used_at ?? 0));
    render();
  }).catch(error => { list.replaceChildren(node("p", "读取手稿失败：" + errorMessage(error))); });
  search.focus();
}

/** "查看相关手稿" and block markers share this list (§6.5). */
export function openRelatedNotesPopover(options: { x: number; y: number; title: string; notes: InkLinkedNote[]; onClose?: () => void }): void {
  const root = popover(options.x, options.y, "相关手稿", options.onClose ?? (() => {}));
  root.classList.add("ink-related-popover");
  root.append(node("h3", options.title));
  const list = node("ul", "", "ink-related-list");
  for (const { note, link } of options.notes) {
    const item = node("li");
    const open = node("button", note.title, "link-button"); open.type = "button";
    open.onclick = () => { closeInkPopover(); eventBus.emit("open-ink-document", { documentId: note.document_id }); };
    item.append(open, node("span", ` ${roleLabel(link.role)}${link.is_primary ? " · 主关联" : ""}`, "ink-related-meta"));
    list.append(item);
  }
  if (!options.notes.length) list.append(node("li", "还没有相关手稿。"));
  root.append(list);
}

// ------------------------------------------------------- context creation

type InkContextCreator = (target: InkLinkTargetRef, choice: InkCreateChoice, draft: InkCreateDraft) => Promise<void>;
let contextCreator: InkContextCreator | undefined;

/** Registered by the app shell: creates the note and opens it (§6.5). */
export function setInkContextCreator(creator: InkContextCreator): void {
  contextCreator = creator;
}

/** "新建关联手稿…" for any target. One draft per popover, so retries after a
 * lost reply reuse the same request id. */
export function startInkCreateFromTarget(options: {
  x: number; y: number;
  target: InkLinkTargetRef;
  targetLabel: string;
  excerpt?: string;
  onClose?: () => void;
}): void {
  const draft = newInkCreateDraft();
  openCreateInkPopover({
    x: options.x, y: options.y,
    targetLabel: options.targetLabel,
    excerpt: options.excerpt,
    onClose: options.onClose,
    submit: async choice => {
      if (!contextCreator) throw new Error("手稿工作区尚未就绪");
      await contextCreator(options.target, choice, draft);
    },
  });
}

// ------------------------------------------------------- note-page linking

export interface InkSourceCandidate { object_id: string; label: string; kind: string }
let sourceCandidates: () => InkSourceCandidate[] = () => [];

/** Registered by the app shell from the current map snapshot. */
export function setInkSourceCandidateProvider(provider: () => InkSourceCandidate[]): void {
  sourceCandidates = provider;
}

/**
 * "添加关联" on the note page (§6.2): search source objects and other notes.
 * There is no "最近阅读片段" list; Markdown ranges are linked from the reader.
 */
export function openAddLinkPopover(options: {
  x: number; y: number;
  documentId: string;
  hasPrimary: boolean;
  onLinked: (receipt: InkLinkReceipt) => void;
}): void {
  const root = popover(options.x, options.y, "添加关联", () => {});
  root.classList.add("ink-add-link-popover");
  const search = node("input");
  search.type = "search"; search.placeholder = "搜索来源对象或其他手稿"; search.setAttribute("aria-label", "搜索关联目标");
  const list = node("div", "", "ink-link-candidates");
  list.setAttribute("role", "listbox"); list.setAttribute("aria-label", "关联目标");
  const role = roleSelect("attempt");
  const primary = node("input"); primary.type = "checkbox"; primary.checked = !options.hasPrimary;
  const primaryLabel = node("label", "", "ink-check"); primaryLabel.append(primary, document.createTextNode(" 设为主关联"));
  const message = node("p", "", "ink-link-message"); message.setAttribute("role", "status");
  const submit = node("button", "关联", "primary-button"); submit.type = "button"; submit.disabled = true;
  const cancel = node("button", "取消", "secondary-button"); cancel.type = "button";
  const actions = node("div", "", "ink-link-actions"); actions.append(submit, cancel);
  const roleLabelEl = node("label", "用途 "); roleLabelEl.append(role);
  root.append(node("h3", "添加关联"), search, list, roleLabelEl, primaryLabel, message, actions);
  type Candidate = { key: string; label: string; detail: string; target: InkLinkTargetRef };
  let candidates: Candidate[] = sourceCandidates().map(item => ({
    key: `src:${item.object_id}`, label: item.label, detail: "来源", target: { kind: "source_object", object_id: item.object_id },
  }));
  let selected: Candidate | undefined;
  let request: { id: string; key: string } | undefined;
  const render = () => {
    const query = search.value.trim().toLowerCase();
    const shown = candidates.filter(item => !query || item.label.toLowerCase().includes(query)).slice(0, 80);
    list.replaceChildren();
    if (!shown.length) { list.append(node("p", "没有匹配的目标。")); return; }
    for (const item of shown) {
      const button = node("button", "", "ink-link-candidate");
      button.type = "button"; button.setAttribute("role", "option");
      button.setAttribute("aria-selected", String(item.key === selected?.key));
      button.append(node("span", item.label), node("span", ` ${item.detail}`, "ink-related-meta"));
      button.onclick = () => { selected = item; submit.disabled = false; render(); };
      list.append(button);
    }
  };
  search.oninput = render;
  cancel.onclick = () => closeInkPopover();
  submit.onclick = () => void (async () => {
    if (!selected) return;
    const key = JSON.stringify([selected.key, role.value, primary.checked]);
    if (request?.key !== key) request = { id: inkId(), key };
    submit.disabled = true;
    message.textContent = "正在关联…";
    try {
      const receipt = await linkInkNote(request.id, options.documentId, selected.target, role.value as InkLinkRole, primary.checked);
      request = undefined;
      options.onLinked(receipt);
      closeInkPopover();
    } catch (error) {
      message.textContent = "关联失败：" + errorMessage(error);
      submit.disabled = false;
    }
  })();
  render();
  void listInkDocuments().then(documents => {
    const notes = documents
      .filter(item => item.lifecycle !== "trashed" && item.document_id !== options.documentId)
      .sort((a, b) => (b.last_used_at ?? 0) - (a.last_used_at ?? 0))
      .map(item => ({ key: `ink:${item.document_id}`, label: item.title, detail: "手稿", target: { kind: "ink_note", document_id: item.document_id } as InkLinkTargetRef }));
    candidates = [...candidates, ...notes];
    render();
  }).catch(() => { /* Source objects remain searchable. */ });
  search.focus();
}
