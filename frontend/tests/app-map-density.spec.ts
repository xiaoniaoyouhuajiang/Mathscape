import { expect, test } from "@playwright/test";

test("large maps declutter readable names without removing or moving objects", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.addInitScript(() => {
    const entity = (id: string, label: string) => ({ id, label, element_kind: "entity", domain_id: id, domain_kind: "SourceSemanticUnit", relation_kind: null, layer: "source", summary: null, lifecycle: "active", parent: null, boundary: false });
    const units = Array.from({ length: 70 }, (_, i) => entity(`scale-${i}`, `示例内容单元 ${i}`));
    // Synthetic public regression data, not copied or derived from a workspace.
    (window as unknown as { __CUSTOM_BUNDLE__: unknown }).__CUSTOM_BUNDLE__ = { snapshot: {
      workspace_id: "synthetic-scale", workspace_root: "/synthetic-scale", artifact_count: 0, conversion_count: 0,
      marker: { configured: false, ready: false, executable: null, version: null, problem: null },
      lean: { root: null, source: null, root_exists: false, profiles: [], problem: null }, pdf_artifacts: [],
      landscape_graph: { schema_version: "mathscape.graph-projection.v1", lens: "source", nodes: units, edges: [] },
    } };
  });
  await page.goto("/tests/app.html");
  await expect(page.locator("#service-state")).toContainText("本地内核已连接");
  await expect.poll(() => page.evaluate(() => window.appHarness.graphPaint().nodes.length)).toBe(70);
  const positions = await page.evaluate(() => window.appHarness.graphPaint().nodes.map(node => ({ id: node.id, position: node.position })));
  await page.evaluate(() => window.appHarness.zoomTo(0.25));
  await expect.poll(() => page.evaluate(() => window.appHarness.graphPaint().nodes.filter(node => Number(node.labelOpacity) === 0).length)).toBeGreaterThan(0);
  const paint = await page.evaluate(() => window.appHarness.graphPaint());
  expect(paint.nodes.some(node => Number(node.labelOpacity) > 0)).toBe(true);
  expect(paint.nodes.every(node => node.renderedFontSize >= 10)).toBe(true);
  expect(paint.nodes.map(node => ({ id: node.id, position: node.position }))).toEqual(positions);
  const hidden = paint.nodes.find(node => Number(node.labelOpacity) === 0)!;
  await page.evaluate(id => window.appHarness.selectNode(id), hidden.id);
  await expect.poll(() => page.evaluate(id => Number(window.appHarness.graphPaint().nodes.find(node => node.id === id)!.labelOpacity), hidden.id)).toBe(1);
  await page.evaluate(() => window.appHarness.zoomTo(2));
  await expect.poll(() => page.evaluate(() => window.appHarness.graphPaint().nodes[0].renderedFontSize)).toBe(22);
  expect(await page.evaluate(() => window.appHarness.graphPaint().nodes.length)).toBe(70);
});
