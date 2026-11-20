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

test("starry chrome keeps text legible, the shared logo local, and keyboard focus visible", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript(() => sessionStorage.setItem("harness.richMap", "1"));
  await page.goto("/tests/app.html");
  await expect(page.locator("#service-state")).toContainText("本地内核已连接");
  const logo = page.locator("img.brand-mark");
  await expect(logo).toHaveAttribute("alt", ""); // Adjacent Mathscape wordmark names the brand.
  expect(await logo.evaluate((image: HTMLImageElement) => image.complete && image.naturalWidth > 0)).toBe(true);
  for (const [text, surface] of [["#import-pdf", "#import-pdf"], [".tagline", ".topbar"], [".field-label", ".panel"]]) {
    expect(await contrast(page, text, surface), text).toBeGreaterThanOrEqual(4.5);
  }
  await page.keyboard.press("Tab");
  await page.locator("#import-pdf").focus();
  await expect(page.locator("#import-pdf")).toBeFocused();
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
  await page.screenshot({ path: testInfo.outputPath("map.png") });
});

test("reading next to handwriting keeps the light paper and readable formulae", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1440, height: 900 });
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
  await expect(page.locator("#ink-document-heading")).toContainText("手稿 A");
  const launcher = (await page.locator(".task-launcher").boundingBox())!;
  const exports = (await page.locator(".ink-export-actions").boundingBox())!;
  expect(launcher.y + launcher.height).toBeLessThanOrEqual(exports.y);
  await page.screenshot({ path: testInfo.outputPath("reading.png") });
});

test("proposal review shares the chrome without hiding debts or source formulae", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1440, height: 900 });
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
  await page.screenshot({ path: testInfo.outputPath("review.png") });
});
