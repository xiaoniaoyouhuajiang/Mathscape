/**
 * N3.0: the inspector column can be resized, which widens or narrows the
 * centre pane (map and file reader). The width is kept in the CSS variable
 * `--detail-width` on the workspace grid and remembered locally.
 */
export const DETAIL_WIDTH_KEY = "mathscape:detail-width";
export const DETAIL_WIDTH_MIN = 200;
export const DETAIL_WIDTH_MAX = 760;
/** The centre pane keeps at least this much room. */
export const CENTER_MIN = 360;

/** Clamp a requested inspector width for a grid of `gridWidth` pixels whose
 * left column currently takes `leftWidth` pixels. */
export function clampDetailWidth(requested: number, gridWidth: number, leftWidth: number): number {
  const room = gridWidth > 0 ? gridWidth - leftWidth - CENTER_MIN : DETAIL_WIDTH_MAX;
  const max = Math.max(DETAIL_WIDTH_MIN, Math.min(DETAIL_WIDTH_MAX, room));
  if (!Number.isFinite(requested)) return DETAIL_WIDTH_MIN;
  return Math.round(Math.min(max, Math.max(DETAIL_WIDTH_MIN, requested)));
}

export function mountDetailResizer(grid: HTMLElement, panel: HTMLElement): HTMLElement {
  const handle = document.createElement("div");
  handle.className = "panel-resizer";
  handle.setAttribute("role", "separator");
  handle.setAttribute("aria-orientation", "vertical");
  handle.setAttribute("aria-label", "调整检查器宽度");
  handle.title = "拖动调整宽度；双击恢复默认";
  handle.tabIndex = 0;
  panel.prepend(handle);
  const left = () => (grid.firstElementChild as HTMLElement | null)?.getBoundingClientRect().width ?? 0;
  const apply = (width: number | undefined, persist: boolean): void => {
    if (width === undefined) {
      grid.style.removeProperty("--detail-width");
      handle.removeAttribute("aria-valuenow");
      if (persist) try { localStorage.removeItem(DETAIL_WIDTH_KEY); } catch { /* Optional preference. */ }
    } else {
      const value = clampDetailWidth(width, grid.getBoundingClientRect().width, left());
      grid.style.setProperty("--detail-width", value + "px");
      handle.setAttribute("aria-valuenow", String(value));
      if (persist) try { localStorage.setItem(DETAIL_WIDTH_KEY, String(value)); } catch { /* Optional preference. */ }
    }
    window.dispatchEvent(new Event("resize"));
  };
  try {
    const stored = Number(localStorage.getItem(DETAIL_WIDTH_KEY));
    if (stored > 0) apply(stored, false);
  } catch { /* Optional preference. */ }
  handle.setAttribute("aria-valuemin", String(DETAIL_WIDTH_MIN));
  handle.setAttribute("aria-valuemax", String(DETAIL_WIDTH_MAX));
  let dragging: number | undefined;
  handle.addEventListener("pointerdown", event => {
    if (event.button !== 0) return;
    event.preventDefault();
    dragging = event.pointerId;
    try { handle.setPointerCapture(event.pointerId); } catch { /* Synthetic pointers. */ }
    handle.classList.add("is-dragging");
  });
  handle.addEventListener("pointermove", event => {
    if (event.pointerId !== dragging) return;
    apply(grid.getBoundingClientRect().right - event.clientX, false);
  });
  const finish = (event: PointerEvent) => {
    if (event.pointerId !== dragging) return;
    dragging = undefined;
    handle.classList.remove("is-dragging");
    apply(panel.getBoundingClientRect().width, true);
  };
  handle.addEventListener("pointerup", finish);
  handle.addEventListener("pointercancel", finish);
  handle.addEventListener("dblclick", () => apply(undefined, true));
  handle.addEventListener("keydown", event => {
    const current = panel.getBoundingClientRect().width;
    if (event.key === "ArrowLeft") apply(current + 24, true);
    else if (event.key === "ArrowRight") apply(current - 24, true);
    else if (event.key === "Home") apply(undefined, true);
    else return;
    event.preventDefault();
  });
  return handle;
}
