import { InkPadStore } from "./pad-store";
import "./lab.css";
import "./setup.css";

/** First-time tablet setup. The HTTP guide only downloads the public CA and
 * polls HTTPS; the HTTPS check page verifies storage before writing starts. */
interface SetupOpen {https_port:number;expires_at:number;trusted:boolean;certificate_name:string;fingerprint:string}
const root=document.querySelector<HTMLElement>("#setup-root")!;
const fragment=new URLSearchParams(location.hash.slice(1));
const token=fragment.get("setup")??"";
// The writing hand-off code never leaves this device except in HTTPS bodies.
const grant=fragment.get("grant")??"";
const validGrant=/^[0-9a-f]{64}$/.test(grant);
interface CheckReply {complete:boolean;document_id:string|null;title:string|null}
interface HandoffReply {state:"resume"|"pair"|"claimed"|"waiting"|"unbound";reason?:"confirm"|"desktop";pairing_token?:string;document_id?:string;title?:string|null}
const element=<T extends HTMLElement>(selector:string)=>root.querySelector<T>(selector)!;
const validToken=/^[0-9a-f]{64}$/.test(token);

function shell(body:string):void {
  root.innerHTML=`<main class="setup-page"><header class="setup-header"><span class="lab-mark">M</span><div><strong>设置平板书写</strong><span>让这台平板信任当前电脑上的 Mathscape</span></div></header>${body}</main>`;
}
function fail(message:string):void {shell(`<section class="setup-card setup-alert" role="alert"><h1>无法继续设置</h1><p>${message}</p><p>请在电脑的“平板书写 → 首次设置平板”中重新生成设置二维码。</p></section>`);}

async function guide():Promise<void> {
  const response=await fetch(`/api/setup/open?t=${token}`,{cache:"no-store"});
  if(!response.ok){fail(response.status===410?"设置链接已过期或已取消。":"电脑暂时无法提供设置信息。");return;}
  const setup=await response.json() as SetupOpen;
  const httpsBase=`https://${location.hostname}:${setup.https_port}`;
  shell(`<ol class="setup-steps">
    <li class="setup-card" data-step="download"><span class="setup-number">1</span><div><h2>保存电脑的信任证书</h2><p>证书只用于让平板确认正在连接这台电脑，不包含手稿或任何私钥。</p><a id="setup-download" class="lab-primary setup-action" href="/setup/ca.crt?t=${token}" download="${setup.certificate_name}">下载证书</a><p class="setup-file">文件名：<b>${setup.certificate_name}</b>，通常保存在“下载”文件夹。</p></div></li>
    <li class="setup-card" data-step="install"><span class="setup-number">2</span><div><h2>在系统设置中安装为 CA 证书</h2><p class="setup-shortcut">最简单：打开“设置”，在顶部搜索 <b>CA 证书</b>，选择刚下载的文件。</p>
      <details open><summary>Samsung Galaxy（如 Tab S8）</summary><p>设置 → 安全和隐私 → 更多安全设置 → 从设备存储安装 → <b>CA 证书</b> → 仍然安装 → 选择文件</p></details>
      <details><summary>其他 Android 设备</summary><p>设置 → 安全和隐私 → 更多安全设置 → 加密与凭据 → 安装证书 → <b>CA 证书</b></p></details>
      <ul class="setup-notes"><li>请选择“CA 证书”，不要选“VPN 和应用”或“WLAN”证书。</li><li>系统会提示“网络可能受到监控”，这是安装自建证书的标准提示；可随时在“用户证书”中删除。</li><li>如果要求设置锁屏密码，这是 Android 安装证书的前提。</li></ul></div></li>
    <li class="setup-card" data-step="check"><span class="setup-number">3</span><div><h2>返回本页，自动检测</h2><p id="setup-trust" role="status">正在检查这台平板是否已信任电脑…</p><button id="setup-recheck" type="button">重新检测</button><a id="setup-continue" class="lab-primary setup-action" hidden href="${httpsBase}/pad/setup.html#setup=${token}${validGrant?`&grant=${grant}`:""}">完成安装检测</a></div></li>
  </ol><p class="setup-footnote">证书指纹：<code>${setup.fingerprint.match(/.{1,8}/g)!.join(" ")}</code><br>本页仅在电脑开启设置期间可用，请在家庭或办公室等可信网络中完成。</p>`);
  element("#setup-download").addEventListener("click",()=>{element("[data-step=download]").classList.add("is-done");element("[data-step=install]").classList.add("is-current");});
  let checking=false;
  const probe=async():Promise<void>=>{
    if(checking)return;checking=true;
    try {
      const result=await fetch(`${httpsBase}/api/setup/probe?t=${token}`,{cache:"no-store",mode:"cors"});
      if(result.status===410||result.status===403){element("#setup-trust").textContent="设置链接已失效，请在电脑重新生成设置二维码。";return;}
      if(!result.ok)throw new Error(String(result.status));
      for(const step of ["download","install"])element(`[data-step=${step}]`).classList.add("is-done");
      element("[data-step=check]").classList.add("is-current");
      element("#setup-trust").textContent="这台平板已信任当前电脑。点击下方按钮完成存储与离线能力检测。";
      element("#setup-continue").hidden=false;element("#setup-recheck").hidden=true;stop();
    } catch {
      element("#setup-trust").textContent="尚未检测到证书。完成第 2 步后返回本页，会自动重新检测。";
    } finally {checking=false;}
  };
  const timer=setInterval(()=>void probe(),3000);
  const stop=()=>clearInterval(timer);
  element("#setup-recheck").addEventListener("click",()=>void probe());
  document.addEventListener("visibilitychange",()=>{if(!document.hidden)void probe();});
  void probe();
}

async function storageCheck():Promise<boolean> {
  try {
    return await new Promise<boolean>(resolve=>{
      const request=indexedDB.open("mathscape-inkpad-setup-check",1);
      request.onupgradeneeded=()=>request.result.createObjectStore("probe");
      request.onsuccess=()=>{const database=request.result;const tx=database.transaction("probe","readwrite");tx.objectStore("probe").put(Date.now(),"probe");tx.oncomplete=()=>{database.close();resolve(true);};tx.onerror=()=>{database.close();resolve(false);};};
      request.onerror=()=>resolve(false);request.onblocked=()=>resolve(false);
    });
  } catch {return false;}
}
async function workerCheck():Promise<boolean> {
  if(!("serviceWorker" in navigator))return false;
  try {
    // Registering here also caches the pad before the first writing session.
    await navigator.serviceWorker.register("/pad/sw.js",{scope:"/pad/"});
    return await Promise.race([navigator.serviceWorker.ready.then(()=>true),new Promise<boolean>(resolve=>setTimeout(()=>resolve(false),8000))]);
  } catch {return false;}
}
/** Credentials this tablet already holds for the manuscript, newest lease first. */
async function localCredentials(documentId:string):Promise<string[]> {
  try {
    const drafts=await (await InkPadStore.open()).drafts();
    const matching=drafts.filter(draft=>draft.binding.lease.document_id===documentId).sort((a,b)=>b.binding.lease.writer_epoch-a.binding.lease.writer_epoch);
    return [...new Set(matching.map(draft=>draft.binding.credential))].slice(0,16);
  } catch {return [];}
}
async function startWriting(reply:CheckReply):Promise<void> {
  const card=element("#setup-start"),text=element("#setup-start-text"),button=element<HTMLButtonElement>("#setup-start-button");
  card.hidden=false;element("#setup-start-title").textContent=reply.title?`继续书写《${reply.title}》`:"继续书写这份手稿";
  const credentials=await localCredentials(reply.document_id!);
  let target:string|undefined,timer:ReturnType<typeof setTimeout>|undefined,stopped=false;
  const stop=()=>{stopped=true;if(timer)clearTimeout(timer);};
  const poll=async():Promise<void> => {
    try {
      const response=await fetch("/api/setup/handoff",{method:"POST",cache:"no-store",headers:{"Content-Type":"application/json"},body:JSON.stringify({token,grant,credentials})});
      if(!response.ok){target=undefined;text.textContent=response.status===410?"设置链接已结束。请在电脑上点击“生成配对二维码”后扫码书写。":"电脑没有允许这台平板进入手稿，请在电脑上重新开始设置。";stop();return;}
      const value=await response.json() as HandoffReply;
      if(value.state==="resume"){target=`/pad/#doc=${encodeURIComponent(value.document_id!)}`;text.textContent="这台平板已持有这份手稿的书写权，点击即可继续。";stop();}
      else if(value.state==="pair"&&/^[0-9a-f]{64}$/.test(value.pairing_token??"")){target=`/pad/#pair=${value.pairing_token}`;text.textContent="电脑已准备好，点击开始书写。电脑会显示只读镜像。";}
      else if(value.state==="claimed"){target="/pad/";text.textContent="这份手稿已在画板中打开，点击返回画板。";stop();}
      else if(value.state==="unbound"){target=undefined;text.textContent="请回到电脑生成书写二维码后扫码开始。";stop();}
      else {target=undefined;text.textContent=value.reason==="confirm"?"这份手稿正在由另一台设备书写。请在电脑上确认转交给这台平板。":"正在等待电脑保存当前内容…";}
    } catch {target=undefined;text.textContent="暂时无法连接电脑，正在重试…";}
    finally {button.disabled=!target;if(!stopped)timer=setTimeout(()=>void poll(),1500);}
  };
  button.onclick=()=>{if(target){stop();location.href=target;}};
  await poll();
}
async function verify():Promise<void> {
  shell(`<section class="setup-card setup-verify"><h1>正在检测平板能力…</h1><ul id="setup-results" class="setup-results"></ul><p id="setup-summary" role="status"></p></section><section id="setup-start" class="setup-card setup-start" hidden><h2 id="setup-start-title">继续书写</h2><p id="setup-start-text" role="status">正在准备手稿…</p><button id="setup-start-button" class="lab-primary setup-action" type="button" disabled>开始书写</button></section>`);
  const check={secure_context:window.isSecureContext,indexed_db:await storageCheck(),service_worker:await workerCheck(),user_agent:navigator.userAgent};
  const rows:[string,boolean,string][]=[["安全连接",check.secure_context,"证书已生效"],["本机保存",check.indexed_db,"断网时笔迹可保存在平板"],["离线画板",check.service_worker,"断网后仍可刷新画板"]];
  element("#setup-results").innerHTML=rows.map(([name,ok,detail])=>`<li class="${ok?"is-ok":"is-bad"}"><b>${ok?"✓":"!"}</b><span>${name}</span><small>${ok?detail:"未通过"}</small></li>`).join("");
  let reported=false,reply:CheckReply|undefined;
  try {
    const response=await fetch("/api/setup/check",{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({token,check})});
    reported=response.ok;if(reported)reply=await response.json() as CheckReply;
  } catch { /* the result below explains that the computer did not receive it */ }
  const ready=check.secure_context&&check.indexed_db;
  element("h1").textContent=ready?"设置完成":"设置尚未完成";
  if(reported&&ready&&validGrant&&reply?.complete&&reply.document_id){
    element("#setup-summary").textContent=check.service_worker?"这台平板已可用于书写，以后无需重复设置。":"可以书写并自动同步；离线刷新不可用，请保持画板页面打开。";
    await startWriting(reply);return;
  }
  element("#setup-summary").textContent=!reported?"检测结果未能送达电脑，设置链接可能已过期。请在电脑重新生成设置二维码。":ready?(check.service_worker?"这台平板已可用于书写。请回到电脑，生成书写二维码后扫码开始。以后无需重复设置。":"可以开始书写并自动同步；离线刷新不可用，请保持画板页面打开。请回到电脑生成书写二维码。"):"浏览器本机存储不可用。请关闭无痕/隐私模式，或换用 Chrome / Samsung Internet 后重新扫码。";
}

if(!validToken)fail("设置链接不完整。");
else if(location.protocol==="https:")void verify().catch(error=>fail(String(error)));
else void guide().catch(()=>fail("无法连接电脑，请确认平板和电脑在同一局域网。"));
