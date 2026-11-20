/** Local display preference, independent of colour theme and workspace data. */
export const MAP_STYLES = { constellation: "星座", classic: "原版" } as const;
export type MapStyle = keyof typeof MAP_STYLES;
export const MAP_STYLE_STORAGE_KEY = "mathscape.mapStyle";
export const MAP_STYLE_CHANGED = "mathscape:mapstylechange";

export function isMapStyle(value: unknown): value is MapStyle {
  return typeof value === "string" && Object.hasOwn(MAP_STYLES, value);
}

export function getMapStyle(): MapStyle {
  const value = document.documentElement.dataset.mapStyle;
  return isMapStyle(value) ? value : "constellation";
}

function apply(style: MapStyle): void {
  document.documentElement.dataset.mapStyle = style;
  window.dispatchEvent(new CustomEvent(MAP_STYLE_CHANGED, { detail: { style } }));
}

export function setMapStyle(style: MapStyle): void {
  try { localStorage.setItem(MAP_STYLE_STORAGE_KEY, style); } catch { /* Still usable for this window. */ }
  apply(style);
}

export function initializeMapStyle(): void {
  let saved: string | null = null;
  try { saved = localStorage.getItem(MAP_STYLE_STORAGE_KEY); } catch { /* Use the default. */ }
  apply(isMapStyle(saved) ? saved : "constellation");
  window.addEventListener("storage", event => {
    if (event.key !== MAP_STYLE_STORAGE_KEY && event.key !== null) return;
    apply(isMapStyle(event.newValue) ? event.newValue : "constellation");
  });
}
