import { expect, test } from "@playwright/test";
import type { Core } from "cytoscape";
import { readFileSync } from "node:fs";
import type { TutorialProjection } from "../src/tutorial/projection";

const twoBooks = JSON.parse(readFileSync(new URL("../../fixtures/emir/fibonacci-two-books/tutorial-projection.json", import.meta.url), "utf8")) as TutorialProjection;

test.describe("Landscape map layout stability & auxiliary noise filtering", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => sessionStorage.setItem("harness.richMap", "1"));
  });

  test("both Fibonacci study outlines and orbits stay inside a dock-narrowed canvas", async ({ page }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    await page.addInitScript(fixture => {
      sessionStorage.setItem("harness.sandbox", "1");
      fixture.graph.nodes.filter(node => node.domain_kind === "SourceArtifact").forEach(node => {
        node.label = node.label.replace(/\.md$/, " · 页码未记录 · 解析正文");
      });
      (window as unknown as { __CUSTOM_BUNDLE__: unknown }).__CUSTOM_BUNDLE__ = {
        snapshot: {
          workspace_id: "fibonacci-dock-regression", workspace_root: "/fibonacci-dock-regression", artifact_count: 2, conversion_count: 0,
          marker: { configured: false, ready: false, executable: null, version: null, problem: null },
          lean: { root: null, source: null, root_exists: false, profiles: [], problem: null }, pdf_artifacts: [],
          landscape_graph: fixture.graph,
        },
        learningView: fixture.learning,
      };
    }, twoBooks);
    await page.setViewportSize({ width: 1280, height: 820 });
    await page.goto("/tests/app.html");
    await page.waitForFunction(() => Boolean(window.appHarness));
    await expect(page.locator("#service-state")).toContainText("本地内核已连接");
    await expect.poll(() => page.evaluate(() => window.appHarness.graphPaint().nodes.length)).toBeGreaterThan(0);
    await page.locator(".topbar .sandbox-banner").getByRole("button", { name: "对话" }).click();
    await page.locator('.preset-button[data-preset="study"]').click();
    await expect(page.locator("#study-hud")).toBeVisible();
    for (const width of [1280, 1180]) {
      await page.setViewportSize({ width, height: width === 1280 ? 820 : 780 });
      for (const material of twoBooks.learning.materials) {
        await page.locator(".study-material-select").selectOption(material.artifact_id);
        await expect.poll(() => page.evaluate(() => {
          const cy = (document.querySelector("#graph") as unknown as { _cyreg: { cy: Core } })._cyreg.cy;
          const top = document.querySelector("#study-hud")!.getBoundingClientRect().bottom - document.querySelector("#graph")!.getBoundingClientRect().top;
          const labels = cy.nodes(".spine-header-node, .spine-scope-node, .spine-unit-node, .orbit-semantic-node, .orbit-outer-node").filter(node => !node.hasClass("is-out-of-focus")).map(node => {
            const box = node.renderedBoundingBox({ includeNodes: false, includeEdges: false, includeLabels: true, includeOverlays: false, includeUnderlays: false });
            return { label: node.data("label"), box, width: cy.width(), height: cy.height(), top };
          });
          const overlaps = labels.flatMap((a, i) => labels.slice(i + 1).filter(b => a.box.x1 < b.box.x2 && b.box.x1 < a.box.x2 && a.box.y1 < b.box.y2 && b.box.y1 < a.box.y2).map(b => [a.label, b.label]));
          return { overlaps, clipped: labels.filter(({ box, width, height, top }) => box.x1 < 0 || box.x2 > width || box.y1 < top || box.y2 > height) };
        })).toEqual({ overlaps: [], clipped: [] });
      }
    }
  });

  test("dock resize refits source nodes without moving them or restoring a stale study camera", async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 820 });
    await page.emulateMedia({ reducedMotion: "reduce" });
    await page.addInitScript(() => sessionStorage.setItem("harness.sandbox", "1"));
    await page.goto("/tests/app.html");
    await page.waitForFunction(() => Boolean(window.appHarness));
    await expect.poll(() => page.evaluate(() => window.appHarness.graphPaint().nodes.length)).toBeGreaterThan(0);
    const positions = await page.evaluate(() => window.appHarness.graphPaint().nodes.map(({ id, position }) => ({ id, position })));
    const width = (await page.locator("#graph").boundingBox())!.width;
    const dock = page.getByRole("complementary", { name: "沙箱对话" });
    const expectNodesInside = async () => {
      await expect.poll(() => page.evaluate(() => {
        const cy = (document.querySelector("#graph") as unknown as { _cyreg: { cy: Core } })._cyreg.cy;
        return cy.nodes().every(node => {
          const box = node.renderedBoundingBox({ includeLabels: false, includeOverlays: false });
          return box.x1 >= 0 && box.y1 >= 0 && box.x2 <= cy.width() && box.y2 <= cy.height();
        });
      })).toBe(true);
    };

    await page.evaluate(() => window.appHarness.zoomTo(2));
    await page.locator(".topbar .sandbox-banner").getByRole("button", { name: "对话" }).click();
    await expect(dock).toBeVisible();
    await expect.poll(async () => (await page.locator("#graph").boundingBox())!.width).toBeLessThan(width);
    await expectNodesInside();
    expect(await page.evaluate(() => window.appHarness.graphPaint().nodes.map(({ id, position }) => ({ id, position })))).toEqual(positions);

    // A resize notification alone must not discard a user's pan/zoom.
    await page.evaluate(() => { window.appHarness.zoomTo(2); window.appHarness.panTo(100, 50); });
    const camera = await page.evaluate(() => window.appHarness.viewport());
    await page.evaluate(() => window.dispatchEvent(new Event("resize")));
    await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
    expect(await page.evaluate(() => window.appHarness.viewport())).toEqual(camera);

    // Saving that camera in study mode must not revive it at a different canvas size.
    await page.locator('.preset-button[data-preset="study"]').click();
    await expect(page.locator("#study-hud")).toBeVisible();
    await page.locator(".study-material-select").selectOption("md-1");
    await dock.locator('button[title="收起为浮动入口"]').click();
    await expect.poll(async () => (await page.locator("#graph").boundingBox())!.width).toBe(width);
    await page.getByRole("button", { name: "退出研读" }).click();
    await expect(page.locator("#study-hud")).toBeHidden();
    await expectNodesInside();
    expect(await page.evaluate(() => window.appHarness.viewport())).not.toEqual(camera);
    expect(await page.evaluate(() => window.appHarness.graphPaint().nodes.map(({ id, position }) => ({ id, position })))).toEqual(positions);
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

    // Selection Spotlight: tapping a node highlights it + 1-hop neighbors and recedes unrelated nodes
    await page.evaluate(() => window.appHarness.selectNode("r-cite-unit"));
    expect(await page.evaluate(() => window.appHarness.withClass("is-selected-primary"))).toEqual(["r-cite-unit"]);
    const selectedNeighbors = await page.evaluate(() => window.appHarness.withClass("is-selected-neighbor"));
    expect(selectedNeighbors).toContain("r-sem-c");
    expect(selectedNeighbors).toContain("r-span-1");
    expect(await page.evaluate(() => window.appHarness.withClass("is-selection-receded")).then(ids => ids.length)).toBeGreaterThan(0);
    await expect(page.locator("#node-detail").getByRole("button", { name: "打开并定位原文" })).toBeVisible();
    await expect(page.locator("#node-detail").getByRole("button", { name: "打开 PDF 第 2 页" })).toBeVisible();

    // Focus Mode ("以此为中心聚焦"): dims out-of-focus nodes persistently and marks focus root
    await page.locator("#node-detail").getByRole("button", { name: "以此为中心聚焦" }).click();
    await expect(page.locator("#focus-bar")).toBeVisible();
    expect(await page.evaluate(() => window.appHarness.withClass("is-focus-root"))).toEqual(["r-cite-unit"]);
    const outOfFocus = await page.evaluate(() => window.appHarness.withClass("is-out-of-focus"));
    expect(outOfFocus).toContain("r-unit-thm");
    expect(outOfFocus).not.toContain("r-sem-c");

    await page.locator("#exit-focus").click();
    await expect(page.locator("#focus-bar")).toBeHidden();
    expect(await page.evaluate(() => window.appHarness.withClass("is-out-of-focus"))).toEqual([]);

    // In-Map Study Posture ("研读" / "学习视图"): morphs selected material into a linear indented spine + right semantic orbit, then restores organic positions on exit
    await page.locator('.preset-button[data-preset="study"]').click();
    await expect(page.locator("#study-hud")).toBeVisible();
    await page.locator(".study-material-select").selectOption("md-1");
    await expect.poll(async () => {
      const pThm = await page.evaluate(() => window.appHarness.nodePosition("r-unit-thm"));
      const pCite = await page.evaluate(() => window.appHarness.nodePosition("r-cite-unit"));
      const pEx = await page.evaluate(() => window.appHarness.nodePosition("r-unit-ex"));
      const pSem = await page.evaluate(() => window.appHarness.nodePosition("r-sem-c"));
      return Boolean(
        pThm && pCite && pEx && pSem &&
        pThm.y < pCite.y && pCite.y < pEx.y &&
        pThm.x < pCite.x &&
        pSem.x > pCite.x + 200
      );
    }).toBe(true);

    // Spotlight signal highlights aligned/diff units without removing unaligned spine context
    await page.locator('[data-study-signal="novel_and_diff"]').click();
    expect(await page.evaluate(() => window.appHarness.withClass("is-study-signal-hit"))).toContain("r-cite-unit");
    expect(await page.evaluate(() => window.appHarness.withClass("is-study-signal-muted"))).toContain("r-unit-thm");

    // Exiting Study Posture unfolds every node cleanly back to its deterministic organic coordinates
    await page.getByRole("button", { name: "退出研读" }).click();
    await expect(page.locator("#study-hud")).toBeHidden();
    await expect.poll(() => page.evaluate(() => window.appHarness.nodePosition("r-sem-c"))).toEqual(orig);
  });
});
