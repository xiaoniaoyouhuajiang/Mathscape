/**
 * PDF reader zoom (N3.0). Pure helpers: the reader keeps a zoom *mode*
 * (fit width, fit page or a fixed scale), resolves it to a PDF.js scale for
 * the current page and viewport, and remembers it per document.
 */
export type PdfZoomMode = { kind: "fit-width" } | { kind: "fit-page" } | { kind: "scale"; value: number };

export const PDF_ZOOM_MIN = 0.25;
export const PDF_ZOOM_MAX = 5;
export const PDF_ZOOM_STEPS = [0.25, 0.33, 0.5, 0.67, 0.75, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2, 2.5, 3, 4, 5];
/** The default mode, also what Ctrl/Cmd+0 returns to. */
export const PDF_ZOOM_DEFAULT: PdfZoomMode = { kind: "fit-width" };
/** Never allocate canvases larger than this many device pixels. */
export const PDF_MAX_CANVAS_PIXELS = 32 * 1024 * 1024;

export function clampZoom(value: number): number {
  if (!Number.isFinite(value)) return 1;
  return Math.min(PDF_ZOOM_MAX, Math.max(PDF_ZOOM_MIN, value));
}

/** The PDF.js scale for `mode`; page sizes are at scale 1, in CSS pixels. */
export function resolveZoom(mode: PdfZoomMode, page: { width: number; height: number }, available: { width: number; height: number }): number {
  if (mode.kind === "scale") return clampZoom(mode.value);
  const width = Math.max(1, available.width) / Math.max(1, page.width);
  if (mode.kind === "fit-width") return clampZoom(width);
  return clampZoom(Math.min(width, Math.max(1, available.height) / Math.max(1, page.height)));
}

/** Next preset step above (direction 1) or below (-1) the current scale. */
export function stepZoom(current: number, direction: 1 | -1): number {
  const epsilon = 1e-3;
  if (direction > 0) return PDF_ZOOM_STEPS.find(step => step > current + epsilon) ?? PDF_ZOOM_MAX;
  return [...PDF_ZOOM_STEPS].reverse().find(step => step < current - epsilon) ?? PDF_ZOOM_MIN;
}

/** Device pixel ratio for the backing canvas, lowered for huge pages. */
export function canvasRatio(devicePixelRatio: number, cssWidth: number, cssHeight: number): number {
  const ratio = Number.isFinite(devicePixelRatio) && devicePixelRatio > 0 ? devicePixelRatio : 1;
  const area = Math.max(1, cssWidth) * Math.max(1, cssHeight);
  return Math.min(ratio, Math.sqrt(PDF_MAX_CANVAS_PIXELS / area));
}

/**
 * New scroll offset that keeps the content point under `anchor` (a viewport
 * coordinate) fixed while the content grows from `before` to `after` pixels.
 */
export function anchoredScroll(scroll: number, anchor: number, before: number, after: number): number {
  if (before <= 0) return 0;
  return Math.max(0, ((scroll + anchor) / before) * after - anchor);
}

export function zoomLabel(mode: PdfZoomMode, scale: number): string {
  const percent = Math.round(scale * 100) + "%";
  if (mode.kind === "fit-width") return `适合宽度（${percent}）`;
  if (mode.kind === "fit-page") return `适合页面（${percent}）`;
  return percent;
}

const KEY = "mathscape:pdf-zoom:";

export function parseZoomMode(value: unknown): PdfZoomMode | undefined {
  if (typeof value !== "object" || value === null) return undefined;
  const mode = value as Record<string, unknown>;
  if (mode.kind === "fit-width" || mode.kind === "fit-page") return { kind: mode.kind };
  if (mode.kind === "scale" && typeof mode.value === "number" && Number.isFinite(mode.value)) return { kind: "scale", value: clampZoom(mode.value) };
  return undefined;
}

/** Per-document zoom; unknown or unreadable values fall back to the default. */
export function loadZoom(documentId: string, storage: Pick<Storage, "getItem"> | undefined = globalThis.localStorage): PdfZoomMode {
  try {
    const raw = storage?.getItem(KEY + documentId);
    return (raw && parseZoomMode(JSON.parse(raw))) || PDF_ZOOM_DEFAULT;
  } catch {
    return PDF_ZOOM_DEFAULT;
  }
}

export function saveZoom(documentId: string, mode: PdfZoomMode, storage: Pick<Storage, "setItem"> | undefined = globalThis.localStorage): void {
  try { storage?.setItem(KEY + documentId, JSON.stringify(mode)); } catch { /* Optional preference. */ }
}
