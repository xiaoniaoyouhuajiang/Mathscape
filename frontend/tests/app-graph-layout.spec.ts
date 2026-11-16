import { expect, test } from "@playwright/test";

test.describe("Landscape map layout stability & auxiliary noise filtering", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => sessionStorage.setItem("harness.richMap", "1"));
  });

  test("hiding all kinds and expanding in different orders produces identical organic coordinates; auxiliary metadata shows only in 总览", async ({ page }) => {
    await page.setViewportSize({ width: 1400, height: 860 });
    await page.goto("/tests/app.html");
    await expect(page.locator("#graph")).toBeVisible();
    await expect.poll(() => page.evaluate(() => Object.keys(window.appHarness.labels()).length)).toBeGreaterThan(0);

    const kindToggle = (kind: string) => page.locator(`#node-filter-group input[data-node-kind="${kind}"]`);
    const snapshotPositions = () =>
      page.evaluate(() => {
        const labels = window.appHarness.labels();
        const out: Record<string, { x: number; y: number } | null> = {};
        for (const id of Object.keys(labels)) {
          out[id] = window.appHarness.nodePosition(id);
        }
        return out;
      });

    // Default "来源内容" preset hides auxiliary parse metadata (`r-meta-1`) while showing primary markdown (`r-md-1`)
    const initialLabels = await page.evaluate(() => window.appHarness.labels());
    expect(initialLabels["r-md-1"]).toBe("正确表述.md");
    expect(initialLabels["r-meta-1"]).toBeUndefined();
    // No synthetic spaghetti contracted edges in normal map view
    expect(await page.evaluate(() => window.appHarness.withClass("contracted-edge"))).toEqual([]);

    // Clicking "总览" reveals `r-meta-1` and `r-span-1` in place without disturbing primary nodes
    const mdBeforeOverview = await page.evaluate(() => window.appHarness.nodePosition("r-md-1"));
    await page.getByRole("button", { name: "总览" }).click();
    const overviewLabels = await page.evaluate(() => window.appHarness.labels());
    expect(overviewLabels["r-meta-1"]).toContain("解析元数据");
    expect(overviewLabels["r-span-1"]).toBe("来源范围");
    expect(await page.evaluate(() => window.appHarness.nodePosition("r-md-1"))).toEqual(mdBeforeOverview);

    const kinds = ["SourceArtifact", "SourceSemanticUnit", "SourceSpan", "SemanticEntity"];

    // Uncheck all 4 checkboxes -> empty map
    for (const k of kinds) {
      await kindToggle(k).uncheck();
    }
    await expect(page.locator("#empty-state")).not.toHaveClass(/is-hidden/);

    // Expand in order A: Material -> Unit -> Span -> Shared
    for (const k of ["SourceArtifact", "SourceSemanticUnit", "SourceSpan", "SemanticEntity"]) {
      await kindToggle(k).check();
    }
    const posOrderA = await snapshotPositions();

    // Uncheck all again, then expand in reverse order B: Shared -> Span -> Unit -> Material
    for (const k of kinds) {
      await kindToggle(k).uncheck();
    }
    for (const k of ["SemanticEntity", "SourceSpan", "SourceSemanticUnit", "SourceArtifact"]) {
      await kindToggle(k).check();
    }
    const posOrderB = await snapshotPositions();

    expect(posOrderB).toEqual(posOrderA);

    // Manual drag pins are respected until "整理布局" resets them
    const orig = posOrderA["r-sem-c"]!;
    await page.evaluate(() => window.appHarness.setNodePosition("r-sem-c", 777, -333));
    await page.getByRole("button", { name: "整理布局" }).click();
    const resetPos = await page.evaluate(() => window.appHarness.nodePosition("r-sem-c"));
    expect(resetPos).toEqual(orig);
  });
});
