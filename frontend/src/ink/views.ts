/**
 * Keeps every view of note links current (N2 §6.3): the map overlay and the
 * inspector re-query after link writes ("ink-links-changed") and after note
 * lifecycle or title writes ("ink-notes-changed": trash, restore, undo,
 * clean-blank, rename). Shared by the app shell and the browser harnesses.
 */
import { eventBus } from "../core/events";
import { getInkLinkOverlay, type InkLinkOverlay } from "./links";

export interface InkViewSync {
  /** Re-reads the overlay; stale replies never overwrite a newer one. */
  reloadOverlay(): Promise<void>;
  dispose(): void;
}

export function connectInkViews(options: {
  applyOverlay: (overlay: InkLinkOverlay) => void;
  refreshInspector: () => void;
}): InkViewSync {
  let revision = 0;
  const reloadOverlay = async () => {
    const current = ++revision;
    try {
      const overlay = await getInkLinkOverlay();
      if (current === revision) options.applyOverlay(overlay);
    } catch {
      // Older hosts have no overlay; the map stays as projected.
    }
  };
  const refresh = () => {
    void reloadOverlay();
    options.refreshInspector();
  };
  const offLinks = eventBus.on("ink-links-changed", refresh);
  const offNotes = eventBus.on("ink-notes-changed", refresh);
  return { reloadOverlay, dispose: () => { offLinks(); offNotes(); } };
}
