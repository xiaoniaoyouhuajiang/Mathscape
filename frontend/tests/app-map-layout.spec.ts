import { expect, test } from "@playwright/test";

test.describe("deterministic landscape coordinates", () => {
  test("filtering nodes in different orders reuses the same base positions", async ({ page }) => {
    await page.setViewportSize({ width: 1400, height: 860 });
    await page.goto("/tests/app.html");
    await expect(page.locator("#sandbox-button")).toBeVisible();
    await expect.poll(() => page.evaluate(() => Boolean(window.appHarness && Object.keys(window.appHarness.labels()).length > 0))).toBe(true);

    const kinds = ["SourceArtifact", "SourceSemanticUnit", "SourceSpan", "SemanticEntity"];
    const ids = ["r-unit-thm", "r-unit-ex", "r-pdf-notes", "r-md-1"];
    const positions = () =>
      page.evaluate((nodeIds) =>
        Object.fromEntries(nodeIds.map((id) => [id, window.appHarness.nodePosition(id)])),
        ids,
      );
    const toggle = (kind: string) => page.locator(`#node-filter-group input[data-node-kind="${kind}"]`);
    const clear = async () => {
      for (const kind of kinds) await toggle(kind).uncheck();
      await expect(page.locator("#empty-state")).not.toHaveClass(/is-hidden/);
    };

    await clear();
    for (const kind of ["SourceArtifact", "SourceSemanticUnit", "SourceSpan", "SemanticEntity"]) {
      await toggle(kind).check();
    }
    const forward = await positions();

    await clear();
    for (const kind of ["SemanticEntity", "SourceSpan", "SourceSemanticUnit", "SourceArtifact"]) {
      await toggle(kind).check();
    }
    const reverse = await positions();

    expect(reverse).toEqual(forward);
  });
});
