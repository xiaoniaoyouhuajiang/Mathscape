import type { Core } from "cytoscape";
import { THEME_CHANGED } from "../shell/theme-manager";
import { MAP_STYLE_CHANGED } from "../shell/map-style";

/** Cytoscape paints a canvas and cannot resolve CSS variables itself. */
export function graphPalette() {
  const css = getComputedStyle(document.documentElement);
  const read = (name: string) => css.getPropertyValue(`--ms-${name}`).trim();
  return {
    text: read("text"), muted: read("text-muted"), secondary: read("text-secondary"),
    canvas: read("map-label-halo"), panel: read("panel-raised"), accent: read("accent"),
    gold: read("star-gold"), warning: read("warning"), error: read("error"),
    success: read("success"), candidate: read("candidate"), core: read("map-node-core"),
    edge: read("map-edge-secondary"), primaryEdge: read("map-edge-primary"),
  };
}

export function resolveGraphColor(value: string): string {
  const variable = value.match(/^var\((--[\w-]+)\)$/)?.[1];
  return variable ? getComputedStyle(document.documentElement).getPropertyValue(variable).trim() : value;
}

/** Repaint in place: theme changes must not reset selection, zoom or layout. */
export function watchGraphTheme(core: Core, update: () => void): void {
  window.addEventListener(THEME_CHANGED, update);
  core.one("destroy", () => window.removeEventListener(THEME_CHANGED, update));
}

/** Map appearance is separate from the review graph's theme-only repaint. */
export function watchGraphAppearance(core: Core, update: () => void): void {
  watchGraphTheme(core, update);
  window.addEventListener(MAP_STYLE_CHANGED, update);
  core.one("destroy", () => window.removeEventListener(MAP_STYLE_CHANGED, update));
}
