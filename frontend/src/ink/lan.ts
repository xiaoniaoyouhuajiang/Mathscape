import QRCode from "qrcode";
import { safeInvoke } from "../core/ipc";
import { inkId,type InkDocumentMetadata,type InkOperation,type InkStroke } from "./model";
import type { StoredInkSession } from "./persistence";

export interface InkLanStatus {enabled:boolean;running:boolean;port:number;urls:string[];certificate_file_sha256:string|null;assets_ready:boolean;error:string|null}
export interface InkWriterView {document_id:string;head_seq:number;writer_epoch:number;session:StoredInkSession|null;remote:boolean;pairing_pending:boolean}
export interface InkLanMirror {delta:{metadata:InkDocumentMetadata;operations:InkOperation[];head_seq:number;server_head:number;writer:InkWriterView;snapshot_count:number};connected:boolean;preview:InkStroke|null;client:{pending_count:number;locally_durable:boolean;secure_context:boolean;page_id:string}|null}
export function inkLanMirror(documentId:string,afterSeq:number):Promise<InkLanMirror>{return safeInvoke("get_ink_lan_mirror",{documentId,afterSeq});}

export class InkLanPanel {
  private link:{urls:string[];expires_at:number;document_id:string}|undefined;
  private documentId="";
  private takeoverRequest:{documentId:string;expectedEpoch:number;expectedHead:number;requestId:string}|undefined;
  constructor(private readonly host:HTMLElement,private readonly prepare:()=>Promise<{documentId:string;head:number}>,private readonly view:()=>InkWriterView|undefined,private readonly changed:()=>Promise<void>) {
    host.className="ink-lan-panel";
    host.innerHTML=`<div class="ink-lan-heading"><strong>平板书写</strong><button data-action="close" type="button">收起</button></div><p>首次连接：先保存设备信任证书，将证书安装到平板，再扫码。设备与电脑需要在同一局域网。</p><div class="ink-lan-actions"><label>端口<input data-port type="number" min="1024" max="65535" value="1443"></label><button data-action="start" type="button">开启服务</button><button data-action="certificate" type="button">保存设备信任证书</button><button data-action="pair" type="button">生成配对二维码</button><button data-action="cancel" type="button">取消待配对</button><button data-action="reclaim" type="button">收回书写权</button><button data-action="stop" type="button">停止平板服务</button></div><div class="ink-lan-pairing" hidden><div><label>电脑地址<select data-address aria-label="配对地址"></select></label><canvas data-qr aria-label="平板配对二维码"></canvas></div><div><input data-link readonly aria-label="配对链接"><button data-action="copy" type="button">复制配对链接</button><p>二维码两分钟有效，仅供一台设备配对。配对完成后无需主动发送文件，书写操作会自动同步到电脑。</p></div></div><p data-status role="status"></p><details><summary>设备信任与恢复说明</summary><p>Android：把导出的 .crt 文件传到平板，在系统安全设置的证书安装入口安装为 CA 证书，再打开二维码中的 HTTPS 地址。关闭服务或收回书写权会使旧设备凭据失效；旧端未同步内容保留在设备上，可以导出恢复草稿后在电脑导入。</p><code data-fingerprint></code></details>`;
    const button=(action:string)=>host.querySelector<HTMLButtonElement>(`[data-action="${action}"]`)!;
    const run=(action:()=>Promise<void>)=>void action().catch(error=>this.status(error instanceof Error?error.message:String(error)));
    button("close").onclick=()=>{host.hidden=true;};
    button("start").onclick=()=>run(async()=>{const port=Number(host.querySelector<HTMLInputElement>("[data-port]")!.value);const status=await safeInvoke<InkLanStatus>("start_ink_lan",{port});this.renderStatus(status);await this.changed();});
    button("certificate").onclick=()=>run(async()=>{const path=await safeInvoke<string|null>("export_ink_trust_certificate");if(path)this.status("证书已保存，请传到平板并安装为 CA 证书。完成后再扫码。");});
    button("pair").onclick=()=>run(async()=>{
      const context=await prepare();this.documentId=context.documentId;
      this.link=await safeInvoke("create_ink_pairing",{documentId:context.documentId,expectedHead:context.head});
      const select=host.querySelector<HTMLSelectElement>("[data-address]")!;select.replaceChildren();
      for(const url of this.link!.urls){const location=new URL(url);select.add(new Option(location.hostname+(location.hostname==="127.0.0.1"||location.hostname==="localhost"?"（本机测试）":""),url));}
      host.querySelector<HTMLElement>(".ink-lan-pairing")!.hidden=false;await this.renderQr();await this.changed();this.status("等待平板扫码；电脑暂时作为只读镜像。二维码过期后可以重新生成。");
    });
    button("cancel").onclick=()=>run(async()=>{const context=await prepare();await safeInvoke("cancel_ink_pairing",{documentId:context.documentId});this.clearQr();await this.changed();this.status("待配对二维码已取消。");});
    button("reclaim").onclick=()=>run(async()=>{
      const current=this.view();if(!current||current.document_id!==this.documentId)return;
      if(!this.takeoverRequest)this.takeoverRequest={documentId:current.document_id,expectedEpoch:current.writer_epoch,expectedHead:current.head_seq,requestId:inkId()};
      try{await safeInvoke("reclaim_ink_writer",this.takeoverRequest);}catch(error){if(String(error).includes("head conflict")||String(error).includes("writer epoch conflict"))this.takeoverRequest=undefined;throw error;}
      this.takeoverRequest=undefined;this.clearQr();await this.changed();this.status("电脑已收回书写权。旧端未同步操作不会自动合入新会话，请在平板导出恢复草稿。");
    });
    button("stop").onclick=()=>run(async()=>{await safeInvoke("stop_ink_lan");this.clearQr();this.renderStatus(await safeInvoke<InkLanStatus>("get_ink_lan_status"));await this.changed();this.status("平板服务已停止，连接凭据已撤销；电脑中的已确认笔迹保留。");});
    button("copy").onclick=()=>run(async()=>{const value=host.querySelector<HTMLInputElement>("[data-link]")!.value;await navigator.clipboard.writeText(value);this.status("配对链接已复制。");});
    host.querySelector<HTMLSelectElement>("[data-address]")!.onchange=()=>run(()=>this.renderQr());
  }
  async open(documentId:string):Promise<void>{this.host.hidden=false;if(this.documentId!==documentId)this.clearQr();this.documentId=documentId;this.renderStatus(await safeInvoke<InkLanStatus>("get_ink_lan_status"));}
  private async renderQr():Promise<void>{const url=this.host.querySelector<HTMLSelectElement>("[data-address]")!.value;if(!url)return;this.host.querySelector<HTMLInputElement>("[data-link]")!.value=url;await QRCode.toCanvas(this.host.querySelector<HTMLCanvasElement>("[data-qr]")!,url,{width:190,margin:2});}
  private clearQr():void{this.link=undefined;this.host.querySelector<HTMLElement>(".ink-lan-pairing")!.hidden=true;}
  private renderStatus(status:InkLanStatus):void {
    this.host.querySelector<HTMLInputElement>("[data-port]")!.value=String(status.port);
    this.host.querySelector<HTMLElement>("[data-fingerprint]")!.textContent=status.certificate_file_sha256?"证书文件 SHA-256："+status.certificate_file_sha256:"";
    this.host.querySelector<HTMLButtonElement>('[data-action="pair"]')!.disabled=!status.running||!status.assets_ready;
    this.status(status.error??(status.running?(status.assets_ready?"HTTPS 平板服务已开启。":"服务已开启，InkPad 页面尚未构建，请重新启动已构建的应用。"):"平板服务尚未开启。"));
  }
  private status(message:string):void{this.host.querySelector<HTMLElement>("[data-status]")!.textContent=message;}
}
