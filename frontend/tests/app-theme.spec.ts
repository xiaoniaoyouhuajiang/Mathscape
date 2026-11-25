import { expect, test, type Page } from "@playwright/test";

function luminance(color: string): number {
  const channels = color.match(/[\d.]+/g)!.slice(0, 3).map(Number).map(value => {
    const srgb = value / 255;
    return srgb <= 0.04045 ? srgb / 12.92 : ((srgb + 0.055) / 1.055) ** 2.4;
  });
  return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
}

async function contrast(page: Page, foreground: string, background = foreground): Promise<number> {
  const colors = await page.evaluate(({ foreground, background }) => ({
    text: getComputedStyle(document.querySelector(foreground)!).color,
    surface: getComputedStyle(document.querySelector(background)!).backgroundColor,
  }), { foreground, background });
  const a = luminance(colors.text), b = luminance(colors.surface);
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

for (const theme of ["obsidian", "parchment", "slate"] as const) {
  test.describe(theme, () => {
    test.beforeEach(async ({ page }) => {
      await page.setViewportSize({ width: 1600, height: 1000 });
      await page.addInitScript(theme => localStorage.setItem("mathscape.theme", theme), theme);
    });

    test("map has legible chrome, crisp brand, quiet labels and keyboard focus", async ({ page }, testInfo) => {
      await page.addInitScript(() => {
        sessionStorage.setItem("harness.richMap", "1");
        sessionStorage.setItem("harness.themeStates", "1");
      });
      await page.goto("/tests/app.html");
      await expect(page.locator("#service-state")).toContainText("本地内核已连接");
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      const logo = page.locator("svg.brand-mark");
      await expect(logo).toBeVisible();
      await expect(logo).toHaveAttribute("aria-hidden", "true");
      expect((await logo.boundingBox())!.width).toBe(38);
      for (const [text, surface] of [["#import-pdf", "#import-pdf"], [".tagline", ".topbar"], [".field-label", ".panel"]]) {
        expect(await contrast(page, text, surface), text).toBeGreaterThanOrEqual(4.5);
      }
      await page.keyboard.press("Tab");
      await page.locator("#import-pdf").focus();
      const focus = await page.locator("#import-pdf").evaluate(button => {
        const style = getComputedStyle(button);
        return { visible: button.matches(":focus-visible"), width: style.outlineWidth, type: style.outlineStyle };
      });
      expect(focus.visible).toBe(true);
      expect(focus.type).toBe("solid");
      expect(parseFloat(focus.width)).toBeGreaterThanOrEqual(2);
      await page.locator("#import-pdf").evaluate(button => (button as HTMLButtonElement).blur());
      await page.getByRole("button", { name: "总览", exact: true }).click();
      await expect.poll(() => page.evaluate(() => Object.keys(window.appHarness.labels()).length)).toBeGreaterThan(5);
      await page.getByRole("button", { name: "整理布局", exact: true }).click();
      const paint = await page.evaluate(() => window.appHarness.graphPaint());
      expect(new Set(paint.nodes.map(node => node.shape)).size).toBeGreaterThan(2);
      expect(paint.nodes.every(node => Number(node.labelBackground) === 0)).toBe(true);
      const candidate = paint.nodes.find(node => node.id === "r-theme-candidate")!;
      const debt = paint.nodes.find(node => node.id === "r-theme-debt")!;
      expect(candidate.staged).toBe(true);
      expect(candidate.label).toContain("〔未审批〕");
      expect(candidate.shape).toBe("star");
      expect(candidate.border).toBe("dashed");
      expect(debt.shape).toBe("tag");
      expect(debt.border).toBe("dashed");
      expect(debt.borderColor).not.toBe(candidate.borderColor);
      await expect(page.locator("#graph-legend")).toContainText("候选／未审批");
      await page.screenshot({ path: testInfo.outputPath(`map-${theme}.png`) });
    });

    test("reading next to handwriting keeps light paper and readable formulae", async ({ page }, testInfo) => {
      await page.goto("/tests/app.html");
      await page.locator("#ink-button").click();
      await page.getByRole("button", { name: /手稿 A/ }).click();
      await page.locator("#ink-split-toggle").click();
      await page.locator("#ink-split-empty").getByRole("button", { name: "正确表述.md" }).click();
      const paper = page.locator("#ink-split-reader .markdown-body");
      await expect(paper).toBeVisible();
      await expect(paper.locator(".katex")).toHaveCount(1);
      await expect(paper.locator(".katex-error")).toHaveCount(0);
      await expect(paper).toHaveCSS("color-scheme", "light");
      expect(await contrast(page, "#ink-split-reader .markdown-body")).toBeGreaterThanOrEqual(7);
      expect(luminance(await paper.evaluate(element => getComputedStyle(element).backgroundColor))).toBeGreaterThan(0.8);
      const returnButton = (await page.locator("#ink-close").boundingBox())!;
      const title = (await page.locator("#ink-document-heading").boundingBox())!;
      expect(Math.abs(returnButton.y - title.y)).toBeLessThan(28);
      const heights = await page.locator("#ink-list-toggle, #ink-save-now, #desktop-ink-erase, #desktop-ink-pan").evaluateAll(buttons => buttons.map(button => button.getBoundingClientRect().height));
      expect(new Set(heights).size).toBe(1);
      const launcher = (await page.locator(".task-launcher").boundingBox())!;
      const exports = (await page.locator(".ink-export-actions").boundingBox())!;
      expect(launcher.y + launcher.height).toBeLessThanOrEqual(exports.y);
      await page.screenshot({ path: testInfo.outputPath(`reading-${theme}.png`) });
    });

    test("review keeps debts distinct from current-item gold and embeds quiet formulae", async ({ page }, testInfo) => {
      await page.addInitScript(() => sessionStorage.setItem("harness.proposals", "1"));
      await page.goto("/tests/app.html");
      await page.locator("#sandbox-button").click();
      page.once("dialog", dialog => void dialog.accept());
      await Promise.all([page.waitForEvent("load"), page.getByRole("button", { name: "在本窗口打开…" }).click()]);
      await page.locator(".topbar .sandbox-banner").getByRole("button", { name: "对话" }).click();
      await page.getByRole("complementary", { name: "沙箱对话" }).getByRole("button", { name: "审核变更" }).click();
      const review = page.locator(".review-workspace");
      await expect(review).toBeVisible();
      await expect(review.locator(".review-debt-body")).toContainText("缺少分页标记");
      await expect(review.locator(".review-fragment .katex").first()).toBeVisible();
      await expect(review.getByRole("button", { name: "接受本提案", exact: true })).toBeEnabled();
      expect(await contrast(page, ".review-meta", ".review-workspace")).toBeGreaterThanOrEqual(4.5);
      const roles = await page.evaluate(() => {
        const css = getComputedStyle(document.documentElement);
        return { warning: css.getPropertyValue("--ms-warning"), gold: css.getPropertyValue("--ms-star-gold") };
      });
      expect(roles.warning).not.toBe(roles.gold);
      await expect(review.locator(".review-fragment.markdown-body").first()).toHaveCSS("color-scheme", theme === "parchment" ? "light" : "dark");
      const fragment = review.locator(".review-fragment.markdown-body").first();
      const brightness = luminance(await fragment.evaluate(element => getComputedStyle(element).backgroundColor));
      expect(brightness < 0.08).toBe(theme !== "parchment");
      for (const selector of [".brand", ".tagline", "#service-state"]) {
        expect(await contrast(page, selector, ".topbar"), selector).toBeGreaterThanOrEqual(4.5);
      }
      const dialogueButton = page.locator(".sandbox-banner").getByRole("button", { name: "对话", exact: true });
      await expect(dialogueButton).toBeEnabled();
      const dialogueColors = await dialogueButton.evaluate(button => {
        const style = getComputedStyle(button);
        return { text: style.color, background: style.backgroundColor, opacity: style.opacity };
      });
      expect((Math.max(luminance(dialogueColors.text), luminance(dialogueColors.background)) + 0.05) / (Math.min(luminance(dialogueColors.text), luminance(dialogueColors.background)) + 0.05)).toBeGreaterThanOrEqual(4.5);
      expect(dialogueColors.opacity).toBe("1");
      // Focus the review: the conversation remains available via its banner.
      await page.getByRole("button", { name: "关闭对话", exact: true }).click();
      await page.screenshot({ path: testInfo.outputPath(`review-${theme}.png`) });
    });
  });
}

test("both theme controls stay in sync, persist, and repaint without moving the map", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto("/tests/app.html");
  await expect(page.locator("#service-state")).toContainText("本地内核已连接");
  const before = await page.evaluate(() => {
    const id = Object.keys(window.appHarness.labels())[0];
    window.appHarness.selectNode(id);
    window.appHarness.panTo(30, 40);
    return { paint: window.appHarness.graphPaint(), viewport: window.appHarness.viewport(), id };
  });
  await page.locator("#theme-toggle-button").click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "parchment");
  const after = await page.evaluate(() => ({ paint: window.appHarness.graphPaint(), viewport: window.appHarness.viewport() }));
  expect(after.viewport).toEqual(before.viewport);
  expect(after.paint.nodes.map(n => [n.id, n.shape, n.border, n.position])).toEqual(before.paint.nodes.map(n => [n.id, n.shape, n.border, n.position]));
  expect(after.paint.nodes[0].color).not.toBe(before.paint.nodes[0].color);
  await page.locator("#llm-settings-button").click();
  const themeGroup = page.getByRole("radiogroup", { name: "显示主题" });
  await expect(themeGroup.locator(".display-choice-card--theme")).toHaveCount(3);
  await expect(themeGroup.locator(".pi-mascot-svg--pinned")).toHaveCount(3);
  await expect(themeGroup.locator('input[name="theme"][value="parchment"]')).toBeChecked();
  await expect(themeGroup.locator('[data-theme-choice="parchment"]')).toHaveClass(/is-selected/);
  await themeGroup.locator('[data-theme-choice="slate"]').click();
  expect(await page.evaluate(() => localStorage.getItem("mathscape.theme"))).toBe("slate");
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "slate");
  await expect(page.locator("#theme-toggle-button")).toHaveAttribute("aria-label", /青岩静夜/);
  expect(errors).toEqual([]);
});

test("an invalid saved theme falls back without preventing startup", async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("mathscape.theme", "unknown"));
  await page.goto("/tests/app.html");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "obsidian");
  await expect(page.locator("#service-state")).toContainText("本地内核已连接");
});

test("unavailable theme persistence does not break switching", async ({ page }) => {
  await page.addInitScript(() => {
    const get = Storage.prototype.getItem, set = Storage.prototype.setItem;
    Storage.prototype.getItem = function(key) {
      if (key === "mathscape.theme") throw new DOMException("blocked", "SecurityError");
      return get.call(this, key);
    };
    Storage.prototype.setItem = function(key, value) {
      if (key === "mathscape.theme") throw new DOMException("blocked", "SecurityError");
      return set.call(this, key, value);
    };
  });
  await page.goto("/tests/app.html");
  await expect(page.locator("#service-state")).toContainText("本地内核已连接");
  await page.locator("#theme-toggle-button").click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "parchment");
});
