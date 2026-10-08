/** Registration-level review. Eligibility belongs to Rust; the UI never traverses for retirement. */
import { invoke } from "@tauri-apps/api/core";
import cytoscape, { type Core, type ElementDefinition } from "cytoscape";
import { textElement as el } from "../core/dom";
import { eventBus } from "../core/events";
import { errorMessage } from "../core/ipc";
import { markdownDocument } from "../reader/selection";
import { markdownRenderer } from "../reader/markdown";
import { ArtifactReader } from "../reader/reader";
import { contentInfo } from "../reader/content";
import { getSourceBacklink } from "../reader/backlink";
import { renderAnchorFragment } from "../review/anchor-fragment";
import type { ImpactItem } from "../map-edit/types";
import type { DiscoveryReversalPlan, DiscoveryReversalReceipt, RegistrationSummary, SourceRegistrationCommitPlan, RegistrationReceipt, HistoricalMapObject, RegistrationSourceContext } from "./registration-types";
import "./registration-workspace.css";

const labels: Record<string,string> = {
  create:"新建", reuse:"复用", reactivate:"重新激活", revise:"修订", block:"阻断", retire:"将退役", preserve:"保留",
  SourceSpan:"原文证据", SourceSemanticUnit:"内容单元", SourceUnitRelation:"来源关系",
  already_inactive:"已经退役", reused_object:"原登记仅复用", preexisting_revised_object:"既有对象的内容修订不回滚",
  subsequently_revised:"登记后已修改", adopted_by_other_registration:"其他有效登记仍采纳",
  referenced_by_live_object:"范围外活动对象依赖", required_by_preserved_object:"保留对象的必要依赖",
  created_by_registration:"本次登记创建", reactivated_by_registration:"本次登记重新激活",
  "endpoint:source":"关系来源", "endpoint:target":"关系目标", "endpoint:evidence":"关系证据",
  parent:"父单元",source_span:"原文证据",title_span:"标题原文",label_span:"编号原文",reference_span:"引用原文",reference_target:"引用目标",trust_evidence:"信任证据",
};
const newId = () => `mathscape:registration-request/${crypto.randomUUID()}`;
function action(text:string,run:()=>void):HTMLButtonElement { const b=el("button","",text);b.type="button";b.onclick=run;return b; }
function notifyChange():void { eventBus.emit("source-map-changed");eventBus.emit("discovery-runs-changed"); }
interface SourceDetail {
  artifact_id:string|null;byte_range:{start:number;end:number}|null;excerpt:string|null;
  markdown_block_ids:string[];display_title:string|null;
}

export async function openRegistrationReversal(registration:RegistrationSummary):Promise<void> {
  const w=new RegistrationWorkspace("撤回本次登记",registration.decision_id);
  w.reversal=registration;
  await w.replan();
}
export async function openRegistrationHistory(registration:RegistrationSummary):Promise<void> {
  const w=new RegistrationWorkspace(registration.state==="withdrawn"?"已撤回的登记与原始对象":"登记时的对象快照",registration.decision_id);
  try {
    const snapshot=await invoke<{objects:HistoricalMapObject[]}>("get_registration_history",{decisionId:registration.decision_id});
    w.history=snapshot.objects;
    w.showHistory(registration);
  } catch(e) {w.feedback.textContent=errorMessage(e);}
}
export function openSourceRegistrationReview(plan:SourceRegistrationCommitPlan,onRegistered?:()=>void):void {
  const w=new RegistrationWorkspace("来源登记变更审核",plan.request.decision_id);
  w.registrationPlan=plan;w.onRegistered=onRegistered;w.render();
}

class RegistrationWorkspace {
  readonly dialog=el("dialog","registration-workspace","");
  readonly list=el("section","registration-list","");
  readonly preview=el("section","registration-preview markdown-body","");
  readonly footer=el("footer","registration-footer","");
  readonly feedback=el("p","registration-feedback","");
  readonly reason=el("input","registration-reason","");
  private readonly controls=el("div","registration-actions","");
  private readonly counts=el("p","registration-counts","");
  private readonly graphHost=el("div","registration-graph","");
  private core:Core|null=null;
  private reader:ArtifactReader|null=null;
  private previewGeneration=0;
  private renderGeneration=0;
  private readonly sourceContexts=new Map<string,Promise<RegistrationSourceContext>>();
  private readonly selectedConversions=new Map<string,string>();
  private planning=false;
  private submitting=false;
  private dirty=false;
  private locked:string|null=null;
  private originalLabel:string;
  private reversalPlan:DiscoveryReversalPlan|null=null;
  reversal:RegistrationSummary|null=null;
  registrationPlan:SourceRegistrationCommitPlan|null=null;
  history:HistoricalMapObject[]=[];
  onRegistered?:()=>void;
  constructor(title:string,readonly decisionId:string) {
    this.originalLabel=title;
    const header=el("header","registration-header","");
    const heading=el("h2","",title);heading.id=`registration-title-${crypto.randomUUID()}`;this.dialog.setAttribute("aria-labelledby",heading.id);
    const close=action("关闭",()=>{if(!this.submitting)this.dialog.close();});
    header.append(heading,close);
    const split=el("div","registration-split","");split.append(this.list,this.preview);
    this.reason.type="text";this.reason.value="重新整理这次发现结果";this.reason.setAttribute("aria-label","撤回原因");
    this.reason.oninput=()=>{this.dirty=true;this.syncControls();};
    this.feedback.setAttribute("role","status");
    this.footer.append(this.counts,this.controls,this.feedback);
    this.dialog.append(header,split,this.footer);
    this.dialog.addEventListener("cancel",e=>{if(this.submitting)e.preventDefault();});
    this.dialog.addEventListener("close",()=>{this.previewGeneration++;this.core?.destroy();this.reader?.close();this.dialog.remove();},{once:true});
    document.body.append(this.dialog);this.dialog.showModal();
    this.preview.append(el("p","","悬停预览原文；点击锁定。撤回只改变采纳和生命周期，材料与历史保留。"));
  }
  async replan():Promise<void> {
    if(this.planning||this.submitting)return;
    this.planning=true;this.syncControls();this.feedback.textContent="正在读取当前地图并重新计算影响…";
    try {
      const head=await invoke<string|null>("get_map_head");
      if(this.reversal) {
        this.reversalPlan=await invoke<DiscoveryReversalPlan>("plan_discovery_registration_reversal",{request:{request_id:newId(),decision_id:this.decisionId,registration_commit_id:this.reversal.registration_commit_id,base_commit:head,reason:this.reason.value}});
      } else if(this.registrationPlan) {
        this.registrationPlan=await invoke<SourceRegistrationCommitPlan>("plan_source_registration_commit",{request:{...this.registrationPlan.request,request_id:newId(),base_commit:head}});
      }
      this.dirty=false;this.feedback.textContent="已计算当前影响，请确认后提交。";this.render();
    } catch(e) {this.feedback.textContent=errorMessage(e);}
    finally{this.planning=false;this.syncControls();}
  }
  render():void {
    const renderGeneration=++this.renderGeneration;
    const impact=this.reversalPlan?.impact??this.registrationPlan?.impact;
    if(!impact)return;
    this.list.replaceChildren();
    if(this.reversalPlan) {
      const label=el("label","registration-reason-label","撤回原因（必填）");label.append(this.reason);this.list.append(label);
      const c=this.reversalPlan.summary;
      this.counts.textContent=c.retire===0?`撤回登记，活动地图不变；保留 ${c.preserve} 个，已经退役 ${c.already_inactive} 个。材料与历史保留。`:`将退役 ${c.retire} 个对象，保留 ${c.preserve} 个，已经退役 ${c.already_inactive} 个。材料文件与历史记录保留。`;
      const groups=new Map<string,ImpactItem[]>();
      for(const i of impact.direct_changes) {
        const group=i.action==="retire"?"将退役":i.reason_codes?.includes("already_inactive")?"已经退役":i.reason_codes?.includes("subsequently_revised")?"后续修改，保留":"共享或必要依赖，保留";
        groups.set(group,[...(groups.get(group)??[]),i]);
      }
      for(const [label,items] of groups) {
        this.list.append(el("h3","",`${label} · ${items.length}`));
        for(const item of items)this.list.append(this.itemRow(item));
      }
    } else if(this.registrationPlan) {
      this.counts.textContent="按已接受提案进行登记。重新激活将生成新 revision；内容替换需逐对象明确批准。";
      for(const item of impact.direct_changes) {
        const row=this.itemRow(item);
        const o=this.registrationPlan.objects.find(o=>o.object_id===item.object_id);
        if(o?.changed_fields.length&&o.current_revision) {
          const permitted=impact.diagnostics.some(d=>d.object_id===o.object_id&&d.code==="replacement_approval_required")||o.action==="revise"||o.action==="reactivate";
          const label=el("label","registration-approval",permitted?"我已阅读差异，批准采用此提案内容":"身份冲突或共享内容：不能在本次登记中替换");
          if(permitted) {
            const checkbox=el("input","","");checkbox.type="checkbox";checkbox.checked=this.registrationPlan.request.replacement_approvals.some(a=>a.object_id===o.object_id);
            checkbox.onchange=()=>{
              if(!this.registrationPlan)return;
              const approvals=this.registrationPlan.request.replacement_approvals.filter(a=>a.object_id!==o.object_id);
              if(checkbox.checked)approvals.push({object_id:o.object_id,expected_revision:o.current_revision!});
              this.registrationPlan.request.replacement_approvals=approvals;this.dirty=true;this.syncControls();void this.replan();
            };
            label.prepend(checkbox);
          }
          row.append(label);
        }
        this.list.append(row);
      }
    }
    for(const d of impact.diagnostics)this.list.append(el("p","registration-error",`${d.code}：${d.message}`));
    this.syncControls();
    if(impact.direct_changes.length<40)void this.renderGraph(renderGeneration);
  }
  private itemRow(i:ImpactItem):HTMLElement {
    const row=el("article",`registration-item state-${i.action}`,"");
    const title=action(`${labels[i.action]??i.action} · ${labels[i.kind]??i.kind} · ${i.display_title??i.object_id}`,()=>{this.locked=i.object_id;void this.showObject(i.object_id,false);});
    title.onmouseenter=()=>{if(!this.submitting)void this.showObject(i.object_id,false);};
    title.onmouseleave=()=>{if(this.locked&&this.locked!==i.object_id)void this.showObject(this.locked,false);};
    row.append(title,el("p","",i.reason));
    if(i.reason_codes?.length||i.related_objects?.length||i.protected_by_decisions?.length) {
      const details=el("details","","");details.append(el("summary","","查看原因与依赖"));
      for(const code of i.reason_codes??[])details.append(el("p","",labels[code]??code));
      for(const ref of i.related_objects??[])details.append(action(`${ref.display_title??ref.object_id} · ${labels[ref.reference_kind]??ref.reference_kind}`,()=>{this.locked=ref.object_id;void this.showObject(ref.object_id,false);}));
      for(const decision of i.protected_by_decisions??[])details.append(action(`查看保护登记 ${decision.slice(-12)}`,()=>void this.openProtectingRegistration(decision)));
      row.append(details);
    }
    return row;
  }
  private async openProtectingRegistration(decision:string):Promise<void> {
    try {const data=await invoke<{objects:HistoricalMapObject[]}>("get_registration_history",{decisionId:decision});const first=data.objects.find(o=>o.kind==="SourceSemanticUnit")??data.objects[0];if(first)await this.showObject(first.object_id,true,decision);}
    catch(e){this.feedback.textContent=errorMessage(e);}
  }
  private syncControls():void {
    const busy=this.planning||this.submitting;
    this.controls.replaceChildren();
    if(this.reversal||this.registrationPlan) {
      const replan=action("重新计算影响",()=>void this.replan());replan.disabled=busy;
      const confirm=action(this.reversal?"确认撤回":"确认登记",()=>void this.commit());
      confirm.disabled=busy||this.dirty||(this.reversalPlan?.impact??this.registrationPlan?.impact)?.status!=="ready"||(this.reversal!==null&&!this.reason.value.trim());
      this.controls.append(replan,confirm);
    }
    const close=action("取消 / 关闭",()=>this.dialog.close());close.disabled=this.submitting;this.controls.append(close);
    this.reason.disabled=busy;
    this.list.querySelectorAll<HTMLInputElement>(".registration-approval input").forEach(c=>c.disabled=busy);
  }
  private async commit():Promise<void> {
    if(this.submitting||this.dirty)return;
    this.submitting=true;this.syncControls();this.feedback.textContent="正在原子提交…";
    try {
      if(this.reversalPlan) {
        const receipt=await invoke<DiscoveryReversalReceipt>("commit_discovery_registration_reversal",{request:{request:this.reversalPlan.request,expected_plan_fingerprint:this.reversalPlan.impact.plan_fingerprint}});
        notifyChange();this.reversal=null;this.reversalPlan=null;
        this.counts.textContent=`已撤回登记：退役 ${receipt.retired_objects.length}，保留 ${receipt.preserved_objects.length}，已退役 ${receipt.already_inactive_objects.length}。历史与文件保留。`;
        this.feedback.textContent="提交成功。原始对象快照仍可从发现记录打开。";
      } else if(this.registrationPlan) {
        const receipt=await invoke<RegistrationReceipt>("commit_source_registration_commit",{request:this.registrationPlan.request,expectedPlanFingerprint:this.registrationPlan.impact.plan_fingerprint});
        notifyChange();this.registrationPlan=null;
        this.counts.textContent=`已登记：新建 ${receipt.created_objects.length}，复用 ${receipt.reused_objects.length}，重新激活 ${receipt.reactivated_objects?.length??0}，修订 ${receipt.revised_objects?.length??0}。`;
        this.feedback.textContent="提交成功。";this.onRegistered?.();
      }
    } catch(e) {
      if(errorMessage(e).includes("already_withdrawn")&&this.reversal) {
        try {
          const receipt=await invoke<DiscoveryReversalReceipt|null>("get_discovery_registration_reversal",{decisionId:this.decisionId});
          if(receipt) {
            notifyChange();this.reversal=null;this.reversalPlan=null;
            this.counts.textContent=`该登记已由另一请求撤回：退役 ${receipt.retired_objects.length}，保留 ${receipt.preserved_objects.length}。${receipt.reason}`;
          }
        } catch { /* Keep the failed request so a subsequent read/retry remains explicit. */ }
      }
      if(/base_commit_conflict|plan_conflict|revision_conflict/.test(errorMessage(e)))this.dirty=true;
      this.feedback.textContent=`提交失败：${errorMessage(e)}。失败保留当前原因与计划；地图变化时请重新计算。响应丢失时可以直接重试同一请求。`;
    } finally{this.submitting=false;this.syncControls();}
  }
  showHistory(r:RegistrationSummary):void {
    this.list.replaceChildren(el("p","",`登记于 ${r.registered_at} · ${r.object_count} 个对象。以下是登记当时的 revision，不代表当前状态。`));
    for(const o of this.history) {
      const row=action(`${labels[o.kind]??o.kind} · ${String(o.payload.payload.display_title??o.object_id)}`,()=>{this.locked=o.object_id;void this.showObject(o.object_id,true);});
      row.onmouseenter=()=>void this.showObject(o.object_id,true);this.list.append(row);
    }
    const receipt=r.reversal_receipt;
    this.counts.textContent=receipt?`已撤回 · ${receipt.reason} · 退役 ${receipt.retired_objects.length}，保留 ${receipt.preserved_objects.length}，已经退役 ${receipt.already_inactive_objects.length}。`:`登记时快照：${r.object_count} 个对象。`;
    this.syncControls();
  }
  private async showObject(id:string,historical:boolean,decision=this.decisionId):Promise<void> {
    const generation=++this.previewGeneration;this.reader?.close();this.reader=null;
    this.preview.replaceChildren(el("h3","",this.originalLabel),el("p","","正在读取原文…"));
    const change=this.registrationPlan?.objects.find(o=>o.object_id===id);
    if(change?.changed_fields.length)this.renderDifference(change.before,change.proposed,change.changed_fields);
    try {
      let detail:SourceDetail;
      try {detail=await invoke<SourceDetail>(historical?"get_registration_object_detail":"get_source_node_detail",historical?{decisionId:decision,objectId:id}:{nodeId:id});}
      catch(e) {const proposed=this.proposedDetail(id);if(historical||!proposed)throw e;detail=proposed;}
      if(generation!==this.previewGeneration||!this.dialog.isConnected)return;
      this.preview.querySelector("p")?.remove();
      if(!detail.artifact_id||!detail.byte_range){this.preview.append(el("p","","此对象没有可预览的 Markdown 定位。"));return;}
      let pendingContext=this.sourceContexts.get(decision);
      if(!pendingContext) {
        pendingContext=invoke<RegistrationSourceContext>("get_source_registration_context",{decisionId:decision});
        this.sourceContexts.set(decision,pendingContext);
        void pendingContext.catch(()=>this.sourceContexts.delete(decision));
      }
      const source=await pendingContext;
      if(generation!==this.previewGeneration||!this.dialog.isConnected)return;
      // An outside dependency can belong to another Markdown. Never apply
      // this registration's conversion hint to a different artifact.
      const selectionKey=`${decision}\0${detail.artifact_id}`;
      let conversion=source.artifact_id===detail.artifact_id?source.conversion_id:null;
      if(conversion===null) {
        const info=await contentInfo(detail.artifact_id);
        if(generation!==this.previewGeneration||!this.dialog.isConnected)return;
        const chosen=this.selectedConversions.get(selectionKey);
        if(chosen&&info.contexts.some(c=>c.conversion_id===chosen))conversion=chosen;
        else if(info.contexts.length===1)conversion=info.contexts[0].conversion_id;
        else if(info.contexts.length>1) {
          const label=el("label","registration-conversion-label","该原文有多个转换来源，请明确选择预览上下文：");
          const select=el("select","registration-conversion","");select.setAttribute("aria-label","预览转换来源");
          select.add(new Option("请选择转换来源",""));
          info.contexts.forEach((context,index)=>{
            const path=context.paths.find(p=>p.artifact_id===detail.artifact_id)?.path;
            select.add(new Option(`${path??`转换来源 ${index+1}`} · ${context.conversion_id.slice(-12)}`,context.conversion_id));
          });
          select.onchange=()=>{
            if(select.value){this.selectedConversions.set(selectionKey,select.value);void this.showObject(id,historical,decision);}
          };
          label.append(select);this.preview.append(label);return;
        }
      }
      const doc=await markdownDocument(detail.artifact_id,conversion);
      if(generation!==this.previewGeneration||!this.dialog.isConnected)return;
      const range=detail.byte_range;
      const bytes=new TextEncoder().encode(doc.source);
      if(range.start<0||range.end>bytes.length||range.end<=range.start)throw new Error("登记定位范围已失效，不做近似匹配");
      const blocks=doc.blocks.filter(b=>b.range.start<range.end&&b.range.end>range.start);
      this.preview.append(renderAnchorFragment(doc,bytes,{...range,block_ids:blocks.map(b=>b.id),excerpt:detail.excerpt??"",purpose:"unit"},blocks));
      const link=await getSourceBacklink({artifact_id:detail.artifact_id,conversion_id:conversion,...range,block_ids:blocks.map(b=>b.id)});
      if(generation!==this.previewGeneration||!this.dialog.isConnected)return;
      if(link.pdf_artifact_id&&link.pdf_page_indices?.length) {
        const bar=el("div","registration-pdf-actions","");
        for(const page of link.pdf_page_indices)bar.append(action(`查看 PDF 第 ${page+1} 页${link.kind==="candidate_pdf_pages"?"（候选）":""}`,()=>{
          const host=el("div","registration-pdf-reader","");this.preview.replaceChildren(action("返回原文",()=>void this.showObject(id,historical,decision)),host);
          this.reader=new ArtifactReader(host);void this.reader.open(link.pdf_artifact_id!,"原始 PDF",{pdfPageIndex:page});
        }));
        this.preview.prepend(bar);
      } else this.preview.append(el("p","",link.reason??"没有确定的 PDF 回链。"));
    } catch(e) {
      if(generation!==this.previewGeneration||!this.dialog.isConnected)return;
      this.preview.append(el("p","registration-error",`预览不可用：${errorMessage(e)}。这不影响后端安全规划。`));
    }
  }
  private proposedDetail(id:string):SourceDetail|null {
    const objects=this.registrationPlan?.objects;
    const change=objects?.find(o=>o.object_id===id);
    if(!change)return null;
    type Proposal={payload?:{payload_kind?:string;payload?:{markdown?:{artifact?:string;byte_range?:{start:number;end:number}};source_spans?:string[]}};endpoints?:{role:string;entity:string}[]};
    const candidate=change.proposed as Proposal;
    const span=candidate.payload?.payload_kind==="span"?candidate:(objects?.find(o=>o.object_id===(candidate.payload?.payload?.source_spans?.[0]??candidate.endpoints?.find(e=>e.role==="evidence")?.entity))?.proposed as Proposal|undefined);
    const locator=span?.payload?.payload?.markdown;
    return locator?.artifact&&locator.byte_range?{artifact_id:locator.artifact,byte_range:locator.byte_range,excerpt:null,markdown_block_ids:[],display_title:change.display_title}:null;
  }
  private renderDifference(before:unknown,proposed:unknown,fields:string[]):void {
    const area=el("section","registration-diff","");area.append(el("h3","","内容替换差异"));
    const extract=(v:unknown):Record<string,unknown>=>((v as {payload?:{payload?:Record<string,unknown>}}|null)?.payload?.payload??{});
    const value=(v:unknown,field:string):unknown=>field==="endpoints"?(v as {endpoints?:unknown}|null)?.endpoints:field==="payload"?(v as {payload?:unknown}|null)?.payload:extract(v)[field];
    const render=(host:HTMLElement,v:unknown,depth=0):void=>{
      if(typeof v==="string") {
        const text=el("div","","");text.innerHTML=markdownRenderer().render(v);host.append(text);
      } else if(Array.isArray(v)&&depth<5) {
        const list=el("ol","registration-diff-values","");
        for(const entry of v){const row=el("li","","");render(row,entry,depth+1);list.append(row);}host.append(list);
        if(v.length===0)host.append(el("span","","空列表"));
      } else if(v!==null&&typeof v==="object"&&depth<5) {
        const dl=el("dl","registration-diff-values","");
        for(const [key,entry] of Object.entries(v)){const dd=el("dd","","");dl.append(el("dt","",labels[key]??key),dd);render(dd,entry,depth+1);}host.append(dl);
      } else host.append(el("pre","",JSON.stringify(v??null,null,2)));
    };
    for(const field of fields) {
      const row=el("div","registration-diff-row","");row.append(el("strong","",field));
      for(const [label,current] of [["当前",value(before,field)],["提案",value(proposed,field)]] as const) {
        const cell=el("div","markdown-body","");cell.append(el("h4","",label));
        render(cell,current);
        row.append(cell);
      }
      area.append(row);
    }
    this.preview.append(area);
  }
  private async renderGraph(generation:number):Promise<void> {
    if(!this.reversalPlan)return;
    try {
      if(!this.dialog.isConnected||!this.reversalPlan||generation!==this.renderGeneration)return;
      this.core?.destroy();this.graphHost.replaceChildren();
      const items=this.reversalPlan.impact.direct_changes;
      const ids=new Set(items.map(i=>i.object_id));
      const elements:ElementDefinition[]=items.filter(i=>i.kind!=="SourceUnitRelation").map(i=>({data:{id:i.object_id,label:i.display_title??labels[i.kind]??i.kind},classes:`state-${i.action}${i.reason_codes?.includes("already_inactive")?" inactive":""}`}));
      for(const r of items.filter(o=>o.kind==="SourceUnitRelation")) {
        const a=r.related_objects?.find(e=>e.reference_kind==="endpoint:source")?.object_id,b=r.related_objects?.find(e=>e.reference_kind==="endpoint:target")?.object_id;
        if(a&&b&&ids.has(a)&&ids.has(b))elements.push({data:{id:r.object_id,source:a,target:b},classes:`state-${r.action}`});
      }
      const wrap=el("details","registration-mini-map","");wrap.append(el("summary","","影响小图：红色虚线将退役，绿色实线保留，灰色已退役"),this.graphHost);this.list.append(wrap);
      wrap.ontoggle=()=>{if(wrap.open)this.core?.resize().layout({name:"cose",animate:false,fit:true}).run();};
      this.core=cytoscape({container:this.graphHost,elements,style:[{selector:"node",style:{label:"data(label)","font-size":9,"text-wrap":"wrap","text-max-width":"90px",width:22,height:22,"background-color":"#59bda7","border-width":2,"border-color":"#59bda7",color:"#deeae6"}},{selector:"node.state-retire",style:{"background-color":"#994c4c","border-style":"dashed","border-color":"#ffafa0"}},{selector:"node.inactive",style:{"background-color":"#687475","border-style":"dotted"}},{selector:"edge",style:{"line-color":"#78928c",width:1.5,"target-arrow-shape":"triangle","target-arrow-color":"#78928c","curve-style":"bezier"}}],layout:{name:"grid"}});
      this.core.on("tap","node",event=>{this.locked=event.target.id();void this.showObject(event.target.id(),false);});
      this.core.on("mouseover","node",event=>void this.showObject(event.target.id(),false));
    } catch { /* The list is authoritative; legacy preview failures do not block withdrawal. */ }
  }
}
