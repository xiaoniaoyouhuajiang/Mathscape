import QRCode from "qrcode";
import { safeInvoke } from "../core/ipc";
import { inkId,type InkDocumentMetadata,type InkOperation,type InkStroke } from "./model";
import type { InkNoteState, StoredInkSession } from "./persistence";

export interface InkLanStatus {enabled:boolean;running:boolean;port:number;urls:string[];certificate_file_sha256:string|null;assets_ready:boolean;error:string|null}
export interface InkWriterView {document_id:string;head_seq:number;writer_epoch:number;session:StoredInkSession|null;remote:boolean;pairing_pending:boolean}
export interface InkLanMirror {delta:{metadata:InkDocumentMetadata;operations:InkOperation[];head_seq:number;server_head:number;writer:InkWriterView;snapshot_count:number;note?:InkNoteState};connected:boolean;preview:InkStroke|null;client:{pending_count:number;locally_durable:boolean;secure_context:boolean;page_id:string}|null}
export type InkSetupHandoff = "unbound"|"idle"|"requested"|"ready"|"resumed"|"claimed";
export interface InkSetupStatus {active:boolean;expires_at:number|null;urls:string[];opened:boolean;certificate_downloaded:boolean;https_trusted:boolean;check:{secure_context:boolean;indexed_db:boolean;service_worker:boolean;user_agent:string}|null;complete:boolean;document_id?:string|null;handoff?:InkSetupHandoff|"";transfer_confirmation?:boolean}
/** What the desktop mirror last learned about the tablet holding the lease. */
export interface InkWriterPresence {connected:boolean;pending:number|null}
export function inkLanMirror(documentId:string,afterSeq:number):Promise<InkLanMirror>{return safeInvoke("get_ink_lan_mirror",{documentId,afterSeq});}

/** Turns store invariant codes into the action the user can take next. */
export function explainInkLanError(error:unknown):string {
  const text=error instanceof Error?error.message:String(error);
  if(text.includes("takeover_required"))return "这份手稿正在由平板书写。可以在这台平板继续，或转交给新设备。";
  if(text.includes("head conflict"))return "手稿刚有新的书写内容，请稍候再试一次。";
  if(text.includes("writer epoch conflict")||text.includes("writer_epoch_conflict"))return "书写权刚刚发生变化，请稍候重试。";
  if(text.includes("invalid pairing expiry"))return "电脑时间异常，无法生成二维码。";
  if(text.startsWith("ink invariant failed:")||text.startsWith("ink "))return "操作未完成，手稿内容没有变化。请稍后重试。";
  return text;
}
function seenAt(value:string|undefined):string {
  if(!value)return "";
  const time=value.startsWith("unix:")?Number(value.slice(5))*1000:Date.parse(value);
  return Number.isFinite(time)?new Date(time).toLocaleString("zh-CN",{hour12:false}):"";
}
type LinkKind="pair"|"transfer"|"resume";
const LINK_NOTES:Record<LinkKind,string>={
  pair:"二维码两分钟有效，仅供一台设备配对。配对完成后无需主动发送文件，书写操作会自动同步到电脑。",
  transfer:"转交二维码两分钟有效。新设备扫码前，原平板仍可继续书写和同步；扫码后原平板停止写入，未同步内容保留在原平板。",
  resume:"用正在书写这份手稿的平板扫码或打开链接即可继续。链接不含配对凭据，不会转移书写权。",
};

export class InkLanPanel {
  private link:{urls:string[];expires_at:number;document_id:string}|undefined;
  private linkKind:LinkKind|undefined;
  private linkShownAt=0;
  private documentId="";
  private lanStatus:InkLanStatus|undefined;
  private takeoverRequest:{documentId:string;expectedEpoch:number;expectedHead:number;requestId:string}|undefined;
  private setupTimer:ReturnType<typeof setInterval>|undefined;
  private writerTimer:ReturnType<typeof setInterval>|undefined;
  private handoffBusy=false;
  private setupFinished=false;
  constructor(private readonly host:HTMLElement,private readonly prepare:(documentId?:string)=>Promise<{documentId:string;head:number|null}>,private readonly view:()=>InkWriterView|undefined,private readonly changed:()=>Promise<void>,private readonly presence:()=>InkWriterPresence|undefined=()=>undefined) {
    host.className="ink-lan-panel";
    host.innerHTML=`<div class="ink-lan-heading"><strong>平板书写</strong><button data-action="close" type="button">收起</button></div><div class="ink-lan-setup-intro"><div><strong>第一次在这台平板上使用？</strong><p>扫码后按平板上的步骤安装信任证书，只需一次；完成后在平板上点击“开始书写”即可进入当前手稿。设备与电脑需要在同一局域网。</p></div><button data-action="setup" type="button" class="is-primary">首次设置平板</button></div><div class="ink-lan-setup" hidden><div><label>电脑地址<select data-setup-address aria-label="设置地址"></select></label><canvas data-setup-qr aria-label="平板设置二维码"></canvas></div><div><strong>用平板扫描设置二维码</strong><ol class="ink-lan-setup-progress"><li data-setup-step="opened">平板已打开设置页</li><li data-setup-step="certificate_downloaded">已下载信任证书</li><li data-setup-step="https_trusted">平板已信任这台电脑</li><li data-setup-step="complete">存储与离线能力检测完成</li><li data-setup-step="writing">平板已进入手稿</li></ol><p data-setup-status></p><div class="ink-lan-warning" data-setup-transfer hidden><p data-setup-transfer-text></p><div class="ink-lan-actions"><button data-action="setup-transfer" type="button" class="is-primary">转交给这台平板</button></div></div><input data-setup-link readonly aria-label="设置链接"><div class="ink-lan-actions"><button data-action="setup-copy" type="button">复制设置链接</button><button data-action="setup-cancel" type="button">结束设置</button></div></div></div><div class="ink-lan-writer" hidden><div><strong>这份手稿正在由平板书写</strong><p data-writer-detail></p></div><div class="ink-lan-actions"><button data-action="resume" type="button" class="is-primary">在这台平板继续</button><button data-action="transfer" type="button">换一台设备书写</button><button data-action="reclaim" type="button">收回书写权</button></div><div class="ink-lan-warning" data-transfer-warning hidden><p data-transfer-text></p><div class="ink-lan-actions"><button data-action="transfer-confirm" type="button">仍然转交</button><button data-action="transfer-cancel" type="button">先去原平板同步</button></div></div></div><div class="ink-lan-actions"><label>端口<input data-port type="number" min="1024" max="65535" value="1443"></label><button data-action="start" type="button">开启服务</button><button data-action="certificate" type="button">手动导出证书</button><button data-action="pair" type="button">生成配对二维码</button><button data-action="cancel" type="button">取消待配对</button><button data-action="stop" type="button">停止平板服务</button></div><div class="ink-lan-pairing" hidden><div><label>电脑地址<select data-address aria-label="配对地址"></select></label><canvas data-qr aria-label="平板二维码"></canvas></div><div><strong data-link-title></strong><input data-link readonly aria-label="平板链接"><button data-action="copy" type="button">复制链接</button><p data-link-note></p></div></div><p data-status role="status"></p><details><summary>设备信任与恢复说明</summary><p>推荐使用“首次设置平板”。也可以手动导出 .crt 文件传到平板，在系统安全设置中安装为 CA 证书。同一 Workspace 重启服务不需要重新安装。平板的书写权在电脑重启后保留；关闭服务、收回书写权或转交给新设备会使旧设备凭据失效；旧端未同步内容保留在设备上，可以导出恢复草稿后在电脑导入。</p><code data-fingerprint></code></details>`;
    const button=(action:string)=>host.querySelector<HTMLButtonElement>(`[data-action="${action}"]`)!;
    const run=(action:()=>Promise<void>)=>void action().catch(error=>this.status(explainInkLanError(error))).finally(()=>this.renderWriter());
    button("close").onclick=()=>{host.hidden=true;this.stopWriterPolling();};
    button("setup").onclick=()=>run(async()=>{
      this.setupFinished=false;
      const setup=await safeInvoke<InkSetupStatus>("start_ink_setup",{documentId:this.documentId||null});
      const select=host.querySelector<HTMLSelectElement>("[data-setup-address]")!;select.replaceChildren();
      for(const url of setup.urls){const location=new URL(url);if(location.hostname==="localhost")continue;select.add(new Option(location.hostname+(location.hostname==="127.0.0.1"?"（本机测试）":""),url));}
      host.querySelector<HTMLElement>(".ink-lan-setup")!.hidden=false;await this.renderSetupQr();this.renderSetup(setup);
      this.renderStatus(await safeInvoke<InkLanStatus>("get_ink_lan_status"));
      // Polling continues while the panel is collapsed: the desktop must answer the tablet's hand-off.
      this.stopSetupPolling();this.setupTimer=setInterval(()=>void safeInvoke<InkSetupStatus>("get_ink_setup_status").then(status=>this.renderSetup(status)).catch(()=>undefined),1000);
    });
    button("setup-cancel").onclick=()=>run(async()=>{await safeInvoke("cancel_ink_setup");this.clearSetup();await this.changed();this.status("平板设置已结束，设置链接已失效。");});
    button("setup-copy").onclick=()=>run(async()=>{await navigator.clipboard.writeText(host.querySelector<HTMLInputElement>("[data-setup-link]")!.value);this.status("设置链接已复制。");});
    button("setup-transfer").onclick=()=>run(()=>this.prepareHandoff(true));
    host.querySelector<HTMLSelectElement>("[data-setup-address]")!.onchange=()=>run(()=>this.renderSetupQr());
    button("start").onclick=()=>run(async()=>{const port=Number(host.querySelector<HTMLInputElement>("[data-port]")!.value);const status=await safeInvoke<InkLanStatus>("start_ink_lan",{port});this.renderStatus(status);await this.changed();});
    button("certificate").onclick=()=>run(async()=>{const path=await safeInvoke<string|null>("export_ink_trust_certificate");if(path)this.status("证书已保存，请传到平板并安装为 CA 证书。完成后再扫码。");});
    button("pair").onclick=()=>run(async()=>{
      const context=await prepare();this.documentId=context.documentId;
      if(this.remoteWriter()){this.status(explainInkLanError("takeover_required"));return;}
      const link=await safeInvoke<{urls:string[];expires_at:number;document_id:string}>("create_ink_pairing",{documentId:context.documentId,expectedHead:context.head});
      await this.showLink(link,"pair");await this.changed();this.status("等待平板扫码；电脑暂时作为只读镜像。二维码过期后可以重新生成。");
    });
    button("cancel").onclick=()=>run(async()=>{const context=await prepare();await safeInvoke("cancel_ink_pairing",{documentId:context.documentId});this.clearQr();await this.changed();this.status("待配对二维码已取消。");});
    button("resume").onclick=()=>run(async()=>{
      const status=this.lanStatus??await safeInvoke<InkLanStatus>("get_ink_lan_status");
      if(!status.running)throw new Error("请先开启平板书写服务。");
      await this.showLink({urls:status.urls.map(url=>`${url}#doc=${encodeURIComponent(this.documentId)}`),expires_at:0,document_id:this.documentId},"resume");
      this.status("在正在书写的平板上打开这个地址即可继续；也可以直接在平板画板上点击“重试连接”。");
    });
    button("transfer").onclick=()=>run(async()=>{
      const presence=this.presence();
      const warning=!presence?.connected?"原平板当前未连接，可能仍有未同步内容。转交后，这些内容只能在原平板上“导出恢复草稿”后导入电脑。":(presence.pending??0)>0?`原平板还有 ${presence.pending} 个操作未同步。建议先在原平板点击“立即同步”，待同步为 0 后再转交。`:"";
      if(warning){host.querySelector<HTMLElement>("[data-transfer-text]")!.textContent=warning;host.querySelector<HTMLElement>("[data-transfer-warning]")!.hidden=false;return;}
      await this.issueTransfer();
    });
    button("transfer-confirm").onclick=()=>run(()=>this.issueTransfer());
    button("transfer-cancel").onclick=()=>{host.querySelector<HTMLElement>("[data-transfer-warning]")!.hidden=true;};
    button("reclaim").onclick=()=>run(async()=>{
      const current=this.view();if(!current||current.document_id!==this.documentId)return;
      if(!this.takeoverRequest)this.takeoverRequest={documentId:current.document_id,expectedEpoch:current.writer_epoch,expectedHead:current.head_seq,requestId:inkId()};
      try{await safeInvoke("reclaim_ink_writer",this.takeoverRequest);}catch(error){if(String(error).includes("head conflict")||String(error).includes("writer epoch conflict"))this.takeoverRequest=undefined;throw error;}
      this.takeoverRequest=undefined;this.clearQr();await this.changed();this.status("电脑已收回书写权。旧端未同步操作不会自动合入新会话，请在平板导出恢复草稿。");
    });
    button("stop").onclick=()=>run(async()=>{await safeInvoke("stop_ink_lan");this.clearQr();this.clearSetup();this.renderStatus(await safeInvoke<InkLanStatus>("get_ink_lan_status"));await this.changed();this.status("平板服务已停止，连接凭据已撤销；电脑中的已确认笔迹保留。");});
    button("copy").onclick=()=>run(async()=>{const value=host.querySelector<HTMLInputElement>("[data-link]")!.value;await navigator.clipboard.writeText(value);this.status("链接已复制。");});
    host.querySelector<HTMLSelectElement>("[data-address]")!.onchange=()=>run(()=>this.renderQr());
  }
  async open(documentId:string):Promise<void>{
    this.host.hidden=false;if(this.documentId!==documentId)this.clearQr();this.documentId=documentId;
    this.renderStatus(await safeInvoke<InkLanStatus>("get_ink_lan_status"));
    this.renderWriter();this.stopWriterPolling();this.writerTimer=setInterval(()=>this.renderWriter(),1000);
  }
  private remoteWriter():InkWriterView|undefined {const view=this.view();return view&&view.document_id===this.documentId&&view.remote?view:undefined;}
  private async issueTransfer():Promise<void> {
    const view=this.remoteWriter();if(!view)throw new Error("书写权已经不在平板，请直接生成配对二维码。");
    const link=await safeInvoke<{urls:string[];expires_at:number;document_id:string}>("create_ink_pairing",{documentId:view.document_id,expectedHead:view.head_seq,allowTransfer:true});
    this.host.querySelector<HTMLElement>("[data-transfer-warning]")!.hidden=true;
    await this.showLink(link,"transfer");await this.changed();this.status("用新设备扫描转交二维码。新设备领取后，原平板会停止写入。");
  }
  /** Flushes desktop writes, then asks the service for the tablet's one-time pairing. */
  private async prepareHandoff(confirm:boolean):Promise<void> {
    if(this.handoffBusy)return;this.handoffBusy=true;
    try{
      const current=await safeInvoke<InkSetupStatus>("get_ink_setup_status");
      if(!current.active||!current.document_id)return;
      const context=await this.prepare(current.document_id);
      const head=context.documentId===current.document_id?context.head:null;
      const next=await safeInvoke<InkSetupStatus>("prepare_ink_setup_handoff",{expectedHead:head,confirmTransfer:confirm});
      this.renderSetup(next);await this.changed();
    }finally{this.handoffBusy=false;}
  }
  private async showLink(link:{urls:string[];expires_at:number;document_id:string},kind:LinkKind):Promise<void> {
    this.link=link;this.linkKind=kind;this.linkShownAt=Date.now();
    const select=this.host.querySelector<HTMLSelectElement>("[data-address]")!;select.replaceChildren();
    for(const url of link.urls){const location=new URL(url);select.add(new Option(location.hostname+(location.hostname==="127.0.0.1"||location.hostname==="localhost"?"（本机测试）":""),url));}
    this.host.querySelector<HTMLElement>("[data-link-title]")!.textContent=kind==="resume"?"在原平板继续书写":kind==="transfer"?"用新设备扫码接手书写":"用平板扫码开始书写";
    this.host.querySelector<HTMLElement>("[data-link-note]")!.textContent=LINK_NOTES[kind];
    this.host.querySelector<HTMLElement>(".ink-lan-pairing")!.hidden=false;await this.renderQr();
  }
  private async renderQr():Promise<void>{const url=this.host.querySelector<HTMLSelectElement>("[data-address]")!.value;if(!url)return;this.host.querySelector<HTMLInputElement>("[data-link]")!.value=url;await QRCode.toCanvas(this.host.querySelector<HTMLCanvasElement>("[data-qr]")!,url,{width:190,margin:2});}
  private async renderSetupQr():Promise<void>{const url=this.host.querySelector<HTMLSelectElement>("[data-setup-address]")!.value;if(!url)return;this.host.querySelector<HTMLInputElement>("[data-setup-link]")!.value=url;await QRCode.toCanvas(this.host.querySelector<HTMLCanvasElement>("[data-setup-qr]")!,url,{width:190,margin:2});}
  private renderSetup(setup:InkSetupStatus):void {
    if(this.setupFinished)return;
    for(const step of ["opened","certificate_downloaded","https_trusted","complete"] as const){
      // Later evidence implies earlier steps: an already trusted tablet may skip the download.
      const done=setup[step]||(step!=="complete"&&(setup.https_trusted||setup.complete))||(step==="opened"&&setup.certificate_downloaded);
      this.host.querySelector(`[data-setup-step="${step}"]`)!.classList.toggle("is-done",done);
    }
    const handoff=setup.handoff||"unbound",writing=handoff==="resumed"||handoff==="claimed";
    const writingStep=this.host.querySelector<HTMLElement>('[data-setup-step="writing"]')!;
    writingStep.hidden=handoff==="unbound";writingStep.classList.toggle("is-done",writing);
    const confirm=this.host.querySelector<HTMLElement>("[data-setup-transfer]")!;confirm.hidden=true;
    const text=this.host.querySelector<HTMLElement>("[data-setup-status]")!;
    if(!setup.active){text.textContent="设置链接已失效，可以重新开始设置。";this.stopSetupPolling();return;}
    if(setup.complete&&handoff==="unbound"){
      text.textContent=setup.check?.service_worker?"这台平板已完成设置。现在可以生成书写二维码。":"平板可以书写并自动同步；离线刷新不可用，书写时请保持画板打开。";
      this.status("平板设置完成，请点击“生成配对二维码”开始书写。");this.stopSetupPolling();
    } else if(setup.complete){
      const offline=setup.check?.service_worker?"":"离线刷新不可用，书写时请保持画板打开。";
      if(writing){
        text.textContent="平板已进入手稿书写，设置已结束。"+offline;
        this.status(handoff==="resumed"?"平板使用已有书写权继续这份手稿。":"平板已接手这份手稿，电脑显示只读镜像。");
        this.setupFinished=true;this.stopSetupPolling();
        void safeInvoke("cancel_ink_setup").catch(()=>undefined).then(()=>this.changed());
      } else if(handoff==="ready")text.textContent="设置完成。请在平板上点击“开始书写”；电脑暂时显示只读镜像。"+offline;
      else if(handoff==="requested"&&setup.transfer_confirmation){
        const presence=this.presence();
        text.textContent="设置完成。这份手稿正在由另一台设备书写，需要你确认是否交给刚设置的平板。";
        this.host.querySelector<HTMLElement>("[data-setup-transfer-text]")!.textContent=presence?.connected?((presence.pending??0)>0?`原设备还有 ${presence.pending} 个操作未同步，建议先在原设备上同步。`:"原设备已连接且没有待同步内容。"):"原设备当前未连接，可能仍有未同步内容；转交后这些内容只能从原设备导出恢复草稿。";
        confirm.hidden=false;
      } else if(handoff==="requested"){text.textContent="设置完成，正在保存电脑上的内容并准备书写…";void this.prepareHandoff(false).catch(error=>this.status(explainInkLanError(error)));}
      else text.textContent="设置完成，等待平板进入手稿…";
    } else if(setup.check)text.textContent="平板本机存储不可用，请关闭无痕模式或更换浏览器后重新扫码。";
    else if(setup.https_trusted)text.textContent="证书已生效，等待平板完成能力检测…";
    else if(setup.certificate_downloaded)text.textContent="等待在平板“设置”中安装 CA 证书，完成后返回设置页。";
    else text.textContent=setup.opened?"平板已打开设置页，等待下载证书。":"等待平板扫码。设置链接 15 分钟内有效。";
  }
  /** Shows hand-off choices when another device holds this manuscript's lease. */
  private renderWriter():void {
    const view=this.remoteWriter(),running=!!this.lanStatus?.running;
    const card=this.host.querySelector<HTMLElement>(".ink-lan-writer")!;card.hidden=!view;
    this.host.querySelector<HTMLButtonElement>('[data-action="pair"]')!.hidden=!!view;
    this.host.querySelector<HTMLButtonElement>('[data-action="cancel"]')!.disabled=!(this.view()?.pairing_pending||this.link&&this.linkKind!=="resume");
    // A claimed or expired transfer code disappears; the grace period covers a mirror refresh in flight.
    if(this.linkKind==="transfer"&&!this.view()?.pairing_pending&&Date.now()-this.linkShownAt>3000)this.clearQr();
    if(!view){this.host.querySelector<HTMLElement>("[data-transfer-warning]")!.hidden=true;if(this.linkKind==="resume")this.clearQr();return;}
    for(const action of ["resume","transfer"])this.host.querySelector<HTMLButtonElement>(`[data-action="${action}"]`)!.disabled=!running;
    const presence=this.presence(),seen=seenAt(view.session?.last_seen_at);
    const state=presence?.connected?`平板已连接 · 待同步 ${presence.pending??0} 个操作`:`平板当前未连接${seen?` · 最近同步 ${seen}`:""}`;
    this.host.querySelector<HTMLElement>("[data-writer-detail]")!.textContent=state+(view.pairing_pending?" · 转交二维码等待新设备扫码":"")+(running?"":" · 请先开启平板服务");
  }
  private stopSetupPolling():void{if(this.setupTimer){clearInterval(this.setupTimer);this.setupTimer=undefined;}}
  private stopWriterPolling():void{if(this.writerTimer){clearInterval(this.writerTimer);this.writerTimer=undefined;}}
  private clearSetup():void{this.stopSetupPolling();this.host.querySelector<HTMLElement>(".ink-lan-setup")!.hidden=true;}
  private clearQr():void{this.link=undefined;this.linkKind=undefined;this.host.querySelector<HTMLElement>(".ink-lan-pairing")!.hidden=true;}
  private renderStatus(status:InkLanStatus):void {
    this.lanStatus=status;
    this.host.querySelector<HTMLInputElement>("[data-port]")!.value=String(status.port);
    this.host.querySelector<HTMLElement>("[data-fingerprint]")!.textContent=status.certificate_file_sha256?"证书文件 SHA-256："+status.certificate_file_sha256:"";
    this.host.querySelector<HTMLButtonElement>('[data-action="pair"]')!.disabled=!status.running||!status.assets_ready;
    const remote=this.remoteWriter();
    this.status(status.error??(status.running?(status.assets_ready?(remote?"HTTPS 平板服务已开启。这份手稿由平板书写，可以在原平板继续或转交给新设备。":"HTTPS 平板服务已开启。"):"服务已开启，InkPad 页面尚未构建，请重新启动已构建的应用。"):(remote?"平板服务尚未开启。这份手稿由平板书写，开启服务后原平板会自动重连。":"平板服务尚未开启。")));
  }
  private status(message:string):void{this.host.querySelector<HTMLElement>("[data-status]")!.textContent=message;}
}
