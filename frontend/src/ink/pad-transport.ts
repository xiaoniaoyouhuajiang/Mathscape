import { inkId, type InkDocument, type InkStroke } from "./model";
import type { InkOperationAck, InkOutboxItem } from "./outbox";
import type { PadBinding, PadLease } from "./pad-store";

export const INK_LAN_PROTOCOL="mathscape-ink-lan-1";
export class PadTransportError extends Error { constructor(readonly code:string,message:string){super(message);} }
export interface PadContext { type:"session_context";protocol_version:string;lease:PadLease;document:InkDocument;head_seq:number;limits:{operation_bytes:number;document_bytes:number;queue_bytes:number} }
interface Pending {resolve:(value:Record<string,unknown>)=>void;reject:(error:Error)=>void;timer:ReturnType<typeof setTimeout>}
function tabClientId():string {
  // Survives refresh in this tab, so a half-open old socket can be replaced.
  // The device credential still scopes authorization; another tab has its own ID.
  try{const key="mathscape-inkpad-tab";const existing=sessionStorage.getItem(key);if(existing)return existing;const id=inkId();sessionStorage.setItem(key,id);return id;}catch{return inkId();}
}

export class InkPadTransport {
  private socket:WebSocket|undefined;
  private ready=false;
  private closed=false;
  private retry:ReturnType<typeof setTimeout>|undefined;
  private pending=new Map<string,Pending>();
  private attempts=0;
  private readonly clientId=tabClientId();
  private heartbeat:ReturnType<typeof setInterval>|undefined;
  private lastPreview=0;
  constructor(
    private readonly binding:()=>PadBinding|undefined,
    private readonly context:(context:PadContext)=>Promise<void>,
    private readonly status:(state:"connecting"|"ready"|"offline"|"conflict",message:string,code?:string)=>void,
    private readonly onReady:()=>void,
  ) {}
  isReady():boolean{return this.ready;}
  connect():void {
    if(this.closed||!this.binding()||this.socket?.readyState===WebSocket.OPEN||this.socket?.readyState===WebSocket.CONNECTING)return;
    if(this.retry){clearTimeout(this.retry);this.retry=undefined;}
    const url=new URL("/api/ws",location.href);url.protocol="wss:";
    this.status("connecting","正在连接电脑…");
    const socket=new WebSocket(url);this.socket=socket;
    socket.onopen=()=>socket.send(JSON.stringify({type:"hello",protocol_version:INK_LAN_PROTOCOL,credential:this.binding()!.credential,client_id:this.clientId}));
    socket.onmessage=event=>void this.receive(socket,event.data);
    socket.onerror=()=>{ /* close reports the failure and schedules one retry. */ };
    socket.onclose=()=>{
      if(this.socket!==socket)return;
      this.socket=undefined;this.ready=false;if(this.heartbeat)clearInterval(this.heartbeat);
      for(const task of this.pending.values()){clearTimeout(task.timer);task.reject(new PadTransportError("offline","电脑连接中断，操作仍保留在本机。"));}this.pending.clear();
      if(!this.closed){this.status("offline","电脑连接中断 · 本机可继续书写");this.retry=setTimeout(()=>this.connect(),Math.min(15000,1000*2**Math.min(this.attempts++,4)));}
    };
  }
  private async receive(socket:WebSocket,input:unknown):Promise<void> {
    if(this.socket!==socket||typeof input!=="string")return;
    let message:Record<string,unknown>;
    try{message=JSON.parse(input);}catch{return;}
    if(message.type==="session_context") {
      try {
        const context=message as unknown as PadContext;
        if(context.protocol_version!==INK_LAN_PROTOCOL)throw new PadTransportError("protocol_mismatch","InkPad 版本不兼容。");
        await this.context(context);
        if(this.socket!==socket||this.closed)return;
        this.ready=true;this.attempts=0;this.status("ready","电脑已连接 · 自动同步");this.onReady();
        this.heartbeat=setInterval(()=>this.raw({type:"ping"}),20000);
      }catch(error){this.fence(error instanceof Error?error.message:String(error),error instanceof PadTransportError?error.code:"local_storage_failed");}
      return;
    }
    if(message.type==="operation_ack") {
      const receipt=message.receipt as InkOperationAck;
      const task=this.pending.get(receipt?.request_id);
      if(task){clearTimeout(task.timer);this.pending.delete(receipt.request_id);task.resolve(message);}return;
    }
    if(message.type==="snapshot_receipt") {
      const id=message.request_id as string,task=this.pending.get(id);
      if(task){clearTimeout(task.timer);this.pending.delete(id);task.resolve(message);}return;
    }
    if(message.type==="operation_conflict") {
      const code=String(message.code),error=new PadTransportError(code,String(message.message??"请求未完成。"));
      const id=message.request_id as string,task=this.pending.get(id);
      if(task){clearTimeout(task.timer);this.pending.delete(id);task.reject(error);}
      if(["writer_revoked","credential_expired","authentication_failed","device_busy","protocol_mismatch","idempotency_conflict","sequence_conflict","operation_limit"].includes(code))this.fence(error.message,code);
      return;
    }
    if(message.type==="session_closed") {
      if(message.code==="server_restart"||message.code==="reconnect_required")socket.close();
      else this.fence("书写会话已关闭或写入权已转移；待同步内容保留在本机，可导出恢复草稿。","writer_revoked");
    }
  }
  private fence(message:string,code:string):void {this.closed=true;this.ready=false;this.status("conflict",message,code);this.socket?.close();if(this.retry)clearTimeout(this.retry);}
  private raw(message:unknown):void {if(this.ready&&this.socket?.readyState===WebSocket.OPEN&&this.socket.bufferedAmount<256*1024)this.socket.send(JSON.stringify(message));}
  preview(stroke:InkStroke|undefined):void {
    if(!stroke||performance.now()-this.lastPreview<33||!this.ready)return;
    this.lastPreview=performance.now();
    const step=Math.max(1,Math.ceil(stroke.points.length/2047));
    const points=stroke.points.filter((_,index)=>index%step===0);
    const last=stroke.points.at(-1);if(last&&points.at(-1)!==last)points.push(last);
    this.raw({type:"preview_stroke",stroke:{...stroke,points}});
  }
  report(pending_count:number,locally_durable:boolean,page_id:string):void {this.raw({type:"client_status",pending_count,locally_durable,secure_context:window.isSecureContext,page_id});}
  private request(id:string,message:unknown):Promise<Record<string,unknown>> {
    if(!this.ready||this.socket?.readyState!==WebSocket.OPEN)return Promise.reject(new PadTransportError("offline","电脑尚未连接，待同步内容已保留。"));
    const json=JSON.stringify(message);
    if(new TextEncoder().encode(json).byteLength>16*1024*1024)return Promise.reject(new PadTransportError("operation_limit","单次操作超过传输上限，请导出恢复草稿。"));
    return new Promise((resolve,reject)=>{
      const timer=setTimeout(()=>{this.pending.delete(id);reject(new PadTransportError("ack_timeout","尚未收到电脑的保存确认，将保留原请求后重试。"));this.socket?.close();},30000);
      this.pending.set(id,{resolve,reject,timer});this.socket!.send(json);
    });
  }
  async submit(item:InkOutboxItem,previous:string|null):Promise<InkOperationAck> {
    const message=await this.request(item.request_id,{type:"operation_submit",request:{protocol_version:INK_LAN_PROTOCOL,request_id:item.request_id,document_id:item.document_id,session_id:item.session_id,writer_epoch:item.writer_epoch,operation_id:item.operation.command_id,client_seq:item.operation.seq,previous_operation_id:previous,metadata:item.metadata,operation:item.operation}});
    const receipt=message.receipt as InkOperationAck;
    if(!receipt?.durable||receipt.request_id!==item.request_id||receipt.document_id!==item.document_id||receipt.command_id!==item.operation.command_id||receipt.seq!==item.operation.seq)throw new PadTransportError("invalid_ack","电脑确认与本次操作不匹配，恢复队列保持不变。");
    return receipt;
  }
  async snapshot(id:string,head:number):Promise<Record<string,unknown>> {return this.request(id,{type:"snapshot_request",request_id:id,expected_head:head});}
  close():void {this.closed=true;this.ready=false;if(this.retry)clearTimeout(this.retry);if(this.heartbeat)clearInterval(this.heartbeat);for(const task of this.pending.values()){clearTimeout(task.timer);task.reject(new PadTransportError("offline","连接已关闭，原操作保留在本机。"));}this.pending.clear();this.socket?.close();}
}
