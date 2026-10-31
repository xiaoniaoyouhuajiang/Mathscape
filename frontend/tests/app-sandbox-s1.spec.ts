import { expect, test } from "@playwright/test";

test.describe("S1 sandbox shell", () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1400, height: 860 });
    await page.goto("/tests/app.html");
    await expect(page.locator("#sandbox-button")).toBeVisible();
  });

  test("mother window has no sandbox banner", async ({ page }) => {
    await expect(page.locator(".sandbox-banner")).toHaveCount(0);
    await expect(page.locator(".shell")).not.toHaveClass(/is-sandbox/);
  });

  test("entering asks first; declining keeps the mother workspace", async ({ page }) => {
    await page.locator("#sandbox-button").click();
    const messages: string[] = [];
    page.once("dialog", (dialog) => { messages.push(dialog.message()); void dialog.dismiss(); });
    await page.getByRole("button", { name: "在本窗口打开…" }).click();
    await expect.poll(() => messages.length).toBe(1);
    expect(messages[0]).toContain("sbx_demo");
    expect(messages[0]).toContain("/sandbox/sessions/sbx_demo/workspace");
    expect(await page.evaluate(() => window.appHarness.sandboxCalls.length)).toBe(0);
    await expect(page.locator(".sandbox-banner")).toHaveCount(0);
  });

  test("enter → banner in top bar → dialogue round with process cards → return", async ({ page }) => {
    await page.locator("#sandbox-button").click();
    page.once("dialog", (dialog) => void dialog.accept());
    await Promise.all([page.waitForEvent("load"), page.getByRole("button", { name: "在本窗口打开…" }).click()]);

    const banner = page.locator(".topbar .sandbox-banner");
    await expect(banner).toBeVisible();
    await expect(banner).toContainText("沙箱");
    await expect(banner).toContainText("sbx_demo");
    await expect(page.locator(".shell")).toHaveClass(/is-sandbox/);

    await banner.getByRole("button", { name: "对话" }).click();
    const dock = page.getByRole("complementary", { name: "沙箱对话" });
    await expect(dock).toBeVisible();
    await expect(dock.locator(".sandbox-dock-status")).toHaveText("本机 Codex 0.124.0 · 7 个 Mathscape 工具");
    const modelSelect = dock.locator(".sandbox-dock-model-select");
    await expect(modelSelect).toBeVisible();
    await expect(modelSelect).toHaveValue("gpt-test");
    await modelSelect.selectOption("gpt-alt");
    await expect(modelSelect).toHaveValue("gpt-alt");
    await expect.poll(async () => {
      const calls = await page.evaluate(() => window.appHarness.sandboxCalls.map((c) => c.command));
      return calls;
    }).toContain("set_model");
    await expect(dock.locator(".sandbox-dock-resize")).toBeVisible();
    await dock.locator("button[title=\"收起为浮动入口\"]").click();
    await expect(dock).toBeHidden();
    const floatHandle = page.getByRole("button", { name: "展开沙箱对话" });
    await expect(floatHandle).toBeVisible();
    await floatHandle.click();
    await expect(dock).toBeVisible();
    await page.evaluate(() => {
      window.appHarness.sandboxRound.delayMs = 40;
      window.appHarness.sandboxRound.gapMs = 1200;
    });
    await dock.locator("textarea").fill("紧集在哪？");
    await dock.getByRole("button", { name: "发送" }).click();
    await expect(dock.locator(".sandbox-line.role-user")).toHaveText(/紧集在哪？/);
    const live = dock.locator(".sandbox-line.is-live");
    await expect(live.locator(".sandbox-turn-status")).toHaveText("正在调用 workspace.search");
    await expect(live.locator(".sandbox-step.is-running .sandbox-step-badge")).toHaveText("进行中");
    await expect(live.locator(".sandbox-step-title")).toHaveText(["MCP 工具 7/7", "workspace.search"]);
    const agent = dock.locator(".sandbox-line.role-agent");
    await expect(agent.locator(".sandbox-line-text")).toHaveText(/定理 3\.2/);
    await expect(agent.locator(".sandbox-turn-status")).toHaveText("已完成");
    const summary = agent.locator(".sandbox-process-summary");
    await expect(summary).toContainText("已完成 1 步");
    await expect(summary).toContainText("120/30 tokens");
    await expect(agent.locator(".sandbox-step")).toHaveCount(0);
    await summary.click();
    await expect(agent.locator(".sandbox-step-title")).toHaveText(["MCP 工具 7/7", "workspace.search"]);
    await expect(agent.locator(".sandbox-process-usage")).toHaveText("用量：输入 120 · 输出 30");
    await agent.locator(".sandbox-step-head").nth(1).click();
    await expect(agent.locator(".sandbox-step-pre").first()).toContainText("query=紧集");
    await expect(dock.getByRole("button", { name: "发送" })).toBeEnabled();
    await page.evaluate(() => {
      window.appHarness.sandboxRound.delayMs = 30;
      window.appHarness.sandboxRound.gapMs = 0;
    });
    await page.screenshot({ path: "../.test-runs/sandbox-s1.png" });

    const prompts: string[] = [];
    page.once("dialog", (dialog) => { prompts.push(dialog.message()); void dialog.accept(); });
    await Promise.all([page.waitForEvent("load"), banner.getByRole("button", { name: "返回母工作区" }).click()]);
    expect(prompts[0]).toContain("返回母工作区");
    await expect(page.locator("#sandbox-button")).toBeVisible();
    await expect(page.locator(".sandbox-banner")).toHaveCount(0);
  });

  test("a real click on the model bar reaches the native picker (header drag must not swallow it)", async ({ page }) => {
    await page.locator("#sandbox-button").click();
    page.once("dialog", (dialog) => void dialog.accept());
    await Promise.all([page.waitForEvent("load"), page.getByRole("button", { name: "在本窗口打开…" }).click()]);
    await page.locator(".topbar .sandbox-banner").getByRole("button", { name: "对话" }).click();
    const dock = page.getByRole("complementary", { name: "沙箱对话" });
    const modelSelect = dock.locator(".sandbox-dock-model-select");
    await expect(modelSelect).toBeEnabled();
    await expect(modelSelect.locator("option")).toHaveCount(2);
    await expect(dock.locator(".sandbox-dock-model-hint")).toBeHidden();
    await modelSelect.evaluate((select) => {
      const seen: string[] = [];
      (window as unknown as { __selectSeen: string[] }).__selectSeen = seen;
      // Bubble-phase listener on document runs after the header's handler.
      document.addEventListener("pointerdown", (event) => {
        if (event.target === select) seen.push(`pointerdown:${event.defaultPrevented ? "prevented" : "ok"}`);
      });
      select.addEventListener("mousedown", () => seen.push("mousedown"));
    });
    const box = (await modelSelect.boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await expect(dock.locator(".sandbox-dock-header")).not.toHaveClass(/is-dragging/);
    await page.mouse.up();
    await page.keyboard.press("Escape");
    const seen = await page.evaluate(() => (window as unknown as { __selectSeen: string[] }).__selectSeen);
    expect(seen).toEqual(["pointerdown:ok", "mousedown"]);
  });

  test("effort follows the selected model, persists via set_effort, and locks during a round", async ({ page }) => {
    await page.locator("#sandbox-button").click();
    page.once("dialog", (dialog) => void dialog.accept());
    await Promise.all([page.waitForEvent("load"), page.getByRole("button", { name: "在本窗口打开…" }).click()]);
    await page.locator(".topbar .sandbox-banner").getByRole("button", { name: "对话" }).click();
    const dock = page.getByRole("complementary", { name: "沙箱对话" });
    const modelSelect = dock.locator(".sandbox-dock-model-select");
    const effortSelect = dock.locator(".sandbox-dock-effort-select");
    await expect(effortSelect).toBeVisible();
    await expect(effortSelect).toBeEnabled();
    await expect(effortSelect).toHaveValue("high");
    await expect(effortSelect.locator("option")).toHaveText([
      "low · 低（默认）", "medium · 中", "high · 高", "xhigh · 超高", "max · 最大", "ultra · 极限",
    ]);

    await effortSelect.selectOption("ultra");
    await expect(effortSelect).toHaveValue("ultra");
    await expect.poll(() => page.evaluate(() => window.appHarness.sandboxCalls.filter((c) => c.command === "set_effort").map((c) => c.args.effort)))
      .toEqual(["ultra"]);

    // gpt-alt has no ultra: the list is rebuilt and the effort falls back to its default.
    await modelSelect.selectOption("gpt-alt");
    await expect(effortSelect.locator("option")).toHaveCount(5);
    await expect(effortSelect).toHaveValue("medium");
    await expect(effortSelect.locator("option", { hasText: "ultra" })).toHaveCount(0);

    await page.evaluate(() => { window.appHarness.sandboxRound.delayMs = 1500; });
    await dock.locator("textarea").fill("紧集在哪？");
    await dock.getByRole("button", { name: "发送" }).click();
    await expect(effortSelect).toBeDisabled();
    await expect(modelSelect).toBeDisabled();
    await expect(dock.locator(".sandbox-line.role-agent")).toHaveText(/定理 3\.2/);
    await expect(effortSelect).toBeEnabled();
    await expect(modelSelect).toBeEnabled();
    await page.evaluate(() => { window.appHarness.sandboxRound.delayMs = 30; });
  });
});

