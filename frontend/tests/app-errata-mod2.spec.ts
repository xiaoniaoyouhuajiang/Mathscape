import { expect, test } from "@playwright/test";

test.describe("Errata Module 2 + Continuity Closures", () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1400, height: 860 });
  });

  test("left-reader errata live-syncs right ink link chip and narrow split PDF reference stays scoped to left pane", async ({ page }) => {
    await page.goto("/tests/app.html");
    await expect(page.locator("#ink-button")).toBeVisible();

    // Open md-1 and create a Markdown-linked handwrite note (enters left-reader + right-ink split)
    await page.evaluate(() => {
      window.dispatchEvent(new CustomEvent("mathscape:open-source", { detail: { artifactId: "md-1", title: "正确表述.md" } }));
    });
    const block = page.locator("#reader [data-source-block='b1']");
    await block.click();
    await page.locator("#reader .block-action-pill").getByRole("button", { name: "✎ 手写此块" }).click();
    await page.locator(".ink-create-popover").getByRole("button", { name: "在电脑上书写" }).click();

    await expect(page.locator("#ink-workspace")).toHaveClass(/is-split/);
    const linkChip = page.locator("#ink-link-bar .ink-link-chip .link-button").first();
    await expect(linkChip).toContainText("正确表述");

    // Edit the Markdown block in the left reader pane
    const splitBlock = page.locator("#ink-split-reader [data-source-block='b1']");
    await splitBlock.click();
    await splitBlock.locator(".block-pill-errata").click();
    const editor = page.locator("#ink-split-reader .errata-inline-editor");
    await expect(editor).toBeVisible();

    // Open `对照 PDF 原页` inside narrow `#ink-split-reader`: overlays left pane ONLY without crushing right ink canvas!
    const inkBefore = (await page.locator("#ink-desktop-host").boundingBox())!;
    await editor.locator(".errata-pdf-toggle-btn").click();
    const refPane = page.locator("#ink-split-reader .errata-pdf-reference-pane");
    await expect(refPane).toBeVisible();
    await expect(refPane.locator(".pdf-viewport canvas").first()).toBeVisible({ timeout: 10000 });
    const inkDuring = (await page.locator("#ink-desktop-host").boundingBox())!;
    const refBox = (await refPane.boundingBox())!;
    expect(Math.abs(inkDuring.width - inkBefore.width)).toBeLessThanOrEqual(2);
    expect(refBox.x + refBox.width).toBeLessThanOrEqual(inkDuring.x + 2);
    await refPane.getByRole("button", { name: "✕ 关闭对照" }).click();
    await expect(refPane).toHaveCount(0);

    // Save the errata -> right-hand ink link chip updates IMMEDIATELY without reload!
    const textarea = editor.getByLabel("校对后的 Markdown 源码");
    await textarea.fill("# 组合恒等式（已校对）\n\n设 $C_n^r + C_n^{r+1} = C_{n+1}^{r+1}$。");
    await textarea.press("Control+Enter");
    await expect(editor).toBeHidden();
    await expect(splitBlock).toContainText("组合恒等式（已校对）");
    await expect(linkChip).toContainText("组合恒等式（已校对）");
  });

  test("map node shows non-blocking errata badge, Inspector shows Diff + sandbox prefill + mark reviewed clears badge", async ({ page }) => {
    await page.addInitScript(() => sessionStorage.setItem("harness.sandbox", "1"));
    await page.goto("/tests/app.html");
    await expect(page.locator("#graph")).toBeVisible();

    // Open md-1 and save an errata affecting `unit-thm`
    await page.evaluate(() => {
      window.dispatchEvent(new CustomEvent("mathscape:open-source", { detail: { artifactId: "md-1", title: "正确表述.md" } }));
    });
    const block = page.locator("#reader [data-source-block='b1']");
    await block.click();
    await block.locator(".block-pill-errata").click();
    await page.locator(".errata-inline-editor").getByLabel("校对后的 Markdown 源码").fill("# 正确表述（校对版）\n\n设 $a_n \\to a$ 收敛。");
    await page.locator(".errata-inline-editor").getByRole("button", { name: "保存校对" }).click();
    await expect(page.locator(".errata-inline-editor")).toBeHidden();

    // Return to map -> `r-unit-thm` shows non-blocking `.has-errata-review` class and `已校对` suffix
    await page.locator("#reader .reader-close-overlay-btn").click();
    await expect.poll(async () => page.evaluate(() => window.appHarness.nodeClasses("r-unit-thm"))).toContain("has-errata-review");
    await expect.poll(async () => page.evaluate(() => window.appHarness.labels()["r-unit-thm"])).toContain("已校对");

    // Select `r-unit-thm` -> Inspector shows effective excerpt + non-blocking `原文已校对` review card with Diff
    await page.evaluate(() => window.appHarness.selectNode("r-unit-thm"));
    const reviewCard = page.locator("#node-detail .detail-errata-review");
    await expect(reviewCard).toBeVisible();
    await expect(reviewCard).toContainText("原文已校对");
    await expect(page.locator("#node-detail .source-preview")).toContainText("正确表述（校对版）");
    await reviewCard.locator("summary").click();
    await expect(reviewCard.locator(".errata-diff-line.is-add").first()).toContainText("正确表述（校对版）");

    // Open sandbox dialogue dock so `交给沙盒复核陈述` prefills the composer
    await page.getByRole("button", { name: /对话|沙箱对话/ }).first().click();
    await expect(page.locator(".sandbox-dock")).toBeVisible();
    await reviewCard.getByRole("button", { name: "交给沙盒复核陈述" }).click();
    await expect(reviewCard.locator(".detail-errata-sent-pill")).toBeVisible();
    await expect(page.locator(".sandbox-dock-input")).toContainText("请复核已入图节点");
    await expect(page.locator(".sandbox-dock-input")).toContainText("正确表述（校对版）");

    // Click `标记已复核` -> review card and map `.has-errata-review` badge both clear cleanly
    await reviewCard.getByRole("button", { name: "标记已复核" }).click();
    await expect(reviewCard).toHaveCount(0);
    await expect.poll(async () => page.evaluate(() => window.appHarness.nodeClasses("r-unit-thm"))).not.toContain("has-errata-review");
    await expect.poll(async () => page.evaluate(() => window.appHarness.labels()["r-unit-thm"])).not.toContain("已校对");
  });

  test("agent dialogue @ mention indexes active live ink notes and clicking chip opens the ink workspace", async ({ page }) => {
    await page.addInitScript(() => sessionStorage.setItem("harness.sandbox", "1"));
    await page.goto("/tests/app.html");
    await expect(page.locator("#graph")).toBeVisible();

    await page.getByRole("button", { name: /对话|沙箱对话/ }).first().click();
    await expect(page.locator(".sandbox-dock")).toBeVisible();

    const composer = page.locator(".sandbox-dock-input");
    await composer.click();
    await page.keyboard.type("请看 @手稿 A");

    const menu = page.locator(".sandbox-composer-menu");
    await expect(menu).toBeVisible();
    await expect(menu).toContainText("手稿 A");
    await expect(menu).toContainText("手稿");

    await menu.locator("[data-mention-index]").first().click();
    await expect(menu).toBeHidden();
    const chip = composer.locator(".sandbox-cite");
    await expect(chip).toHaveText("手稿 A");

    // Send message and click the rendered cite chip in the transcript to jump directly to the note!
    await page.locator(".sandbox-dock").getByRole("button", { name: "发送" }).click();
    const logChip = page.locator(".sandbox-dock-log .sandbox-cite").filter({ hasText: "手稿 A" }).first();
    await expect(logChip).toBeVisible({ timeout: 5000 });
    await logChip.click();
    await expect(page.locator("#ink-workspace")).toBeVisible();
    await expect(page.locator("#ink-document-heading")).toContainText("手稿 A");
  });

  test("entering split from a scrolled long Markdown never jumps back to a stale off-screen focused/selected block", async ({ page }) => {
    await page.goto("/tests/app.html");
    await expect(page.locator("#ink-button")).toBeVisible();

    // Open long Markdown with initial `.source-block-focus` on block 1, then click block 1 so it also has `.is-selected-block`
    await page.evaluate(() => {
      window.dispatchEvent(new CustomEvent("mathscape:open-source", {
        detail: { artifactId: "md-long", title: "长文档.md", focus: { start: 0, end: 20, blockIds: ["lb-1"] } },
      }));
    });
    await expect(page.locator("#reader [data-source-block='lb-1']")).toBeVisible();
    await page.locator("#reader [data-source-block='lb-1']").click();

    // Scroll far down to block 22 (block 1 is now completely off-screen above viewport!)
    await page.locator("#reader [data-source-block='lb-22']").scrollIntoViewIfNeeded();
    await page.waitForTimeout(100);
    const beforeInView = await page.evaluate(() => {
      const host = document.querySelector("#reader")!.getBoundingClientRect();
      const b22 = document.querySelector("#reader [data-source-block='lb-22']")!.getBoundingClientRect();
      return b22.bottom > host.top && b22.top < host.bottom;
    });
    expect(beforeInView).toBe(true);

    // Open an ink note via `open-ink-document` (same path as clicking a ✎ badge without changing selection)
    await page.evaluate(() => {
      const docA = window.appHarness.documents.a;
      window.dispatchEvent(new CustomEvent("mathscape:open-ink-document", { detail: { documentId: docA } }));
    });
    await expect(page.locator("#ink-workspace")).toHaveClass(/is-split/);
    await page.waitForTimeout(150);

    // Left reader MUST remain anchored near block 22, NOT yanked back up to stale block 1!
    const afterCheck = await page.evaluate(() => {
      const host = document.querySelector("#reader")!.getBoundingClientRect();
      const b1 = document.querySelector("#reader [data-source-block='lb-1']")!.getBoundingClientRect();
      const b22 = document.querySelector("#reader [data-source-block='lb-22']")!.getBoundingClientRect();
      return {
        b1Visible: b1.bottom > host.top + 8 && b1.top < host.bottom - 8,
        b22Visible: b22.bottom > host.top - 80 && b22.top < host.bottom + 80,
      };
    });
    expect(afterCheck.b1Visible).toBe(false);
    expect(afterCheck.b22Visible).toBe(true);
  });
});
