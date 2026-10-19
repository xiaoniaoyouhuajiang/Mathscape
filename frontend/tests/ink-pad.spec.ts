import { expect,test,type Browser,type Page } from "@playwright/test";
import { readFile,writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const output=fileURLToPath(new URL("../../.test-runs/ink-lan-browser/",import.meta.url));
let serial=Date.now();
async function control(action:string,extra:Record<string,unknown>={}){const id=++serial;await writeFile(output+"control.json",JSON.stringify({...extra,id,action}));await expect.poll(async()=>{try{return JSON.parse(await readFile(output+"ready.json","utf8")).action_id;}catch{return 0;}}).toBe(id);return JSON.parse(await readFile(output+"ready.json","utf8"));}
async function writeLine(page:Page,dy=0){const canvas=page.locator("#pad-board .ink-input");await expect(canvas).toBeVisible();const box=(await canvas.boundingBox())!,x=box.x+box.width/2,y=box.y+box.height/2+dy;await page.mouse.move(x-60,y);await page.mouse.down();await page.mouse.move(x+60,y,{steps:10});await page.mouse.up();}
test.beforeEach(async({page})=>{const fixture=await control("fresh");await page.goto(fixture.url);await expect(page.locator("#pad-connection")).toContainText("电脑已连接");});
test.afterEach(async()=>{await control("resume");});

test("HTTPS InkPad automatically persists writing and creates a desktop snapshot",async({page})=>{
  await writeLine(page);await expect(page.locator("#pad-local-status")).toContainText("待同步 0");await expect(page.locator("#pad-desktop-status")).toContainText("head 1");
  const state=await control("status");expect(state.document.document.operations).toHaveLength(1);
  await page.getByRole("button",{name:"保存快照",exact:true}).click();await expect(page.locator("#pad-status")).toContainText("电脑已保存快照");
  expect((await control("status")).snapshots).toHaveLength(1);
  await page.reload();await expect(page.locator("#pad-desktop-status")).toContainText("head 1");await expect(page.locator("#pad-connection")).toContainText("电脑已连接");
});

test("the tablet writing surface refits when changing from landscape to portrait",async({page})=>{
  await page.setViewportSize({width:1280,height:800});
  await page.setViewportSize({width:800,height:1280});
  await expect.poll(()=>page.locator("#pad-board .ink-settled").evaluate((element)=>{
    const canvas=element as HTMLCanvasElement;
    return canvas.getContext("2d")!.getImageData(Math.floor(canvas.width/2),Math.floor(canvas.height/2),1,1).data[3];
  })).toBe(255);
  await writeLine(page);await expect(page.locator("#pad-desktop-status")).toContainText("head 1");
  expect((await control("status")).document.document.operations).toHaveLength(1);
});

test("offline handwriting survives page reload and drains after the computer returns",async({page,context})=>{
  await writeLine(page,-30);await expect(page.locator("#pad-desktop-status")).toContainText("head 1");
  await expect.poll(()=>page.evaluate(()=>!!navigator.serviceWorker.controller)).toBe(true);
  await control("pause");await context.setOffline(true);await writeLine(page,30);await expect(page.locator("#pad-local-status")).toContainText("待同步 1");
  await page.reload();await expect(page.locator("#pad-local-status")).toContainText("待同步 1");
  await context.setOffline(false);await control("resume");await expect(page.locator("#pad-desktop-status")).toContainText("head 2");await expect(page.locator("#pad-local-status")).toContainText("待同步 0");
  expect((await control("status")).document.document.operations).toHaveLength(2);
});

test("desktop process restart restores the same authenticated writer and operation history",async({page})=>{
  await writeLine(page);await expect(page.locator("#pad-desktop-status")).toContainText("head 1");await control("restart");await expect(page.locator("#pad-connection")).toContainText("电脑已连接");
  await writeLine(page,40);await expect(page.locator("#pad-desktop-status")).toContainText("head 2");expect((await control("status")).document.document.operations).toHaveLength(2);
});

test("cached drafts retain their own writer and history when switching documents",async({page})=>{
  await writeLine(page);await expect(page.locator("#pad-desktop-status")).toContainText("head 1");
  const original=await page.locator("#pad-drafts").inputValue();
  const next=await control("fresh");await page.goto(next.url);await expect(page.locator("#pad-connection")).toContainText("电脑已连接");
  await expect(page.locator("#pad-desktop-status")).toContainText("head 0");const second=await page.locator("#pad-drafts").inputValue();
  await writeLine(page,30);await expect(page.locator("#pad-desktop-status")).toContainText("head 1");
  await page.locator("#pad-drafts").selectOption(original);await expect(page.locator("#pad-connection")).toContainText("电脑已连接");await expect(page.locator("#pad-desktop-status")).toContainText("head 1");
  await writeLine(page,-30);await expect(page.locator("#pad-desktop-status")).toContainText("head 2");
  await page.locator("#pad-drafts").selectOption(second);await expect(page.locator("#pad-connection")).toContainText("电脑已连接");await expect(page.locator("#pad-desktop-status")).toContainText("head 1");
  expect((await control("status")).document.document.operations).toHaveLength(1);
});

test("an old epoch keeps its offline draft after the desktop takes back writing",async({page,context})=>{
  await writeLine(page,-30);await expect(page.locator("#pad-desktop-status")).toContainText("head 1");await context.setOffline(true);await control("pause");await writeLine(page,30);await expect(page.locator("#pad-local-status")).toContainText("待同步 1");
  await control("reclaim");await control("resume");await context.setOffline(false);await expect(page.locator("#pad-connection")).toContainText("会话");
  await expect(page.locator("#pad-local-status")).toContainText("待同步 1");expect((await control("status")).document.document.operations).toHaveLength(1);
  const downloaded=page.waitForEvent("download");await page.getByRole("button",{name:"导出恢复草稿"}).click();const file=await downloaded;const json=JSON.parse(await readFile((await file.path())!,"utf8"));expect(json.operations).toHaveLength(2);
});

test("a failed local transaction preserves the drawing and retries the original operation",async({page})=>{
  await page.evaluate(()=>{
    const original=IDBDatabase.prototype.transaction;
    const state=globalThis as typeof globalThis & {restoreInkStorage?:()=>void};
    IDBDatabase.prototype.transaction=function(...args:Parameters<typeof original>){
      const transaction=original.apply(this,args);
      if(args[1]==="readwrite"&&Array.from(transaction.objectStoreNames).includes("operations"))queueMicrotask(()=>transaction.abort());
      return transaction;
    };
    state.restoreInkStorage=()=>{IDBDatabase.prototype.transaction=original;};
  });
  await writeLine(page);await expect(page.locator("#pad-local-status")).toContainText("未保存");
  await expect(page.locator("#pad-status")).toContainText("本机保存失败");
  await expect(page.getByRole("button",{name:"笔",exact:true})).toBeDisabled();
  expect((await control("status")).document.document.operations).toHaveLength(0);
  const downloaded=page.waitForEvent("download");await page.getByRole("button",{name:"导出恢复草稿"}).click();
  expect(JSON.parse(await readFile((await (await downloaded).path())!,"utf8")).operations).toHaveLength(1);
  await page.evaluate(()=>(globalThis as typeof globalThis & {restoreInkStorage?:()=>void}).restoreInkStorage?.());
  await page.getByRole("button",{name:"立即同步",exact:true}).click();
  await expect(page.locator("#pad-desktop-status")).toContainText("head 1");await expect(page.locator("#pad-local-status")).toContainText("待同步 0");
  await page.reload();await expect(page.locator("#pad-desktop-status")).toContainText("head 1");
  expect((await control("status")).document.document.operations).toHaveLength(1);
});

test("first-time setup downloads the public CA, detects HTTPS trust and reports readiness",async({page})=>{
  const fixture=await control("setup");
  await page.goto(fixture.setup_url);
  await expect(page.getByRole("heading",{name:"保存电脑的信任证书"})).toBeVisible();
  const downloaded=page.waitForEvent("download");await page.getByRole("link",{name:"下载证书"}).click();
  const file=await downloaded;expect(file.suggestedFilename()).toMatch(/^Mathscape-InkPad-[0-9a-f]{8}\.crt$/);
  expect((await readFile((await file.path())!)).toString("latin1")).not.toContain("PRIVATE KEY");
  await expect(page.locator("#setup-trust")).toContainText("已信任当前电脑");
  await page.getByRole("link",{name:"完成安装检测"}).click();
  await expect(page.getByRole("heading",{name:"设置完成"})).toBeVisible();
  await expect(page.locator("#setup-summary")).toContainText("生成书写二维码");
  const status=(await control("status")).setup;
  expect(status).toMatchObject({opened:true,certificate_downloaded:true,https_trusted:true,complete:true});
});

/** A second tablet: its own IndexedDB, so it holds no credential for the manuscript. */
async function otherDevice(browser:Browser):Promise<Page>{const context=await browser.newContext({ignoreHTTPSErrors:true,acceptDownloads:true,viewport:{width:1100,height:820}});return context.newPage();}
async function finishSetup(page:Page,url:string){
  await page.goto(url);await expect(page.locator("#setup-trust")).toContainText("已信任当前电脑");
  await page.getByRole("link",{name:"完成安装检测"}).click();await expect(page.getByRole("heading",{name:"设置完成"})).toBeVisible();
}

test("setup on the tablet that already holds the manuscript resumes it without a second QR code",async({page})=>{
  await writeLine(page);await expect(page.locator("#pad-desktop-status")).toContainText("head 1");
  const fixture=await control("setup_bound");await finishSetup(page,fixture.setup_url);
  await expect(page.locator("#setup-start-text")).toContainText("已持有这份手稿的书写权");
  await page.getByRole("button",{name:"开始书写",exact:true}).click();
  await expect(page.locator("#pad-connection")).toContainText("电脑已连接");await expect(page.locator("#pad-desktop-status")).toContainText("head 1");
  expect(new URL(page.url()).hash).toBe("");
  await writeLine(page,40);await expect(page.locator("#pad-desktop-status")).toContainText("head 2");
  expect((await control("status")).setup.handoff).toBe("resumed");
});

test("setup on a new tablet waits for desktop confirmation, then takes over writing",async({page,browser})=>{
  await writeLine(page);await expect(page.locator("#pad-desktop-status")).toContainText("head 1");
  const fixture=await control("setup_bound");const tablet=await otherDevice(browser);await finishSetup(tablet,fixture.setup_url);
  await expect(tablet.locator("#setup-start-text")).toContainText("等待电脑");
  expect((await control("prepare_handoff")).setup).toMatchObject({handoff:"requested",transfer_confirmation:true});
  await expect(tablet.locator("#setup-start-text")).toContainText("确认转交");
  await expect(tablet.getByRole("button",{name:"开始书写",exact:true})).toBeDisabled();
  await writeLine(page,40);await expect(page.locator("#pad-desktop-status")).toContainText("head 2");
  await control("confirm_handoff");
  await tablet.getByRole("button",{name:"开始书写",exact:true}).click();
  await expect(tablet.locator("#pad-connection")).toContainText("电脑已连接");await expect(tablet.locator("#pad-desktop-status")).toContainText("head 2");
  await expect(page.locator("#pad-connection")).toContainText("写入权已转移");
  await writeLine(tablet,-40);await expect(tablet.locator("#pad-desktop-status")).toContainText("head 3");
  const state=await control("status");expect(state.setup.handoff).toBe("claimed");expect(state.document.document.operations).toHaveLength(3);
  await tablet.context().close();
});

test("a resume link reopens the stored session and a transfer QR moves writing to another device",async({page,browser})=>{
  await writeLine(page);await expect(page.locator("#pad-desktop-status")).toContainText("head 1");
  const fixture=await control("status");
  await page.goto(`${fixture.base}/pad/#doc=missing-document`);await expect(page.locator("#pad-status")).toContainText("没有这份手稿的书写授权");
  await page.goto(`${fixture.base}/pad/#doc=${encodeURIComponent(fixture.document_id)}`);
  await expect(page.locator("#pad-connection")).toContainText("电脑已连接");await expect(page.locator("#pad-desktop-status")).toContainText("head 1");
  const offer=await control("transfer");const tablet=await otherDevice(browser);
  await writeLine(page,40);await expect(page.locator("#pad-desktop-status")).toContainText("head 2");
  await tablet.goto(offer.transfer_url);await expect(tablet.locator("#pad-connection")).toContainText("电脑已连接");await expect(tablet.locator("#pad-desktop-status")).toContainText("head 2");
  await expect(page.locator("#pad-connection")).toContainText("写入权已转移");
  await tablet.context().close();
});

async function renameOnPad(page:Page,title:string){await page.getByRole("button",{name:"改名",exact:true}).click();await page.locator("#pad-rename-input").fill(title);await page.getByRole("button",{name:"保存标题",exact:true}).click();}

test("N1 a desktop rename reaches the tablet and a tablet rename reaches the desktop",async({page})=>{
  await control("rename",{title:"电脑上的标题"});await expect(page.locator("#pad-title")).toHaveText("电脑上的标题");
  await writeLine(page);await expect(page.locator("#pad-desktop-status")).toContainText("head 1");
  expect((await control("status")).note.title).toBe("电脑上的标题");
  await renameOnPad(page,"  平板上的标题 ");await expect(page.locator("#pad-status")).toContainText("标题已同步");
  await expect(page.locator("#pad-title")).toHaveText("平板上的标题");
  expect((await control("status")).note).toMatchObject({title:"平板上的标题",title_mode:"manual",title_revision:2});
  await page.reload();await expect(page.locator("#pad-connection")).toContainText("电脑已连接");await expect(page.locator("#pad-title")).toHaveText("平板上的标题");
});

test("N1 an offline tablet rename is kept locally and a conflicting desktop rename asks first",async({page,context})=>{
  await writeLine(page);await expect(page.locator("#pad-desktop-status")).toContainText("head 1");
  await control("pause");await context.setOffline(true);await expect(page.locator("#pad-connection")).not.toContainText("电脑已连接");
  await renameOnPad(page,"平板离线标题");await expect(page.locator("#pad-status")).toContainText("连接电脑后自动同步");
  await writeLine(page,40);await expect(page.locator("#pad-local-status")).toContainText("待同步 1");
  await control("rename",{title:"电脑同时改的"});
  await context.setOffline(false);await control("resume");await expect(page.locator("#pad-connection")).toContainText("电脑已连接");
  await expect(page.locator("#pad-title-choice")).toBeVisible();await expect(page.locator("#pad-title-choice-text")).toContainText("电脑同时改的");
  // Replayed queued metadata must not overwrite the server's title.
  await expect(page.locator("#pad-desktop-status")).toContainText("head 2");expect((await control("status")).note.title).toBe("电脑同时改的");
  await page.getByRole("button",{name:"使用「平板离线标题」"}).click();await expect(page.locator("#pad-title")).toHaveText("平板离线标题");
  expect((await control("status")).note.title).toBe("平板离线标题");
});

test("N1 trashing a connected note fences the tablet and restoring it requires re-pairing",async({page,context})=>{
  await writeLine(page,-30);await expect(page.locator("#pad-desktop-status")).toContainText("head 1");
  await control("pause");await context.setOffline(true);await writeLine(page,30);await expect(page.locator("#pad-local-status")).toContainText("待同步 1");
  await control("resume");await context.setOffline(false);await expect(page.locator("#pad-connection")).toContainText("电脑已连接");
  await expect(page.locator("#pad-desktop-status")).toContainText("head 2");
  await control("trash");await expect(page.locator("#pad-status")).toContainText("回收站");
  await expect(page.getByRole("button",{name:"笔",exact:true})).toBeDisabled();await expect(page.getByRole("button",{name:"改名",exact:true})).toBeDisabled();
  expect((await control("status")).note.lifecycle).toBe("trashed");
  const receipt=(await control("restore")).lifecycle_receipt;expect(receipt.requires_repairing).toBe(true);
  await page.reload();await expect(page.getByRole("button",{name:"笔",exact:true})).toBeDisabled();
  expect((await control("status")).document.document.operations).toHaveLength(2);
});

test("N1 a tablet that was offline when the note was trashed keeps its pending writing and can export it",async({page,context})=>{
  await writeLine(page,-30);await expect(page.locator("#pad-desktop-status")).toContainText("head 1");
  await control("pause");await context.setOffline(true);await writeLine(page,30);await expect(page.locator("#pad-local-status")).toContainText("待同步 1");
  await control("trash");await context.setOffline(false);await control("resume");
  await expect(page.locator("#pad-status")).toContainText("回收站");await expect(page.getByRole("button",{name:"笔",exact:true})).toBeDisabled();
  await expect(page.locator("#pad-local-status")).toContainText("待同步 1");expect((await control("status")).document.document.operations).toHaveLength(1);
  const downloaded=page.waitForEvent("download");await page.getByRole("button",{name:"导出恢复草稿"}).click();
  expect(JSON.parse(await readFile((await (await downloaded).path())!,"utf8")).operations).toHaveLength(2);
});

async function lassoAll(page:Page){const box=(await page.locator("#pad-board .ink-input").boundingBox())!,x=box.x+box.width/2,y=box.y+box.height/2;await page.mouse.move(x-110,y-90);await page.mouse.down();for(const [px,py] of [[x+110,y-90],[x+110,y+90],[x-110,y+90],[x-110,y-85]])await page.mouse.move(px,py,{steps:6});await page.mouse.up();}
test("N3.0 the tablet palette offers six presets, defaults to #193a66 and remembers its own choice",async({page})=>{
  const palette=page.getByRole("group",{name:"笔迹颜色"});
  await expect(palette.locator(".ink-swatch")).toHaveCount(6);await expect(palette).toHaveAttribute("data-color","#193a66");
  await writeLine(page,-45);await expect(page.locator("#pad-desktop-status")).toContainText("head 1");
  await palette.getByRole("button",{name:"绿"}).click();await expect(palette.getByRole("button",{name:"绿"})).toHaveAttribute("aria-pressed","true");
  await writeLine(page,45);await expect(page.locator("#pad-desktop-status")).toContainText("head 2");
  const colors=(await control("status")).document.document.operations.map((operation:{stroke:{brush:{color:string}}})=>operation.stroke.brush.color);
  expect(colors).toEqual(["#193a66","#2e7d32"]);
  await page.reload();await expect(page.locator("#pad-connection")).toContainText("电脑已连接");
  await expect(page.getByRole("group",{name:"笔迹颜色"})).toHaveAttribute("data-color","#2e7d32");
  expect(await page.evaluate(()=>localStorage.getItem("mathscape:ink-pad-color"))).toBe("#2e7d32");
  await expect(page.getByRole("combobox",{name:"最近手稿"})).toBeVisible();
  await expect(page.locator("#pad-drafts option").first()).toContainText("最近手稿");
});
test("N3.0 the tablet deletes a lasso selection with one erase that a single undo restores",async({page})=>{
  const remove=page.getByRole("button",{name:"删除所选",exact:true});
  await writeLine(page,-45);await writeLine(page,45);await expect(page.locator("#pad-desktop-status")).toContainText("head 2");
  await expect(remove).toBeDisabled();
  await page.getByRole("button",{name:"套索选择",exact:true}).click();await lassoAll(page);
  await expect(remove).toBeEnabled();await remove.click();await expect(remove).toBeDisabled();
  await expect(page.locator("#pad-desktop-status")).toContainText("head 3");
  const erase=(await control("status")).document.document.operations.at(-1);
  expect(erase.kind).toBe("erase");expect(erase.stroke_ids).toHaveLength(2);
  await page.getByRole("button",{name:"撤销",exact:true}).click();await expect(page.locator("#pad-desktop-status")).toContainText("head 4");
  const projection=(await control("status")).document.document.operations.at(-1);expect(projection.kind).toBe("undo");
});

test("N3a the tablet restyles a lasso selection with one undoable command",async({page})=>{
  const restyle=page.getByRole("button",{name:"改颜色",exact:true});
  const palette=page.getByRole("group",{name:"笔迹颜色"});
  await writeLine(page,-45);await writeLine(page,45);await expect(page.locator("#pad-desktop-status")).toContainText("head 2");
  await expect(restyle).toBeDisabled();
  await page.getByRole("button",{name:"套索选择",exact:true}).click();await lassoAll(page);
  await expect(restyle).toBeEnabled();
  await palette.getByRole("button",{name:"红"}).click();
  await restyle.click();
  await expect(page.locator("#pad-desktop-status")).toContainText("head 3");
  const restyleOp=(await control("status")).document.document.operations.at(-1);
  expect(restyleOp.kind).toBe("restyle");expect(restyleOp.stroke_ids).toHaveLength(2);expect(restyleOp.color).toBe("#c62828");
  await page.getByRole("button",{name:"撤销",exact:true}).click();await expect(page.locator("#pad-desktop-status")).toContainText("head 4");
  expect((await control("status")).document.document.operations.at(-1).kind).toBe("undo");
});
