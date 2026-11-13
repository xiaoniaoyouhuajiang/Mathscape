/**
 * 顶栏布局：学习视图 / 手写 / 沙箱是一组分段按钮，窄窗口收进「视图」下拉；
 * 设置、帮助是图标按钮；导入 PDF 在最右。任何宽度下按钮文字都不换行。
 */
import { expect, test, type Page } from "@playwright/test";

/** 顶栏里每个可见按钮的文字行数（按文字矩形的不同行顶部计）。 */
async function buttonLineCounts(page: Page) {
  return page.evaluate(() => [...document.querySelectorAll<HTMLButtonElement>(".topbar button")]
    .filter((button) => button.offsetParent !== null && button.getBoundingClientRect().width > 0)
    .map((button) => {
      // 每个文字节点各自的行数（图标和文字的基线不同，不能混在一起数）。
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
    return [...document.querySelectorAll<HTMLElement>(".topbar > *, .topbar-actions > *, .view-switch > *")]
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
      const counts = await buttonLineCounts(page);
      expect(counts.length).toBeGreaterThan(3);
      for (const item of counts) expect(item.lines, `${item.id} @${width}`).toBeLessThanOrEqual(1);
      expect(await topbarFitsOneRow(page)).toBe(true);
      // 导入 PDF 是最右边的按钮。
      const rights = await page.evaluate(() => [...document.querySelectorAll<HTMLElement>(".topbar-actions > *")]
        .filter((node) => node.offsetParent !== null).map((node) => ({ id: node.id, right: node.getBoundingClientRect().right })));
      expect(rights.at(-1)?.id).toBe("import-pdf");
    });
  }

  test("wide: segmented view switch; settings and help are icon buttons with tooltips", async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 860 });
    await page.goto("/tests/app.html");
    const group = page.getByRole("group", { name: "视图" });
    await expect(group).toBeVisible();
    await expect(group.getByRole("button")).toHaveText(["学习视图", "手写", "沙箱"]);
    await expect(page.locator("#view-menu-button")).toBeHidden();
    await expect(page.getByRole("button", { name: "设置" })).toHaveAttribute("title", "设置");
    await expect(page.getByRole("button", { name: "帮助" })).toHaveAttribute("title", "帮助");
    await expect(page.locator("#workspace-search-button svg")).toBeVisible();
    await expect(page.locator("#workspace-search-button")).toContainText("搜索");
  });

  test("narrow: the view switch collapses into a 视图 dropdown", async ({ page }) => {
    await page.setViewportSize({ width: 1024, height: 760 });
    await page.goto("/tests/app.html");
    const menu = page.locator("#view-menu-button");
    await expect(menu).toBeVisible();
    await expect(page.locator("#ink-button")).toBeHidden();
    await menu.click();
    await expect(menu).toHaveAttribute("aria-expanded", "true");
    await expect(page.locator("#ink-button")).toBeVisible();
    await expect(page.locator("#learning-view-button")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.locator("#ink-button")).toBeHidden();
    await menu.click();
    await page.locator("#ink-button").click();
    await expect(menu).toHaveAttribute("aria-expanded", "false");
    await expect(page.locator("#ink-button")).toBeHidden();
  });

  test("sandbox banner shows a short copy id, full text on hover", async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 860 });
    await page.goto("/tests/app.html");
    await page.locator("#sandbox-button").click();
    page.once("dialog", (dialog) => void dialog.accept());
    await Promise.all([page.waitForEvent("load"), page.getByRole("button", { name: "在本窗口打开…" }).click()]);
    const text = page.locator(".topbar .sandbox-banner-text");
    await expect(text).toContainText("sbx_demo");
    await expect(text).toHaveAttribute("title", /sbx_demo · 改动只写入隔离副本，检查点 .+ 不受影响/);
    for (const item of await buttonLineCounts(page)) expect(item.lines, item.id).toBeLessThanOrEqual(1);
    expect(await topbarFitsOneRow(page)).toBe(true);
  });
});
