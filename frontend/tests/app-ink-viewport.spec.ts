import { expect, test, type Page } from "@playwright/test";
import { isFittedCentered, settledGeometry } from "./support/ink-geometry";

// N3c layout: collapsible notes list (per-mode, persisted) and a viewport that
// fits on entering 并排 and keeps the viewed centre on later size changes.
// Assertions use real geometry (getBoundingClientRect + the canvas transform).

const toggle = (page: Page) => page.locator("#ink-list-toggle");
const panel = (page: Page) => page.locator("#ink-session-panel");

async function openNoteA(page: Page): Promise<void> {
  await page.locator("#ink-button").click();
  await page.getByRole("button", { name: /手稿 A/ }).click();
  await expect(page.locator("#ink-document-heading")).toContainText("手稿 A");
}

async function enterSplit(page: Page): Promise<void> {
  await page.locator("#ink-split-toggle").click();
  await expect(page.locator("#ink-workspace")).toHaveClass(/is-split/);
}

const settled = settledGeometry;

function expectSameCenter(a: { x: number; y: number }, b: { x: number; y: number }): void {
  expect(Math.abs(a.x - b.x)).toBeLessThan(1);
  expect(Math.abs(a.y - b.y)).toBeLessThan(1);
}

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 860 });
  await page.goto("/tests/app.html");
  await expect(page.locator("#ink-button")).toBeVisible();
});

test("notes list: expanded in normal, collapsed in split, remembered per mode; 新建 works collapsed", async ({ page }) => {
  await openNoteA(page);
  await expect(panel(page)).toBeVisible();
  await expect(toggle(page)).toHaveAttribute("aria-expanded", "true");
  await expect(page.locator("#ink-new-document-compact")).toBeHidden();

  await enterSplit(page);
  await expect(panel(page)).toBeHidden();
  await expect(toggle(page)).toHaveAttribute("aria-expanded", "false");
  // The canvas takes the freed width: it starts right after the split divider.
  const resizer = (await page.locator("#ink-split-resizer").boundingBox())!;
  const board = await settled(page);
  expect(Math.abs(board.board.left - (resizer.x + resizer.width))).toBeLessThan(8);

  // 新建 from the header while the list is collapsed.
  const before = await page.locator("#ink-session-list .ink-session-item").count();
  await page.locator("#ink-new-document-compact").click();
  await expect(page.locator("#ink-session-list .ink-session-item")).toHaveCount(before + 1);
  await expect(page.locator("#ink-document-heading")).not.toContainText("手稿 A");
  await expect(panel(page)).toBeHidden();

  // Expand in split → remembered for split only.
  await toggle(page).click();
  await expect(panel(page)).toBeVisible();
  await expect(toggle(page)).toHaveAttribute("aria-expanded", "true");
  const expandedBoard = await settled(page);
  expect(expandedBoard.board.width).toBeLessThan(board.board.width - 100);
  // Collapse in normal → remembered for normal only.
  await page.locator("#ink-split-toggle").click();
  await expect(page.locator("#ink-workspace")).not.toHaveClass(/is-split/);
  await expect(panel(page)).toBeVisible();
  await toggle(page).click();
  await expect(panel(page)).toBeHidden();
  await page.locator("#ink-split-toggle").click();
  await expect(panel(page)).toBeVisible();
  await page.locator("#ink-split-toggle").click();
  await expect(panel(page)).toBeHidden();
  expect(await page.evaluate(() => [localStorage.getItem("mathscape:ink-list-expanded"), localStorage.getItem("mathscape:ink-list-expanded-split")])).toEqual(["0", "1"]);

  await page.reload();
  await page.locator("#ink-button").click();
  await expect(toggle(page)).toHaveAttribute("aria-expanded", "false");
  await expect(panel(page)).toBeHidden();
});

test("entering split fits the paper inside the visible canvas (it used to keep the old offset)", async ({ page }) => {
  await openNoteA(page);
  const normal = await settled(page);
  expect(isFittedCentered(normal)).toBe(true);
  await enterSplit(page);
  const split = await settled(page);
  expect(split.board.width).toBeLessThan(normal.board.width - 100);
  expect(isFittedCentered(split), JSON.stringify(split)).toBe(true);

  // Even after zooming and panning in the normal layout, entering split fits again.
  await page.locator("#ink-split-toggle").click();
  await page.getByRole("button", { name: "放大" }).click();
  await page.getByRole("button", { name: "放大" }).click();
  const box = (await page.locator("#desktop-ink-board .ink-input").boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.wheel(-200, 150);
  expect(isFittedCentered(await settled(page))).toBe(false);
  await enterSplit(page);
  expect(isFittedCentered(await settled(page))).toBe(true);
});

test("list toggle, divider drag and window resize keep the viewed centre; 整页 resets", async ({ page }) => {
  await openNoteA(page);
  await enterSplit(page);
  await settled(page);
  await page.getByRole("button", { name: "放大" }).click();
  await page.getByRole("button", { name: "放大" }).click();
  const box = (await page.locator("#desktop-ink-board .ink-input").boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.wheel(120, 260);
  const start = await settled(page);
  expect(isFittedCentered(start)).toBe(false);

  await toggle(page).click(); // expand: the canvas narrows
  await expect(panel(page)).toBeVisible();
  const expanded = await settled(page);
  expect(expanded.board.width).toBeLessThan(start.board.width - 100);
  expect(expanded.scale).toBeCloseTo(start.scale, 6);
  expectSameCenter(expanded.centerWorld, start.centerWorld);

  await toggle(page).click(); // collapse again
  await expect(panel(page)).toBeHidden();
  const collapsed = await settled(page);
  expectSameCenter(collapsed.centerWorld, start.centerWorld);

  const handle = page.getByRole("separator", { name: "调整并排阅读宽度" });
  const h = (await handle.boundingBox())!;
  await page.mouse.move(h.x + h.width / 2, h.y + 120);
  await page.mouse.down();
  await page.mouse.move(h.x + 140, h.y + 120, { steps: 6 });
  await page.mouse.up();
  const dragged = await settled(page);
  expect(dragged.board.width).toBeLessThan(collapsed.board.width - 100);
  expectSameCenter(dragged.centerWorld, start.centerWorld);

  await page.setViewportSize({ width: 1200, height: 800 });
  const resized = await settled(page);
  expectSameCenter(resized.centerWorld, start.centerWorld);
  expect(resized.scale).toBeCloseTo(start.scale, 6);

  await page.getByRole("button", { name: "整页" }).click();
  expect(isFittedCentered(await settled(page))).toBe(true);
});

test("a fitted page stays fitted on exiting split, switching notes and narrowing", async ({ page }) => {
  await openNoteA(page);
  await enterSplit(page);
  expect(isFittedCentered(await settled(page))).toBe(true);
  await page.evaluate((docB) => {
    window.dispatchEvent(new CustomEvent("mathscape:open-ink-document", { detail: { documentId: docB } }));
  }, await page.evaluate(() => window.appHarness.documents.b));
  await expect(page.locator("#ink-document-heading")).toContainText("手稿 B");
  expect(isFittedCentered(await settled(page))).toBe(true);
  await toggle(page).click();
  expect(isFittedCentered(await settled(page))).toBe(true);
  await page.locator("#ink-split-toggle").click();
  await expect(page.locator("#ink-workspace")).not.toHaveClass(/is-split/);
  const normal = await settled(page);
  expect(isFittedCentered(normal)).toBe(true);
  // Returning to the map and back keeps a sane view too.
  await page.locator("#ink-close").click();
  await page.locator("#ink-button").click();
  expect(isFittedCentered(await settled(page))).toBe(true);
});
