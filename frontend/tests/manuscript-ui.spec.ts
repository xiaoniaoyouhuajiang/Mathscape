import { expect, test, type Locator, type Page } from "@playwright/test";

function luminance(color: string): number {
  const [r, g, b] = color.match(/[\d.]+/g)!.slice(0, 3).map(Number).map(value => {
    const s = value / 255;
    return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  });
  return r * 0.2126 + g * 0.7152 + b * 0.0722;
}

async function expectLegible(locator: Locator): Promise<void> {
  const colors = await locator.evaluate(element => {
    const css = getComputedStyle(element);
    return [css.color, css.backgroundColor];
  });
  const [a, b] = colors.map(luminance);
  expect((Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05)).toBeGreaterThanOrEqual(4.5);
}

async function expectSummarySpacing(page: Page): Promise<void> {
  const summary = (await page.locator("#ink-persistence-status").boundingBox())!;
  const push = (await page.locator("#ink-push-tablet").boundingBox())!;
  if (summary.y < push.y + push.height) {
    expect(summary.x - push.x - push.width).toBeGreaterThanOrEqual(12);
  } else {
    expect(summary.y - push.y - push.height).toBeGreaterThanOrEqual(8);
  }
}

for (const theme of ["obsidian", "parchment", "slate"] as const) {
  test.describe(`manuscript UI — ${theme}`, () => {
    test.beforeEach(async ({ page }) => {
      await page.setViewportSize({ width: 1600, height: 1000 });
      await page.addInitScript(theme => {
        localStorage.setItem("mathscape.theme", theme);
        document.addEventListener("DOMContentLoaded", () => {
          document.documentElement.dataset.theme = theme;
        }, { once: true });
      }, theme);
    });

    test("current manuscript stays distinct while connected, offline and editable", async ({ page }, info) => {
      await page.goto("/tests/ink-desktop.html");
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await expect(page.locator("#ink-session-list .ink-session-item")).toHaveCount(2);
      await page.evaluate(() => {
        const h = window.inkDesktopHarness;
        h.behavior.remoteDocumentId = h.documents.a;
      });
      const current = page.locator("#ink-session-list [aria-current]");
      const context = current.locator(".ink-session-context");
      await expect(context).toHaveText("当前查看 · 平板已连接 · 电脑只读");
      await expect(page.locator("#desktop-ink-pen")).toBeDisabled();
      await page.mouse.move(1, 1);
      const colors = await page.locator("#ink-session-list .ink-session-item").evaluateAll(buttons => buttons.map(button => ({
        current: button.hasAttribute("aria-current"), border: getComputedStyle(button).borderLeftColor,
      })));
      expect(colors.find(item => item.current)!.border).not.toBe(colors.find(item => !item.current)!.border);
      await expectSummarySpacing(page);
      await page.screenshot({ path: info.outputPath(`mirror-${theme}.png`) });

      await page.setViewportSize({ width: 720, height: 1000 });
      await expect(context).toHaveText("当前查看 · 平板已连接 · 电脑只读");
      await expect(page.locator("#ink-persistence-status")).toBeHidden();
      expect(await page.locator(".ink-desktop-header").evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true);
      await page.screenshot({ path: info.outputPath(`mirror-720-${theme}.png`) });
      await page.setViewportSize({ width: 1600, height: 1000 });

      await page.evaluate(() => { window.inkDesktopHarness.behavior.remoteConnected = false; });
      await expect(context).toHaveText("当前查看 · 平板离线 · 电脑只读");
      await expect(current).toHaveAttribute("aria-current", "location");
      await expect(page.locator("#desktop-ink-pen")).toBeDisabled();
      await page.evaluate(() => { window.inkDesktopHarness.behavior.remoteDocumentId = ""; });
      await expect(context).toHaveText("当前查看 · 可编辑");
      await expect(page.locator("#desktop-ink-pen")).toBeEnabled();
      await page.getByRole("button", { name: /手稿 B/ }).click();
      await expect(current).toContainText("手稿 B");
      await expect(page.locator("#ink-session-list .ink-session-context:visible")).toHaveCount(1);
    });

    for (const width of [1600, 720]) {
      test(`header preserves grouped commands and canvas space at ${width}px`, async ({ page }, info) => {
        await page.setViewportSize({ width, height: 1000 });
        await page.goto("/tests/ink-desktop.html");
        await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
        await expect(page.locator("#ink-document-heading")).toHaveText("手稿 A");
        const header = page.locator(".ink-desktop-header");
        const bounds = (await header.boundingBox())!;
        const sizes = await page.locator("#ink-list-toggle, #ink-save-now, #desktop-ink-erase, #desktop-ink-pan").evaluateAll(buttons => buttons.map(button => button.getBoundingClientRect().height));
        expect(sizes.every(height => height >= 32)).toBe(true);
        expect(new Set(sizes).size).toBe(1);
        expect(await header.evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true);
        await expect(header.getByRole("group", { name: "手稿管理" })).toBeVisible();
        await expect(header.getByRole("group", { name: "平板连接" })).toBeVisible();
        const back = (await page.locator("#ink-close").boundingBox())!;
        const title = (await page.locator("#ink-document-heading").boundingBox())!;
        expect(Math.abs(back.y - title.y)).toBeLessThan(28);
        if (width >= 1000) {
          expect(bounds.height).toBeLessThanOrEqual(112);
          await expectSummarySpacing(page);
        }
        expect((await page.locator("#desktop-ink-board").boundingBox())!.height).toBeGreaterThan(550);
        await page.screenshot({ path: info.outputPath(`header-${width}-${theme}.png`) });
      });
    }

    test("paper manuscript marker is purple, legible and opens the linked note", async ({ page }, info) => {
      await page.goto("/tests/app.html");
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await page.waitForFunction(() => "appHarness" in window);
      await page.evaluate(() => {
        const h = window.appHarness;
        h.links.push({
          link_id: "purple-link", document_id: h.documents.b, target_kind: "markdown", target_key: "md:md-1:b1", target_id: "md-1",
          target: { kind: "markdown", artifact_id: "md-1", content_hash: "sha256:test", first_block: "b1", last_block: "b1", block_ids: ["b1"], start: 0, end: 1, block_hash: "h", excerpt: "正确表述", label: "正确表述" },
          target_revision: "sha256:test", target_label: "正确表述", role: "check", is_primary: true, state: "active", created_at: 1, removed_at: null, target_status: "active",
        });
      });
      await page.locator("#ink-button").click();
      await page.getByRole("button", { name: /手稿 A/ }).click();
      await page.locator("#ink-split-toggle").click();
      await page.locator("#ink-split-empty").getByRole("button", { name: "正确表述.md" }).click();
      const marker = page.locator("#ink-split-reader .ink-block-marker");
      await expect(marker).toHaveText("✎ 1");
      await expectLegible(marker);
      const [r, g, b] = await marker.evaluate(element => getComputedStyle(element).color.match(/[\d.]+/g)!.slice(0, 3).map(Number));
      expect(b).toBeGreaterThan(r);
      expect(r).toBeGreaterThan(g + 20);
      await expectSummarySpacing(page);
      await page.screenshot({ path: info.outputPath(`reader-marker-${theme}.png`) });
      await marker.click();
      await page.locator(".ink-related-popover").getByRole("button", { name: "手稿 B" }).click();
      await expect(page.locator("#ink-document-heading")).toHaveText("手稿 B");
    });

    test("chat inline code, code blocks and formulae remain readable", async ({ page }, info) => {
      await page.addInitScript(() => sessionStorage.setItem("harness.sandbox", "1"));
      await page.goto("/tests/app.html");
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await expect(page.locator(".sandbox-banner")).toBeVisible();
      await page.locator(".sandbox-banner").getByRole("button", { name: "对话", exact: true }).click();
      await page.evaluate(() => {
        window.appHarness.sandboxRound.replyText = "## 格式检查\n\n使用 `material.read` 查看笔记。\n\n> 引用内容保持清晰。\n\n```ts\nconst count = 3;\n```\n\n公式 $a_n = n + 1$。";
      });
      const dock = page.getByRole("complementary", { name: "沙箱对话" });
      await dock.locator(".sandbox-dock-input").fill("检查格式显示");
      await dock.getByRole("button", { name: "发送", exact: true }).click();
      const reply = dock.locator(".sandbox-line.role-agent .sandbox-line-text");
      await expect(reply.locator("p code")).toHaveText("material.read");
      await expectLegible(reply.locator("p code"));
      await expectLegible(reply.locator("pre"));
      await expect(reply.locator("pre code")).toHaveText("const count = 3;");
      await expect(reply.locator(".katex")).toHaveCount(1);
      await expect(reply.locator(".katex-error")).toHaveCount(0);
      await page.screenshot({ path: info.outputPath(`chat-${theme}.png`) });
    });
  });
}
