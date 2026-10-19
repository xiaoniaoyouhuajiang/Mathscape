/**
 * N3c: side-by-side reading — reader left, 手稿 right, with a draggable
 * divider that reuses the N3.0 panel-resizer patterns (persist, keyboard,
 * double-click reset, re-clamp on window / container resize).
 */
export const INK_SPLIT_WIDTH_KEY = "mathscape:ink-split-reader-width";
export const INK_SPLIT_MIN = 280;
export const INK_SPLIT_MAX = 900;
/** The 手稿 pane keeps at least this much room. */
export const INK_PANE_MIN = 360;
/** Below this window width, split mode exits with a hint. */
export const INK_SPLIT_WINDOW_MIN = 960;

export function clampInkSplitWidth(
  requested: number,
  containerWidth: number,
): number {
  const room = containerWidth > 0 ? containerWidth - INK_PANE_MIN : INK_SPLIT_MAX;
  const max = Math.max(INK_SPLIT_MIN, Math.min(INK_SPLIT_MAX, room));
  if (!Number.isFinite(requested)) return INK_SPLIT_MIN;
  return Math.round(Math.min(max, Math.max(INK_SPLIT_MIN, requested)));
}

export interface InkSplitHandle {
  setEnabled(enabled: boolean): void;
  isEnabled(): boolean;
  /** Preferred reader width in px, or undefined for default (~45%). */
  destroy(): void;
}

export function mountInkSplitResizer(
  container: HTMLElement,
  readerPane: HTMLElement,
  handle: HTMLElement,
): InkSplitHandle {
  handle.className = "ink-split-resizer panel-resizer";
  handle.setAttribute("role", "separator");
  handle.setAttribute("aria-orientation", "vertical");
  handle.setAttribute("aria-label", "调整并排阅读宽度");
  handle.title = "拖动调整宽度；双击恢复默认";
  handle.tabIndex = 0;
  handle.setAttribute("aria-valuemin", String(INK_SPLIT_MIN));
  handle.setAttribute("aria-valuemax", String(INK_SPLIT_MAX));

  const room = () => container.getBoundingClientRect().width;
  let preferred: number | undefined;
  let applied: string | undefined;
  let enabled = false;

  const persist = (): void => {
    try {
      if (preferred === undefined) localStorage.removeItem(INK_SPLIT_WIDTH_KEY);
      else localStorage.setItem(INK_SPLIT_WIDTH_KEY, String(preferred));
    } catch { /* optional */ }
  };

  const update = (): void => {
    if (!enabled) {
      container.style.removeProperty("--ink-split-reader-width");
      handle.removeAttribute("aria-valuenow");
      applied = undefined;
      return;
    }
    let next: string;
    if (preferred === undefined) {
      // Default: about 42% of the container, clamped.
      const value = clampInkSplitWidth(room() * 0.42, room());
      next = value + "px";
      handle.setAttribute("aria-valuenow", String(value));
    } else {
      const value = clampInkSplitWidth(preferred, room());
      next = value + "px";
      handle.setAttribute("aria-valuenow", String(value));
    }
    container.style.setProperty("--ink-split-reader-width", next);
    if (next !== applied) {
      applied = next;
      window.dispatchEvent(new Event("resize"));
    }
  };

  const choose = (width: number | undefined, save: boolean): void => {
    preferred = width === undefined ? undefined : clampInkSplitWidth(width, room());
    update();
    if (save) persist();
  };

  try {
    const stored = Number(localStorage.getItem(INK_SPLIT_WIDTH_KEY));
    if (stored > 0) preferred = Math.round(Math.min(INK_SPLIT_MAX, Math.max(INK_SPLIT_MIN, stored)));
  } catch { /* optional */ }

  let frame = 0;
  const schedule = (): void => {
    if (frame || !enabled) return;
    frame = requestAnimationFrame(() => { frame = 0; update(); });
  };
  const onWindowResize = (): void => { if (enabled) update(); };
  window.addEventListener("resize", onWindowResize);
  let observer: ResizeObserver | undefined;
  if (typeof ResizeObserver !== "undefined") {
    observer = new ResizeObserver(schedule);
    observer.observe(container);
  }

  let dragging: number | undefined;
  handle.addEventListener("pointerdown", event => {
    if (!enabled || event.button !== 0) return;
    event.preventDefault();
    dragging = event.pointerId;
    try { handle.setPointerCapture(event.pointerId); } catch { /* synthetic */ }
    handle.classList.add("is-dragging");
  });
  handle.addEventListener("pointermove", event => {
    if (event.pointerId !== dragging) return;
    choose(event.clientX - container.getBoundingClientRect().left, false);
  });
  const finish = (event: PointerEvent) => {
    if (event.pointerId !== dragging) return;
    dragging = undefined;
    handle.classList.remove("is-dragging");
    choose(readerPane.getBoundingClientRect().width, true);
  };
  handle.addEventListener("pointerup", finish);
  handle.addEventListener("pointercancel", finish);
  handle.addEventListener("dblclick", () => { if (enabled) choose(undefined, true); });
  handle.addEventListener("keydown", event => {
    if (!enabled) return;
    const current = readerPane.getBoundingClientRect().width;
    if (event.key === "ArrowLeft") choose(current - 24, true);
    else if (event.key === "ArrowRight") choose(current + 24, true);
    else if (event.key === "Home") choose(undefined, true);
    else return;
    event.preventDefault();
  });

  return {
    setEnabled(next: boolean) {
      enabled = next;
      update();
    },
    isEnabled: () => enabled,
    destroy() {
      window.removeEventListener("resize", onWindowResize);
      observer?.disconnect();
    },
  };
}
