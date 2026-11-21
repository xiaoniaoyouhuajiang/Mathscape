import { expect, test, type Page } from "@playwright/test";

async function writeLine(page: Page, dy = 0): Promise<void> {
  const box = (await page.locator("#desktop-ink-board .ink-input").boundingBox())!;
  const x = box.x + box.width / 2, y = box.y + box.height / 2 + dy;
  await page.mouse.move(x - 60,y); await page.mouse.down(); await page.mouse.move(x + 60,y,{steps:8}); await page.mouse.up();
}

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/ink-desktop.html");
  await expect(page.locator("#ink-session-list .ink-session-item")).toHaveCount(2);
});

test("JSON and PNG exports report the selected full path", async ({ page }) => {
  // The session list renders before the current document finishes opening.
  await expect.poll(() => page.evaluate(() => window.inkDesktopHarness.workspace.currentDocumentId())).toBeTruthy();
  const id = await page.evaluate(() => window.inkDesktopHarness.workspace.currentDocumentId());
  await page.locator("#desktop-ink-export-json").click();
  await expect(page.locator("#desktop-ink-status")).toHaveText(`当前手稿 Ink JSON 已保存到 /tmp/mathscape-ink-${id}-current.json。`);
  await page.locator("#desktop-ink-export-png").click();
  await expect(page.locator("#desktop-ink-status")).toHaveText(`当前页面 PNG 预览已保存到 /tmp/mathscape-ink-${id}-current.png；完整编辑历史请使用 Ink JSON。`);
  const files = await page.evaluate(() => window.inkDesktopHarness.exports);
  expect(JSON.parse(files[0].contents).document_id).toBe(id);
  expect(JSON.parse(files[1].contents).slice(0, 8)).toEqual([137, 80, 78, 71, 13, 10, 26, 10]);
});

test("export cancellation is neutral and writes nothing", async ({ page }) => {
  await page.evaluate(() => { window.inkDesktopHarness.exportBehavior.cancelExport = true; });
  for (const format of ["json", "png"]) {
    await page.locator(`#desktop-ink-export-${format}`).click();
    await expect(page.locator("#desktop-ink-status")).toHaveText("已取消导出。");
  }
  expect(await page.evaluate(() => window.inkDesktopHarness.exports)).toHaveLength(0);
});

test("export errors use the error status", async ({ page }) => {
  await page.evaluate(() => { window.inkDesktopHarness.exportBehavior.failExport = true; });
  await page.locator("#desktop-ink-export-json").click();
  await expect(page.locator("#desktop-ink-status")).toContainText("Ink JSON 导出失败：");
  expect(await page.evaluate(() => window.inkDesktopHarness.exports)).toHaveLength(0);
});

test("saved snapshots follow their document across session switches", async ({ page }) => {
  await page.getByRole("button", { name: "保存快照" }).click(); await page.locator("#ink-dialog [data-choice=ok]").click();
  await expect(page.locator("#ink-snapshot-list .ink-session-item")).toHaveCount(1);
  await expect(page.locator("#ink-snapshot-list")).not.toContainText("Invalid Date");

  await page.getByRole("button", { name: /手稿 B/ }).click();
  await expect(page.locator("#ink-session-list .ink-session-item.is-active")).toContainText("手稿 B");
  await expect(page.locator("#ink-snapshot-list")).toContainText("还没有历史快照");

  await page.getByRole("button", { name: /手稿 A/ }).click();
  await expect(page.locator("#ink-session-list .ink-session-item.is-active")).toContainText("手稿 A");
  await expect(page.locator("#ink-snapshot-list .ink-session-item")).toHaveCount(1);
  await expect(page.locator("#ink-snapshot-list")).toContainText("第 0 步");
  await expect(page.locator("#ink-snapshot-list .ink-session-item")).toHaveAttribute("title", "快照截止 head 0");
  await expect(page.locator("#ink-snapshot-list")).not.toContainText("head 0");
});

test("an older snapshot query cannot overwrite the current document list", async ({ page }) => {
  await page.getByRole("button", { name: "保存快照" }).click(); await page.locator("#ink-dialog [data-choice=ok]").click();
  await page.evaluate(() => {
    const harness = (window as unknown as { inkDesktopHarness: { documents: { a: string }; behavior: { delayedDocumentId: string; delayMs: number } } }).inkDesktopHarness;
    harness.behavior.delayedDocumentId = harness.documents.a;
    harness.behavior.delayMs = 250;
  });
  await page.getByRole("button", { name: /手稿 A/ }).click();
  await page.getByRole("button", { name: /手稿 B/ }).click();
  await expect(page.locator("#ink-session-list .ink-session-item.is-active")).toContainText("手稿 B");
  await expect(page.locator("#ink-snapshot-list")).toContainText("还没有历史快照");
  await page.waitForTimeout(300);
  await expect(page.locator("#ink-snapshot-list")).toContainText("还没有历史快照");
});

test("the reader makes snapshot mode and persistence state explicit", async ({ page }) => {
  await expect(page.locator("#ink-mode-badge")).toHaveText("当前手稿");
  await expect(page.locator("#ink-editability-badge")).toHaveText("可编辑");
  await page.getByRole("button", { name: "保存快照" }).click(); await page.locator("#ink-dialog [data-choice=ok]").click();
  const snapshot = page.locator("#ink-snapshot-list .ink-session-item");
  await expect(snapshot).toHaveCount(1);
  await snapshot.click();
  await expect(page.locator("#ink-mode-badge")).toHaveText("历史快照");
  await expect(page.locator("#ink-editability-badge")).toHaveText("只读");
  await expect(page.getByRole("button", { name: "继续编辑当前手稿" })).toBeVisible();
  await expect(page.locator("#desktop-ink-pen")).toBeDisabled();
  await expect(page.locator("#desktop-ink-export-json")).toBeEnabled();
  await page.getByRole("button", { name: "继续编辑当前手稿" }).click();
  await expect(page.locator("#ink-mode-badge")).toHaveText("当前手稿");
  await expect(page.locator("#ink-editability-badge")).toHaveText("可编辑");
});

test("snapshot selection registers evidence and reopens it from the map after later erasing", async ({ page }) => {
  await writeLine(page,-45); await writeLine(page,45);
  await expect(page.locator("#ink-save-badge")).toContainText("桌面已保存 · head 2");
  await page.getByRole("button",{name:"保存快照",exact:true}).click(); await page.locator("#ink-dialog [data-choice=ok]").click();
  await page.locator("#ink-snapshot-list .ink-session-item").click();
  await page.getByRole("button",{name:"选择本页全部笔迹"}).click();
  await expect(page.locator("#desktop-ink-evidence .ink-selection-summary")).toContainText("已选 2 笔");
  await page.evaluate(() => { window.inkDesktopHarness.behavior.loseEvidenceAck = true; });
  await page.getByRole("button",{name:"登记选区为来源证据"}).click();
  await expect(page.locator(".ink-evidence-message")).toContainText("响应丢失");
  await page.getByRole("button",{name:"登记选区为来源证据"}).click();
  await expect(page.locator(".ink-evidence-message")).toContainText("已登记");
  expect(await page.evaluate(() => window.inkDesktopHarness.evidenceRequests.map(request => request.request_id))).toEqual(expect.arrayContaining([expect.any(String)]));
  expect(await page.evaluate(() => new Set(window.inkDesktopHarness.evidenceRequests.map(request => request.request_id)).size)).toBe(1);
  await page.getByRole("button",{name:"继续编辑当前手稿"}).click();
  await page.getByRole("button",{name:"整笔擦除",exact:true}).click();
  const box = (await page.locator("#desktop-ink-board .ink-input").boundingBox())!;
  await page.mouse.click(box.x + box.width / 2,box.y + box.height / 2 - 45);
  await expect(page.locator("#ink-save-badge")).toContainText("head 3");
  await page.getByRole("button",{name:"返回地图"}).click();
  await page.getByRole("button",{name:"查看地图证据"}).click();
  await page.getByRole("button",{name:"打开并定位手稿证据"}).click();
  await expect(page.locator("#ink-reader-test")).toContainText("历史快照 · 只读 · 截止 head 2");
  await expect(page.locator("#ink-reader-test .ink-selection-summary")).toContainText("已选 2 笔");
  await expect(page.locator("#ink-reader-test .ink-input")).toBeVisible();
  await page.getByRole("button",{name:"继续编辑当前手稿"}).click();
  await expect(page.locator("#ink-save-badge")).toContainText("head 3");
  await expect(page.locator("#desktop-ink-summary")).toContainText("1 笔");
});

test("snapshot waits for operations added during an in-flight save", async ({ page }) => {
  await page.evaluate(() => {window.inkDesktopHarness.behavior.appendDelay = 180;});
  await writeLine(page,-25); await writeLine(page,25);
  await page.getByRole("button",{name:"保存快照",exact:true}).click(); await page.locator("#ink-dialog [data-choice=ok]").click();
  await expect(page.locator("#ink-snapshot-list")).toContainText("第 2 步");
  const saved = await page.evaluate(() => {
    const h = window.inkDesktopHarness;
    return { operations: h.storedDocuments.get(h.documents.a)!.operations.length, snapshot: JSON.parse(h.snapshots.get(h.documents.a)![0].manifest_json).document.operations.length };
  });
  expect(saved).toEqual({operations:2,snapshot:2});
});

test("a delayed snapshot open cannot override a newer document selection", async ({ page }) => {
  await page.getByRole("button",{name:"保存快照",exact:true}).click(); await page.locator("#ink-dialog [data-choice=ok]").click();
  await page.evaluate(() => {window.inkDesktopHarness.behavior.snapshotDelay = 250;});
  await page.locator("#ink-snapshot-list .ink-session-item").click();
  await page.getByRole("button",{name:/手稿 B/}).click();
  await expect(page.locator("#ink-document-heading")).toHaveText("手稿 B");
  await page.waitForTimeout(300);
  await expect(page.locator("#ink-document-heading")).toHaveText("手稿 B");
  await expect(page.locator("#ink-mode-badge")).toHaveText("当前手稿");
});

test("P3 pairing displays a QR and switches the desktop into a read-only mirror",async({page})=>{
  await page.getByRole("button",{name:"平板书写",exact:true}).click();
  await page.getByRole("button",{name:"生成配对二维码",exact:true}).click();
  await expect(page.locator("[data-qr]")).toBeVisible();
  expect(await page.locator("[data-link]").inputValue()).toContain("#pair=");
  await expect(page.locator("#ink-mode-badge")).toHaveText("平板镜像");
  await expect(page.locator("#desktop-ink-pen")).toBeDisabled();
  await page.getByRole("button",{name:"取消待配对",exact:true}).click();
  await expect(page.locator("#ink-mode-badge")).toHaveText("当前手稿");await expect(page.locator("#desktop-ink-pen")).toBeEnabled();
});

test("P3.1 setup shows a separate QR and tracks tablet trust progress",async({page})=>{
  // The tablet has not asked to start writing, so the desktop stays editable.
  await page.evaluate(()=>{(window as unknown as {inkDesktopHarness:{behavior:{autoRequest:boolean}}}).inkDesktopHarness.behavior.autoRequest=false;});
  await page.getByRole("button",{name:"平板书写",exact:true}).click();
  await page.getByRole("button",{name:"首次设置平板",exact:true}).click();
  await expect(page.locator("[data-setup-qr]")).toBeVisible();
  expect(await page.locator("[data-setup-link]").inputValue()).toContain("#setup=");
  await expect(page.locator("[data-setup-address] option")).toHaveCount(1);
  await expect(page.locator('[data-setup-step="complete"]')).toHaveClass(/is-done/);
  await expect(page.locator("[data-setup-status]")).toContainText("等待平板进入手稿");
  await expect(page.locator("#ink-mode-badge")).toHaveText("当前手稿");
  await page.getByRole("button",{name:"结束设置",exact:true}).click();
  await expect(page.locator(".ink-lan-setup")).toBeHidden();
});

type Harness={behavior:Record<string,unknown>&{pairingArgs:Array<Record<string,unknown>>};documents:{a:string}};
async function setBehavior(page:Page,values:Record<string,unknown>){await page.evaluate(values=>{Object.assign((window as unknown as {inkDesktopHarness:Harness}).inkDesktopHarness.behavior,values);},values);}
async function remoteHolder(page:Page,values:Record<string,unknown>={}){
  await page.evaluate(values=>{const harness=(window as unknown as {inkDesktopHarness:Harness}).inkDesktopHarness;Object.assign(harness.behavior,{remoteDocumentId:harness.documents.a,strictPairing:true,...values});},values);
  await expect(page.locator("#ink-mode-badge")).toHaveText("平板镜像");
}

test("P3.1.1 a tablet-held manuscript offers continue and transfer instead of a raw error",async({page})=>{
  await remoteHolder(page);
  await page.getByRole("button",{name:"平板书写",exact:true}).click();
  await expect(page.locator(".ink-lan-writer")).toBeVisible();
  await expect(page.locator("[data-writer-detail]")).toContainText("平板已连接");
  await expect(page.getByRole("button",{name:"生成配对二维码",exact:true})).toBeHidden();
  await page.getByRole("button",{name:"在这台平板继续",exact:true}).click();
  const resume=await page.locator("[data-link]").inputValue();
  expect(resume).toContain("#doc=");expect(resume).not.toContain("pair=");
  await page.getByRole("button",{name:"换一台设备书写",exact:true}).click();
  expect(await page.locator("[data-link]").inputValue()).toContain("#pair=");
  await expect(page.locator("[data-link-note]")).toContainText("原平板仍可继续书写");
  const args=await page.evaluate(()=>(window as unknown as {inkDesktopHarness:Harness}).inkDesktopHarness.behavior.pairingArgs);
  expect(args.at(-1)).toMatchObject({allowTransfer:true});
  await expect(page.locator(".ink-lan-panel")).not.toContainText("takeover_required");
});

test("P3.1.1 transferring away from an offline or unsynced tablet asks first",async({page})=>{
  await remoteHolder(page,{remoteConnected:false});
  await page.getByRole("button",{name:"平板书写",exact:true}).click();
  await expect(page.locator("[data-writer-detail]")).toContainText("平板当前未连接");
  await page.getByRole("button",{name:"换一台设备书写",exact:true}).click();
  await expect(page.locator("[data-transfer-warning]")).toContainText("导出恢复草稿");
  await expect(page.locator(".ink-lan-pairing")).toBeHidden();
  await page.getByRole("button",{name:"先去原平板同步",exact:true}).click();
  await setBehavior(page,{remoteConnected:true,remotePending:3});
  await expect(page.locator("[data-writer-detail]")).toContainText("待同步 3");
  await page.getByRole("button",{name:"换一台设备书写",exact:true}).click();
  await expect(page.locator("[data-transfer-warning]")).toContainText("3 个操作未同步");
  await page.getByRole("button",{name:"仍然转交",exact:true}).click();
  await expect(page.locator(".ink-lan-pairing")).toBeVisible();
  expect(await page.locator("[data-link]").inputValue()).toContain("#pair=");
});

test("P3.1.1 setup is bound to the manuscript and hands the tablet straight into writing",async({page})=>{
  await page.getByRole("button",{name:"平板书写",exact:true}).click();
  await page.getByRole("button",{name:"首次设置平板",exact:true}).click();
  await expect(page.locator('[data-setup-step="writing"]')).toBeVisible();
  await expect(page.locator("[data-setup-status]")).toContainText("平板上点击“开始书写”");
  const args=await page.evaluate(()=>(window as unknown as {inkDesktopHarness:Harness}).inkDesktopHarness.behavior.pairingArgs);
  expect(args.at(-1)).toMatchObject({setup:true,confirmTransfer:false,expectedHead:0});
  await expect(page.locator("#ink-mode-badge")).toHaveText("平板镜像");
  await setBehavior(page,{handoff:"claimed"});
  await expect(page.locator('[data-setup-step="writing"]')).toHaveClass(/is-done/);
  await expect(page.locator("[data-status]")).toContainText("平板已接手这份手稿");
});

test("P3.1.1 setup for a manuscript held by another tablet needs an explicit transfer",async({page})=>{
  await remoteHolder(page);
  await page.getByRole("button",{name:"平板书写",exact:true}).click();
  await page.getByRole("button",{name:"首次设置平板",exact:true}).click();
  await expect(page.locator("[data-setup-transfer]")).toBeVisible();
  await expect(page.locator("[data-setup-status]")).toContainText("正在由另一台设备书写");
  await page.getByRole("button",{name:"转交给这台平板",exact:true}).click();
  await expect(page.locator("[data-setup-status]")).toContainText("平板上点击“开始书写”");
  const args=await page.evaluate(()=>(window as unknown as {inkDesktopHarness:Harness}).inkDesktopHarness.behavior.pairingArgs);
  expect(args.at(-1)).toMatchObject({setup:true,confirmTransfer:true});
});
