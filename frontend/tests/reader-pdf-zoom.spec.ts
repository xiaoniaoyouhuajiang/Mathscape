import { expect, test, type Page } from "@playwright/test";

// N3.0: "阅读PDF时无法调整尺寸" — PDF zoom (fit width by default, presets,
// ±, Ctrl/Cmd +/-/0, Ctrl+wheel) re-rendered at the device pixel ratio,
// anchored scrolling and per-document persistence.
test.use({ deviceScaleFactor: 2, viewport: { width: 1100, height: 760 } });

const viewport = (page: Page) => page.locator(".pdf-viewport");
const activeCanvas = (page: Page) => page.locator(".pdf-viewport .pdf-page-slot.is-active-page canvas");
const scale = async (page: Page) => Number(await viewport(page).getAttribute("data-scale"));
const canvasState = (page: Page) => activeCanvas(page).evaluate((canvas: HTMLCanvasElement) => ({
  cssWidth: parseFloat(canvas.style.width), width: canvas.width, ratio: Number(canvas.dataset.ratio),
}));
const open = async (page: Page) => {
  await page.evaluate(async () => (window as any).harness.reader.open("pdf", "Zoom"));
  await expect(activeCanvas(page)).toBeVisible();
};
/**
 * A bitmap is crisp when it has one device pixel per CSS pixel at `ratio`.
 * The CSS width round-trips through a string and the bitmap is `ceil`ed, so
 * allow 1 device pixel of float rounding — far below the ~10% gap of a
 * stretched bitmap left over from the previous zoom step.
 */
const isCrisp = (state: { cssWidth: number; width: number; ratio: number }) => Math.abs(state.width - state.cssWidth * state.ratio) <= 1;
/** Waits for the settled (crisp) bitmap at the current scale. */
const crisp = async (page: Page) => {
  await expect.poll(async () => {
    const [state, value] = await Promise.all([canvasState(page), scale(page)]);
    return Math.abs(state.cssWidth - 300 * value) <= 1 && isCrisp(state);
  }).toBe(true);
};

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/reader.html");
  await page.evaluate(() => localStorage.clear());
});

test("PDF opens fitted to the width and renders at the device pixel ratio", async ({ page }) => {
  await open(page);
  await expect(page.getByLabel("缩放")).toHaveValue("fit-width");
  const width = await viewport(page).evaluate(element => element.clientWidth);
  expect(await scale(page)).toBeCloseTo((width - 2) / 300, 2);
  await crisp(page);
  const state = await canvasState(page);
  expect(state.ratio).toBe(2);
  expect(isCrisp(state)).toBe(true);
  await expect(page.getByLabel("缩放").locator("option:checked")).toHaveText(/^适合宽度（\d+%）$/);
});

test("zoom buttons, presets and Ctrl/Cmd shortcuts re-render crisply", async ({ page }) => {
  await open(page);
  await page.getByLabel("缩放").selectOption("s:1");
  await crisp(page);
  expect(await scale(page)).toBe(1);
  await page.getByRole("button", { name: "放大" }).click();
  await expect.poll(() => scale(page)).toBe(1.1);
  await crisp(page);
  await page.getByRole("button", { name: "缩小" }).click();
  await page.getByRole("button", { name: "缩小" }).click();
  await expect.poll(() => scale(page)).toBe(0.9);
  await crisp(page);

  await page.keyboard.press("Control+=");
  await expect.poll(() => scale(page)).toBe(1);
  await page.keyboard.press("Meta+=");
  await expect.poll(() => scale(page)).toBe(1.1);
  await page.keyboard.press("Control+-");
  await expect.poll(() => scale(page)).toBe(1);
  await page.keyboard.press("Control+0");
  await expect(page.getByLabel("缩放")).toHaveValue("fit-width");
  await crisp(page);

  await page.getByLabel("缩放").selectOption("fit-page");
  const box = await viewport(page).evaluate(element => ({ width: element.clientWidth, height: element.clientHeight }));
  await expect.poll(() => scale(page)).toBeCloseTo(Math.min((box.width - 2) / 300, Math.max(200, box.height) / 400), 2);
});

test("Ctrl+wheel zooms around the pointer and keeps that point in place", async ({ page }) => {
  await open(page);
  await page.getByLabel("缩放").selectOption("s:3");
  await crisp(page);
  await viewport(page).evaluate(element => { element.scrollTop = 200; });
  const rect = (await viewport(page).boundingBox())!;
  const before = await viewport(page).evaluate(element => ({ top: element.scrollTop, height: parseFloat(element.querySelector("canvas")!.style.height) }));
  await page.mouse.move(rect.x + rect.width / 2, rect.y + 100);
  await page.keyboard.down("Control");
  await page.mouse.wheel(0, -100);
  await page.keyboard.up("Control");
  await expect.poll(() => scale(page)).toBeGreaterThan(3);
  const after = await viewport(page).evaluate(element => ({ top: element.scrollTop, height: parseFloat(element.querySelector("canvas")!.style.height) }));
  // The content fraction under the pointer is unchanged.
  expect((after.top + 100) / after.height).toBeCloseTo((before.top + 100) / before.height, 2);
  await expect(page.getByLabel("缩放").locator("option:checked")).toHaveText(/^\d+%$/);
  await crisp(page);
  // A plain wheel still scrolls instead of zooming.
  const zoomed = await scale(page);
  await page.mouse.wheel(0, 120);
  await expect.poll(() => viewport(page).evaluate(element => element.scrollTop)).toBeGreaterThan(after.top);
  expect(await scale(page)).toBe(zoomed);
});

test("the zoom is remembered per document and the outline can be hidden", async ({ page }) => {
  await open(page);
  await page.getByLabel("缩放").selectOption("s:1.5");
  await crisp(page);
  const widthWithOutline = await viewport(page).evaluate(element => element.clientWidth);
  await page.getByRole("button", { name: "目录" }).click();
  await expect(page.locator(".pdf-outline")).toBeHidden();
  await expect.poll(() => viewport(page).evaluate(element => element.clientWidth)).toBeGreaterThan(widthWithOutline);
  await page.evaluate(() => (window as any).harness.reader.close());
  await open(page);
  await expect(page.getByLabel("缩放")).toHaveValue("s:1.5");
  expect(await scale(page)).toBe(1.5);
  await expect(page.locator(".pdf-outline")).toBeHidden();
  expect(await page.evaluate(() => localStorage.getItem("mathscape:pdf-zoom:pdf"))).toBe(JSON.stringify({ kind: "scale", value: 1.5 }));
  // Page navigation keeps the zoom.
  await page.getByRole("button", { name: "下一页" }).click();
  await expect(page.getByLabel("PDF 页码")).toHaveValue("2");
  expect(await scale(page)).toBe(1.5);
});

/** Colour at the bottom-left of the rendered bitmap: page 1 is red, page 2 blue. */
const renderedPage = (page: Page) => activeCanvas(page).evaluate((canvas: HTMLCanvasElement) => {
  const [r, , b, a] = canvas.getContext("2d")!.getImageData(4, canvas.height - 4, 1, 1).data;
  return a === 0 ? "blank" : r > 200 && b < 80 ? "page-1" : b > 200 && r < 80 ? "page-2" : `rgb(${r},${b})`;
});

test("zooming while the next page is still loading renders that page at the new zoom", async ({ page }) => {
  await open(page);
  await page.getByLabel("缩放").selectOption("s:1");
  await crisp(page);
  await expect.poll(() => renderedPage(page)).toBe("page-1");
  // Same task: the page request cannot resolve before the zoom lands.
  await page.evaluate(() => {
    const buttons = [...document.querySelectorAll<HTMLButtonElement>("button")];
    buttons.find(button => button.textContent === "下一页")!.click();
    buttons.find(button => button.getAttribute("aria-label") === "放大")!.click();
  });
  await expect(page.getByLabel("页码")).toHaveValue("2");
  await expect.poll(() => scale(page)).toBe(1.1);
  await crisp(page);
  await expect.poll(() => renderedPage(page)).toBe("page-2");
  // And back again, zooming out twice mid-navigation.
  await page.evaluate(() => {
    const buttons = [...document.querySelectorAll<HTMLButtonElement>("button")];
    buttons.find(button => button.textContent === "上一页")!.click();
    buttons.find(button => button.getAttribute("aria-label") === "缩小")!.click();
    buttons.find(button => button.getAttribute("aria-label") === "缩小")!.click();
  });
  await expect(page.getByLabel("页码")).toHaveValue("1");
  await expect.poll(() => scale(page)).toBe(0.9);
  await crisp(page);
  await expect.poll(() => renderedPage(page)).toBe("page-1");
});

test("continuous scroll default, single-page toggle, keyboard PageDown/PageUp, TextLayer selection, search, and backlink landing highlight", async ({ page }) => {
  await page.evaluate(async () => {
    await (window as any).harness.reader.open("pdf", "Continuous & TextLayer", {
      pdfPageIndex: 1,
      focusExcerpt: "Proof by Induction Step",
    });
  });

  // 1. Opens in continuous scroll mode at target page 2 with golden backlink pulse + TextLayer phrase highlight
  // The pulse is checked first: it lasts 2.6 s from the moment the landing page has rendered.
  const slot2Shell = page.locator(".pdf-page-slot[data-page-number='2'] .pdf-page-shell");
  await expect(slot2Shell).toHaveClass(/is-backlink-target/);
  await expect(page.getByRole("button", { name: "连续滚动" })).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByLabel("PDF 页码")).toHaveValue("2");
  await expect(slot2Shell.locator(".textLayer")).toContainText("Proof by Induction Step");
  await expect(slot2Shell.locator(".textLayer .pdf-backlink-text-hit").first()).toBeVisible();
  await expect(slot2Shell).not.toHaveClass(/is-backlink-target/);

  // 2. Keyboard PageUp / PageDown navigates cleanly
  await page.keyboard.press("PageUp");
  await expect(page.getByLabel("PDF 页码")).toHaveValue("1");
  await page.keyboard.press("PageDown");
  await expect(page.getByLabel("PDF 页码")).toHaveValue("2");

  // 3. Cross-page PDF find bar (`查找`) jumps to Page 1 match and highlights TextLayer span
  await page.getByRole("button", { name: "查找" }).click();
  await expect(page.locator(".pdf-find-bar")).toBeVisible();
  await page.getByLabel("查找 PDF 文字").fill("Generating Function");
  await expect(page.locator(".pdf-find-count")).toHaveText("1 / 1");
  await expect(page.getByLabel("PDF 页码")).toHaveValue("1");
  await expect(page.locator(".pdf-page-slot[data-page-number='1'] .textLayer .pdf-find-hit-active").first()).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".pdf-find-bar")).toBeHidden();

  // 4. Toggling to single-page mode hides inactive page slots and persists preference
  await page.getByRole("button", { name: "连续滚动" }).click();
  await expect(page.getByRole("button", { name: "单页视图" })).toHaveAttribute("aria-pressed", "false");
  await expect(page.locator(".pdf-page-slot[data-page-number='1']")).toBeVisible();
  await expect(page.locator(".pdf-page-slot[data-page-number='2']")).toBeHidden();
});
