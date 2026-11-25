/**
 * 顶栏布局：无居中「视图」下拉组；搜索、手写、沙箱及主题 / 设置 / 帮助图标按钮与
 * 「导入 PDF」统一归于右端，导入 PDF 在最右。任何宽度下按钮文字都不换行。
 */
import { expect, test, type Page } from "@playwright/test";

/** 顶栏里每个可见按钮的文字行数（按文字矩形的不同行顶部计）。 */
async function buttonLineCounts(page: Page) {
  return page.evaluate(() => [...document.querySelectorAll<HTMLButtonElement>(".topbar button")]
    .filter((button) => button.offsetParent !== null && button.getBoundingClientRect().width > 0)
    .map((button) => {
      const walker = document.createTreeWalker(button, NodeFilter.SHOW_TEXT);
      let lines = 0;
      for (let node = walker.nextNode(); node; node = walker.nextNode()) {
        if (!node.textContent?.trim()) continue;
        const range = document.createRange();
        range.selectNodeContents(node);
        const rects = [...range.getClientRects()].filter((r) => r.width > 0 && r.height > 0);
        const tops: number[] = [];
        for (const rect of rects) if (!tops.some((top) => Math.abs(top - rect.top) < rect.height / 2)) tops.push(rect.top);
        lines = Math.max(lines, tops.length);
      }
      return { id: button.id || button.textContent?.trim() || "", lines, height: button.getBoundingClientRect().height };
    }));
}

async function topbarFitsOneRow(page: Page) {
  return page.evaluate(() => {
    const bar = document.querySelector<HTMLElement>(".topbar")!.getBoundingClientRect();
    return [...document.querySelectorAll<HTMLElement>(".topbar > *, .topbar-actions > *")]
      .filter((node) => node.offsetParent !== null && getComputedStyle(node).position !== "absolute")
      .every((node) => {
        const rect = node.getBoundingClientRect();
        return rect.top >= bar.top - 1 && rect.bottom <= bar.bottom + 1 && rect.right <= bar.right + 1;
      });
  });
}

test.describe("top bar", () => {
  for (const width of [1440, 1280, 1024]) {
    test(`no button text wraps at ${width}px`, async ({ page }) => {
      await page.setViewportSize({ width, height: 860 });
      await page.goto("/tests/app.html");
      await expect(page.locator("#import-pdf")).toBeVisible();
      await expect(page.locator("#view-menu-button")).toHaveCount(0);
      await expect(page.locator(".topbar-center")).toHaveCount(0);
      await expect(page.locator(".topbar-actions #ink-button")).toBeVisible();
      await expect(page.locator(".topbar-actions #sandbox-button")).toBeVisible();
      const counts = await buttonLineCounts(page);
      expect(counts.length).toBeGreaterThan(3);
      for (const item of counts) expect(item.lines, `${item.id} @${width}`).toBeLessThanOrEqual(1);
      expect(await topbarFitsOneRow(page)).toBe(true);
      // 导入 PDF 是最右边的按钮，且搜索 / 手写 / 沙箱按序位于右端区域。
      const rights = await page.evaluate(() => [...document.querySelectorAll<HTMLElement>(".topbar-actions > *")]
        .filter((node) => node.offsetParent !== null).map((node) => ({ id: node.id, left: node.getBoundingClientRect().left, right: node.getBoundingClientRect().right })));
      expect(rights.at(-1)?.id).toBe("import-pdf");
      const searchLeft = rights.find((r) => r.id === "workspace-search-button")!.left;
      const inkLeft = rights.find((r) => r.id === "ink-button")!.left;
      const sandboxLeft = rights.find((r) => r.id === "sandbox-button")!.left;
      expect(searchLeft).toBeGreaterThan(width * 0.42);
      expect(inkLeft).toBeGreaterThan(searchLeft);
      expect(sandboxLeft).toBeGreaterThan(inkLeft);
    });
  }

  test("right-aligned controls: search, ink, sandbox, icon buttons, and import PDF without center view dropdown", async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 860 });
    await page.goto("/tests/app.html");
    await expect(page.locator("#view-menu-button")).toHaveCount(0);
    await expect(page.locator("#ink-button")).toHaveText("手写");
    await expect(page.locator("#sandbox-button")).toHaveText("沙箱");
    await expect(page.getByRole("button", { name: "设置" })).toHaveAttribute("title", "设置");
    await expect(page.getByRole("button", { name: "帮助" })).toHaveAttribute("title", "帮助");
    await expect(page.locator("#workspace-search-button svg")).toBeVisible();
    await expect(page.locator("#workspace-search-button")).toContainText("搜索");
  });

  test("sandbox banner hugs the left brand block while ink and sandbox stay right-aligned", async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 860 });
    await page.goto("/tests/app.html");
    await page.locator("#sandbox-button").click();
    page.once("dialog", (dialog) => void dialog.accept());
    await Promise.all([page.waitForEvent("load"), page.getByRole("button", { name: "在本窗口打开…" }).click()]);
    const text = page.locator(".topbar .sandbox-banner-text");
    await expect(text).toContainText("sbx_demo");
    await expect(text).toHaveAttribute("title", /sbx_demo · 改动只写入隔离副本，检查点 .+ 不受影响/);
    await expect(page.locator("#view-menu-button")).toHaveCount(0);
    await expect(page.locator(".topbar-actions #ink-button")).toBeVisible();
    await expect(page.locator(".topbar-actions #sandbox-button")).toBeVisible();
    const gap = await page.evaluate(() => {
      const banner = document.querySelector<HTMLElement>(".topbar .sandbox-banner")!.getBoundingClientRect();
      const actions = document.querySelector<HTMLElement>(".topbar .topbar-actions")!.getBoundingClientRect();
      return actions.left - banner.right;
    });
    expect(gap).toBeGreaterThan(24);
    for (const item of await buttonLineCounts(page)) expect(item.lines, item.id).toBeLessThanOrEqual(1);
    expect(await topbarFitsOneRow(page)).toBe(true);
  });
});
