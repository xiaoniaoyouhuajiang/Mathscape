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

/**
 * The user's preferred width is kept (and persisted) as chosen; the applied
 * width is that preference clamped to the room actually available now. The
 * clamp is re-evaluated whenever the window, the outer container or the left
 * column changes size, so shrinking the window never pushes the inspector
 * off-screen and growing it back restores the preferred width.
 */
export function mountDetailResizer(grid: HTMLElement, panel: HTMLElement): HTMLElement {
  const handle = document.createElement("div");
  handle.className = "panel-resizer";
  handle.setAttribute("role", "separator");
  handle.setAttribute("aria-orientation", "vertical");
  handle.setAttribute("aria-label", "调整检查器宽度");
  handle.title = "拖动调整宽度；双击恢复默认";
  handle.tabIndex = 0;
  panel.prepend(handle);
  const leftColumn = () => grid.firstElementChild as HTMLElement | null;
  const left = () => leftColumn()?.getBoundingClientRect().width ?? 0;
  /** Width the grid may use: its outer container, not the (possibly overflowing) tracks. */
  const room = () => {
    const own = grid.getBoundingClientRect().width;
    const outer = grid.parentElement?.clientWidth ?? own;
    return outer > 0 ? Math.min(own || outer, outer) : own;
  };
  let preferred: number | undefined;
  let applied: string | undefined;
  const persist = (): void => {
    try {
      if (preferred === undefined) localStorage.removeItem(DETAIL_WIDTH_KEY);
      else localStorage.setItem(DETAIL_WIDTH_KEY, String(preferred));
    } catch { /* Optional preference. */ }
  };
  /** Apply the effective width; announce a layout change only when it moved. */
  const update = (): void => {
    let next: string | undefined;
    if (preferred === undefined) {
      grid.style.removeProperty("--detail-width");
      handle.removeAttribute("aria-valuenow");
    } else {
      const value = clampDetailWidth(preferred, room(), left());
      next = value + "px";
      grid.style.setProperty("--detail-width", next);
      handle.setAttribute("aria-valuenow", String(value));
    }
    if (next !== applied) {
      applied = next;
      // Lets width-dependent views (graph, PDF fit modes) refit.
      window.dispatchEvent(new Event("resize"));
    }
  };
  /** A manual choice: clamp to the current room, then keep it as the preference. */
  const choose = (width: number | undefined, save: boolean): void => {
    preferred = width === undefined ? undefined : clampDetailWidth(width, room(), left());
    update();
    if (save) persist();
  };
  try {
    const stored = Number(localStorage.getItem(DETAIL_WIDTH_KEY));
    if (stored > 0) { preferred = Math.round(Math.min(DETAIL_WIDTH_MAX, Math.max(DETAIL_WIDTH_MIN, stored))); update(); }
  } catch { /* Optional preference. */ }
  handle.setAttribute("aria-valuemin", String(DETAIL_WIDTH_MIN));
  handle.setAttribute("aria-valuemax", String(DETAIL_WIDTH_MAX));
  // Re-clamp on window resize and on container / left-column (sidebar toggle) changes.
  let frame = 0;
  const schedule = (): void => {
    if (frame || preferred === undefined) return;
    frame = requestAnimationFrame(() => { frame = 0; update(); });
  };
  window.addEventListener("resize", () => { if (preferred !== undefined) update(); });
  if (typeof ResizeObserver !== "undefined") {
    const observer = new ResizeObserver(schedule);
    if (grid.parentElement) observer.observe(grid.parentElement);
    observer.observe(grid);
    const column = leftColumn();
    if (column && column !== panel) observer.observe(column);
  }
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
    choose(grid.getBoundingClientRect().left + room() - event.clientX, false);
  });
  const finish = (event: PointerEvent) => {
    if (event.pointerId !== dragging) return;
    dragging = undefined;
    handle.classList.remove("is-dragging");
    choose(panel.getBoundingClientRect().width, true);
  };
  handle.addEventListener("pointerup", finish);
  handle.addEventListener("pointercancel", finish);
  handle.addEventListener("dblclick", () => choose(undefined, true));
  handle.addEventListener("keydown", event => {
    const current = panel.getBoundingClientRect().width;
    if (event.key === "ArrowLeft") choose(current + 24, true);
    else if (event.key === "ArrowRight") choose(current - 24, true);
    else if (event.key === "Home") choose(undefined, true);
    else return;
    event.preventDefault();
  });
  return handle;
}
