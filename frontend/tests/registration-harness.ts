import { mockIPC } from "@tauri-apps/api/mocks";
import { openRegistrationReversal,openRegistrationHistory,openSourceRegistrationReview } from "../src/discovery/registration-workspace";
import type { RegistrationSummary,SourceRegistrationCommitPlan,DiscoveryReversalReceipt } from "../src/discovery/registration-types";
import type { ImpactPlan } from "../src/map-edit/types";
const text=String.raw`# 定理

定理：$a_n\to a$。

证明：$|a_n-a|<\epsilon$。
`;
const bytes=new TextEncoder().encode(text),start=new TextEncoder().encode(text.slice(0,text.indexOf("定理："))).length;
const range={start,end:bytes.length};
const span={payload_kind:"span",payload:{markdown:{artifact:"md",byte_range:range}}};
const unit={payload_kind:"semantic_unit",payload:{display_title:"定理 $a_n\\to a$",source_spans:["span"],surface_content_artifact:"md"}};
const objects=[{object_id:"span",revision:"r1",kind:"SourceSpan",lifecycle:"active",payload:span,endpoints:[]},{object_id:"unit",revision:"r1",kind:"SourceSemanticUnit",lifecycle:"active",payload:unit,endpoints:[]},{object_id:"shared",revision:"r1",kind:"SourceSemanticUnit",lifecycle:"active",payload:unit,endpoints:[]}];
const summary:RegistrationSummary={decision_id:"decision",registration_commit_id:"original-commit",run_id:"run",state:"registered",registered_at:"2026-10-07",selected_unit_refs:["u1"],object_count:3,reversal_receipt:null};
const behavior={conflict:false,lost:false,missingPreview:false,relationDifference:false,patched:false,noHint:false,otherArtifactHint:false,invalidHint:false,multipleContexts:false,standalone:false};
const markerConversion="mathscape:conversion/marker/preview-fixture";
const patchConversion="mathscape:conversion/markdown-patch/preview-fixture";
const otherConversion="mathscape:conversion/marker/other-fixture";
const conversion=()=>behavior.patched?patchConversion:markerConversion;
const contexts=()=>behavior.standalone?[]:behavior.multipleContexts?[otherConversion,conversion()]:[conversion()];
const markdownRequests:any[]=[];
const backlinkRequests:any[]=[];
let head="head1",receipt:DiscoveryReversalReceipt|null=null;
const commits:unknown[]=[];
let registered=0;
const impact=(request:any,items:any[],ready=true):ImpactPlan=>({version:"mathscape-impact-plan-1",request_id:request.request_id,base_commit:request.base_commit,plan_fingerprint:request.request_id+"/fp",status:ready?"ready":"blocked",direct_changes:items,affected_objects:[],diagnostics:[]});
const item=(id:string,action:string,reason_codes:string[]=[])=>({object_id:id,kind:id==="span"?"SourceSpan":"SourceSemanticUnit",display_title:id==="span"?"原文证据":id==="shared"?"共享定义":"定理",action,reason:"确定性后端规划",current_revision:"r1",proposed_revision:action==="retire"?"r2":null,reason_codes,related_objects:[],protected_by_decisions:id==="shared"?["other-decision"]:[]});
function registration(request:any):SourceRegistrationCommitPlan {
  const approved=request.replacement_approvals.length>0;
  const old={payload:{payload_kind:"semantic_unit",payload:{display_title:"$x^2$",source_spans:["span"]}},endpoints:[],lifecycle:"deprecated"};
  const proposed={payload:{payload_kind:"semantic_unit",payload:{display_title:"$x^3$",source_spans:["span"]}},endpoints:[],lifecycle:"active"};
  const i=impact(request,[item("unit",approved?"reactivate":"block")],approved);
  if(!approved)i.diagnostics=[{code:"replacement_approval_required",object_id:"unit",message:"需要明确批准"}];
  if(behavior.relationDifference) {
    i.direct_changes[0].kind="SourceUnitRelation";i.direct_changes[0].display_title="引用关系";
    return {version:"mathscape-source-registration-commit-plan-1",request,impact:i,requires_confirmation:true,objects:[{object_id:"unit",action:"block",display_title:"引用关系",current_revision:"r1",proposed_revision:null,changed_fields:["endpoints"],before:{payload:{payload_kind:"unit_relation",payload:{relation:"cites"}},endpoints:[{role:"target",entity:"mathscape:test/old-theorem",ordinal:null}]},proposed:{payload:{payload_kind:"unit_relation",payload:{relation:"cites"}},endpoints:[{role:"target",entity:"mathscape:test/new-theorem",ordinal:null}]}}]};
  }
  return {version:"mathscape-source-registration-commit-plan-1",request,impact:i,requires_confirmation:true,objects:[{object_id:"unit",action:approved?"reactivate":"block",display_title:"修订的定理",current_revision:"r1",proposed_revision:approved?"r3":null,changed_fields:["display_title"],before:old,proposed}]};
}
mockIPC((command,payload:any)=>{
  if(command==="get_map_head")return head;
  if(command==="get_registration_history")return {base_commit:"original-commit",objects,adopted_by:{}};
  if(command==="plan_discovery_registration_reversal")return {version:"mathscape-discovery-reversal-plan-1",request:payload.request,impact:impact(payload.request,[item("unit","retire",["created_by_registration"]),item("span","retire",["created_by_registration"]),item("shared","preserve",["adopted_by_other_registration"])]),summary:{total:3,retire:2,preserve:1,already_inactive:0}};
  if(command==="commit_discovery_registration_reversal") {
    commits.push(payload.request);
    if(behavior.conflict){behavior.conflict=false;head="head2";throw new Error("base_commit_conflict: 地图已变化");}
    if(receipt)return receipt;
    receipt={version:"mathscape-discovery-reversal-receipt-1",request_id:payload.request.request.request_id,decision_id:"decision",registration_commit_id:"original-commit",plan_fingerprint:payload.request.expected_plan_fingerprint,state:"withdrawn",decided_at:"2026-10-07",actor:"test",reason:payload.request.request.reason,map_commit:null,retired_objects:["unit","span"],preserved_objects:["shared"],already_inactive_objects:[]};
    summary.state="withdrawn";summary.reversal_receipt=receipt;
    if(behavior.lost){behavior.lost=false;throw new Error("connection closed after commit");}
    return receipt;
  }
  if(command==="list_discovery_registrations")return [summary];
  if(command==="get_source_registration_context")return {artifact_id:behavior.otherArtifactHint?"another-md":"md",conversion_id:behavior.noHint||behavior.standalone?null:behavior.invalidHint?"mathscape:conversion/not-this-markdown":conversion()};
  if(command==="get_artifact_content_info")return {artifact_id:"md",media_type:"text/markdown",byte_size:bytes.length,contexts:contexts().map(id=>({conversion_id:id,paths:[{path:"book/book.md",artifact_id:"md"}]}))};
  if(command==="get_source_node_detail"||command==="get_registration_object_detail") {
    if(behavior.missingPreview)throw new Error("原文材料不可用");
    return {artifact_id:"md",byte_range:range,excerpt:text,markdown_block_ids:["b2"],display_title:"定理"};
  }
  if(command==="get_markdown_document") {
    markdownRequests.push(payload);
    if(payload.conversionId===null&&contexts().length>0)throw new Error("请选择 Markdown 的转换来源");
    if(payload.conversionId!==null&&!contexts().includes(payload.conversionId))throw new Error("转换上下文不属于此 Markdown");
    return {snapshot:{artifact_id:"md",conversion_id:payload.conversionId,content_hash:"h",anchor_scheme:"scheme",fingerprint:"f"},source:text,blocks:[{id:"b1",kind:"heading",range:{start:0,end:start}},{id:"b2",kind:"text",range}]};
  }
  if(command==="get_source_backlink") {
    backlinkRequests.push(payload.request);
    return {kind:"markdown_only",reason:"fixture has no PDF"};
  }
  if(command==="plan_source_registration_commit")return registration(payload.request);
  if(command==="commit_source_registration_commit")return {version:"v2",decision_id:"new-decision",commit_id:"new-commit",committed_at:"now",created_objects:[],reused_objects:[],reactivated_objects:["unit"],revised_objects:[]};
  throw new Error("Unexpected command: "+command);
});
Object.assign(window,{registrationHarness:{behavior,commits,markdownRequests,backlinkRequests,markerConversion,patchConversion,otherConversion,open:()=>openRegistrationReversal(summary),history:()=>openRegistrationHistory(summary),register:()=>openSourceRegistrationReview(registration({request_id:"request",decision_id:"new-decision",base_commit:head,acceptance_plan_fingerprint:"acceptance",replacement_approvals:[]}),()=>registered++),registered:()=>registered}});
