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
    // A2: sidebar by default — beside the map, not over it.
    await expect(dock).toHaveClass(/is-sidebar/);
    await expect(page.locator(".shell")).toHaveClass(/has-sandbox-sidebar/);
    await expect(dock.locator(".sandbox-dock-side-resize")).toBeVisible();
    await expect(dock.locator(".sandbox-dock-resize")).toBeHidden();
    await expect.poll(async () => {
      const grid = (await page.locator(".workspace-grid").boundingBox())!;
      const side = (await dock.boundingBox())!;
      return grid.x + grid.width <= side.x + 1;
    }).toBe(true);
    await dock.getByRole("button", { name: "改为浮动窗口" }).click();
    await expect(dock).toHaveClass(/is-floating/);
    await expect(page.locator(".shell")).not.toHaveClass(/has-sandbox-sidebar/);
    await expect(dock.locator(".sandbox-dock-resize")).toBeVisible();
    await dock.getByRole("button", { name: "停靠到右侧栏（与地图同屏）" }).click();
    await expect(dock).toHaveClass(/is-sidebar/);
    await dock.locator("button[title=\"收起为浮动入口\"]").click();
    await expect(page.locator(".shell")).not.toHaveClass(/has-sandbox-sidebar/);
    await expect(dock).toBeHidden();
    const floatHandle = page.getByRole("button", { name: "展开沙箱对话" });
    await expect(floatHandle).toBeVisible();
    await floatHandle.click();
    await expect(dock).toBeVisible();
    await expect(dock).toHaveClass(/is-sidebar/);
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

  test("sidebar width and dock mode persist across reloads; floating still drags", async ({ page }) => {
    await page.locator("#sandbox-button").click();
    page.once("dialog", (dialog) => void dialog.accept());
    await Promise.all([page.waitForEvent("load"), page.getByRole("button", { name: "在本窗口打开…" }).click()]);
    const banner = page.locator(".topbar .sandbox-banner");
    await banner.getByRole("button", { name: "对话" }).click();
    const dock = page.getByRole("complementary", { name: "沙箱对话" });
    await expect(dock).toHaveClass(/is-sidebar/);
    const before = (await dock.boundingBox())!;
    expect(Math.round(before.width)).toBe(420);
    const topbar = (await page.locator(".topbar").boundingBox())!;
    expect(Math.round(before.y)).toBe(Math.round(topbar.y + topbar.height));
    const gridBefore = (await page.locator(".workspace-grid").boundingBox())!;

    const handle = dock.locator(".sandbox-dock-side-resize");
    const hb = (await handle.boundingBox())!;
    await page.mouse.move(hb.x + hb.width / 2, hb.y + 200);
    await page.mouse.down();
    await page.mouse.move(hb.x + hb.width / 2 - 100, hb.y + 200, { steps: 5 });
    await page.mouse.up();
    await expect.poll(async () => Math.round((await dock.boundingBox())!.width)).toBe(520);
    await expect.poll(async () => Math.round((await page.locator(".workspace-grid").boundingBox())!.width))
      .toBe(Math.round(gridBefore.width) - 100);
    await handle.focus();
    await page.keyboard.press("ArrowRight");
    await expect.poll(async () => Math.round((await dock.boundingBox())!.width)).toBe(496);
    expect(await page.evaluate(() => localStorage.getItem("mathscape:sandbox-dialogue-sidebar-width"))).toBe("496");

    // Header does not drag the sidebar.
    const header = dock.locator(".sandbox-dock-header");
    const h = (await header.boundingBox())!;
    await page.mouse.move(h.x + 20, h.y + 8);
    await page.mouse.down();
    await page.mouse.move(h.x - 200, h.y + 100, { steps: 4 });
    await page.mouse.up();
    expect(Math.round((await dock.boundingBox())!.x)).toBe(1400 - 496);

    await page.reload();
    await page.locator(".topbar .sandbox-banner").getByRole("button", { name: "对话" }).click();
    await expect(dock).toHaveClass(/is-sidebar/);
    await expect.poll(async () => Math.round((await dock.boundingBox())!.width)).toBe(496);

    await dock.getByRole("button", { name: "改为浮动窗口" }).click();
    await expect(dock).toHaveClass(/is-floating/);
    const floatX = Math.round((await dock.boundingBox())!.x);
    const f = (await header.boundingBox())!;
    await page.mouse.move(f.x + 20, f.y + 8);
    await page.mouse.down();
    await page.mouse.move(f.x - 160, f.y - 60, { steps: 4 });
    await page.mouse.up();
    await expect.poll(async () => Math.round((await dock.boundingBox())!.x)).toBe(floatX - 180);
    await page.reload();
    await page.locator(".topbar .sandbox-banner").getByRole("button", { name: "对话" }).click();
    await expect(dock).toHaveClass(/is-floating/);
    await expect(page.locator(".shell")).not.toHaveClass(/has-sandbox-sidebar/);
    expect(await page.evaluate(() => localStorage.getItem("mathscape:sandbox-dialogue-mode"))).toBe("floating");
  });

  test("a narrow window renders the sidebar preference as floating", async ({ page }) => {
    await page.locator("#sandbox-button").click();
    page.once("dialog", (dialog) => void dialog.accept());
    await Promise.all([page.waitForEvent("load"), page.getByRole("button", { name: "在本窗口打开…" }).click()]);
    await page.locator(".topbar .sandbox-banner").getByRole("button", { name: "对话" }).click();
    const dock = page.getByRole("complementary", { name: "沙箱对话" });
    await expect(dock).toHaveClass(/is-sidebar/);
    await page.setViewportSize({ width: 740, height: 760 });
    await expect(dock).toHaveClass(/is-floating/);
    await expect(page.locator(".shell")).not.toHaveClass(/has-sandbox-sidebar/);
    await page.setViewportSize({ width: 1400, height: 860 });
    await expect(dock).toHaveClass(/is-sidebar/);
    await expect(page.locator(".shell")).toHaveClass(/has-sandbox-sidebar/);
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
  test("A-PR3: cite chips jump to the map, the read path echoes, and bubbles render LaTeX", async ({ page }) => {
    await page.locator("#sandbox-button").click();
    page.once("dialog", (dialog) => void dialog.accept());
    await Promise.all([page.waitForEvent("load"), page.getByRole("button", { name: "在本窗口打开…" }).click()]);
    await page.locator(".topbar .sandbox-banner").getByRole("button", { name: "对话" }).click();
    const dock = page.getByRole("complementary", { name: "沙箱对话" });
    await expect(dock).toHaveClass(/is-sidebar/);
    const unitId = await page.evaluate(() => window.appHarness.citeUnitId);
    await page.evaluate(() => { Object.assign(window.appHarness.sandboxRound, { cite: true, delayMs: 30, gapMs: 1200 }); });

    await dock.locator("textarea").fill(`看看 ${unitId} 和 $x^2$`);
    await dock.getByRole("button", { name: "发送" }).click();

    // tool_call: the seed lights up on the map while the call runs.
    await expect.poll(() => page.evaluate(() => window.appHarness.nodeClasses("r-cite-unit"))).toContain("sandbox-echo-seed");
    // tool_result: the returned node is flashed as a hit (the seed pulse is replaced).
    await expect.poll(() => page.evaluate(() => window.appHarness.nodeClasses("r-cite-unit"))).toContain("sandbox-echo-hit");

    const user = dock.locator(".sandbox-line.role-user").last();
    await expect(user.locator(".sandbox-line-text.markdown-body .katex")).toHaveCount(1);
    await expect(user.locator("button.sandbox-cite.is-map")).toHaveText("引理 1.4");

    const agent = dock.locator(".sandbox-line.role-agent").last();
    const body = agent.locator(".sandbox-line-text.markdown-body");
    await expect(body.locator(".katex")).toHaveCount(1);
    const chip = body.locator("button.sandbox-cite.is-map");
    await expect(chip).toHaveText("引理 1.4");
    await expect(chip).toHaveAttribute("data-cite-id", unitId);
    await expect(chip).toHaveAttribute("title", /source-unit\/ab12ab12…/);
    const unknown = body.locator(".sandbox-cite.is-unknown");
    await expect(unknown).toHaveCount(1);
    await expect(unknown).toHaveAttribute("title", /未在地图或本轮工具结果中找到/);
    await expect(body.locator("button.sandbox-cite.is-unknown")).toHaveCount(0);

    // Process card args are chip-ified too, with the full id kept on the chip.
    await agent.locator(".sandbox-process-summary").click();
    await agent.locator(".sandbox-step-head").nth(1).click();
    await expect(agent.locator(".sandbox-step-pre").first().locator("button.sandbox-cite")).toHaveText("引理 1.4");

    // Clicking the chip selects the node and keeps the sidebar dialogue open.
    await expect.poll(() => page.evaluate(() => window.appHarness.nodeView("r-cite-unit")?.selected)).toBe(false);
    await chip.click();
    await expect.poll(() => page.evaluate(() => window.appHarness.nodeView("r-cite-unit")?.selected)).toBe(true);
    await expect(dock).toBeVisible();
    await expect(dock).toHaveClass(/is-sidebar/);
    await expect(page.locator(".shell")).toHaveClass(/has-sandbox-sidebar/);

    await page.evaluate(() => { Object.assign(window.appHarness.sandboxRound, { cite: false, gapMs: 0 }); });
  });

  test("Map Scene Player: live HUD + isolate, settle restores, per-step and whole-turn replay", async ({ page }) => {
    await page.locator("#sandbox-button").click();
    page.once("dialog", (dialog) => void dialog.accept());
    await Promise.all([page.waitForEvent("load"), page.getByRole("button", { name: "在本窗口打开…" }).click()]);
    await page.locator(".topbar .sandbox-banner").getByRole("button", { name: "对话" }).click();
    const dock = page.getByRole("complementary", { name: "沙箱对话" });
    await expect(dock).toHaveClass(/is-sidebar/);
    const unitId = await page.evaluate(() => window.appHarness.citeUnitId);
    await page.evaluate(() => { Object.assign(window.appHarness.sandboxRound, { cite: true, delayMs: 30, gapMs: 1200 }); });
    const viewBefore = await page.evaluate(() => window.appHarness.viewport());

    await dock.locator("textarea").fill(`看看 ${unitId}`);
    await dock.getByRole("button", { name: "发送" }).click();

    // Announce: HUD on the map with the Chinese title (map label while running).
    const hud = page.locator(".canvas-panel .scene-hud");
    await expect(hud).toBeVisible();
    await expect(hud.locator(".scene-hud-title")).toHaveText("查看节点 · 引理 1.4");
    // Isolate: the focus stays lit, everything else uses the focus-mode dim.
    await expect.poll(() => page.evaluate(() => window.appHarness.nodeClasses("r-cite-unit"))).toContain("sandbox-echo-seed");
    expect(await page.evaluate(() => window.appHarness.nodeClasses("r-cite-unit"))).not.toContain("is-out-of-focus");
    expect(await page.evaluate(() => window.appHarness.nodeClasses("r-pdf-notes"))).toContain("is-out-of-focus");
    // The map is locked while a scene plays.
    await expect(page.locator("#graph")).toHaveClass(/is-scene-playing/);
    await expect.poll(() => page.evaluate(() => window.appHarness.nodeClasses("r-cite-unit"))).toContain("sandbox-echo-hit");
    await expect(hud.locator(".scene-hud-summary")).toContainText("地图上 1 个");

    // Settle: HUD gone, dim and lock cleared, camera back where it was.
    await expect(hud).toBeHidden({ timeout: 8000 });
    await expect(page.locator("#graph")).not.toHaveClass(/is-scene-playing/);
    expect(await page.evaluate(() => window.appHarness.nodeClasses("r-pdf-notes"))).not.toContain("is-out-of-focus");
    await expect.poll(() => page.evaluate(() => window.appHarness.viewport())).toEqual(viewBefore);

    // Per-step replay from the process card.
    const agent = dock.locator(".sandbox-line.role-agent").last();
    await expect(agent.locator(".sandbox-turn-status")).toHaveText("已完成");
    await agent.locator(".sandbox-process-summary").click();
    const stepReplay = agent.locator(".sandbox-step-replay");
    await expect(stepReplay).toHaveCount(1);
    await expect(stepReplay).toBeEnabled();
    await stepReplay.click();
    await expect(hud).toBeVisible();
    await expect(hud.locator(".scene-hud-title")).toHaveText("查看节点 · 引理 1.4");
    await expect(hud.locator(".scene-hud-step")).toBeHidden();
    await expect.poll(() => page.evaluate(() => window.appHarness.nodeClasses("r-cite-unit"))).toContain("sandbox-echo-hit");
    await expect(hud).toBeHidden({ timeout: 8000 });

    // Whole-turn replay; 停止 ends it at once and restores the camera.
    const all = agent.locator(".sandbox-process-replay");
    await expect(all).toHaveText("▶ 回放全部");
    await all.click();
    await expect(hud).toBeVisible();
    await expect(page.locator("#graph")).toHaveClass(/is-scene-playing/);
    await hud.getByRole("button", { name: "停止" }).click();
    await expect(hud).toBeHidden();
    await expect(page.locator("#graph")).not.toHaveClass(/is-scene-playing/);
    expect(await page.evaluate(() => window.appHarness.nodeClasses("r-pdf-notes"))).not.toContain("is-out-of-focus");
    expect(await page.evaluate(() => window.appHarness.viewport())).toEqual(viewBefore);

    await page.evaluate(() => { Object.assign(window.appHarness.sandboxRound, { cite: false, gapMs: 0 }); });
  });
});

