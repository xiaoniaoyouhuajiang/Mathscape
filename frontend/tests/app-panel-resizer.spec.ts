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

test("N3.0 a wide inspector is re-clamped when the window shrinks and re-expands when it grows back", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 860 });
  await page.goto("/tests/app.html");
  const panel = page.locator(".detail-panel");
  const handle = page.getByRole("separator", { name: "调整检查器宽度" });
  await expect(handle).toBeVisible();
  const box = (await handle.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + 200);
  await page.mouse.down();
  await page.mouse.move(5, box.y + 200, { steps: 8 });
  await page.mouse.up();
  await expect.poll(async () => Math.round((await panel.boundingBox())!.width)).toBe(760);
  expect(await page.evaluate(() => localStorage.getItem("mathscape:detail-width"))).toBe("760");

  const layout = () => page.evaluate(() => {
    const detail = document.querySelector(".detail-panel")!.getBoundingClientRect();
    const leftColumn = document.querySelector(".detail-panel")!.parentElement!.firstElementChild!.getBoundingClientRect();
    return {
      panelRight: detail.right,
      panelWidth: detail.width,
      // The centre column sits between the left column and the inspector.
      centerWidth: detail.left - leftColumn.right,
      overflow: document.documentElement.scrollWidth - window.innerWidth,
    };
  });
  await page.setViewportSize({ width: 960, height: 860 });
  await expect.poll(async () => (await layout()).panelRight).toBeLessThanOrEqual(960.5);
  const narrow = await layout();
  expect(narrow.overflow).toBeLessThanOrEqual(0);
  expect(narrow.panelWidth).toBeLessThan(760);
  expect(narrow.centerWidth).toBeGreaterThanOrEqual(359.5);
  // The preference is kept, so the inspector widens again with the window.
  expect(await page.evaluate(() => localStorage.getItem("mathscape:detail-width"))).toBe("760");
  await page.setViewportSize({ width: 1600, height: 860 });
  await expect.poll(async () => Math.round((await panel.boundingBox())!.width)).toBe(760);
  expect((await layout()).panelRight).toBeLessThanOrEqual(1600.5);
});

test("N3.0 expanding the source sidebar re-clamps a wide inspector without a window resize", async ({ page }) => {
  await page.setViewportSize({ width: 1100, height: 860 });
  await page.goto("/tests/app.html");
  const panel = page.locator(".detail-panel");
  const toggle = page.locator("#toggle-sources");
  if (!(await page.locator(".workspace-grid").evaluate(grid => grid.classList.contains("sources-collapsed")))) await toggle.click();
  await expect(page.locator(".workspace-grid")).toHaveClass(/sources-collapsed/);
  const handle = page.getByRole("separator", { name: "调整检查器宽度" });
  const box = (await handle.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + 200);
  await page.mouse.down();
  await page.mouse.move(5, box.y + 200, { steps: 8 });
  await page.mouse.up();
  const wide = Math.round((await panel.boundingBox())!.width);
  expect(wide).toBeGreaterThan(600);

  await toggle.click();
  await expect(page.locator(".workspace-grid")).not.toHaveClass(/sources-collapsed/);
  await expect.poll(async () => (await panel.boundingBox())!.x + (await panel.boundingBox())!.width).toBeLessThanOrEqual(1100.5);
  const centre = await page.evaluate(() => {
    const detail = document.querySelector(".detail-panel")!.getBoundingClientRect();
    return detail.left - document.querySelector(".workspace-grid")!.firstElementChild!.getBoundingClientRect().right;
  });
  expect(centre).toBeGreaterThanOrEqual(359.5);
  await toggle.click();
  await expect.poll(async () => Math.round((await panel.boundingBox())!.width)).toBe(wide);
});
