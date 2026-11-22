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

// 「解析这几页」 adds snapshots, so the map is redrawn; that must not close the
// document the pages are arriving in.
test("pages parsed from a gap arrive in the open reader while the map refreshes", async ({ page }) => {
  await page.addInitScript(() => {
    sessionStorage.setItem("harness.sandbox", "1");
    sessionStorage.setItem("harness.canonical", "1");
    sessionStorage.setItem("harness.canonicalGap", "1");
  });
  await page.setViewportSize({ width: 1400, height: 860 });
  await page.goto("/tests/app.html");
  await expect(page.locator("#graph")).toBeVisible();
  await expect.poll(() => page.evaluate(() => Object.keys(window.appHarness.labels()).length)).toBeGreaterThan(0);
  await page.evaluate(() => window.appHarness.selectNode("r-cite-unit"));
  await page.locator("#node-detail").getByRole("button", { name: "打开并定位原文" }).click();

  const reader = page.locator("#reader");
  const located = reader.locator(".canonical-reader [data-source-block='b1']");
  await expect(located).toBeInViewport();
  await located.evaluate(block => { (block as unknown as { kept: boolean }).kept = true; });
  const loads = await page.evaluate(() => window.appHarness.canonicalGap.snapshotLoads);

  await reader.getByRole("button", { name: "解析第 5–6 页" }).click();
  await expect(reader.locator(".canonical-reader [data-source-block='nb1']")).toBeVisible();
  await expect(reader.locator(".canonical-gap")).toHaveCount(0);
  expect(await page.evaluate(() => window.appHarness.canonicalGap.requests)).toEqual([
    { canonical_id: "mathscape:canonical-markdown/v1/mathscape.marker.math-ocr.v1/marker-1/pdf-notes", first_page: 5, last_page: 6 },
  ]);
  // The map was read again, and the reader is still open on the same nodes.
  await expect.poll(() => page.evaluate(() => window.appHarness.canonicalGap.snapshotLoads)).toBeGreaterThan(loads);
  await expect(page.locator("#graph")).toBeVisible();
  await expect(reader.locator("h2").first()).toHaveText("notes.pdf · 规范解析正文");
  await expect(located).toBeInViewport();
  expect(await located.evaluate(block => (block as unknown as { kept?: boolean }).kept)).toBe(true);
});


// An agent's read of unparsed pages re-renders the map when the PDF's summary
// turns to 「解析中」 and back; the document being read stays open through both.
test("the map's 「解析中」 sync for an agent's read keeps the open reader", async ({ page }) => {
  await page.addInitScript(() => {
    sessionStorage.setItem("harness.sandbox", "1");
    sessionStorage.setItem("harness.canonical", "1");
  });
  await page.setViewportSize({ width: 1400, height: 860 });
  await page.goto("/tests/app.html");
  await expect(page.locator("#graph")).toBeVisible();
  await expect.poll(() => page.evaluate(() => Object.keys(window.appHarness.labels()).length)).toBeGreaterThan(0);
  await page.locator(".topbar .sandbox-banner").getByRole("button", { name: "对话" }).click();
  const dock = page.getByRole("complementary", { name: "沙箱对话" });
  await page.evaluate(() => { Object.assign(window.appHarness.sandboxRound, { act: "parsing", delayMs: 30, gapMs: 6500 }); });
  await page.evaluate(() => window.appHarness.selectNode("r-cite-unit"));
  await page.locator("#node-detail").getByRole("button", { name: "打开并定位原文" }).click();

  const reader = page.locator("#reader");
  const located = reader.locator(".canonical-reader [data-source-block='b1']");
  await expect(located).toBeInViewport();
  await located.evaluate(block => { (block as unknown as { kept: boolean }).kept = true; });

  await dock.locator(".sandbox-dock-input").fill("读第 3–4 页");
  await dock.getByRole("button", { name: "发送" }).click();
  const parsing = dock.locator(".sandbox-step-title").filter({ hasText: "解析中" });
  await expect(parsing).toHaveCount(1, { timeout: 6000 });
  await expect(located).toBeInViewport();
  // The read ends and the summary goes back: a second map render.
  await expect(parsing).toHaveCount(0, { timeout: 8000 });
  await expect(dock.locator(".sandbox-line.role-agent").last().locator(".sandbox-turn-status")).toHaveText("已完成", { timeout: 10000 });
  await expect(reader.locator("h2").first()).toHaveText("notes.pdf · 规范解析正文");
  await expect(located).toBeInViewport();
  expect(await located.evaluate(block => (block as unknown as { kept?: boolean }).kept)).toBe(true);
});
