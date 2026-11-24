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
    await expect(dock.locator(".sandbox-dock-status")).toHaveText("本机 Codex 0.124.0 · 8 个 Mathscape 工具");
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
    await dock.locator(".sandbox-dock-input").fill("紧集在哪？");
    await dock.getByRole("button", { name: "发送" }).click();
    await expect(dock.locator(".sandbox-line.role-user")).toHaveText(/紧集在哪？/);
    const live = dock.locator(".sandbox-line.is-live");
    await expect(live.locator(".sandbox-turn-status")).toHaveText("正在调用 workspace.search");
    await expect(live.locator(".sandbox-step.is-running .sandbox-step-badge")).toHaveText("进行中");
    await expect(live.locator(".sandbox-step-title")).toHaveText(["MCP 工具 8/8", "workspace.search"]);
    const agent = dock.locator(".sandbox-line.role-agent");
    await expect(agent.locator(".sandbox-line-text")).toHaveText(/定理 3\.2/);
    await expect(agent.locator(".sandbox-turn-status")).toHaveText("已完成");
    const summary = agent.locator(".sandbox-process-summary");
    await expect(summary).toContainText("已完成 1 步");
    await expect(summary).toContainText("120/30 tokens");
    await expect(agent.locator(".sandbox-step")).toHaveCount(0);
    await summary.click();
    await expect(agent.locator(".sandbox-step-title")).toHaveText(["MCP 工具 8/8", "workspace.search"]);
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

  test("new dialogue thread clears the agent context but keeps the sandbox copy", async ({ page }) => {
    await page.locator("#sandbox-button").click();
    page.once("dialog", (dialog) => void dialog.accept());
    await Promise.all([page.waitForEvent("load"), page.getByRole("button", { name: "在本窗口打开…" }).click()]);
    await page.locator(".topbar .sandbox-banner").getByRole("button", { name: "对话" }).click();
    const dock = page.getByRole("complementary", { name: "沙箱对话" });
    await page.evaluate(() => {
      window.appHarness.sandboxRound.delayMs = 20;
      window.appHarness.sandboxRound.gapMs = 0;
    });
    const threadSelect = dock.locator(".sandbox-dock-thread-select");
    await expect(threadSelect).toBeDisabled();
    await expect(threadSelect.locator("option")).toHaveText(["对话 1（当前）"]);

    await dock.locator(".sandbox-dock-input").fill("调试旧功能");
    await dock.getByRole("button", { name: "发送" }).click();
    await expect(dock.locator(".sandbox-line.role-agent .sandbox-turn-status")).toHaveText("已完成");

    await dock.getByRole("button", { name: "新对话" }).click();
    await expect(threadSelect.locator("option")).toHaveText(["对话 1", "对话 2（当前）"]);
    await expect(threadSelect).toHaveValue("d2");
    await expect(dock.locator(".sandbox-line")).toHaveCount(0);
    await expect(dock.locator(".sandbox-empty")).toContainText("沙箱副本里的状态都还在");

    await dock.locator(".sandbox-dock-input").fill("新问题");
    await dock.getByRole("button", { name: "发送" }).click();
    await expect(dock.locator(".sandbox-line.role-agent .sandbox-turn-status")).toHaveText("已完成");
    await expect(dock.locator(".sandbox-line.role-user")).toHaveText([/新问题/]);

    // Inactive thread preview: offers both "继续此对话" and "回到当前对话".
    await threadSelect.selectOption("d1");
    await expect(dock.locator(".sandbox-line.role-user")).toHaveText([/调试旧功能/]);
    await expect(dock.locator(".sandbox-dock-readonly")).toBeVisible();
    await expect(dock.getByRole("button", { name: "继续此对话" })).toBeEnabled();
    await expect(dock.getByRole("button", { name: "发送" })).toBeDisabled();
    await dock.getByRole("button", { name: "回到当前对话" }).click();
    await expect(threadSelect).toHaveValue("d2");
    await expect(dock.locator(".sandbox-dock-readonly")).toBeHidden();
    await expect(dock.getByRole("button", { name: "发送" })).toBeEnabled();

    // Reactivate earlier thread d1 and continue chatting on d1 without mixing into d2.
    await threadSelect.selectOption("d1");
    await dock.getByRole("button", { name: "继续此对话" }).click();
    await expect(threadSelect.locator("option")).toHaveText(["对话 1（当前）", "对话 2"]);
    await expect(threadSelect).toHaveValue("d1");
    await expect(dock.locator(".sandbox-dock-readonly")).toBeHidden();
    await expect(dock.getByRole("button", { name: "发送" })).toBeEnabled();

    await dock.locator(".sandbox-dock-input").fill("回到对话1追问");
    await dock.getByRole("button", { name: "发送" }).click();
    await expect(dock.locator(".sandbox-line.role-agent .sandbox-turn-status").last()).toHaveText("已完成");
    await expect(dock.locator(".sandbox-line.role-user")).toHaveText([/调试旧功能/, /回到对话1追问/]);

    // Switching view back to d2 shows d2's isolated messages and is now inactive until switched.
    await threadSelect.selectOption("d2");
    await expect(dock.locator(".sandbox-line.role-user")).toHaveText([/新问题/]);
    await expect(dock.locator(".sandbox-dock-readonly")).toBeVisible();

    const commands = await page.evaluate(() => window.appHarness.sandboxCalls.map((c) => c.command));
    expect(commands.filter((c) => c === "new_thread")).toHaveLength(1);
    expect(commands.filter((c) => c === "activate_thread")).toHaveLength(1);
  });

  test("default desktop keeps the dock, reader and wrapped controls in separate space", async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 820 });
    await page.evaluate(() => localStorage.setItem("mathscape.theme", "parchment"));
    await page.locator("#sandbox-button").click();
    page.once("dialog", (dialog) => void dialog.accept());
    await Promise.all([page.waitForEvent("load"), page.getByRole("button", { name: "在本窗口打开…" }).click()]);
    await page.locator(".topbar .sandbox-banner").getByRole("button", { name: "对话" }).click();
    const dock = page.getByRole("complementary", { name: "沙箱对话" });
    const grid = page.locator(".workspace-grid");
    const canvas = page.locator(".canvas-panel");
    await expect(grid).toHaveClass(/sources-compact/);
    await expect(grid).toHaveClass(/sources-collapsed/);
    const main = (await canvas.boundingBox())!;
    expect(main.width).toBeGreaterThanOrEqual(500);
    const side = (await dock.boundingBox())!;
    const inspector = (await page.locator(".detail-panel").boundingBox())!;
    expect(main.x + main.width).toBeLessThanOrEqual(inspector.x + 1);
    expect(inspector.x + inspector.width).toBeLessThanOrEqual(side.x + 1);
    const toolbar = (await page.locator(".canvas-toolbar").boundingBox())!;
    expect(toolbar.height).toBeLessThan(150);
    for (const button of await page.locator(".preset-button").all()) {
      const box = (await button.boundingBox())!;
      expect(box.x).toBeGreaterThanOrEqual(toolbar.x);
      expect(box.x + box.width).toBeLessThanOrEqual(toolbar.x + toolbar.width);
      expect(box.y + box.height).toBeLessThanOrEqual(toolbar.y + toolbar.height);
    }
    const headerFits = await page.locator(".topbar").evaluate((header) => {
      const boxes = Array.from(header.children).map((child) => child.getBoundingClientRect());
      return boxes.every((box, index) => !index || box.left >= boxes[index - 1].right - 1);
    });
    expect(headerFits).toBe(true);
    await page.locator(".view-menu-button").click();
    await expect(page.locator("#learning-view-button")).toBeVisible();
    await page.keyboard.press("Escape");

    // The temporary source drawer remains usable without narrowing the content
    // or overwriting the normal directory preference.
    const sourceToggle = page.locator("#toggle-sources");
    await sourceToggle.click();
    await expect(sourceToggle).toHaveAttribute("aria-expanded", "true");
    expect((await canvas.boundingBox())!.width).toBe(main.width);
    expect(await page.evaluate(() => localStorage.getItem("mathscape:sources-collapsed"))).toBeNull();
    await page.keyboard.press("Escape");
    await expect(sourceToggle).toHaveAttribute("aria-expanded", "false");
    await page.evaluate(() => window.dispatchEvent(new CustomEvent("mathscape:open-source", {
      detail: { artifactId: "md-1", conversionId: "c1", title: "正确表述.md" },
    })));
    const reader = page.locator("#reader");
    await expect(reader).toBeVisible();
    const paper = (await reader.boundingBox())!;
    expect((await page.locator(".canvas-toolbar").boundingBox())!.height).toBeLessThan(150);
    expect(paper.width).toBeGreaterThanOrEqual(500);
    expect(paper.y).toBeGreaterThanOrEqual(toolbar.y + toolbar.height - 1);
    expect(paper.x + paper.width).toBeLessThanOrEqual(side.x);

    await dock.getByRole("button", { name: "关闭对话", exact: true }).click();
    await expect(grid).not.toHaveClass(/sources-compact/);
    await expect(sourceToggle).toHaveAttribute("aria-expanded", "true");
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
    await dock.locator(".sandbox-dock-input").fill("紧集在哪？");
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

    await dock.locator(".sandbox-dock-input").fill(`看看 ${unitId} 和 $x^2$`);
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

  test("Composer chips: known ids chip in the input, send expands them, unknown ids stay text", async ({ page }) => {
    await page.locator("#sandbox-button").click();
    page.once("dialog", (dialog) => void dialog.accept());
    await Promise.all([page.waitForEvent("load"), page.getByRole("button", { name: "在本窗口打开…" }).click()]);
    await page.locator(".topbar .sandbox-banner").getByRole("button", { name: "对话" }).click();
    const dock = page.getByRole("complementary", { name: "沙箱对话" });
    const unitId = await page.evaluate(() => window.appHarness.citeUnitId);
    const stranger = `mathscape:source-unit/${"f".repeat(64)}`;
    const input = dock.locator(".sandbox-dock-input");
    await expect(input).toHaveClass(/is-empty/);

    // Paste: the landscape-known id becomes a short-label chip, the unknown one stays text.
    await input.click();
    await input.evaluate((el, text) => {
      const data = new DataTransfer();
      data.setData("text/plain", text);
      el.dispatchEvent(new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true }));
    }, `看看 ${unitId} 和 ${stranger}`);
    const chip = input.locator(".sandbox-composer-chip");
    await expect(chip).toHaveCount(1);
    await expect(chip).toHaveText("引理 1.4");
    await expect(chip).toHaveAttribute("data-cite-id", unitId);
    await expect(chip).toHaveAttribute("contenteditable", "false");
    await expect(input).toContainText(stranger);
    await expect(input).not.toContainText(unitId);

    // Copy puts the full ids on the clipboard.
    const copied = await input.evaluate((el) => {
      const range = document.createRange();
      range.selectNodeContents(el);
      const selection = window.getSelection()!;
      selection.removeAllRanges();
      selection.addRange(range);
      const data = new DataTransfer();
      el.dispatchEvent(new ClipboardEvent("copy", { clipboardData: data, bubbles: true, cancelable: true }));
      return data.getData("text/plain");
    });
    expect(copied).toBe(`看看 ${unitId} 和 ${stranger}`);

    // Clicking the chip jumps the map to its node.
    await chip.click();
    await expect.poll(() => page.evaluate(() => window.appHarness.nodeView("r-cite-unit")?.selected)).toBe(true);

    // Typing: an id is chipped once a delimiter follows it.
    await input.evaluate((el) => {
      const range = document.createRange();
      range.selectNodeContents(el);
      range.collapse(false);
      window.getSelection()!.removeAllRanges();
      window.getSelection()!.addRange(range);
    });
    await page.keyboard.type(` 再看 ${unitId}`);
    await expect(chip).toHaveCount(1);
    await page.keyboard.type(" ");
    await expect(chip).toHaveCount(2);

    // Backspace removes a chip whole (caret right after the second chip, past the space).
    await page.keyboard.press("Backspace");
    await page.keyboard.press("Backspace");
    await expect(chip).toHaveCount(1);
    await page.keyboard.type("。");
    // Shift+Enter is a line break, not a send.
    await page.keyboard.press("Shift+Enter");
    await page.keyboard.type("第二行");

    // Double-click turns a chip back into its raw id for editing.
    await chip.first().dblclick();
    await expect(chip).toHaveCount(0);
    await expect(input).toContainText(unitId);
    // Leaving the composer chips it again.
    await dock.locator(".sandbox-dock-status").click();
    await expect(chip).toHaveCount(1);

    await page.keyboard.press("Tab");
    await input.focus();
    await page.keyboard.press("Enter");
    await expect.poll(() => page.evaluate(() => window.appHarness.sandboxCalls.filter((c) => c.command === "send").map((c) => c.args.text)))
      .toEqual([`看看 ${unitId} 和 ${stranger} 再看 。\n第二行`]);
    await expect(input).toHaveClass(/is-empty/);
    await expect(input.locator(".sandbox-composer-chip")).toHaveCount(0);
    // The sent bubble renders the full id as a reply-style chip.
    await expect(dock.locator(".sandbox-line.role-user").last().locator("button.sandbox-cite.is-map")).toHaveText("引理 1.4");
  });

  test("Composer chips: token model — trailing space, ←/→ select the chip, Backspace/Delete remove it whole", async ({ page }) => {
    await page.locator("#sandbox-button").click();
    page.once("dialog", (dialog) => void dialog.accept());
    await Promise.all([page.waitForEvent("load"), page.getByRole("button", { name: "在本窗口打开…" }).click()]);
    await page.locator(".topbar .sandbox-banner").getByRole("button", { name: "对话" }).click();
    const dock = page.getByRole("complementary", { name: "沙箱对话" });
    const unitId = await page.evaluate(() => window.appHarness.citeUnitId);
    const input = dock.locator(".sandbox-dock-input");
    const chip = input.locator(".sandbox-composer-chip");
    const selected = input.locator(".sandbox-composer-chip.is-selected");
    // Where the caret is: collapsed in a text node (an element-level position
    // right after a chip is painted at line start in WebKit), its x versus
    // the chips, the visible text, and whether any U+FEFF anchor is in the DOM.
    const caretInfo = () => input.evaluate((el) => {
      const range = window.getSelection()!.getRangeAt(0);
      const rect = range.collapsed ? range.getClientRects()[0] ?? null : null;
      const chips = Array.from(el.querySelectorAll(".sandbox-composer-chip")).map((c) => c.getBoundingClientRect());
      return {
        collapsed: range.collapsed,
        inText: range.collapsed && range.startContainer.nodeType === Node.TEXT_NODE && el.contains(range.startContainer),
        x: rect ? rect.left : null,
        chipLeft: chips.map((c) => c.left),
        chipRight: chips.map((c) => c.right),
        text: el.textContent ?? "",
        anchors: ((el.textContent ?? "").match(/\uFEFF/g) ?? []).length,
      };
    });
    const paste = (text: string) => input.evaluate((el, text) => {
      const data = new DataTransfer();
      data.setData("text/plain", text);
      el.dispatchEvent(new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true }));
    }, text);
    const reset = async () => {
      await input.evaluate((el) => el.replaceChildren());
      await input.focus();
    };
    /** Caret collapsed in text right of chip `i` (and no anchor needed for it). */
    const expectAfter = async (i: number) => {
      const info = await caretInfo();
      expect(info.collapsed).toBe(true);
      expect(info.inText).toBe(true);
      expect(info.x).not.toBeNull();
      expect(info.x!).toBeGreaterThanOrEqual(info.chipRight[i] - 1);
      return info;
    };

    // Paste an id alone, then with text before it: chip + one space, caret after the space, no anchors.
    await input.click();
    for (const text of [unitId, `看看 ${unitId}`]) {
      await reset();
      await paste(text);
      await expect(chip).toHaveCount(1);
      const info = await expectAfter(0);
      expect(info.text).toBe(text === unitId ? "引理 1.4 " : "看看 引理 1.4 ");
      expect(info.anchors).toBe(0);
      expect(info.x!).toBeGreaterThan(info.chipRight[0] + 1); // past the space
    }
    // Typing continues after the space.
    await page.keyboard.type("好");
    await expect.poll(async () => (await caretInfo()).text).toBe("看看 引理 1.4 好");

    // ← from after the space: caret right after the chip, then the chip selected, then before it.
    await reset();
    await paste(`看看 ${unitId}`);
    await page.keyboard.press("ArrowLeft");
    const justAfter = await expectAfter(0);
    expect(justAfter.x!).toBeLessThanOrEqual(justAfter.chipRight[0] + 2);
    await expect(selected).toHaveCount(0);
    await page.keyboard.press("ArrowLeft");
    await expect(selected).toHaveCount(1);
    expect((await caretInfo()).collapsed).toBe(false);
    await page.keyboard.press("ArrowLeft");
    await expect(selected).toHaveCount(0);
    const before = await caretInfo();
    expect(before.collapsed).toBe(true);
    expect(before.x!).toBeLessThanOrEqual(before.chipLeft[0] + 1);
    // → mirrors: select, then right after the chip, then past the space (native).
    await page.keyboard.press("ArrowRight");
    await expect(selected).toHaveCount(1);
    await page.keyboard.press("ArrowRight");
    await expect(selected).toHaveCount(0);
    const back = await expectAfter(0);
    expect(back.x!).toBeLessThanOrEqual(back.chipRight[0] + 2);
    await page.keyboard.press("ArrowRight");
    expect((await expectAfter(0)).x!).toBeGreaterThan(back.x! + 1);

    // Backspace on the selected chip removes it whole; re-pasting lands after the chip again.
    await page.keyboard.press("ArrowLeft");
    await page.keyboard.press("ArrowLeft");
    await expect(selected).toHaveCount(1);
    await page.keyboard.press("Backspace");
    await expect(chip).toHaveCount(0);
    await expect.poll(async () => (await caretInfo()).text).toBe("看看  ");
    await paste(unitId);
    await expect(chip).toHaveCount(1);
    const repasted = await expectAfter(0);
    expect(repasted.text).toBe("看看 引理 1.4 "); // the existing space is reused
    expect(repasted.anchors).toBe(0);

    // Delete everything, then paste again: the caret is after the new chip, not at line start.
    await reset();
    await paste(unitId);
    await page.keyboard.press("Backspace"); // the space
    await page.keyboard.press("Backspace"); // the chip, whole
    await expect(chip).toHaveCount(0);
    expect((await caretInfo()).text).toBe("");
    await paste(unitId);
    await expect(chip).toHaveCount(1);
    expect((await expectAfter(0)).text).toBe("引理 1.4 ");
    await paste(unitId);
    await expect(chip).toHaveCount(2);
    await expectAfter(1);

    // Delete with the caret right before a chip removes it whole.
    await reset();
    await paste(unitId);
    for (let i = 0; i < 3; i += 1) await page.keyboard.press("ArrowLeft"); // after → selected → before
    await expect(selected).toHaveCount(0);
    await page.keyboard.press("Delete");
    await expect(chip).toHaveCount(0);
    expect((await caretInfo()).text).toBe(" ");

    // Typing over a selected chip replaces it.
    await reset();
    await paste(`看看 ${unitId}`);
    await page.keyboard.press("ArrowLeft");
    await page.keyboard.press("ArrowLeft");
    await expect(selected).toHaveCount(1);
    await page.keyboard.type("它");
    await expect(chip).toHaveCount(0);
    await expect.poll(async () => (await caretInfo()).text).toBe("看看 它 ");

    // Two chips: the caret follows the second; Backspace ×2 removes its space then the chip whole.
    await reset();
    await paste(`${unitId} ${unitId}`);
    await expect(chip).toHaveCount(2);
    await expectAfter(1);
    await page.keyboard.press("Backspace");
    await page.keyboard.press("Backspace");
    await expect(chip).toHaveCount(1);
    await expectAfter(0);

    await page.keyboard.press("Enter");
    await expect.poll(() => page.evaluate(() => window.appHarness.sandboxCalls.filter((c) => c.command === "send").map((c) => c.args.text)))
      .toEqual([unitId]); // send trims
  });

  test("Composer @ mentions: @ opens landscape suggestions, keywords filter, picking inserts a full-id chip", async ({ page }) => {
    await page.locator("#sandbox-button").click();
    page.once("dialog", (dialog) => void dialog.accept());
    await Promise.all([page.waitForEvent("load"), page.getByRole("button", { name: "在本窗口打开…" }).click()]);
    await page.locator(".topbar .sandbox-banner").getByRole("button", { name: "对话" }).click();
    const dock = page.getByRole("complementary", { name: "沙箱对话" });
    const { unitId, artId } = await page.evaluate(() => ({ unitId: window.appHarness.citeUnitId, artId: window.appHarness.citeArtId }));
    const input = dock.locator(".sandbox-dock-input");
    const chip = input.locator(".sandbox-composer-chip");
    const menu = page.locator(".sandbox-composer-menu");
    const options = menu.getByRole("option");
    const sends = () => page.evaluate(() => window.appHarness.sandboxCalls.filter((c) => c.command === "send").map((c) => c.args.text));
    const caretInfo = () => input.evaluate((el) => {
      const range = window.getSelection()!.getRangeAt(0);
      const rect = range.getClientRects()[0] ?? null;
      const chips = Array.from(el.querySelectorAll(".sandbox-composer-chip")).map((c) => c.getBoundingClientRect());
      return {
        inText: range.collapsed && range.startContainer.nodeType === Node.TEXT_NODE && el.contains(range.startContainer),
        x: rect ? rect.left : null,
        chipRight: chips.map((c) => c.right),
      };
    });

    await input.click();
    await expect(menu).toHaveCount(0);
    // Canonical-id map nodes (2) + active 32-hex live ink notes (2) are offered.
    // Commit the CJK prefix like an IME; zero-delay repeated WebKit insertText
    // commands can leave the caret between the identical characters.
    await page.keyboard.insertText("看看 ");
    await page.keyboard.type("@");
    await expect(menu).toBeVisible();
    await expect(options).toHaveCount(4);
    await expect(input).toHaveAttribute("aria-expanded", "true");
    // Filter by label, by type (中文 kind label) and by short id.
    await page.keyboard.type("紧集");
    await expect(options).toHaveCount(1);
    await expect(options.first().locator(".sandbox-composer-menu-label")).toHaveText("紧集讲义.pdf");
    await expect(options.first().locator(".sandbox-composer-menu-meta")).toHaveText(/材料文件 · artifact\/cd34cd34…/);
    for (let i = 0; i < 2; i += 1) await page.keyboard.press("Backspace");
    await expect(options).toHaveCount(4);
    await page.keyboard.type("内容单元");
    await expect(options).toHaveCount(1);
    await expect(options.first()).toHaveAttribute("data-domain-id", unitId);
    for (let i = 0; i < 4; i += 1) await page.keyboard.press("Backspace");
    await page.keyboard.type("ab12ab");
    await expect(options).toHaveCount(1);
    await expect(options.first()).toHaveAttribute("data-domain-id", unitId);
    await page.keyboard.type("zz");
    await expect(menu).toHaveCount(0);
    for (let i = 0; i < 8; i += 1) await page.keyboard.press("Backspace");
    await expect(options).toHaveCount(4);

    // ↑/↓ move the active option (wrapping); Enter picks it and does not send.
    await expect(options.nth(0)).toHaveAttribute("aria-selected", "true");
    await page.keyboard.press("ArrowDown");
    await expect(options.nth(1)).toHaveAttribute("aria-selected", "true");
    await page.keyboard.press("ArrowDown");
    await expect(options.nth(2)).toHaveAttribute("aria-selected", "true");
    await page.keyboard.press("ArrowDown");
    await expect(options.nth(3)).toHaveAttribute("aria-selected", "true");
    await page.keyboard.press("ArrowDown");
    await expect(options.nth(0)).toHaveAttribute("aria-selected", "true");
    await page.keyboard.press("ArrowUp");
    await expect(options.nth(3)).toHaveAttribute("aria-selected", "true");
    await page.keyboard.press("ArrowDown");
    await expect(options.nth(0)).toHaveAttribute("aria-selected", "true");
    await page.keyboard.type("引理");
    await expect(options).toHaveCount(1);
    await page.keyboard.press("Enter");
    await expect(menu).toHaveCount(0);
    await expect(chip).toHaveCount(1);
    await expect(chip).toHaveText("引理 1.4");
    await expect(chip).toHaveAttribute("data-cite-id", unitId);
    await expect(chip).toHaveAttribute("data-cite-node", "r-cite-unit");
    expect(await sends()).toEqual([]);
    // The caret sits in text after the chip (and the space the pick added).
    const info = await caretInfo();
    expect(info.inText).toBe(true);
    expect(info.x!).toBeGreaterThanOrEqual(info.chipRight[0] - 1);
    await page.keyboard.type("好");
    await expect.poll(() => input.evaluate((el) => (el.textContent ?? "").replace(/\uFEFF/g, ""))).toBe("看看 引理 1.4 好");

    // Tab picks too; a CJK character right before @ still opens the menu.
    await page.keyboard.type(" 和@讲义");
    await expect(options).toHaveCount(1);
    await page.keyboard.press("Tab");
    await expect(chip).toHaveCount(2);
    await expect(chip.nth(1)).toHaveAttribute("data-cite-id", artId);
    await expect(input).toBeFocused();
    const second = await caretInfo();
    expect(second.inText).toBe(true);
    expect(second.x!).toBeGreaterThanOrEqual(second.chipRight[1] - 1);

    // A mouse pick keeps focus in the composer.
    await page.keyboard.type("@");
    await options.filter({ hasText: "引理 1.4" }).click();
    await expect(chip).toHaveCount(3);
    await expect(chip.nth(2)).toHaveAttribute("data-cite-id", unitId);
    await expect(input).toBeFocused();

    // An email-ish @ does not open the menu.
    await page.keyboard.type("a@b");
    await expect(menu).toHaveCount(0);
    for (let i = 0; i < 3; i += 1) await page.keyboard.press("Backspace");

    // Click-away dismisses.
    await page.keyboard.type("@");
    await expect(menu).toBeVisible();
    await dock.locator(".sandbox-dock-status").click();
    await expect(menu).toHaveCount(0);
    // Esc dismisses (without closing the dock) and that @ stays closed while typing on.
    await input.evaluate((el) => {
      el.focus();
      const range = document.createRange();
      range.selectNodeContents(el);
      range.collapse(false);
      window.getSelection()!.removeAllRanges();
      window.getSelection()!.addRange(range);
    });
    await expect(menu).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(menu).toHaveCount(0);
    await expect(dock).toBeVisible();
    await page.keyboard.type("引");
    await expect(menu).toHaveCount(0);
    await page.keyboard.press("Backspace");
    await page.keyboard.press("Backspace");

    // Paste chipify and Backspace-deletes-chip still work alongside mentions.
    await page.keyboard.press("Backspace");
    await page.keyboard.press("Backspace");
    await expect(chip).toHaveCount(2);
    await page.keyboard.type(" ");

    // Send expands every chip back to its full id.
    await page.keyboard.press("Enter");
    await expect.poll(sends).toEqual([`看看 ${unitId} 好 和${artId}`]);
    await expect(input).toHaveClass(/is-empty/);
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

    await dock.locator(".sandbox-dock-input").fill(`看看 ${unitId}`);
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
  /** Enter the sandbox, open the sidebar dock, set the harness round. */
  async function sandboxDock(page: import("@playwright/test").Page, round: Record<string, unknown>) {
    await page.locator("#sandbox-button").click();
    page.once("dialog", (dialog) => void dialog.accept());
    await Promise.all([page.waitForEvent("load"), page.getByRole("button", { name: "在本窗口打开…" }).click()]);
    await page.locator(".topbar .sandbox-banner").getByRole("button", { name: "对话" }).click();
    const dock = page.getByRole("complementary", { name: "沙箱对话" });
    await expect(dock).toHaveClass(/is-sidebar/);
    await page.evaluate((value) => { Object.assign(window.appHarness.sandboxRound, value); }, round);
    return dock;
  }

  test("Scene Act: workspace.search types the query in the HUD, then hits land", async ({ page }) => {
    const dock = await sandboxDock(page, { act: "search", delayMs: 30, gapMs: 2500 });
    await dock.locator(".sandbox-dock-input").fill("搜一下紧集");
    await dock.getByRole("button", { name: "发送" }).click();
    const hud = page.locator(".canvas-panel .scene-hud");
    const typed = hud.locator(".scene-hud-type");
    await expect(typed).toBeVisible();
    await expect(hud.locator(".scene-hud-title")).toHaveText("搜索 · 紧集 开覆盖");
    // Mid-typing the line holds a prefix of the query; then the whole query.
    await expect(typed).toHaveClass(/is-typing/);
    const partial = await typed.locator(".scene-hud-type-text").textContent();
    expect("紧集 开覆盖".startsWith(partial ?? "x")).toBe(true);
    await expect(typed.locator(".scene-hud-type-text")).toHaveText("紧集 开覆盖");
    await expect(typed).not.toHaveClass(/is-typing/);
    // Nothing is lit before the result; then the hits light on the map.
    expect(await page.evaluate(() => window.appHarness.nodeClasses("r-cite-unit"))).not.toContain("sandbox-echo-hit");
    await expect.poll(() => page.evaluate(() => window.appHarness.nodeClasses("r-cite-unit")), { timeout: 6000 }).toContain("sandbox-echo-hit");
    await expect(typed.locator(".scene-hud-type-text")).toHaveText("紧集 开覆盖");
    await expect(hud).toBeHidden({ timeout: 8000 });
    await expect(typed).toBeHidden();
  });

  test("Scene clump: a sparse multi-node focus is pulled together while playing and glides back on settle; replay clumps again", async ({ page }) => {
    const dock = await sandboxDock(page, { act: "search", delayMs: 30, gapMs: 1500 });
    const unit = "r-cite-unit";
    const pdf = "r-pdf-notes";
    // Make the two search hits a sparse pair.
    const start = await page.evaluate(({ unit }) => window.appHarness.nodePosition(unit), { unit });
    expect(start).not.toBeNull();
    await page.evaluate(({ pdf, x, y }) => window.appHarness.setNodePosition(pdf, x, y), { pdf, x: start!.x + 1400, y: start!.y + 600 });
    const before = await page.evaluate(({ unit, pdf }) => ({ unit: window.appHarness.nodePosition(unit)!, pdf: window.appHarness.nodePosition(pdf)! }), { unit, pdf });
    const gap = (p: { unit: { x: number; y: number }; pdf: { x: number; y: number } }) => Math.hypot(p.unit.x - p.pdf.x, p.unit.y - p.pdf.y);
    const positions = () => page.evaluate(({ unit, pdf }) => ({ unit: window.appHarness.nodePosition(unit)!, pdf: window.appHarness.nodePosition(pdf)! }), { unit, pdf });
    const viewBefore = await page.evaluate(() => window.appHarness.viewport());

    await dock.locator(".sandbox-dock-input").fill("搜一下紧集");
    await dock.getByRole("button", { name: "发送" }).click();
    const hud = page.locator(".canvas-panel .scene-hud");
    await expect.poll(() => page.evaluate(() => window.appHarness.nodeClasses("r-cite-unit")), { timeout: 6000 }).toContain("sandbox-echo-hit");
    // While the hits are lit the pair sits much closer together (capped pull).
    await expect.poll(async () => gap(await positions())).toBeLessThan(gap(before) * 0.75);
    const during = await positions();
    expect(Math.hypot(during.pdf.x - before.pdf.x, during.pdf.y - before.pdf.y)).toBeLessThanOrEqual(521);
    // Settle: positions tween back to the snapshot, camera returns.
    await expect(hud).toBeHidden({ timeout: 8000 });
    await expect.poll(positions).toEqual(before);
    await expect.poll(() => page.evaluate(() => window.appHarness.viewport())).toEqual(viewBefore);

    // Replay takes the same path: clumps again, 停止 brings everything home.
    const agent = dock.locator(".sandbox-line.role-agent").last();
    await expect(agent.locator(".sandbox-turn-status")).toHaveText("已完成");
    await agent.locator(".sandbox-process-summary").click();
    await agent.locator(".sandbox-step-replay").last().click();
    await expect(hud).toBeVisible();
    await expect.poll(async () => gap(await positions())).toBeLessThan(gap(before) * 0.75);
    await hud.getByRole("button", { name: "停止" }).click();
    await expect(hud).toBeHidden();
    await expect.poll(positions).toEqual(before);
    expect(await page.evaluate(() => window.appHarness.viewport())).toEqual(viewBefore);
  });

  test("Scene Act: pdf.outline opens a 目录 window beside the PDF; settle closes it; replay reopens it", async ({ page }) => {
    const dock = await sandboxDock(page, { act: "outline", delayMs: 30, gapMs: 400 });
    await dock.locator(".sandbox-dock-input").fill("看看目录");
    await dock.getByRole("button", { name: "发送" }).click();
    const hud = page.locator(".canvas-panel .scene-hud");
    await expect(hud.locator(".scene-hud-title")).toHaveText("PDF 目录 · notes.pdf");
    const win = page.locator(".canvas-panel .scene-act[data-act='outline']");
    await expect(win).toBeVisible();
    await expect(win).toHaveClass(/is-anchored/);
    await expect(win).toHaveAttribute("data-anchor", "r-pdf-notes");
    await expect(win.locator(".scene-act-row")).toHaveCount(2);
    await expect(win.locator(".scene-act-row").nth(0)).toContainText("第一章 拓扑空间");
    await expect(win.locator(".scene-act-row").nth(0)).toContainText("第 1 页");
    await expect(win.locator(".scene-act-row").nth(1)).toContainText("1.1 紧集");
    await expect(win.locator(".scene-act-row").nth(1)).toContainText("第 2 页");
    await expect(win.locator(".scene-act-row").nth(1)).toHaveAttribute("data-level", "2");
    expect(await page.evaluate(() => window.appHarness.nodeClasses("r-pdf-notes"))).toContain("sandbox-echo-seed");
    await expect(hud).toBeHidden({ timeout: 10000 });
    await expect(win).toHaveCount(0);

    // Replay from the process card rebuilds the window from the round's events.
    const agent = dock.locator(".sandbox-line.role-agent").last();
    await expect(agent.locator(".sandbox-turn-status")).toHaveText("已完成");
    await agent.locator(".sandbox-process-replay").click();
    await expect(win).toBeVisible();
    await expect(win.locator(".scene-act-row")).toHaveCount(2);
    await hud.getByRole("button", { name: "停止" }).click();
    await expect(win).toHaveCount(0);
  });

  test("Sync ensure: a scanned-page material.read waiting on Marker plays as 解析中 until the body lands", async ({ page }) => {
    const dock = await sandboxDock(page, { act: "parsing", delayMs: 30, gapMs: 6500 });
    await dock.locator(".sandbox-dock-input").fill("读第 3–4 页");
    await dock.getByRole("button", { name: "发送" }).click();
    const hud = page.locator(".canvas-panel .scene-hud");
    await expect(hud.locator(".scene-hud-title")).toHaveText("阅读材料 · notes.pdf · 第 3–4 页");
    // The map PDF node says 解析中 → the in-flight read becomes a parsing block.
    const chip = hud.locator(".scene-hud-parsing");
    await expect(chip).toBeVisible({ timeout: 6000 });
    await expect(chip).toHaveText("解析中");
    await expect(hud).toHaveClass(/is-parsing/);
    await expect(hud.locator(".scene-hud-summary")).toContainText("解析中 第3–4页 · 等待 Marker 完成");
    expect(await page.evaluate(() => window.appHarness.nodeClasses("r-pdf-notes"))).toContain("scene-parsing");
    await expect(dock.locator(".sandbox-step-title").filter({ hasText: "解析中" })).toHaveCount(1);
    // The result lands: the chip goes, the result frame records the parse.
    await expect(hud.locator(".scene-hud-summary")).toContainText("解析完成 第3–4页", { timeout: 8000 });
    await expect(chip).toBeHidden();
    await expect(dock.locator(".sandbox-step-title").filter({ hasText: "解析中" })).toHaveCount(0);
    await expect(hud).toBeHidden({ timeout: 10000 });
    expect(await page.evaluate(() => window.appHarness.nodeClasses("r-pdf-notes"))).not.toContain("scene-parsing");
  });

  test("Scene Act: ink.page_image shows the page image at the stage corner (note is not a map node)", async ({ page }) => {
    const dock = await sandboxDock(page, { act: "page_image", delayMs: 30, gapMs: 400 });
    await dock.locator(".sandbox-dock-input").fill("看看手稿");
    await dock.getByRole("button", { name: "发送" }).click();
    const hud = page.locator(".canvas-panel .scene-hud");
    await expect(hud.locator(".scene-hud-title")).toHaveText("手稿页图 · 手稿 A · 第 1/3 页");
    const win = page.locator(".canvas-panel .scene-act[data-act='page_image']");
    await expect(win).toBeVisible();
    await expect(win).toHaveClass(/is-stage/);
    await expect(win.locator(".scene-act-title")).toHaveText("手稿 A");
    await expect(win.locator(".scene-act-meta")).toHaveText("第 1/3 页");
    const img = win.locator("img.scene-act-image");
    await expect(img).toHaveAttribute("src", /^data:image\/png;base64,/);
    await expect.poll(() => img.evaluate((node: HTMLImageElement) => node.naturalWidth)).toBe(1);
    // No fake map node for the note's document id.
    expect(await page.evaluate(() => window.appHarness.nodeClasses("ink-doc-1"))).toBeNull();
    await expect(hud).toBeHidden({ timeout: 10000 });
    await expect(win).toHaveCount(0);
  });

  test("Scene Act: ink.page_image without image bytes degrades to text meta", async ({ page }) => {
    const dock = await sandboxDock(page, { act: "page_image_meta", delayMs: 30, gapMs: 400 });
    await dock.locator(".sandbox-dock-input").fill("看看手稿");
    await dock.getByRole("button", { name: "发送" }).click();
    const hud = page.locator(".canvas-panel .scene-hud");
    await expect(hud.locator(".scene-hud-summary")).toContainText("仅元数据（无页图）");
    const win = page.locator(".canvas-panel .scene-act[data-act='page_image']");
    await expect(win).toBeVisible();
    await expect(win.locator(".scene-act-empty")).toHaveText("未取得页图（仅元数据）");
    await expect(win.locator("img")).toHaveCount(0);
  });

  test("Scene Act: pdf.page_image shows the page image beside the PDF node", async ({ page }) => {
    const dock = await sandboxDock(page, { act: "pdf_page_image", delayMs: 30, gapMs: 400 });
    await dock.locator(".sandbox-dock-input").fill("看看 PDF 页");
    await dock.getByRole("button", { name: "发送" }).click();
    const hud = page.locator(".canvas-panel .scene-hud");
    await expect(hud.locator(".scene-hud-title")).toContainText("PDF 页图");
    await expect(hud.locator(".scene-hud-title")).toContainText("第 2/5 页");
    const win = page.locator(".canvas-panel .scene-act[data-act='page_image']");
    await expect(win).toBeVisible();
    await expect(win.locator(".scene-act-title")).toHaveText("PDF 页图");
    await expect(win.locator(".scene-act-meta")).toHaveText("第 2/5 页");
    const img = win.locator("img.scene-act-image");
    await expect(img).toHaveAttribute("src", /^data:image\/png;base64,/);
    await expect.poll(() => img.evaluate((node: HTMLImageElement) => node.naturalWidth)).toBe(1);
    await expect(hud).toBeHidden({ timeout: 10000 });
    await expect(win).toHaveCount(0);
  });

  test("Scene Act: pdf.page_image without image bytes degrades to text meta", async ({ page }) => {
    const dock = await sandboxDock(page, { act: "pdf_page_image_meta", delayMs: 30, gapMs: 400 });
    await dock.locator(".sandbox-dock-input").fill("看看 PDF 页");
    await dock.getByRole("button", { name: "发送" }).click();
    const hud = page.locator(".canvas-panel .scene-hud");
    await expect(hud.locator(".scene-hud-summary")).toContainText("仅元数据（无页图）");
    const win = page.locator(".canvas-panel .scene-act[data-act='page_image']");
    await expect(win).toBeVisible();
    await expect(win.locator(".scene-act-empty")).toHaveText("未取得页图（仅元数据）");
    await expect(win.locator("img")).toHaveCount(0);
  });

});
