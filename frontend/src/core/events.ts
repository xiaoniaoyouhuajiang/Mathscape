import type { ReaderFocus, ReaderReturnTarget } from "../reader/reader";
import type { PatchReviewMode } from "../reader/selection";
import type { DiscoveryOutcome, ReviewRunMeta } from "../review/types";
import type { WorkspaceSettings } from "../workspace-settings";
import type { InkNoteLink } from "../ink/links";

export interface OpenSourceDetail {
  artifactId: string;
  conversionId?: string | null;
  title: string;
  focus?: ReaderFocus;
  pdfPageIndex?: number;
  candidatePages?: number[];
  backlinkNote?: string;
  focusExcerpt?: string;
  returnTo?: ReaderReturnTarget;
  patchRunId?: string;
  patchMode?: PatchReviewMode;
  refreshMap?: boolean;
}

export interface OpenReviewDetail {
  outcome: DiscoveryOutcome;
  meta?: ReviewRunMeta;
}

export interface MathscapeEventMap {
  /** `preserveReader`: redraw the map without closing the open reader document. */
  "source-map-changed": { reason?: string; preserveReader?: boolean } | void;
  "discovery-runs-changed": void;
  "open-source": OpenSourceDetail;
  "open-ink-document": { documentId: string };
  /** N2: a note link changed; overlays and reader markers reload. */
  "ink-links-changed": void;
  /** N2: a note's lifecycle or title changed (trash, restore, undo, clean
   * blank, rename, tablet rename); link views drop or relabel it. */
  "ink-notes-changed": void;
  /** N2: open the target of a note link from the note page. */
  "open-ink-link-target": { link: InkNoteLink };
  /** N3c: toggle or set side-by-side reading (reader | 手稿). */
  "ink-split-toggle": { enabled?: boolean } | void;
  /** N3d: open the workspace search panel (⌘K / Ctrl+K, 搜索 buttons). */
  "open-workspace-search": { query?: string } | void;
  "open-review": OpenReviewDetail;
  /** Manual proposal review of an investigation with nothing pending. */
  "open-review-empty": { investigationId: string };
  "reader-opened": void;
  "open-settings": void;
  "workspace-settings-changed": WorkspaceSettings;
  /** 语义层「反馈给 agent」：对话框插入引用（从不发送），插入成功时调用 `accept`。 */
  "insert-agent-feedback": { text: string; accept: () => void };
  /** 打开学习视图（只读）。 */
  "open-learning-view": void;
  /** 打开只读教程「两本书讲的是不是同一个东西」。 */
  "open-tutorial": void;
  /** 从地图 / Inspector 跳转到指定提案审核（默认打开地图联动分屏模式）。 */
  "open-proposal-for-object": { changeSetId?: string | null; objectId?: string | null; mapLinked?: boolean };
  /** 从阅读器内已入图单元徽章跳转到主地图对应领域节点。 */
  "locate-domain-object": { objectId: string; title?: string };
  /** 块级 Markdown 勘误保存或撤销；通知阅读器、手稿题卡与地图复核提示同步刷新。 */
  "markdown-errata-changed": {
    artifactId: string;
    errataId: string;
    action: "saved" | "reverted" | "discovery_patch";
    affectedUnitIds: string[];
  };
  /** 已入图单元的勘误复核状态更新（标记已复核 / 交沙盒复核）；刷新地图微标与详情卡。 */
  "errata-reviews-changed": { unitId?: string } | void;
  /** 审核工作区处于地图联动分屏模式时，驱动主地图高亮提案涉及的子图与 1 跳上下文。 */
  "review-map-scope": {
    active: boolean;
    nodeIds: string[];
    relationIds: string[];
    focusDomainId?: string | null;
    description?: string;
  };
}

export type MathscapeEventName = keyof MathscapeEventMap;
export type EventHandler<T> = (payload: T) => void;
export type Unsubscribe = () => void;

/**
 * Type-safe event bus backed by native EventTarget.
 *
 * It bridges automatically with window CustomEvents prefixed by "mathscape:"
 * to maintain complete backward compatibility during gradual refactoring.
 */
export class MathscapeEventBus {
  private readonly target = new EventTarget();
  private isBridging = false;

  constructor() {
    this.setupWindowBridge();
  }

  /**
   * Listen to external window.dispatchEvent("mathscape:*") calls
   * and forward them into the typed event bus.
   */
  private setupWindowBridge(): void {
    if (typeof window === "undefined") return;

    const eventNames: MathscapeEventName[] = [
      "source-map-changed",
      "discovery-runs-changed",
      "open-source",
      "open-ink-document",
      "ink-links-changed",
      "ink-notes-changed",
      "open-ink-link-target",
      "ink-split-toggle",
      "open-workspace-search",
      "open-review",
      "open-review-empty",
      "reader-opened",
      "open-settings",
      "workspace-settings-changed",
      "locate-domain-object",
    ];

    for (const name of eventNames) {
      window.addEventListener(`mathscape:${name}`, ((event: Event) => {
        if (this.isBridging) return;
        const customEvent = event as CustomEvent<unknown>;
        const payload = customEvent.detail;
        this.emitInternal(name, payload as MathscapeEventMap[typeof name], false);
      }) as EventListener);
    }
  }

  /**
   * Internal emitter that dispatches to EventTarget and optionally to window.
   */
  private emitInternal<K extends MathscapeEventName>(
    event: K,
    payload: MathscapeEventMap[K],
    dispatchToWindow: boolean,
  ): void {
    this.target.dispatchEvent(new CustomEvent(event, { detail: payload }));

    if (dispatchToWindow && typeof window !== "undefined") {
      this.isBridging = true;
      try {
        window.dispatchEvent(
          new CustomEvent(`mathscape:${event}`, { detail: payload }),
        );
      } finally {
        this.isBridging = false;
      }
    }
  }

  /**
   * Emit a strongly typed event.
   */
  emit<K extends MathscapeEventName>(
    event: K,
    ...args: MathscapeEventMap[K] extends void ? [payload?: void] : [payload: MathscapeEventMap[K]]
  ): void {
    const payload = args[0] as MathscapeEventMap[K];
    this.emitInternal(event, payload, true);
  }

  /**
   * Subscribe to a typed event. Returns an unsubscribe function.
   */
  on<K extends MathscapeEventName>(
    event: K,
    handler: EventHandler<MathscapeEventMap[K]>,
  ): Unsubscribe {
    const listener = (e: Event) => {
      const customEvent = e as CustomEvent<MathscapeEventMap[K]>;
      handler(customEvent.detail);
    };

    this.target.addEventListener(event, listener);
    return () => {
      this.target.removeEventListener(event, listener);
    };
  }

  /**
   * Subscribe to a typed event only once.
   */
  once<K extends MathscapeEventName>(
    event: K,
    handler: EventHandler<MathscapeEventMap[K]>,
  ): Unsubscribe {
    const listener = (e: Event) => {
      this.target.removeEventListener(event, listener);
      const customEvent = e as CustomEvent<MathscapeEventMap[K]>;
      handler(customEvent.detail);
    };

    this.target.addEventListener(event, listener);
    return () => {
      this.target.removeEventListener(event, listener);
    };
  }
}

/** Global singleton event bus instance. */
export const eventBus = new MathscapeEventBus();
