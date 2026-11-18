import { expect, test, type Page } from "@playwright/test";

async function addSourcePrimary(page: Page, objectId = "unit-thm", label = "定理 3.2"): Promise<void> {
  await page.evaluate(({ objectId, label }) => {
    const h = window.appHarness;
    h.links.push({
      link_id: "split-primary", document_id: h.documents.a, target_kind: "source_object",
      target_key: "src:" + objectId, target_id: objectId, target: { kind: "source_object", object_id: objectId },
      target_revision: null, target_label: label, role: "study", is_primary: true, state: "active",
      created_at: 1, removed_at: null, target_status: "active",
    });
  }, { objectId, label });
}

async function openNoteInSplit(page: Page): Promise<void> {
  await page.locator("#ink-button").click();
  await page.getByRole("button", { name: /手稿 A/ }).click();
  await page.locator("#ink-split-toggle").click();
}

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

  test("source-unit primary resolves its artifact and focuses the original block", async ({ page }) => {
    await addSourcePrimary(page);
    await openNoteInSplit(page);
    await page.getByRole("button", { name: "打开主关联「定理 3.2」" }).click();
    await expect(page.locator("#ink-split-reader .markdown-body")).toContainText("正确表述");
    await expect(page.locator("#ink-split-reader .source-block-focus")).toHaveAttribute("data-source-block", "b1");
    expect(await page.evaluate(() => window.appHarness.detailRequests)).toEqual(["unit-thm"]);
    expect(await page.evaluate(() => window.appHarness.contentRequests)).toEqual(["md-1"]);
    await expect(page.locator("#ink-document-heading")).toContainText("手稿 A");
  });

  test("artifact primary can be read directly without a source-unit lookup", async ({ page }) => {
    await addSourcePrimary(page, "md-1", "正确表述.md");
    await openNoteInSplit(page);
    await page.getByRole("button", { name: "打开主关联「正确表述.md」" }).click();
    await expect(page.locator("#ink-split-reader .markdown-body")).toContainText("正确表述");
    expect(await page.evaluate(() => window.appHarness.detailRequests)).toEqual([]);
  });

  test("a failed source stays selectable across reopening and can be retried", async ({ page }) => {
    await page.evaluate(() => window.appHarness.contentFailures.add("md-1"));
    await openNoteInSplit(page);
    const picker = page.locator("#ink-split-empty");
    await picker.getByRole("button", { name: "正确表述.md" }).click();
    await expect(picker).toBeVisible();
    await expect(picker.getByRole("alert")).toContainText("Artifact 不存在");
    for (let i = 0; i < 2; i++) {
      await page.locator("#ink-split-toggle").click();
      await page.locator("#ink-split-toggle").click();
      await expect(picker).toBeVisible();
    }
    expect(await page.evaluate(() => window.appHarness.contentRequests)).toEqual(["md-1"]);
    await page.evaluate(() => window.appHarness.contentFailures.clear());
    await picker.getByRole("button", { name: "正确表述.md" }).click();
    await expect(page.locator("#ink-split-reader .markdown-body")).toContainText("正确表述");
    await expect(picker).toBeHidden();
    await expect(page.locator("#ink-document-heading")).toContainText("手稿 A");
  });

  test("a delayed source lookup cannot reopen the reader after returning to the map", async ({ page }) => {
    await addSourcePrimary(page);
    await page.evaluate(() => { window.appHarness.sourceDetailBehavior.delayMs = 250; });
    await openNoteInSplit(page);
    await page.getByRole("button", { name: "打开主关联「定理 3.2」" }).click();
    await page.locator("#ink-close").click();
    await expect(page.locator(".workspace-grid")).toBeVisible();
    await page.waitForTimeout(350);
    await expect(page.locator("#ink-workspace")).toBeHidden();
    await expect(page.locator("#reader")).toBeHidden();
    expect(await page.evaluate(() => window.appHarness.contentRequests)).toEqual([]);
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

  test("返回地图 after split restores the map; re-enter split still works", async ({ page }) => {
    await expect(page.locator("#graph")).toBeVisible();
    const before = await page.locator("#graph").boundingBox();
    expect(before!.width).toBeGreaterThan(100);
    expect(before!.height).toBeGreaterThan(100);

    await page.locator("#ink-button").click();
    await page.locator("#ink-split-toggle").click();
    await expect(page.locator("#ink-workspace")).toHaveClass(/is-split/);
    // Left pane must not be blank (picker or document).
    const leftHasContent = page.locator("#ink-split-empty, #ink-split-reader #reader:not([hidden])").first();
    await expect(leftHasContent).toBeVisible();
    await page.locator("#ink-split-empty").getByRole("button", { name: "正确表述.md" }).click();
    await expect(page.locator("#ink-split-empty")).toBeHidden({ timeout: 10000 });

    // Header 返回地图 (same control as 手写), not only 退出并排.
    await page.locator("#ink-button").click();
    await expect(page.locator("#ink-button")).toHaveText("手写");
    await expect(page.locator("#ink-workspace")).toBeHidden();
    await expect(page.locator(".workspace-grid")).toBeVisible();

    const graph = page.locator("#graph");
    await expect(graph).toBeVisible();
    await expect.poll(async () => {
      const box = await graph.boundingBox();
      return box ? Math.round(box.width * box.height) : 0;
    }).toBeGreaterThan(100 * 100);
    // Cytoscape / SVG content should be present (nonzero canvas or nodes).
    await expect.poll(async () => page.evaluate(() => {
      const host = document.querySelector("#graph");
      if (!host) return 0;
      const canvas = host.querySelector("canvas");
      if (canvas && canvas.width > 0 && canvas.height > 0) return canvas.width * canvas.height;
      return host.querySelectorAll(".graph-node, [id], canvas, svg").length;
    })).toBeGreaterThan(0);

    const layout = await page.evaluate(() => {
      const reader = document.querySelector("#reader") as HTMLElement;
      const ink = document.querySelector("#ink-workspace") as HTMLElement;
      const grid = document.querySelector(".workspace-grid") as HTMLElement;
      return {
        readerParentIsHome: !reader.parentElement?.id?.includes("ink-split"),
        readerHidden: reader.hidden,
        inkHidden: ink.hidden,
        inkHasSplit: ink.classList.contains("is-split"),
        gridHidden: grid.hidden,
      };
    });
    expect(layout.readerHidden).toBe(true);
    expect(layout.inkHidden).toBe(true);
    expect(layout.inkHasSplit).toBe(false);
    expect(layout.gridHidden).toBe(false);
    expect(layout.readerParentIsHome).toBe(true);

    // Re-enter split: last-open document should fill the left pane.
    await page.locator("#ink-button").click();
    await page.locator("#ink-split-toggle").click();
    await expect(page.locator("#ink-workspace")).toHaveClass(/is-split/);
    await expect(page.locator("#ink-split-empty")).toBeHidden();
    await expect(page.locator("#ink-split-reader")).toContainText("正确表述");
  });

  test("退出并排 then 返回地图 also restores the map", async ({ page }) => {
    await page.locator("#ink-button").click();
    await page.locator("#ink-split-toggle").click();
    await expect(page.locator("#ink-workspace")).toHaveClass(/is-split/);
    await page.locator("#ink-split-toggle").click();
    await expect(page.locator("#ink-workspace")).not.toHaveClass(/is-split/);
    await page.locator("#ink-button").click();
    await expect(page.locator(".workspace-grid")).toBeVisible();
    await expect(page.locator("#graph")).toBeVisible();
    const box = await page.locator("#graph").boundingBox();
    expect(box!.width).toBeGreaterThan(100);
    expect(box!.height).toBeGreaterThan(100);
  });

  test("creating a note from an open Markdown reader (desktop or tablet) auto-enters split with compact left toolbar", async ({ page }) => {
    await page.evaluate(() => {
      window.dispatchEvent(new CustomEvent("mathscape:open-source", { detail: { artifactId: "md-1", title: "正确表述.md" } }));
    });
    await expect(page.locator("#reader .markdown-body")).toContainText("正确表述");
    // In full-screen reader, discovery drawer starts expanded
    await expect(page.locator("#reader .selection-discovery-drawer")).toBeVisible();

    await page.locator("#reader [data-source-block='b1']").click();
    const pill = page.locator("#reader .block-action-pill");
    await expect(pill.getByRole("button", { name: "✎ 手写此块" })).toBeVisible();
    await pill.getByRole("button", { name: "✎ 手写此块" }).click();
    await page.locator(".ink-create-popover").getByRole("button", { name: "用平板书写" }).click();

    // Automatically transitions into left-source + right-ink split workspace + tablet pairing panel
    await expect(page.locator("#ink-workspace")).toBeVisible();
    await expect(page.locator("#ink-workspace")).toHaveClass(/is-split/);
    await expect(page.locator("#ink-split-toggle")).toHaveText("退出并排");
    await expect(page.locator("#ink-split-reader #reader")).toBeVisible();
    await expect(page.locator("#ink-split-reader .markdown-body")).toContainText("正确表述");
    // Discovery drawer auto-collapses inside split left pane to maximize reading space
    await expect(page.locator("#ink-split-reader .selection-discovery-drawer")).toBeHidden();
    // Block marker updates live on the left pane and tablet pairing opens on the right pane
    await expect(page.locator("#ink-split-reader .ink-block-marker")).toHaveText("✎ 1");
    await expect(page.locator("#ink-document-heading")).toContainText("尝试 · 正确表述");
    await expect(page.locator("#ink-lan-panel")).toBeVisible();
  });

});
