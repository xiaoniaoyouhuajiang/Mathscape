import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/llm-settings.html");
  await page.waitForFunction(() => "harness" in window);
});

test("saves a relay profile, redacts the key and verifies connectivity", async ({ page }) => {
  await expect(page.locator("#settings-label")).toHaveText("LLM 未配置");
  await page.getByRole("button", { name: "LLM 未配置" }).click();
  await expect(page.getByRole("heading", { name: "LLM 配置" })).toBeVisible();
  await page.getByLabel("供应商").selectOption("custom-relay");
  await expect(page.getByLabel("通信协议")).toHaveValue("open_ai_chat_completions");
  await page.getByLabel("模型 ID").fill("relay-model");
  await page.getByLabel("Base URL（可选）").fill("https://relay.example/v1");
  await page.getByLabel("API key").fill("fixture-secret");
  await page.getByRole("button", { name: "保存并测试" }).click();

  await expect(page.locator(".settings-result")).toContainText("连接成功");
  await expect(page.locator(".settings-result")).toContainText("MATHSCAPE_OK");
  await expect(page.getByLabel("API key")).toHaveValue("");
  await expect(page.locator("#settings-label")).toHaveText("中转站 · relay-model");
  const request = await page.evaluate(() => (window as any).harness.saves[0]);
  expect(request.api_key).toBe("fixture-secret");
  expect(request.endpoint).toBe("https://relay.example/v1");
  expect(JSON.stringify(await page.evaluate(() => (window as any).harness.controller)))
    .not.toContain("fixture-secret");
});

test("preserves or explicitly clears a saved credential", async ({ page }) => {
  await page.getByRole("button", { name: "LLM 未配置" }).click();
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
