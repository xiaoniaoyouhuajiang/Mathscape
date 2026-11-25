import { expect, test, type Page } from "@playwright/test";
import type { Core } from "cytoscape";

async function syntheticMap(page: Page): Promise<void> {
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.addInitScript(() => {
    const node = (id: string, kind = "SourceSemanticUnit") => ({ id, label: `Synthetic mathematical content ${id}`, element_kind: "entity", domain_id: id, domain_kind: kind, relation_kind: null, layer: "source", summary: kind === "SourceArtifact" ? "application/pdf · synthetic fixture" : null, lifecycle: "active", parent: null, boundary: false });
    (window as unknown as { __CUSTOM_BUNDLE__: unknown }).__CUSTOM_BUNDLE__ = { snapshot: {
      workspace_id: "synthetic-styles", workspace_root: "/synthetic-styles", artifact_count: 1, conversion_count: 0,
      marker: { configured: false, ready: false, executable: null, version: null, problem: null },
      lean: { root: null, source: null, root_exists: false, profiles: [], problem: null }, pdf_artifacts: [],
      landscape_graph: { schema_version: "mathscape.graph-projection.v1", lens: "source", nodes: [
        ...Array.from({ length: 40 }, (_, i) => node(`unit-${i}`)), node("article", "SourceArtifact"), node("span", "SourceSpan"),
      ], edges: [] },
    } };
  });
  await page.goto("/tests/app.html");
  await expect(page.locator("#service-state")).toContainText("本地内核已连接");
  await page.getByRole("button", { name: "总览", exact: true }).click();
  await page.evaluate(() => {
    window.appHarness.zoomTo(0.4);
    const cy = (document.querySelector("#graph") as unknown as { _cyreg: { cy: Core } })._cyreg.cy;
    cy.add({ group: "edges", classes: "evidence-edge semantic-edge is-undirected", data: { id: "evidence", source: "article", target: "span", label: "原文证据" } });
  });
}

async function paint(page: Page) {
  return page.evaluate(() => {
    const cy = (document.querySelector("#graph") as unknown as { _cyreg: { cy: Core } })._cyreg.cy;
    return {
      width: cy.$id("unit-0").width(), pdfWidth: cy.$id("article").width(),
      halo: String(cy.$id("unit-0").style("background-image")), evidence: Number(cy.$id("evidence").style("opacity")),
      hidden: cy.nodes(".is-auto-label-hidden").length,
      selected: cy.$id("article").hasClass("is-selected-primary"),
      pose: { zoom: cy.zoom(), pan: cy.pan(), nodes: cy.nodes().map(n => ({ id: n.id(), p: { ...n.position() } })), edges: cy.edges().map(e => e.id()) },
    };
  });
}

const mapStyleRadio = (page: Page, value: string) => page.locator(`input[name="map-style"][value="${value}"]`);
const themeRadio = (page: Page, value: string) => page.locator(`input[name="theme"][value="${value}"]`);
const pickMapStyle = (page: Page, value: string) => page.locator(`[data-map-style-choice="${value}"]`).click();
const pickTheme = (page: Page, value: string) => page.locator(`[data-theme-choice="${value}"]`).click();

for (const theme of ["obsidian", "parchment", "slate"]) {
  test(`${theme}: original and constellation switch in place, persist, and stay independent of theme`, async ({ page }) => {
    await page.addInitScript(theme => {
      if (localStorage.getItem("mathscape.theme") === null) localStorage.setItem("mathscape.theme", theme);
    }, theme);
    await syntheticMap(page);
    await expect.poll(async () => (await paint(page)).hidden).toBeGreaterThan(0);
    const initial = await paint(page);
    expect(initial.width).toBe(13);
    expect(initial.pdfWidth).toBe(22);
    expect(initial.evidence).toBe(0.07);
    expect(initial.halo).toContain("data:image/svg");
    await page.locator("#llm-settings-button").click();
    await expect(mapStyleRadio(page, "constellation")).toBeChecked();
    await pickMapStyle(page, "classic");
    await expect.poll(async () => (await paint(page)).hidden).toBe(0);
    const classic = await paint(page);
    expect(classic.width).toBe(22);
    expect(classic.pdfWidth).toBe(31);
    expect(classic.evidence).toBe(1);
    expect(classic.halo).not.toContain("data:image/svg");
    expect(classic.pose).toEqual(initial.pose);
    await expect(page.locator(".map-label-leaders path")).toHaveAttribute("d", "");
    const nextTheme = theme === "obsidian" ? "parchment" : "obsidian";
    await pickTheme(page, nextTheme);
    await expect(mapStyleRadio(page, "classic")).toBeChecked();
    expect((await paint(page)).width).toBe(22);
    expect((await paint(page)).pose).toEqual(initial.pose);
    await page.getByRole("button", { name: "关闭", exact: true }).click();
    await page.evaluate(() => window.appHarness.selectNode("article"));
    const selectedPose = (await paint(page)).pose;
    await page.locator("#llm-settings-button").click();
    await pickMapStyle(page, "constellation");
    await expect.poll(async () => (await paint(page)).hidden).toBeGreaterThan(0);
    const restored = await paint(page);
    expect(restored.selected).toBe(true);
    expect(restored.pose).toEqual(selectedPose);
    expect(restored.width).toBe(13);
    expect(restored.evidence).toBe(1);
    await pickMapStyle(page, "classic");
    await page.reload();
    await expect(page.locator("#service-state")).toContainText("本地内核已连接");
    await expect(mapStyleRadio(page, "classic")).toBeChecked();
    await expect(themeRadio(page, nextTheme)).toBeChecked();
    const other = await page.context().newPage();
    await other.goto("/tests/app.html");
    await expect(mapStyleRadio(other, "classic")).toBeChecked();
    await page.locator("#llm-settings-button").click();
    await pickMapStyle(page, "constellation");
    await expect(mapStyleRadio(other, "constellation")).toBeChecked();
    await other.close();
  });
}

test("invalid saved style falls back, and unavailable storage does not prevent switching", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("mathscape.mapStyle", "unknown-style");
    Storage.prototype.setItem = () => { throw new Error("unavailable"); };
  });
  await syntheticMap(page);
  await page.locator("#llm-settings-button").click();
  await expect(mapStyleRadio(page, "constellation")).toBeChecked();
  await pickMapStyle(page, "classic");
  await expect.poll(async () => (await paint(page)).width).toBe(22);
  await pickMapStyle(page, "constellation");
  await expect.poll(async () => (await paint(page)).width).toBe(13);
});
