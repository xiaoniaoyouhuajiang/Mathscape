/**
 * A2 dock layout (A-PR2): the dialogue either floats over the map
 * (`floating`, the S1 window) or docks as a right-edge sidebar (`sidebar`)
 * that shrinks the map pane instead of covering it. Pure helpers so the
 * clamping/persistence rules are unit-testable without a DOM.
 */

export type DockMode = "floating" | "sidebar";

/** Map-first default: dialogue beside the map, not over it. */
export const DEFAULT_DOCK_MODE: DockMode = "sidebar";
export const DEFAULT_SIDEBAR_WIDTH = 420;
export const MIN_SIDEBAR_WIDTH = 300;
export const MAX_SIDEBAR_WIDTH = 900;
/**
 * Compact workspace: source rail 48 + map/reader ≥512 + inspector 280.
 * The source directory becomes a temporary drawer instead of squeezing
 * the main content when a conversation is docked beside it.
 */
export const MIN_MAP_WIDTH = 840;
/** Below this viewport width a sidebar would starve the map; render floating. */
export const SIDEBAR_MIN_VIEWPORT = MIN_SIDEBAR_WIDTH + MIN_MAP_WIDTH;

export function parseDockMode(raw: string | null | undefined): DockMode {
  return raw === "floating" || raw === "sidebar" ? raw : DEFAULT_DOCK_MODE;
}

export function maxSidebarWidth(viewportWidth: number): number {
  return Math.max(MIN_SIDEBAR_WIDTH, Math.min(MAX_SIDEBAR_WIDTH, viewportWidth - MIN_MAP_WIDTH));
}

export function clampSidebarWidth(width: number, viewportWidth: number): number {
  const value = Number.isFinite(width) ? width : DEFAULT_SIDEBAR_WIDTH;
  return Math.round(Math.min(maxSidebarWidth(viewportWidth), Math.max(MIN_SIDEBAR_WIDTH, value)));
}

export function parseSidebarWidth(raw: string | null | undefined, viewportWidth: number): number {
  const parsed = raw == null || raw === "" ? NaN : Number(raw);
  return clampSidebarWidth(Number.isFinite(parsed) ? parsed : DEFAULT_SIDEBAR_WIDTH, viewportWidth);
}

/** The preference is kept as chosen; a too-narrow window only renders floating. */
export function effectiveDockMode(preferred: DockMode, viewportWidth: number): DockMode {
  return preferred === "sidebar" && viewportWidth < SIDEBAR_MIN_VIEWPORT ? "floating" : preferred;
}
