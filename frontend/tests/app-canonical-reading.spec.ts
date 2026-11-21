import { expect, test } from "@playwright/test";

// Reading from the map lands in the stitched 「规范解析正文」; workflows that
// act on one parse run (discovery, review) keep opening that snapshot.
test.describe("source backtracking reads in the stitched view", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
      sessionStorage.setItem("harness.sandbox", "1");
      sessionStorage.setItem("harness.canonical", "1");
    });
    await page.setViewportSize({ width: 1400, height: 860 });
    await page.goto("/tests/app.html");
    await expect(page.locator("#graph")).toBeVisible();
    await expect.poll(() => page.evaluate(() => Object.keys(window.appHarness.labels()).length)).toBeGreaterThan(0);
  });

  test("a unit's 「打开并定位原文」 opens the stitched view at its block, earlier pages above it", async ({ page }) => {
    await page.evaluate(() => window.appHarness.selectNode("r-cite-unit"));
    await page.locator("#node-detail").getByRole("button", { name: "打开并定位原文" }).click();

    const reader = page.locator("#reader");
    await expect(reader.locator(".canonical-reader")).toBeVisible();
    await expect(reader.locator("h2").first()).toHaveText("notes.pdf · 规范解析正文");
    const located = reader.locator(".canonical-reader [data-source-block='b1']");
    await expect(located).toHaveClass(/source-block-focus/);
    await expect(located).toBeInViewport();
    // The earlier parse run is part of the same document, so scrolling up continues.
    await expect(reader.locator(".canonical-reader [data-source-block='lb-1']")).toHaveCount(1);
    expect(await reader.evaluate(host => host.scrollTop)).toBeGreaterThan(0);

    // One click back to the block after reading earlier material.
    await reader.evaluate(host => { host.scrollTop = 0; });
    const chip = reader.locator(".reader-return-point");
    await expect(chip).toBeInViewport();
    await expect(chip).toContainText("回到出发处 · 定理 3.2");
    await chip.locator(".reader-return-btn").click();
    await expect(located).toBeInViewport();
  });

  test("a discovery-style open keeps the snapshot, which offers the way into the stitched view", async ({ page }) => {
    await page.evaluate(() => {
      window.dispatchEvent(new CustomEvent("mathscape:open-source", { detail: { artifactId: "md-1", conversionId: "c1", title: "正确表述.md" } }));
    });
    const reader = page.locator("#reader");
    await expect(reader.locator("[data-source-block='b1']")).toBeVisible();
    await expect(reader.locator(".canonical-reader")).toHaveCount(0);
    await expect(reader.locator(".reader-boundary-card.is-end")).toBeVisible();

    await reader.getByRole("button", { name: "在规范解析正文中继续阅读" }).click();
    await expect(reader.locator(".canonical-reader [data-source-block='b1']")).toHaveClass(/source-block-focus/);
    await expect(reader.locator(".canonical-reader [data-source-block='lb-1']")).toHaveCount(1);
  });
});
