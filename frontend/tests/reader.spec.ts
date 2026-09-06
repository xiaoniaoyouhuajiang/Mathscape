import { test, expect } from "@playwright/test";

// The harness exercises actual reader/PDF.js/KaTeX code with a binary IPC double.
// Rust service tests cover the other side (Pool + catalog + persisted manifest).
test.beforeEach(async ({ page }) => {
  await page.goto("/tests/reader.html");
  await page.waitForFunction(() => "harness" in window);
});

test("PDF range transport, outline destination, page input and cleanup", async ({ page }) => {
  await page.evaluate(async () => (window as any).harness.reader.open("pdf", "Two pages"));
  await expect(page.locator("canvas")).toBeVisible();
  await expect(page.getByLabel("PDF 页码")).toHaveValue("1");
  await page.getByRole("button", { name: "Second page" }).click();
  await expect(page.getByLabel("PDF 页码")).toHaveValue("2");
  await expect.poll(() => page.locator("canvas").evaluate((canvas: HTMLCanvasElement) => {
    return canvas.getContext("2d")!.getImageData(50, 350, 1, 1).data[2];
  })).toBe(255);
  await page.getByLabel("PDF 页码").fill("1");
  await page.getByLabel("PDF 页码").press("Tab");
  await expect(page.getByRole("button", { name: "上一页" })).toBeDisabled();
  const reads = await page.evaluate(() => (window as any).harness.reads);
  expect(reads.some((r: { offset: number }) => r.offset > 0)).toBeTruthy();
  expect(reads.every((r: { length: number }) => r.length <= 4 * 1024 * 1024)).toBeTruthy();
  await page.evaluate(() => (window as any).harness.reader.close());
  await expect(page.locator("#reader")).toBeEmpty();
});

test("Markdown four math delimiters, local image mapping, raw toggle and unsafe content", async ({ page }) => {
  const external: string[] = [];
  page.on("request", request => { if (request.url().startsWith("https://example.com")) external.push(request.url()); });
  await page.evaluate(async () => (window as any).harness.reader.open("md", "Math notes"));
  await expect(page.locator(".katex")).toHaveCount(4);
  await expect(page.locator(".katex-error")).toHaveCount(0);
  await expect(page.locator(".katex-display")).toHaveCount(2);
  await expect(page.locator("article img")).toHaveCount(1);
  expect(await page.locator("article img").getAttribute("src")).toMatch(/^blob:/);
  await expect.poll(() => page.locator("article img").evaluate((img: HTMLImageElement) => img.naturalWidth)).toBe(1);
  await expect(page.locator("article script")).toHaveCount(0);
  await expect(page.locator("article code")).toHaveText("$code_not_math$\n");
  expect(external).toEqual([]);
  await page.getByRole("button", { name: "查看原文" }).click();
  await expect(page.locator(".katex")).toHaveCount(0);
  await page.getByRole("button", { name: "查看渲染" }).click();
  await expect(page.locator(".katex")).toHaveCount(4);
});

test("virtual path boundary, read errors, JSON and switching during a load", async ({ page }) => {
  const paths = await page.evaluate(() => {
    const resolve = (window as any).harness.resolveProductPath;
    return ["../plot.png", "../../outside.png", "/etc/passwd", "file:///etc/passwd", "%2e%2e/%2e%2e/out", "https://example.com/a", "images/a%20b.png"]
      .map(ref => resolve("book/book.md", ref));
  });
  expect(paths).toEqual(["plot.png", null, null, null, null, null, "book/images/a b.png"]);
  await page.evaluate(async () => {
    const { reader } = (window as any).harness;
    await Promise.all([reader.open("pdf", "Old"), reader.open("json", "Metadata")]);
  });
  await expect(page.locator("canvas")).toHaveCount(0);
  await expect(page.locator("pre")).toContainText('"page": 1');
  await page.evaluate(async () => (window as any).harness.reader.open("bad", "Broken"));
  await expect(page.locator("#reader")).toContainText("Invalid PDF");
  await page.evaluate(async () => (window as any).harness.reader.open("missing", "Missing"));
  await expect(page.locator("#reader")).toContainText("Artifact 不存在");
});
