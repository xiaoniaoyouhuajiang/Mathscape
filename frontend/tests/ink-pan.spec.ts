import { expect, test, type Page } from "@playwright/test";
import { settledGeometry as boardGeometry } from "./support/ink-geometry";

// N3c hand tool on the PC: dragging pans only the viewport, never the strokes.

const input = (page: Page) => page.locator("#desktop-ink-board .ink-input");
const opCount = (page: Page) => page.evaluate(() => [...window.inkDesktopHarness.storedDocuments.values()].reduce((n, d) => n + d.operations.length, 0));
const cursor = (page: Page) => input(page).evaluate(el => getComputedStyle(el).cursor);

async function drag(page: Page, dx: number, dy: number): Promise<void> {
  const box = (await input(page).boundingBox())!;
  const x = box.x + box.width / 2, y = box.y + box.height / 2;
  await page.mouse.move(x, y); await page.mouse.down(); await page.mouse.move(x + dx, y + dy, { steps: 6 }); await page.mouse.up();
}

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/ink-desktop.html");
  await expect(page.locator("#ink-session-list .ink-session-item")).toHaveCount(2);
  await expect(page.locator("#ink-mode-badge")).toHaveText("当前手稿");
});

test("N3c the 平移 tool drags the view without creating operations; 笔 writes again", async ({ page }) => {
  const pan = page.getByRole("button", { name: "平移", exact: true });
  await expect(pan).toHaveAttribute("aria-pressed", "false");
  const ops = await opCount(page);
  const before = await boardGeometry(page);
  await pan.click();
  await expect(pan).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator("#desktop-ink-pen")).not.toHaveClass(/is-active/);
  await expect.poll(() => cursor(page)).toBe("grab");
  await drag(page, 120, -70);
  const after = await boardGeometry(page);
  expect(after.offsetX - before.offsetX).toBeCloseTo(120, 0);
  expect(after.offsetY - before.offsetY).toBeCloseTo(-70, 0);
  expect(after.scale).toBeCloseTo(before.scale, 6);
  expect(await opCount(page)).toBe(ops);

  // Back to the pen: the same drag now writes one stroke and leaves the view alone.
  await page.getByRole("button", { name: "笔", exact: true }).click();
  await expect(pan).toHaveAttribute("aria-pressed", "false");
  await drag(page, 60, 0);
  await expect.poll(() => opCount(page)).toBe(ops + 1);
  const written = await boardGeometry(page);
  expect(written.offsetX).toBeCloseTo(after.offsetX, 3);
});

test("N3c holding Space pans temporarily and restores the previous tool", async ({ page }) => {
  await page.getByRole("button", { name: "套索选择", exact: true }).click();
  const ops = await opCount(page);
  const before = await boardGeometry(page);
  await page.locator("#desktop-ink-select").focus();
  await page.keyboard.down("Space");
  await expect(page.locator("#desktop-ink-board")).toHaveAttribute("data-pan", "space");
  await expect.poll(() => cursor(page)).toBe("grab");
  await drag(page, -90, 40);
  await page.keyboard.up("Space");
  const after = await boardGeometry(page);
  expect(after.offsetX - before.offsetX).toBeCloseTo(-90, 0);
  expect(after.offsetY - before.offsetY).toBeCloseTo(40, 0);
  expect(await opCount(page)).toBe(ops);
  await expect(page.locator("#desktop-ink-board")).toHaveAttribute("data-pan", "off");
  // The previous tool is back, and Space did not "click" the focused toolbar button.
  await expect(page.locator("#desktop-ink-select")).toHaveClass(/is-active/);
  await expect.poll(() => cursor(page)).not.toBe("grab");
});

test("N3c Space while typing the title is a normal space, not a pan", async ({ page }) => {
  const title = page.locator("#desktop-ink-title");
  await expect(title).toHaveJSProperty("readOnly", false);
  const before = await boardGeometry(page);
  await title.click();
  await title.press("End");
  await page.keyboard.type(" x");
  await expect(title).toHaveValue(/ x$/);
  await expect(page.locator("#desktop-ink-board")).toHaveAttribute("data-pan", "off");
  await page.keyboard.down("Space");
  await expect(page.locator("#desktop-ink-board")).toHaveAttribute("data-pan", "off");
  await page.keyboard.up("Space");
  await title.press("Escape");
  const after = await boardGeometry(page);
  expect(after.offsetX).toBeCloseTo(before.offsetX, 3);
});

test("N3c panning works on a read-only historical snapshot and the trash view", async ({ page }) => {
  await page.getByRole("button", { name: "保存快照" }).click(); await page.locator("#ink-dialog [data-choice=ok]").click();
  await page.locator("#ink-snapshot-list .ink-session-item").click();
  await expect(page.locator("#ink-mode-badge")).toHaveText("历史快照");
  const ops = await opCount(page);
  const pan = page.getByRole("button", { name: "平移", exact: true });
  await expect(pan).toBeEnabled();
  let before = await boardGeometry(page);
  await pan.click();
  await drag(page, 80, 50);
  let after = await boardGeometry(page);
  expect(after.offsetX - before.offsetX).toBeCloseTo(80, 0);
  expect(after.offsetY - before.offsetY).toBeCloseTo(50, 0);
  // Space also pans in read-only.
  await pan.click();
  before = after;
  await page.keyboard.down("Space");
  await drag(page, -30, 0);
  await page.keyboard.up("Space");
  after = await boardGeometry(page);
  expect(after.offsetX - before.offsetX).toBeCloseTo(-30, 0);
  expect(await opCount(page)).toBe(ops);

  // Trash view (read-only).
  await page.getByRole("button", { name: "继续编辑当前手稿" }).click();
  await page.locator(".ink-session-row", { hasText: "手稿 B" }).locator(".ink-session-action").click();
  await page.locator("#ink-dialog-actions button", { hasText: "移到回收站" }).click();
  await page.locator("#ink-tab-trash").click();
  await page.locator("#ink-session-list .ink-session-item", { hasText: "手稿 B" }).click();
  await expect(page.locator("#ink-mode-badge")).toHaveText("回收站");
  before = await boardGeometry(page);
  await page.keyboard.down("Space");
  await drag(page, 0, 64);
  await page.keyboard.up("Space");
  after = await boardGeometry(page);
  expect(after.offsetY - before.offsetY).toBeCloseTo(64, 0);
});

test("N3c a plain wheel / two-finger scroll pans; Ctrl+wheel zooms", async ({ page }) => {
  const box = (await input(page).boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  const before = await boardGeometry(page);
  await page.mouse.wheel(30, 100);
  await expect.poll(async () => (await boardGeometry(page)).offsetY).toBeCloseTo(before.offsetY - 100, 0);
  const panned = await boardGeometry(page);
  expect(panned.offsetX).toBeCloseTo(before.offsetX - 30, 0);
  expect(panned.scale).toBeCloseTo(before.scale, 6);
  await page.keyboard.down("Control");
  await page.mouse.wheel(0, -100);
  await page.keyboard.up("Control");
  await expect.poll(async () => (await boardGeometry(page)).scale).toBeGreaterThan(before.scale * 1.2);
  // 整页 is the one-click reset.
  await page.getByRole("button", { name: "整页" }).click();
  const reset = await boardGeometry(page);
  expect(reset.scale).toBeCloseTo(before.scale, 6);
  expect(reset.offsetX).toBeCloseTo(before.offsetX, 3);
});

test("N3c panning works in the read-only tablet mirror", async ({ page }) => {
  await page.getByRole("button", { name: "平板书写", exact: true }).click();
  await page.getByRole("button", { name: "生成配对二维码", exact: true }).click();
  await expect(page.locator("#ink-editability-badge")).toHaveText("电脑只读");
  const close = page.locator("#ink-lan-panel").getByRole("button", { name: /关闭|收起/ });
  if (await close.count()) await close.first().click();
  const ops = await opCount(page);
  const before = await boardGeometry(page);
  await page.getByRole("button", { name: "平移", exact: true }).click();
  await drag(page, 70, 30);
  const after = await boardGeometry(page);
  expect(after.offsetX - before.offsetX).toBeCloseTo(70, 0);
  expect(after.offsetY - before.offsetY).toBeCloseTo(30, 0);
  expect(await opCount(page)).toBe(ops);
});
