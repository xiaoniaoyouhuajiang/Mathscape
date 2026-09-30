import type { ReaderFocus } from "../reader/reader";
import type { PatchReviewMode } from "../reader/selection";
import type { DiscoveryOutcome, ReviewRunMeta } from "../review/types";
import type { WorkspaceSettings } from "../workspace-settings";

export interface OpenSourceDetail {
  artifactId: string;
  conversionId?: string | null;
  title: string;
  focus?: ReaderFocus;
  pdfPageIndex?: number;
  patchRunId?: string;
  patchMode?: PatchReviewMode;
  refreshMap?: boolean;
}

export interface OpenReviewDetail {
  outcome: DiscoveryOutcome;
  meta?: ReviewRunMeta;
}

export interface MathscapeEventMap {
  "source-map-changed": { reason?: string } | void;
  "discovery-runs-changed": void;
  "open-source": OpenSourceDetail;
  "open-review": OpenReviewDetail;
  "reader-opened": void;
  "open-settings": void;
  "workspace-settings-changed": WorkspaceSettings;
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
      "open-review",
      "reader-opened",
      "open-settings",
      "workspace-settings-changed",
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
