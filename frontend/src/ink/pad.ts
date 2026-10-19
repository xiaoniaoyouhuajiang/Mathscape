import { InkCanvas,type CanvasMetrics,type InkTool } from "./canvas";
import { appendInk,encodeInk,inkId,inkMetadata,normalizeInkTitle,INK_TITLE_MAX,parseInkDocument,projectInk,type InkDocument,type InkStroke,type InkBrush,type CaptureSegment,type InkPage,type InkProjection } from "./model";
import { InkPadStore,type PadBinding,type PadDraft } from "./pad-store";
import { InkPadTransport,PadTransportError,type PadContext,type PadNote,type PadSwitchOffer } from "./pad-transport";
import type { InkOutboxItem } from "./outbox";
import { isDeleteKey,isTypingTarget } from "./keys";
import { createInkPalette,INK_DEFAULT_COLOR,PAD_COLOR_KEY,type InkPalette } from "./palette";
import "./lab.css";
import "./pad.css";

const root=document.querySelector<HTMLElement>("#inkpad-root")!;
function element<T extends HTMLElement>(selector:string):T{return root.querySelector<T>(selector)!;}
function download(blob:Blob,name:string):void{const url=URL.createObjectURL(blob),anchor=document.createElement("a");anchor.href=url;anchor.download=name;document.body.append(anchor);anchor.click();anchor.remove();setTimeout(()=>URL.revokeObjectURL(url),1000);}
function credential():string{return Array.from(crypto.getRandomValues(new Uint8Array(32)),value=>value.toString(16).padStart(2,"0")).join("");}
/** Offline rename intent, kept in the meta store under `rename:<binding key>`. */
interface RenameIntent { request_id:string;document_id:string;title:string;expected:number }
function canonical(value:unknown):string {function ordered(value:unknown):unknown {if(Array.isArray(value))return value.map(ordered);if(value!==null&&typeof value==="object")return Object.fromEntries(Object.entries(value).sort(([a],[b])=>a.localeCompare(b)).map(([key,value])=>[key,ordered(value)]));return value;}return JSON.stringify(ordered(value));}

class InkPad {
  private database:InkPadStore|undefined;
  private binding:PadBinding|undefined;
  private document:InkDocument|undefined;
  private page="";
  private segment:CaptureSegment|undefined;
  private tool:InkTool="pen";
  private board:InkCanvas|undefined;
  private palette:InkPalette|undefined;
  private transport:InkPadTransport|undefined;
  private confirmed=0;
  private pending=0;
  private persisted=true;
  private fenced=false;
  private fenceCode:string|undefined;
  private enqueueTail:Promise<void>=Promise.resolve();
  private buffered=new Map<string,{draft:PadDraft;item:InkOutboxItem}>();
  private flushing:Promise<boolean>|undefined;
  private state:"connecting"|"ready"|"offline"|"conflict"="offline";
  private metrics:CanvasMetrics={pointerEvents:0,coalescedSamples:0,cancelled:0,lastPointer:"—",frameMs:0,activePoints:0};
  private snapshotRequest:{id:string;head:number}|undefined;
  private projection:{document:InkDocument;count:number;value:InkProjection}|undefined;
  private lastMetricsSummary=0;
  private navigationRevision=0;
  private note:PadNote|undefined;
  private renaming:Promise<void>|undefined;

  async start():Promise<void> {
    root.innerHTML=`<main class="lab-pad"><header class="pad-header"><div class="pad-title"><span class="lab-mark">M</span><strong id="pad-title">Mathscape InkPad</strong><button id="pad-rename" type="button" class="pad-rename-button" title="修改手稿标题">改名</button><span class="lab-badge">P3</span></div><span id="pad-connection" role="status">正在准备本机存储…</span></header>
      <form id="pad-rename-form" class="pad-rename" hidden><input id="pad-rename-input" maxlength="${INK_TITLE_MAX}" aria-label="手稿标题" placeholder="留空恢复自动标题" autocomplete="off"><button id="pad-rename-save" type="submit">保存标题</button><button id="pad-rename-cancel" type="button">取消</button></form>
      <div id="pad-title-choice" class="pad-rename" role="group" aria-label="标题冲突" hidden><span id="pad-title-choice-text"></span><button id="pad-title-theirs" type="button"></button><button id="pad-title-mine" type="button"></button></div>
      <nav class="ink-toolbar" aria-label="书写工具"><div class="ink-tool-group"><button id="pad-pen" class="is-active">笔</button><button id="pad-select">套索选择</button><button id="pad-erase">整笔擦除</button><button id="pad-partial_erase">局部擦除</button><button id="pad-undo">撤销</button><button id="pad-redo">重做</button><button id="pad-delete-selection" disabled title="删除套索选中的笔迹，可撤销">删除所选</button><button id="pad-restyle" disabled title="把选中笔迹改成当前颜色，可撤销">改颜色</button></div>
      <div class="ink-tool-group"><span id="pad-palette-host"></span><label class="ink-control">笔宽<input id="pad-width" type="range" min="1" max="16" step="0.5" value="4"></label><label class="ink-control"><input id="pad-finger" type="checkbox">手指书写</label></div>
      <div class="ink-tool-group"><select id="pad-page" aria-label="页面"></select><button id="pad-add-page">新增页面</button><button id="pad-zoom-out" aria-label="缩小">−</button><button id="pad-fit">整页</button><button id="pad-zoom-in" aria-label="放大">＋</button></div></nav>
      <section id="pad-board" class="ink-board" aria-label="平板画板"></section>
      <footer class="pad-footer"><div class="pad-save-state"><span id="pad-local-status">尚未打开手稿</span><span id="pad-desktop-status">尚未连接电脑</span></div><div class="ink-file-actions"><button id="pad-sync">立即同步</button><button id="pad-snapshot">保存快照</button><button id="pad-export">导出恢复草稿</button><button id="pad-png">导出 PNG</button><button id="pad-retry">重试连接</button><select id="pad-drafts" aria-label="最近手稿"></select></div><p id="pad-status" class="lab-status" role="status">请使用 Mathscape 中展示的二维码配对。</p><details class="ink-diagnostics"><summary>输入与恢复诊断</summary><div id="pad-diagnostics"></div></details></footer></main>`;
    // N3.0: shared presets + custom colour; the tablet remembers its own choice.
    this.palette=createInkPalette({storageKey:PAD_COLOR_KEY,idPrefix:"pad"});element("#pad-palette-host").replaceWith(this.palette.element);
    this.events();this.controls();
    try{
      this.database=await InkPadStore.open();
      const target=new URLSearchParams(location.hash.slice(1));
      let missingDocument=false;
      if(target.has("doc")&&!target.has("pair")){
        // Resume links carry no credential; they select one already stored on this device.
        const id=target.get("doc")!;
        const match=(await this.database.drafts()).filter(draft=>draft.binding.lease.document_id===id).sort((a,b)=>b.binding.lease.writer_epoch-a.binding.lease.writer_epoch)[0];
        if(match)await this.database.activate(match.binding);else missingDocument=true;
        history.replaceState(null,"",location.pathname);
      }
      this.binding=missingDocument?undefined:await this.database.active();
      if(this.binding){const cached=await this.database.draft(this.binding.key);if(cached){this.confirmed=cached.confirmed_head;this.mount(parseInkDocument(cached.document),cached.page_id);this.pending=(await this.database.pending(this.binding)).length;}}
      await this.refreshDrafts();
      const token=new URLSearchParams(location.hash.slice(1)).get("pair");
      if(token){
        await this.enqueueTail;this.clearSurface();
        this.message("正在配对当前手稿…");
        await this.acceptPairing(token,"qr");
        history.replaceState(null,"",location.pathname);
      }
      if(this.binding)this.connect();else element("#pad-connection").textContent="尚未配对";
      const quiet=this.fenced||missingDocument;
      if("serviceWorker" in navigator){try{if(!navigator.serviceWorker.controller)await navigator.serviceWorker.register("/pad/sw.js",{scope:"/pad/"});await navigator.serviceWorker.ready;if(!quiet&&!this.fenced)this.message("画板已准备离线恢复；书写会自动同步到电脑。");}catch{if(!quiet&&!this.fenced)this.message("操作队列可恢复，离线页面缓存未完成；保持本页打开或先导出恢复草稿。");}}
      if(missingDocument)this.message("这台设备没有这份手稿的书写授权。请在电脑的“平板书写”中选择“换一台设备书写”，再扫码。");
      setInterval(()=>{this.transport?.report(this.pending,this.persisted,this.page);this.summary();},1000);
    }catch(error){this.message(error instanceof Error?error.message:String(error));element("#pad-connection").textContent="配对或本机恢复存储不可用";}
  }
  private events():void {
    const form=element<HTMLFormElement>("#pad-rename-form"),input=element<HTMLInputElement>("#pad-rename-input");
    element<HTMLButtonElement>("#pad-rename").onclick=()=>{if(!this.document||this.fenced)return;element("#pad-title-choice").hidden=true;form.hidden=false;input.value=this.document.title;input.focus();input.select();};
    element<HTMLButtonElement>("#pad-rename-cancel").onclick=()=>{form.hidden=true;};
    input.onkeydown=event=>{if(event.key==="Escape"){event.preventDefault();form.hidden=true;}};
    form.onsubmit=event=>{event.preventDefault();void this.requestRename(input.value);};
    for(const tool of ["pen","select","erase","partial_erase"] as const)element<HTMLButtonElement>("#pad-"+tool).onclick=()=>{this.board?.resetTransientInteraction();this.tool=tool;for(const value of ["pen","select","erase","partial_erase"])element("#pad-"+value).classList.toggle("is-active",value===tool);this.board?.refresh();this.summary();};
    // N3.0: delete the lasso selection with one multi-stroke erase (one undo restores it).
    element<HTMLButtonElement>("#pad-delete-selection").onclick=()=>{if(!this.document||this.fenced||this.tool!=="select")return;this.board?.deleteSelection();this.summary();};
    // N3a: one restyle command for the whole selection (one undo restores).
    element<HTMLButtonElement>("#pad-restyle").onclick=()=>{if(!this.document||this.fenced||this.tool!=="select"||!this.board?.selectionCount())return;const ids=this.board.selectedIds();const color=this.palette?.value()??INK_DEFAULT_COLOR;this.commit({kind:"restyle",stroke_ids:ids,color});this.board.setSelection(ids);this.summary();};
    window.addEventListener("keydown",event=>{if(!isDeleteKey(event)||isTypingTarget(event.target)||!this.document||this.fenced||this.tool!=="select"||!this.board?.selectionCount())return;event.preventDefault();this.board.deleteSelection();this.summary();});
    for(const kind of ["undo","redo"] as const)element<HTMLButtonElement>("#pad-"+kind).onclick=()=>{if(!this.document||this.fenced)return;this.board?.resetTransientInteraction();const target=projectInk(this.document)[kind].at(-1);if(target)this.commit({kind,target_command_id:target});};
    element<HTMLButtonElement>("#pad-add-page").onclick=()=>{if(!this.document||this.fenced)return;this.board?.resetTransientInteraction();const page:InkPage={id:inkId(),width:1200,height:1600};this.document.pages.push(page);this.commit({kind:"page_add",page});this.page=page.id;this.pages();this.board?.setSelection([]);this.board?.fit();};
    element<HTMLSelectElement>("#pad-page").onchange=()=>{this.board?.resetTransientInteraction();this.page=element<HTMLSelectElement>("#pad-page").value;this.board?.setSelection([]);this.board?.fit();this.summary();};
    element<HTMLButtonElement>("#pad-fit").onclick=()=>this.board?.fit();element<HTMLButtonElement>("#pad-zoom-in").onclick=()=>this.board?.zoom(1.3);element<HTMLButtonElement>("#pad-zoom-out").onclick=()=>this.board?.zoom(1/1.3);
    element<HTMLButtonElement>("#pad-sync").onclick=()=>{this.board?.finish();void this.retryBuffered().then(()=>{if(this.fenceCode==="local_storage_failed"&&!this.buffered.size)this.connect();else void this.flush();});};
    element<HTMLButtonElement>("#pad-retry").onclick=()=>{void this.retryBuffered().then(()=>{if(this.fenced&&!["device_busy","local_storage_failed"].includes(this.fenceCode??""))this.message("会话已经失效，原队列不会写入新会话。请先导出恢复草稿，再在电脑创建恢复手稿。");else{this.transport?.close();this.connect();}});};
    element<HTMLButtonElement>("#pad-snapshot").onclick=()=>void this.snapshot();
    element<HTMLButtonElement>("#pad-export").onclick=()=>{if(!this.document)return;try{this.board?.finish();let json:string;try{json=encodeInk(this.document);}catch{json=JSON.stringify(this.document);}download(new Blob([json],{type:"application/json"}),`mathscape-ink-recovery-${this.document.document_id}.json`);this.message("完整笔迹及编辑历史已生成恢复文件，请确认设备保存成功。");}catch(error){this.message(String(error));}};
    element<HTMLButtonElement>("#pad-png").onclick=()=>{try{this.board?.png().toBlob(blob=>{if(blob)download(blob,"mathscape-ink-page.png");});}catch(error){this.message(String(error));}};
    element<HTMLSelectElement>("#pad-drafts").onchange=()=>void this.openDraft(element<HTMLSelectElement>("#pad-drafts").value);
    window.addEventListener("online",()=>this.transport?.connect());
    window.addEventListener("pagehide",()=>{this.board?.finish();this.transport?.close();});
    window.addEventListener("pageshow",event=>{if(event.persisted&&this.binding)this.connect();});
    window.addEventListener("hashchange",()=>{
      const next=new URLSearchParams(location.hash.slice(1));
      if(!next.has("pair")&&!next.has("doc"))return;
      this.board?.finish();void this.enqueueTail.then(()=>{if(this.buffered.size)this.message("当前笔迹尚未保存到本机，请先导出恢复草稿，再打开新的配对链接。");else location.reload();});
    });
    window.addEventListener("beforeunload",event=>{if(this.buffered.size){event.preventDefault();event.returnValue="";}});
  }
  private brush():InkBrush{return {renderer:"perfect-freehand@1.2.3",color:this.palette?.value()??INK_DEFAULT_COLOR,width:Number(element<HTMLInputElement>("#pad-width").value),smoothing:0.35,streamline:0.15,thinning:0.4,simulate_pressure:false};}
  private segmentId():string {
    if(!this.segment){this.segment={id:inkId(),started_at:new Date().toISOString(),user_agent:navigator.userAgent,secure_context:window.isSecureContext,coalesced_events:typeof PointerEvent.prototype.getCoalescedEvents==="function",pointer_raw_update:"onpointerrawupdate" in window,device_pixel_ratio:window.devicePixelRatio||1,time_origin_ms:performance.timeOrigin,pressure_capability:"unknown",tilt_capability:"unknown"};this.document!.capture_segments.push(this.segment);}return this.segment.id;
  }
  private mount(document:InkDocument,pageId?:string):void {
    this.board?.dispose();this.document=document;this.page=document.pages.some(page=>page.id===pageId)?pageId!:document.pages[0].id;this.segment=undefined;
    this.board=new InkCanvas(element("#pad-board"),{document:()=>this.document!,page:()=>this.document!.pages.find(page=>page.id===this.page)!,segmentId:()=>this.segmentId(),brush:()=>this.brush(),tool:()=>this.tool,fingerWriting:()=>element<HTMLInputElement>("#pad-finger").checked,readOnly:this.fenced,refitOnResize:true,
      onStroke:stroke=>this.commit({kind:"stroke",stroke}),onErase:stroke_ids=>this.commit({kind:"erase",stroke_ids}),onPartialErase:(source_stroke_id,fragments)=>this.commit(fragments.length?{kind:"partial_erase",source_stroke_id,fragments}:{kind:"erase",stroke_ids:[source_stroke_id]}),onTransform:(stroke_ids,matrix)=>this.commit({kind:"transform",stroke_ids,matrix}),onSelection:()=>this.summary(),onMetrics:metrics=>{this.metrics=metrics;if(performance.now()-this.lastMetricsSummary>=100){this.lastMetricsSummary=performance.now();this.summary();}},onMessage:message=>this.message(message),onPreview:stroke=>this.transport?.preview(stroke)});
    element("#pad-title").textContent=document.title;this.pages();this.controls();this.board.refresh();
  }
  /** Tear down the canvas before clearing the document so callbacks cannot read `.pages` on undefined. */
  private clearSurface():void {
    this.board?.finish();
    this.board?.dispose();
    this.board=undefined;
    this.document=undefined;
    this.page="";
    this.segment=undefined;
    this.projection=undefined;
    this.note=undefined;
    this.snapshotRequest=undefined;
    this.fenced=false;
    this.fenceCode=undefined;
    this.confirmed=0;
    this.pending=0;
    element("#pad-title").textContent="Mathscape InkPad";
    this.controls();
  }
  /** Shared with the #pair= bootstrap path: activate the lease then reconnect for session_context → mount. */
  private async acceptPairing(pairingToken:string,via:"qr"|"desktop_push"):Promise<void> {
    if(!this.database)throw new Error("本机存储尚未就绪。");
    if(!/^[0-9a-f]{64}$/.test(pairingToken))throw new Error(via==="desktop_push"?"切换令牌无效。":"二维码凭据不完整，请重新扫码。");
    const pair=await this.database.pairing(pairingToken,credential());
    const response=await fetch("/api/pair",{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({pairing_token:pair.token,resume_secret:pair.credential,device:{user_agent:navigator.userAgent,secure_context:window.isSecureContext,storage_recovery:true,via}})});
    const result=await response.json();
    if(!response.ok)throw new Error(result.message??(via==="desktop_push"?"切换失败，请在电脑重新推送或扫码。":"配对失败，请在电脑重新生成二维码。"));
    const binding:PadBinding={key:`${result.lease.session_id}:${result.lease.writer_epoch}`,credential:pair.credential,lease:result.lease};
    await this.database.activate(binding);
    this.binding=binding;
    this.buffered.clear();
    this.persisted=true;
  }
  private draft(document=this.document!,binding=this.binding!):PadDraft {return {key:binding.key,binding,document:{...inkMetadata(document),operations:document.operations},confirmed_head:this.confirmed,page_id:this.page,updated_at:new Date().toISOString()};}
  private commit(command:Parameters<typeof appendInk>[1]):void {
    if(!this.document||!this.binding||!this.database||this.fenced)return;
    try{
      appendInk(this.document,command);const operation=this.document.operations.at(-1)!;
      const item:InkOutboxItem={request_id:inkId(),document_id:this.document.document_id,session_id:this.binding.lease.session_id,writer_epoch:this.binding.lease.writer_epoch,expected_head:operation.seq-1,client_seq:operation.seq,queued_at:new Date().toISOString(),metadata:inkMetadata(this.document),operation};
      const draft=this.draft();this.buffered.set(item.request_id,{draft,item});this.pending++;this.persisted=false;
      this.enqueueTail=this.enqueueTail.then(async()=>{
        try{await this.database!.enqueue(draft,item);this.buffered.delete(item.request_id);this.persisted=!this.buffered.size;}
        catch(error){this.persisted=false;if(!this.fenced){this.board?.finish();this.fenced=true;this.fenceCode="local_storage_failed";this.transport?.close();this.state="conflict";element("#pad-connection").textContent="本机保存失败 · 已暂停书写";if(this.document)this.mount(this.document,this.page);}this.message("本机保存失败，已暂停书写，内容仍在画板中，请先导出恢复草稿或重试同步。"+String(error));}
        this.summary();
      });
      void this.enqueueTail.then(()=>this.flush());this.board?.refresh();this.summary();
    }catch(error){this.message(String(error));}
  }
  private async retryBuffered():Promise<void> {await this.enqueueTail;for(const [id,value] of this.buffered){try{await this.database!.enqueue(value.draft,value.item);this.buffered.delete(id);}catch(error){this.message(String(error));break;}}this.persisted=!this.buffered.size;this.summary();}
  private connect():void {
    if(!this.binding||!this.database)return;
    this.transport?.close();
    const binding=this.binding;
    const current=()=>this.transport===transport&&this.binding?.key===binding.key;
    const transport=new InkPadTransport(()=>binding,context=>this.restore(context,binding,current),(state,message,code)=>{
      if(!current())return;
      this.state=state;element("#pad-connection").textContent=message;
      if(state==="conflict"){
        this.board?.finish();this.fenced=true;this.fenceCode=code;if(this.document)this.mount(this.document,this.page);this.message(message);
        if(code==="protocol_mismatch"){const status=element("#pad-status");if(!status.querySelector("[data-reload]")){const button=document.createElement("button");button.type="button";button.dataset.reload="";button.textContent="刷新页面";button.onclick=()=>location.reload();status.append(" ",button);}}
        if(["writer_revoked","credential_expired","authentication_failed"].includes(code??""))void this.database?.markNeedsReauth(binding.key).then(()=>this.refreshDrafts());
      }
      if(state==="ready")void this.database?.clearNeedsReauth(binding.key).then(()=>this.refreshDrafts());
      this.controls();this.summary();
    },()=>{void this.sendRename();void (this.flushing??Promise.resolve()).then(()=>{if(current())void this.flush();});},note=>{if(current())void this.applyNote(note);},offer=>{if(current())void this.handleSwitchOffer(offer);});this.transport=transport;transport.connect();
  }
  private async restore(context:PadContext,binding:PadBinding,current:()=>boolean):Promise<void> {
    if(!current())return;
    if(context.lease.document_id!==binding.lease.document_id||context.lease.session_id!==binding.lease.session_id||context.lease.writer_epoch!==binding.lease.writer_epoch)throw new PadTransportError("writer_revoked","会话身份或写入权已变化，原恢复队列将保持不变。");
    this.board?.finish();await this.enqueueTail;
    if(!current())return;
    if(this.buffered.size)throw new Error("本机尚有未成功保存的内容，请导出恢复草稿后重试。");
    const document=parseInkDocument(context.document);const pending=await this.database!.pending(binding);
    for(const item of pending){
      if(item.operation.seq<=context.head_seq){if(canonical(document.operations[item.operation.seq-1])!==canonical(item.operation))throw new PadTransportError("sequence_conflict","本机队列与电脑历史不一致，请导出恢复草稿。");}
      else {if(item.operation.seq!==document.operations.length+1)throw new PadTransportError("sequence_conflict","恢复队列有序号缺口，原内容将保持不变。");Object.assign(document,structuredClone(item.metadata));document.operations.push(structuredClone(item.operation));}
    }
    // Queued metadata carries the title from when it was written; the server's note wins (H1).
    document.title=context.note?.title??context.document.title;this.note=context.note;
    parseInkDocument(document);if(!current())return;
    await this.database!.cache({...this.draft(document,binding),confirmed_head:context.head_seq});
    if(!current())return;
    this.confirmed=context.head_seq;this.pending=pending.length;this.fenced=false;this.fenceCode=undefined;
    this.mount(document,this.page);await this.refreshDrafts();
  }
  private async requestRename(raw:string):Promise<void> {
    const binding=this.binding,document=this.document;if(!binding||!document||!this.database||this.fenced)return;
    let title:string;try{title=normalizeInkTitle(raw)??"";}catch(error){this.message(error instanceof Error?error.message:String(error));return;}
    element("#pad-rename-form").hidden=true;
    if(!this.note){this.message("电脑上的 Mathscape 版本不支持从平板改名，请在电脑上修改标题。");return;}
    if(title===document.title||(!title&&this.note.title_mode==="auto"))return;
    const intent:RenameIntent={request_id:inkId(),document_id:binding.lease.document_id,title,expected:this.note.title_revision};
    await this.database.metaPut("rename:"+binding.key,intent);
    this.message(this.transport?.isReady()?"正在同步标题…":"新标题已保存在本机，连接电脑后自动同步。");
    await this.sendRename();
  }
  /** Sends the stored rename intent once; retries reuse its request id. */
  private async sendRename():Promise<void> {
    if(this.renaming)return this.renaming;
    const task=this.sendRenameOnce();this.renaming=task;try{await task;}finally{if(this.renaming===task)this.renaming=undefined;}
  }
  private async sendRenameOnce():Promise<void> {
    const binding=this.binding,transport=this.transport,database=this.database;
    if(!binding||!database||!transport?.isReady()||this.fenced)return;
    const key="rename:"+binding.key,intent=await database.metaGet<RenameIntent>(key);if(!intent)return;
    try{
      const note=await transport.rename(intent.request_id,intent.title,intent.expected);
      await database.metaDelete(key);await this.applyNote(note);
      this.message(intent.title?"标题已同步到电脑。":"已恢复自动标题。");
    }catch(error){
      const code=error instanceof PadTransportError?error.code:"";
      if(code==="offline"||code==="ack_timeout"){this.message("标题尚未同步，重新连接后会自动重试。");return;}
      await database.metaDelete(key);
      if(code==="title_conflict"&&error instanceof PadTransportError){this.titleConflict(binding,intent,String(error.details.current_title??""),Number(error.details.current_title_revision??intent.expected));return;}
      this.message("改名失败："+(error instanceof Error?error.message:String(error)));
    }
  }
  private titleConflict(binding:PadBinding,intent:RenameIntent,current:string,revision:number):void {
    const box=element("#pad-title-choice");
    element("#pad-title-choice-text").textContent=`标题已在电脑上改为「${current}」。`;
    const theirs=element<HTMLButtonElement>("#pad-title-theirs"),mine=element<HTMLButtonElement>("#pad-title-mine");
    theirs.textContent=`保留「${current}」`;mine.textContent=`使用「${intent.title||"自动标题"}」`;
    theirs.onclick=()=>{box.hidden=true;this.message("已保留电脑上的标题。");};
    mine.onclick=()=>{box.hidden=true;if(this.binding?.key!==binding.key||!this.database)return;
      void this.database.metaPut("rename:"+binding.key,{...intent,request_id:inkId(),expected:revision}).then(()=>this.sendRename());};
    box.hidden=false;
  }
  private async applyNote(note:PadNote):Promise<void> {
    const binding=this.binding;if(!binding||note.document_id!==binding.lease.document_id)return;
    if(this.note&&this.note.title_revision>note.title_revision)return;
    this.note=note;
    if(!this.document)return;
    this.document.title=note.title;element("#pad-title").textContent=note.title;
    try{await this.database?.cache(this.draft());await this.refreshDrafts();}catch{/* The draft title refreshes on the next save. */}
  }
  private async flush():Promise<boolean> {
    if(this.flushing)return this.flushing;
    const task=this.flushSerial();this.flushing=task;try{return await task;}finally{if(this.flushing===task)this.flushing=undefined;}
  }
  private async flushSerial():Promise<boolean> {
    await this.enqueueTail;
    const binding=this.binding,document=this.document,transport=this.transport;
    if(!binding||!document||!transport?.isReady()||!this.database||this.buffered.size||this.fenced)return false;
    try{
      for(;;){const items=await this.database.pending(binding);if(!items.length)break;
        for(const item of items){const previous=document.operations[item.operation.seq-2]?.command_id??null;const receipt=await transport.submit(item,previous);await this.database.ack(item.request_id,binding,receipt.seq);
          if(this.binding?.key===binding.key){this.confirmed=Math.max(this.confirmed,receipt.seq);this.pending=Math.max(0,this.pending-1);this.summary();}}
        await this.enqueueTail;if(this.buffered.size)throw new Error("本机队列尚未保存完成。");
      }
      if(this.binding?.key===binding.key){this.pending=(await this.database.pending(binding)).length;this.summary();}return true;
    }catch(error){this.message("同步尚未完成，原操作会保留后重试。"+(error instanceof Error?error.message:String(error)));return false;}
  }
  private async snapshot():Promise<void> {
    if(!this.document||!this.binding)return;const binding=this.binding;this.board?.finish();await this.enqueueTail;
    if(this.binding!==binding)return;
    if(!await this.flush()){this.message("请先完成同步，再保存包含当前书写内容的快照。");return;}
    if(this.binding!==binding)return;
    if(!this.snapshotRequest)this.snapshotRequest={id:inkId(),head:this.confirmed};
    const request=this.snapshotRequest;
    try{const result=await this.transport!.snapshot(request.id,request.head);if(this.binding===binding&&this.snapshotRequest===request){this.message(`电脑已保存快照，截止 head ${result.through_seq}。可以在 Mathscape 的历史快照中查看。`);this.snapshotRequest=undefined;}}
    catch(error){if(this.binding===binding)this.message("快照尚未确认，重试将沿用原请求。"+String(error));}
  }

  /** N3a: desktop push-switch. Drain the current note first; never discard strokes. */
  private async handleSwitchOffer(offer:PadSwitchOffer):Promise<void> {
    if(!this.database)return;
    if(offer.document_id===this.binding?.lease.document_id){this.message(`电脑推送的是当前手稿「${offer.title}」，无需切换。`);return;}
    const takeover=offer.requires_takeover
      ? "\n\n注意：目标手稿正被其他设备书写。接手后对方未同步的内容会被隔离，对方可以导出恢复草稿。"
      : "";
    const choice=window.confirm(`${offer.message}\n\n切换前会先把当前手稿保存到本机。未同步的内容会保留在「最近手稿」里。${takeover}\n\n要切换到「${offer.title}」吗？`);
    if(!choice){this.message("已取消切换。");return;}
    this.board?.finish();await this.enqueueTail;
    if(this.buffered.size){this.message("当前笔迹尚未保存到本机，请先导出恢复草稿或点“立即同步”，再接受切换。");return;}
    try{
      if(this.transport?.isReady())await this.flush();
    }catch{/* Keep going; local draft is durable. */}
    this.transport?.close();
    // Same teardown as the #pair= bootstrap: dispose the old canvas before clearing the document.
    this.clearSurface();
    try{
      this.message(`正在切换到「${offer.title}」…`);
      await this.acceptPairing(offer.pairing_token,"desktop_push");
      this.connect();
      await this.refreshDrafts();
      // Title/canvas come from session_context → restore → mount; wait until then before claiming success.
    }catch(error){
      this.message("切换未完成："+(error instanceof Error?error.message:String(error)));
      if(this.binding)this.connect();
      return;
    }
    // restore() mounts asynchronously via the transport; surface the intent immediately,
    // then refreshDrafts again once ready so the new title is selected.
    this.message(`正在打开「${offer.title}」…`);
    const title=offer.title;
    const waitReady=async()=>{
      for(let i=0;i<50;i++){
        if(this.document&&this.document.title===title&&this.state==="ready")return true;
        await new Promise(resolve=>setTimeout(resolve,100));
      }
      return !!(this.document&&this.document.title===title);
    };
    if(await waitReady())this.message(`已切换到「${title}」。`);
    else this.message(`已配对「${title}」，正在等待电脑会话…`);
    await this.refreshDrafts();
  }

  private async refreshDrafts():Promise<void> {
    const select=element<HTMLSelectElement>("#pad-drafts");select.replaceChildren();
    for(const draft of await this.database!.drafts()){
      const needs=await this.database!.needsReauth(draft.key);
      select.add(new Option(needs?`${draft.document.title} · 需要电脑重新授权`:`${draft.document.title} · 最近手稿`,draft.key));
    }
    if(this.binding)select.value=this.binding.key;
  }
  private async openDraft(key:string):Promise<void> {
    if(!this.database||(key===this.binding?.key&&this.document))return;const revision=++this.navigationRevision;this.board?.finish();await this.enqueueTail;if(revision!==this.navigationRevision)return;this.transport?.close();
    if(this.buffered.size){element<HTMLSelectElement>("#pad-drafts").value=this.binding?.key??"";this.message("当前笔迹尚未保存到本机，请先导出恢复草稿或重试保存，再切换记录。");return;}
    const draft=await this.database.draft(key);if(!draft||revision!==this.navigationRevision)return;await this.database.activate(draft.binding);const pending=await this.database.pending(draft.binding);if(revision!==this.navigationRevision)return;this.binding=draft.binding;this.snapshotRequest=undefined;this.confirmed=draft.confirmed_head;this.pending=pending.length;this.fenced=false;this.fenceCode=undefined;this.persisted=true;this.mount(parseInkDocument(draft.document),draft.page_id);this.connect();
  }
  private pages():void {const select=element<HTMLSelectElement>("#pad-page");select.replaceChildren();this.document?.pages.forEach((page,index)=>select.add(new Option(`第 ${index+1} 页`,page.id)));select.value=this.page;}
  private controls():void {const disabled=!this.document||this.fenced;element<HTMLButtonElement>("#pad-rename").disabled=disabled;if(disabled)element("#pad-rename-form").hidden=true;for(const name of ["pen","erase","partial_erase","undo","redo","add-page"])element<HTMLButtonElement>("#pad-"+name).disabled=disabled;for(const name of ["export","png"])element<HTMLButtonElement>("#pad-"+name).disabled=!this.document;element<HTMLButtonElement>("#pad-snapshot").disabled=!this.transport?.isReady()||this.fenced;}
  private summary():void {
    if(!this.document)return;
    if(this.projection?.document!==this.document||this.projection.count!==this.document.operations.length)this.projection={document:this.document,count:this.document.operations.length,value:projectInk(this.document)};
    const projection=this.projection.value;element<HTMLButtonElement>("#pad-undo").disabled=this.fenced||!projection.undo.length;element<HTMLButtonElement>("#pad-delete-selection").disabled=this.fenced||this.tool!=="select"||!this.board?.selectionCount();element<HTMLButtonElement>("#pad-restyle").disabled=this.fenced||this.tool!=="select"||!this.board?.selectionCount();element<HTMLButtonElement>("#pad-redo").disabled=this.fenced||!projection.redo.length;
    element("#pad-local-status").textContent=this.persisted?`本机已保存 · 待同步 ${this.pending} 个操作`:`本机尚有 ${this.buffered.size} 个操作未保存`;
    element("#pad-desktop-status").textContent=`电脑已保存至 head ${this.confirmed}`;
    element("#pad-diagnostics").textContent=`安全上下文：${window.isSecureContext} · IndexedDB 恢复队列：${!!this.database}\n${projection.visible.length} 笔 · ${projection.pointCount+this.metrics.activePoints} 点 · 本机 head ${this.document.operations.length}\n输入：${this.metrics.lastPointer} · 合并采样 ${this.metrics.coalescedSamples} · 前景绘制 ${this.metrics.frameMs.toFixed(2)} ms\n连接：${this.state} · 写入 epoch ${this.binding?.lease.writer_epoch??"—"}`;
  }
  private message(text:string):void {element("#pad-status").textContent=text;}
}
void new InkPad().start();
