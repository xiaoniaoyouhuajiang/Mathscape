import { expect, test } from "@playwright/test";

test.describe("N3c side-by-side reading", () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1400, height: 860 });
    await page.goto("/tests/app.html");
    await expect(page.locator("#ink-button")).toBeVisible();
  });

  test("enters and exits split, persists the divider, and re-clamps", async ({ page }) => {
    await page.locator("#ink-button").click();
    await expect(page.locator("#ink-workspace")).toBeVisible();
    const toggle = page.locator("#ink-split-toggle");
    await expect(toggle).toHaveText("并排阅读");
    await toggle.click();
    await expect(page.locator("#ink-workspace")).toHaveClass(/is-split/);
    await expect(page.locator("#ink-split-reader")).toBeVisible();
    const handle = page.getByRole("separator", { name: "调整并排阅读宽度" });
    await expect(handle).toBeVisible();

    const reader = page.locator("#ink-split-reader");
    const before = (await reader.boundingBox())!.width;
    const box = (await handle.boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + 120);
    await page.mouse.down();
    await page.mouse.move(box.x + 160, box.y + 120, { steps: 6 });
    await page.mouse.up();
    await expect.poll(async () => Math.round((await reader.boundingBox())!.width)).toBeGreaterThan(Math.round(before) + 40);
    const stored = Number(await page.evaluate(() => localStorage.getItem("mathscape:ink-split-reader-width")));
    expect(stored).toBeGreaterThan(before);

    await page.reload();
    await page.locator("#ink-button").click();
    await page.locator("#ink-split-toggle").click();
    await expect.poll(async () => Math.round((await page.locator("#ink-split-reader").boundingBox())!.width)).toBe(stored);

    await handle.dblclick();
    await expect.poll(async () => page.evaluate(() => localStorage.getItem("mathscape:ink-split-reader-width"))).toBeNull();

    await expect(page.locator("#ink-workspace")).toHaveClass(/is-split/);
    await page.setViewportSize({ width: 800, height: 860 });
    await expect(page.locator("#ink-workspace")).not.toHaveClass(/is-split/);
    await expect(page.locator("#ink-split-toggle")).toHaveText("并排阅读");

    await page.setViewportSize({ width: 1400, height: 860 });
    await page.locator("#ink-split-toggle").click();
    await expect(page.locator("#ink-workspace")).toHaveClass(/is-split/);
    await page.locator("#ink-split-toggle").click();
    await expect(page.locator("#ink-workspace")).not.toHaveClass(/is-split/);
  });

  test("entering split with no reader shows a picker; picking opens left and keeps the note", async ({ page }) => {
    // User path: map → 手写 → open a note → 并排阅读 (no reader was open).
    await page.locator("#ink-button").click();
    await page.getByRole("button", { name: /手稿 A/ }).click();
    await expect(page.locator("#ink-document-heading")).toContainText("手稿 A");
    await page.locator("#ink-split-toggle").click();
    await expect(page.locator("#ink-workspace")).toHaveClass(/is-split/);

    const empty = page.locator("#ink-split-empty");
    await expect(empty).toBeVisible();
    await expect(empty.getByRole("heading", { name: "并排阅读" })).toBeVisible();
    await expect(empty.getByRole("button", { name: "正确表述.md" })).toBeVisible();
    await expect(empty.getByRole("button", { name: "导入 PDF" })).toBeVisible();

    await empty.getByRole("button", { name: "正确表述.md" }).click();
    await expect(empty).toBeHidden({ timeout: 10000 });
    await expect(page.locator("#ink-split-reader #reader, #ink-split-reader .reader-panel").first()).toBeVisible();
    await expect(page.locator("#ink-split-reader")).toContainText("正确表述");
    // Note stays on the right.
    await expect(page.locator("#ink-document-heading")).toContainText("手稿 A");
    await expect(page.locator("#ink-workspace")).toHaveClass(/is-split/);
  });

  test("bidirectional navigation: open target left, ✎ opens note right", async ({ page }) => {
    const ids = await page.evaluate(() => {
      const h = (window as unknown as { appHarness: { documents: { a: string; b: string } } }).appHarness;
      return { a: h.documents.a, b: h.documents.b };
    });
    await page.locator("#ink-button").click();
    await page.getByRole("button", { name: /手稿 A/ }).click();
    await page.locator("#ink-split-toggle").click();
    await expect(page.locator("#ink-workspace")).toHaveClass(/is-split/);
    // Dismiss empty picker by opening a source so the left pane has a document.
    await page.locator("#ink-split-empty").getByRole("button", { name: "正确表述.md" }).click();
    await expect(page.locator("#ink-split-empty")).toBeHidden({ timeout: 10000 });

    await page.evaluate((docB) => {
      window.dispatchEvent(new CustomEvent("mathscape:open-ink-document", { detail: { documentId: docB } }));
    }, ids.b);
    await expect(page.locator("#ink-session-list .ink-session-item.is-active")).toContainText("手稿 B");
    await expect(page.locator("#ink-workspace")).toHaveClass(/is-split/);
    await expect(page.locator("#ink-split-reader")).toBeVisible();
    // Left still has the reader document.
    await expect(page.locator("#ink-split-empty")).toBeHidden();
  });

  test("canvas host keeps a usable width after divider drag", async ({ page }) => {
    await page.locator("#ink-button").click();
    await page.locator("#ink-split-toggle").click();
    const handle = page.getByRole("separator", { name: "调整并排阅读宽度" });
    const box = (await handle.boundingBox())!;
    await page.mouse.move(box.x + 2, box.y + 100);
    await page.mouse.down();
    await page.mouse.move(5, box.y + 100, { steps: 8 });
    await page.mouse.up();
    const layout = await page.evaluate(() => {
      const reader = document.querySelector("#ink-split-reader")!.getBoundingClientRect();
      const ink = document.querySelector("#ink-desktop-host")!.getBoundingClientRect();
      return { reader: reader.width, ink: ink.width };
    });
    expect(layout.ink).toBeGreaterThanOrEqual(359);
    expect(layout.reader).toBeGreaterThanOrEqual(279);
  });
});
