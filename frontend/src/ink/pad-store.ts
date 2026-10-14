import type { InkDocument } from "./model";
import type { InkOutboxItem } from "./outbox";

export interface PadLease { document_id:string;session_id:string;writer_id:string;writer_epoch:number;expires_at:number;device:unknown }
export interface PadBinding { key:string;credential:string;lease:PadLease }
export interface PadDraft { key:string;binding:PadBinding;document:InkDocument;confirmed_head:number;page_id:string;updated_at:string }
export interface PadPairing { token:string;credential:string }
const QUEUE_LIMIT=64*1024*1024;

/** One transaction stores the draft and its unconfirmed operation. Reading a
 * draft never removes queue entries; only validated durable ACKs do that. */
export class InkPadStore {
  private constructor(private readonly database:IDBDatabase) {}
  static async open():Promise<InkPadStore> {
    if (!globalThis.isSecureContext || !globalThis.indexedDB) throw new Error("此画板需要受信任的 HTTPS 和可用的本机存储，请先安装电脑导出的设备信任证书。");
    const database=await new Promise<IDBDatabase>((resolve,reject)=>{
      const request=indexedDB.open("mathscape-inkpad-p3",1);
      request.onupgradeneeded=()=>{
        request.result.createObjectStore("drafts",{keyPath:"key"});
        request.result.createObjectStore("operations",{keyPath:"request_id"});
        request.result.createObjectStore("history",{keyPath:["key","seq"]});
        request.result.createObjectStore("meta");
      };
      request.onsuccess=()=>resolve(request.result);request.onerror=()=>reject(request.error);request.onblocked=()=>reject(new Error("请关闭其他 InkPad 页面后重试。"));
    });
    const store=new InkPadStore(database);
    await store.write("meta",()=>["probe",true]);
    return store;
  }
  private async read<T>(name:string,key:IDBValidKey):Promise<T|undefined> {
    return new Promise((resolve,reject)=>{const transaction=this.database.transaction(name,"readonly");const request=transaction.objectStore(name).get(key);request.onsuccess=()=>resolve(request.result as T);request.onerror=()=>reject(request.error);});
  }
  private async all<T>(name:string):Promise<T[]> {
    return new Promise((resolve,reject)=>{const transaction=this.database.transaction(name,"readonly");const request=transaction.objectStore(name).getAll();request.onsuccess=()=>resolve(request.result as T[]);request.onerror=()=>reject(request.error);});
  }
  private async write(name:string,value:()=>[IDBValidKey,unknown]):Promise<void> {
    return new Promise((resolve,reject)=>{const transaction=this.database.transaction(name,"readwrite");const [key,data]=value();transaction.objectStore(name).put(data,key);transaction.oncomplete=()=>resolve();transaction.onerror=()=>reject(transaction.error);transaction.onabort=()=>reject(transaction.error);});
  }
  async pairing(token:string,credential:string):Promise<PadPairing> {
    const key="pair:"+token;
    const existing=await this.read<PadPairing>("meta",key);if(existing)return existing;
    const value={token,credential};await this.write("meta",()=>[key,value]);return value;
  }
  async activate(binding:PadBinding):Promise<void> {await this.write("meta",()=>["active",binding]);}
  async active():Promise<PadBinding|undefined> {return this.read("meta","active");}
  async draft(key:string):Promise<PadDraft|undefined> {
    const draft=await this.read<PadDraft>("drafts",key);if(!draft)return undefined;
    const rows=await new Promise<Array<{operation:InkDocument["operations"][number]}>>((resolve,reject)=>{
      const request=this.database.transaction("history","readonly").objectStore("history").getAll(IDBKeyRange.bound([key,0],[key,Number.MAX_SAFE_INTEGER]));
      request.onsuccess=()=>resolve(request.result);request.onerror=()=>reject(request.error);
    });
    draft.document.operations=rows.map(row=>row.operation);return draft;
  }
  async drafts():Promise<PadDraft[]> {return (await this.all<PadDraft>("drafts")).sort((a,b)=>b.updated_at.localeCompare(a.updated_at));}
  async pending(binding:PadBinding):Promise<InkOutboxItem[]> {
    return (await this.all<InkOutboxItem>("operations")).filter(item=>item.session_id===binding.lease.session_id && item.writer_epoch===binding.lease.writer_epoch && item.document_id===binding.lease.document_id).sort((a,b)=>a.operation.seq-b.operation.seq);
  }
  async cache(draft:PadDraft):Promise<void> {
    return new Promise((resolve,reject)=>{const tx=this.database.transaction(["drafts","history"],"readwrite");tx.objectStore("drafts").put({...draft,document:{...draft.document,operations:[]}});for(const operation of draft.document.operations)tx.objectStore("history").put({key:draft.key,seq:operation.seq,operation});tx.oncomplete=()=>resolve();tx.onerror=()=>reject(tx.error);tx.onabort=()=>reject(tx.error);});
  }
  async enqueue(draft:PadDraft,item:InkOutboxItem):Promise<void> {
    const encoded=JSON.stringify(item),bytes=new TextEncoder().encode(encoded).byteLength;
    return new Promise((resolve,reject)=>{
      const tx=this.database.transaction(["drafts","operations","history","meta"],"readwrite");
      let failure:Error|undefined;
      const existing=tx.objectStore("operations").get(item.request_id);
      existing.onsuccess=()=>{
        if(existing.result){const {byte_size:_,...previous}=existing.result;if(JSON.stringify(previous)!==encoded){failure=new Error("恢复队列中的同一请求包含不同内容。");tx.abort();return;}}
        const size=tx.objectStore("meta").get("queue-bytes");
        size.onsuccess=()=>{
          const next=(Number(size.result)||0)-(Number(existing.result?.byte_size)||0)+bytes;
          if(next>QUEUE_LIMIT){failure=new Error("待同步队列达到上限，请恢复电脑连接或导出恢复草稿。");tx.abort();return;}
          tx.objectStore("meta").put(next,"queue-bytes");
          tx.objectStore("drafts").put({...draft,document:{...draft.document,operations:[]}});
          tx.objectStore("operations").put({...item,byte_size:bytes});
          tx.objectStore("history").put({key:draft.key,seq:item.operation.seq,operation:item.operation});
        };
      };
      tx.oncomplete=()=>resolve();tx.onerror=()=>reject(failure??tx.error);tx.onabort=()=>reject(failure??tx.error);
    });
  }
  async ack(requestId:string,binding:PadBinding,seq:number):Promise<void> {
    return new Promise((resolve,reject)=>{
      const tx=this.database.transaction(["operations","meta","drafts"],"readwrite");
      const operation=tx.objectStore("operations").get(requestId);
      operation.onsuccess=()=>{
        const count=tx.objectStore("meta").get("queue-bytes");count.onsuccess=()=>tx.objectStore("meta").put(Math.max(0,(Number(count.result)||0)-(Number(operation.result?.byte_size)||0)),"queue-bytes");
        tx.objectStore("operations").delete(requestId);
        const draft=tx.objectStore("drafts").get(binding.key);draft.onsuccess=()=>{if(draft.result)tx.objectStore("drafts").put({...draft.result,confirmed_head:Math.max(draft.result.confirmed_head,seq)});};
      };
      tx.oncomplete=()=>resolve();tx.onerror=()=>reject(tx.error);tx.onabort=()=>reject(tx.error);
    });
  }
}
