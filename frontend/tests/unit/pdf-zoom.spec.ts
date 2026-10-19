import { describe, expect, it } from "vitest";
import { PDF_MAX_CANVAS_PIXELS, PDF_ZOOM_DEFAULT, anchoredScroll, canvasRatio, clampZoom, loadZoom, parseZoomMode, resolveZoom, saveZoom, stepZoom, zoomLabel } from "../../src/reader/pdf-zoom";
import { clampDetailWidth, CENTER_MIN, DETAIL_WIDTH_MAX, DETAIL_WIDTH_MIN } from "../../src/shell/panel-resizer";

const memory = () => { const values = new Map<string, string>(); return { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => { values.set(key, value); }, values }; };

describe("PDF zoom", () => {
  it("fits width and page, and clamps fixed scales", () => {
    const page = { width: 300, height: 400 };
    expect(resolveZoom({ kind: "fit-width" }, page, { width: 600, height: 300 })).toBe(2);
    expect(resolveZoom({ kind: "fit-page" }, page, { width: 600, height: 400 })).toBe(1);
    expect(resolveZoom({ kind: "scale", value: 9 }, page, { width: 1, height: 1 })).toBe(5);
    expect(resolveZoom({ kind: "fit-width" }, page, { width: 10, height: 10 })).toBe(0.25);
    expect(clampZoom(Number.NaN)).toBe(1);
  });

  it("steps through presets in both directions from any scale", () => {
    expect(stepZoom(1, 1)).toBe(1.1);
    expect(stepZoom(1, -1)).toBe(0.9);
    expect(stepZoom(1.37, 1)).toBe(1.5);
    expect(stepZoom(1.37, -1)).toBe(1.25);
    expect(stepZoom(5, 1)).toBe(5);
    expect(stepZoom(0.25, -1)).toBe(0.25);
  });

  it("keeps the anchored content point in place", () => {
    // Point 150px into a 1000px page, anchor at viewport x=50 → scroll 100.
    expect(anchoredScroll(100, 50, 1000, 2000)).toBe(250);
    expect((anchoredScroll(100, 50, 1000, 2000) + 50) / 2000).toBeCloseTo(150 / 1000);
    expect(anchoredScroll(0, 0, 1000, 500)).toBe(0);
    expect(anchoredScroll(10, 10, 0, 500)).toBe(0);
  });

  it("uses the device pixel ratio but caps very large canvases", () => {
    expect(canvasRatio(2, 800, 1000)).toBe(2);
    expect(canvasRatio(0, 800, 1000)).toBe(1);
    const ratio = canvasRatio(3, 5000, 6000);
    expect(5000 * 6000 * ratio * ratio).toBeLessThanOrEqual(PDF_MAX_CANVAS_PIXELS + 1);
  });

  it("labels modes", () => {
    expect(zoomLabel({ kind: "fit-width" }, 1.234)).toBe("适合宽度（123%）");
    expect(zoomLabel({ kind: "scale", value: 1.5 }, 1.5)).toBe("150%");
  });

  it("persists the zoom per document and ignores invalid values", () => {
    const storage = memory();
    expect(loadZoom("a", storage)).toEqual(PDF_ZOOM_DEFAULT);
    saveZoom("a", { kind: "scale", value: 1.5 }, storage);
    saveZoom("b", { kind: "fit-page" }, storage);
    expect(loadZoom("a", storage)).toEqual({ kind: "scale", value: 1.5 });
    expect(loadZoom("b", storage)).toEqual({ kind: "fit-page" });
    storage.values.set("mathscape:pdf-zoom:c", "{bad");
    expect(loadZoom("c", storage)).toEqual(PDF_ZOOM_DEFAULT);
    expect(parseZoomMode({ kind: "scale", value: "2" })).toBeUndefined();
    expect(parseZoomMode({ kind: "scale", value: 50 })).toEqual({ kind: "scale", value: 5 });
  });
});

describe("inspector splitter clamp", () => {
  it("keeps the inspector within bounds and leaves the centre pane its minimum", () => {
    expect(clampDetailWidth(100, 1400, 260)).toBe(DETAIL_WIDTH_MIN);
    expect(clampDetailWidth(5000, 3000, 260)).toBe(DETAIL_WIDTH_MAX);
    expect(clampDetailWidth(700, 1200, 260)).toBe(1200 - 260 - CENTER_MIN);
    expect(clampDetailWidth(400.4, 1400, 260)).toBe(400);
    expect(clampDetailWidth(Number.NaN, 1400, 260)).toBe(DETAIL_WIDTH_MIN);
    expect(clampDetailWidth(300, 0, 0)).toBe(300);
  });
});
