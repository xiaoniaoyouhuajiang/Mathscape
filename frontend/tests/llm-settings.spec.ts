import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/llm-settings.html");
  await page.waitForFunction(() => "harness" in window);
});

test("saves a relay profile, redacts the key and verifies connectivity", async ({ page }) => {
  await expect(page.locator("#settings-label")).toHaveText("设置");
  await page.locator("#settings-trigger").click();
  await page.getByLabel("配置名称", { exact: true }).fill("主力中转站");
  await expect(page.getByRole("heading", { name: "Mathscape 设置" })).toBeVisible();
  await page.getByLabel("供应商").selectOption("custom-relay");
  await expect(page.getByLabel("通信协议")).toHaveValue("open_ai_chat_completions");
  await page.getByLabel("模型 ID").fill("relay-model");
  await page.getByLabel("Base URL（可选）").fill("https://relay.example/v1");
  await page.getByLabel("API key").fill("fixture-secret");
  await page.getByRole("button", { name: "保存并测试" }).click();

  await expect(page.locator(".llm-settings-result")).toContainText("连接成功");
  await expect(page.locator(".llm-settings-result")).toContainText("MATHSCAPE_OK");
  await expect(page.getByLabel("API key")).toHaveValue("");
  await expect(page.locator("#settings-label")).toHaveText("设置");
  const request = await page.evaluate(() => (window as any).harness.saves[0]);
  expect(request.api_key).toBe("fixture-secret");
  expect(request.endpoint).toBe("https://relay.example/v1");
  expect(JSON.stringify(await page.evaluate(() => (window as any).harness.controller)))
    .not.toContain("fixture-secret");
});

test("persists workspace review and timeout policy independently of the LLM profile", async ({ page }) => {
  await page.locator("#settings-trigger").click();
  await expect(page.getByLabel("默认审核方式")).toHaveValue("review_before_publish");
  await expect(page.getByLabel("内容发现请求总超时（秒）")).toHaveValue("600");
  await page.getByLabel("默认审核方式").selectOption("ask_each_run");
  await page.getByLabel("内容发现请求总超时（秒）").fill("900");
  await page.getByText("高级连接设置").click();
  await page.getByLabel("连接超时（秒）").fill("20");
  await page.getByRole("button", { name: "保存运行策略" }).click();

  await expect(page.locator(".workspace-settings-result")).toContainText("运行策略已保存");
  const request = await page.evaluate(() => (window as any).harness.workspaceSaves[0]);
  expect(request).toEqual({
    discovery_admission: "ask_each_run",
    discovery_timeout_seconds: 900,
    connect_timeout_seconds: 20,
    loogle_resident: true,
  });
  expect(await page.evaluate(() => (window as any).harness.saves.length)).toBe(0);
});

test("Loogle 常驻模式 defaults on and saves off with the workspace policy", async ({ page }) => {
  await page.locator("#settings-trigger").click();
  const toggle = page.getByLabel("Loogle 常驻模式");
  await expect(toggle).toBeChecked();
  await expect(toggle).toBeEnabled();
  await expect(page.locator(".loogle-resident-note")).toContainText("空闲 15 分钟自动关闭");
  await toggle.uncheck();
  await page.getByRole("button", { name: "保存运行策略" }).click();
  await expect(page.locator(".workspace-settings-result")).toContainText("运行策略已保存");
  const request = await page.evaluate(() => (window as any).harness.workspaceSaves[0]);
  expect(request.loogle_resident).toBe(false);
  await page.getByRole("button", { name: "关闭" }).click();
  await page.locator("#settings-trigger").click();
  await expect(page.getByLabel("Loogle 常驻模式")).not.toBeChecked();
});

test("Loogle 常驻模式 is locked when the environment forces it off", async ({ page }) => {
  await page.evaluate(() => (window as any).harness.forceLoogleOff());
  await page.locator("#settings-trigger").click();
  await expect(page.getByLabel("Loogle 常驻模式")).toBeDisabled();
  await expect(page.locator(".loogle-resident-note")).toContainText("MATHSCAPE_LOOGLE_RESIDENT");
});

test("preserves or explicitly clears a saved credential", async ({ page }) => {
  await page.locator("#settings-trigger").click();
  await page.getByLabel("配置名称", { exact: true }).fill("主力模型");
  await page.getByLabel("模型 ID").fill("first-model");
  await page.getByLabel("API key").fill("first-secret");
  await page.getByRole("button", { name: "保存配置" }).click();
  await expect(page.locator(".credential-state")).toContainText("已有密钥引用");

  await page.getByLabel("模型 ID").fill("second-model");
  await page.getByRole("button", { name: "保存配置" }).click();
  await page.getByLabel("清除当前 profile 的已保存密钥").check();
  await expect(page.getByLabel("API key")).toBeDisabled();
  await page.getByRole("button", { name: "保存配置" }).click();
  const saves = await page.evaluate(() => (window as any).harness.saves);
  expect(saves[1].api_key).toBeNull();
  expect(saves[1].clear_api_key).toBe(false);
  expect(saves[2].clear_api_key).toBe(true);
  await expect(page.locator(".credential-state")).toContainText("尚未保存密钥");
});

test("creates and edits separate named profiles without switching the default implicitly", async ({ page }) => {
  await page.locator("#settings-trigger").click();
  await page.getByLabel("配置名称", { exact: true }).fill("主配置");
  await page.getByLabel("模型 ID").fill("model-a");
  await page.getByRole("button", { name: "保存配置", exact: true }).click();
  await expect(page.getByLabel("已有配置（选择以编辑）")).toContainText("主配置 · 默认");
  await page.getByRole("button", { name: "新增配置", exact: true }).click();
  await page.getByLabel("配置名称", { exact: true }).fill("备用配置");
  await page.getByLabel("模型 ID").fill("model-b");
  await page.getByRole("button", { name: "保存配置", exact: true }).click();
  await expect(page.getByLabel("已有配置（选择以编辑）")).toContainText("主配置 · 默认");
  await page.getByRole("button", { name: "将所选配置设为默认" }).click();
  await expect(page.getByLabel("已有配置（选择以编辑）")).toContainText("备用配置 · 默认");
  await page.getByLabel("已有配置（选择以编辑）").selectOption({ label: "主配置" });
  await expect(page.getByLabel("模型 ID")).toHaveValue("model-a");
  await page.getByLabel("已有配置（选择以编辑）").selectOption("");
  await page.getByLabel("配置名称", { exact: true }).fill("第三配置");
  await page.getByLabel("模型 ID").fill("model-c");
  await page.getByRole("button", { name: "保存配置", exact: true }).click();
  await expect(page.getByLabel("已有配置（选择以编辑）").locator("option")).toHaveCount(4);
  await expect(page.getByLabel("已有配置（选择以编辑）")).toContainText("备用配置 · 默认");
});
