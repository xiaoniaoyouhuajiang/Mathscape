import { expect, test } from "@playwright/test";
import type { Core } from "cytoscape";

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

test("overview titles breathe and provenance lights up only for the active node", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.addInitScript(() => {
    const entity = (id: string, label: string, kind = "SourceSemanticUnit") => ({ id, label, element_kind: "entity", domain_id: id, domain_kind: kind, relation_kind: null, layer: "source", summary: kind === "SourceArtifact" ? "application/pdf · synthetic fixture" : null, lifecycle: "active", parent: null, boundary: false });
    const nodes = [
      ...Array.from({ length: 40 }, (_, i) => entity(`unit-${i}`, `示例内容单元 ${i}`)),
      entity("article-a", "A synthetic mathematical article with a deliberately long title.pdf", "SourceArtifact"),
      entity("article-b", "另一篇示例数学文章的完整长标题.pdf", "SourceArtifact"),
      entity("span-a", "来源范围", "SourceSpan"), entity("span-b", "来源范围", "SourceSpan"),
    ];
    (window as unknown as { __CUSTOM_BUNDLE__: unknown }).__CUSTOM_BUNDLE__ = { snapshot: {
      workspace_id: "synthetic-density", workspace_root: "/synthetic-density", artifact_count: 0, conversion_count: 0,
      marker: { configured: false, ready: false, executable: null, version: null, problem: null },
      lean: { root: null, source: null, root_exists: false, profiles: [], problem: null }, pdf_artifacts: [],
      landscape_graph: { schema_version: "mathscape.graph-projection.v1", lens: "source", nodes, edges: [] },
    } };
  });
  await page.goto("/tests/app.html");
  await expect(page.locator("#service-state")).toContainText("本地内核已连接");
  await page.getByRole("button", { name: "总览", exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.appHarness.graphPaint().nodes.length)).toBe(44);
  await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  await page.evaluate(() => {
    window.appHarness.zoomTo(0.5);
    const cy = (document.querySelector("#graph") as unknown as { _cyreg: { cy: Core } })._cyreg.cy;
    const e = cy.extent(), x = (e.x1 + e.x2) / 2, y = (e.y1 + e.y2) / 2;
    cy.nodes().forEach((n, i) => { n.position({ x: x - 2000 - i * 100, y }); });
    cy.$id("article-a").position({ x, y });
    cy.$id("article-b").position({ x: x + 20, y: y + 8 });
    cy.$id("span-a").position({ x: x + 180, y: y + 130 });
    cy.$id("span-b").position({ x: x + 300, y: y + 130 });
    // Public synthetic style fixtures, never copied from a workspace.
    cy.add(["a", "b"].map(id => ({ group: "edges" as const, classes: "evidence-edge semantic-edge is-undirected", data: { id: `ev-${id}`, source: `article-${id}`, target: `span-${id}`, label: "原文证据" } })));
  });
  const opacity = (id: string) => page.evaluate(id => {
    const cy = (document.querySelector("#graph") as unknown as { _cyreg: { cy: Core } })._cyreg.cy;
    const n = cy.$id(id);
    return Number(n.style(n.isNode() ? "text-opacity" : "opacity"));
  }, id);
  const hover = (active: boolean) => page.evaluate(active => {
    const cy = (document.querySelector("#graph") as unknown as { _cyreg: { cy: Core } })._cyreg.cy;
    cy.$id("article-a").emit(active ? "mouseover" : "mouseout");
  }, active);
  await expect.poll(() => opacity("span-a")).toBe(0);
  await expect.poll(() => page.evaluate(() => {
    const cy = (document.querySelector("#graph") as unknown as { _cyreg: { cy: Core } })._cyreg.cy;
    const a = cy.$id("article-a"), b = cy.$id("article-b");
    if (Number(b.style("text-opacity")) === 0) return true;
    const opts = { includeNodes: false, includeEdges: false, includeLabels: true, includeOverlays: false, includeUnderlays: false };
    const x = a.renderedBoundingBox(opts), y = b.renderedBoundingBox(opts);
    return !(x.x1 < y.x2 && y.x1 < x.x2 && x.y1 < y.y2 && y.y1 < x.y2);
  })).toBe(true);
  await expect.poll(() => opacity("article-a")).toBe(1);
  expect(await opacity("ev-a")).toBeLessThan(0.1);
  expect(await opacity("ev-b")).toBeLessThan(0.1);
  await hover(true);
  await expect.poll(() => opacity("ev-a")).toBe(1);
  expect(await opacity("ev-b")).toBeLessThan(0.1);
  await hover(false);
  await expect.poll(() => opacity("ev-a")).toBeLessThan(0.1);
  await page.evaluate(() => window.appHarness.selectNode("article-a"));
  await expect.poll(() => opacity("ev-a")).toBe(1);
  expect(await opacity("ev-b")).toBeLessThan(0.1);
  // Crossing an unrelated edge en route out of the canvas must not leave a
  // stale element-hover highlight on top of the selected node's spotlight.
  await page.evaluate(() => {
    const cy = (document.querySelector("#graph") as unknown as { _cyreg: { cy: Core } })._cyreg.cy;
    cy.$id("ev-b").emit("mouseover");
  });
  await expect.poll(() => opacity("ev-b")).toBe(1);
  await page.locator("#graph").dispatchEvent("mouseleave");
  await expect.poll(() => opacity("ev-b")).toBeLessThan(0.1);
  expect(await opacity("ev-a")).toBe(1);
  expect(await page.evaluate(() => window.appHarness.graphPaint().nodes.length)).toBe(44);
});
