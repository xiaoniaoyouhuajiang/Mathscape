import { expect, test } from "@playwright/test";

// N3.0: on the real app shell the inspector column can be dragged wider or
// narrower (widening or narrowing the centre map / reader pane), the width
// is remembered, and double-click restores the default.
test("N3.0 the inspector splitter resizes the centre pane, persists and resets", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 860 });
  await page.goto("/tests/app.html");
  const panel = page.locator(".detail-panel");
  const handle = page.getByRole("separator", { name: "调整检查器宽度" });
  await expect(handle).toBeVisible();
  const before = (await panel.boundingBox())!.width;
  expect(Math.round(before)).toBe(280);

  const box = (await handle.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + 200);
  await page.mouse.down();
  await page.mouse.move(box.x - 200, box.y + 200, { steps: 6 });
  await page.mouse.up();
  await expect.poll(async () => Math.round((await panel.boundingBox())!.width)).toBeGreaterThan(460);
  const stored = Number(await page.evaluate(() => localStorage.getItem("mathscape:detail-width")));
  expect(stored).toBeGreaterThan(460);

  await page.reload();
  await expect.poll(async () => Math.round((await panel.boundingBox())!.width)).toBe(stored);

  // Keyboard: the separator is focusable and arrows step the width.
  await handle.focus();
  await page.keyboard.press("ArrowRight");
  await expect.poll(async () => Math.round((await panel.boundingBox())!.width)).toBe(stored - 24);

  // Dragging far left is clamped so the centre pane keeps its minimum.
  const again = (await handle.boundingBox())!;
  await page.mouse.move(again.x + again.width / 2, again.y + 200);
  await page.mouse.down();
  await page.mouse.move(5, again.y + 200, { steps: 6 });
  await page.mouse.up();
  await expect.poll(async () => Math.round((await panel.boundingBox())!.width)).toBeLessThanOrEqual(760);

  await handle.dblclick();
  await expect.poll(async () => Math.round((await panel.boundingBox())!.width)).toBe(280);
  expect(await page.evaluate(() => localStorage.getItem("mathscape:detail-width"))).toBeNull();
});
