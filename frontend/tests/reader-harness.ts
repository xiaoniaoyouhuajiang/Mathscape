import { mockIPC } from "@tauri-apps/api/mocks";
import { ArtifactReader } from "../src/reader/reader";
import { ArtifactPdfTransport } from "../src/reader/pdf-transport";
import { markdownRenderer } from "../src/reader/markdown";
import { resolveProductPath, readRange, type ContentInfo } from "../src/reader/content";
import type { AnchoredMarkdown, SelectionRequest } from "../src/reader/selection";
import { mountReviewWorkspace } from "../src/review/workspace";
import { renderAnchorFragment } from "../src/review/anchor-fragment";
import { DiscoveryLedgerController } from "../src/discovery/discovery-ledger";
import { setInkContextCreator, overlappingNotes, markdownPreviewLabel, type InkLinkedNote } from "../src/ink/links";
import { eventBus } from "../src/core/events";

// Tiny reproducible PDF: two pages with selectable text streams and one outline entry, no copyrighted assets.
function samplePdf(): Uint8Array {
  const s1 = "1 0 0 rg 0 0 100 100 re f\n0 0 0 rg BT /F1 14 Tf 24 340 Td (Theorem 1 Generating Function) Tj ET\n";
  const s2 = "0 0 1 rg 0 0 200 200 re f\n0 0 0 rg BT /F1 14 Tf 24 340 Td (Proof by Induction Step) Tj ET\n";
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R /Outlines 7 0 R >>",
    "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Resources << /Font << /F1 9 0 R >> >> /Contents 5 0 R >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Resources << /Font << /F1 9 0 R >> >> /Contents 6 0 R >>",
    `<< /Length ${s1.length} >>\nstream\n${s1}\nendstream`,
    `<< /Length ${s2.length} >>\nstream\n${s2}\nendstream`,
    "<< /Type /Outlines /First 8 0 R /Last 8 0 R /Count 1 >>",
    "<< /Title (Second page) /Parent 7 0 R /Dest [4 0 R /Fit] >>",
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
  ];
  // Padding ensures PDF.js must request ranges rather than receive the whole file initially.
  let data = "%PDF-1.4\n%" + "padding".repeat(40000) + "\n";
  const offsets = [0];
  objects.forEach((body, i) => { offsets.push(data.length); data += `${i + 1} 0 obj\n${body}\nendobj\n`; });
  const xref = data.length;
  data += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  offsets.slice(1).forEach(offset => { data += `${String(offset).padStart(10, "0")} 00000 n \n`; });
  data += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF`;
  return new TextEncoder().encode(data);
}

const markdown = String.raw`# Math reader 中文😀

Inline $x^2$ and \(a_n\to a\).

$$
\frac{1}{n}\sum_{k=1}^n a_k

\longrightarrow a
$$

\[
\omega(s)=(s_1,\ldots,s_k)
\]

![Diagram](images/plot.png)
![Unsafe](https://example.com/tracker.png)

<script>window.untrusted = true</script>

~~~tex
$code_not_math$
~~~
`;
const png = Uint8Array.from(atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aOZkAAAAASUVORK5CYII="), c => c.charCodeAt(0));
const files: Record<string, { media: string; bytes: Uint8Array }> = {
  pdf: { media: "application/pdf", bytes: samplePdf() },
  md: { media: "text/markdown", bytes: new TextEncoder().encode(markdown) },
  png: { media: "image/png", bytes: png },
  json: { media: "application/json", bytes: new TextEncoder().encode('{"page":1}') },
  bad: { media: "application/pdf", bytes: new TextEncoder().encode("not a PDF") },
};
const reads: { id: string; offset: number; length: number }[] = [];
const selectionRequests: SelectionRequest[] = [];
const discoveryRequests: SelectionRequest[] = [];
const acceptanceRequests: unknown[] = [];
const dispositionRequests:any[]=[];
let persistedOutcome:any=null;
let disposition:any=null;
let restoreFailure=false, restoreLostResponse=false, restoredRequest:string|null=null;
const patchDecisions: any[] = [];
const resumeRequests: unknown[] = [];
// N2: Markdown note links shown by the reader (markers, menu, patch hint).
const ink = { links: [] as InkLinkedNote[], unitLinks: [] as InkLinkedNote[], linksByArtifact: {} as Record<string, InkLinkedNote[]>, unitLinksByArtifact: {} as Record<string, InkLinkedNote[]>, originalNoteCount: 0, pageMarkerBlock: -1, creates: [] as unknown[], linkRequests: [] as unknown[], opened: [] as string[] };
setInkContextCreator(async (target, choice, draft) => { ink.creates.push({ target, choice, requestId: draft.requestId }); });
eventBus.on("open-ink-document", ({ documentId }) => { ink.opened.push(documentId); });
const locatedObjects: string[] = [];
eventBus.on("locate-domain-object", ({ objectId }) => { locatedObjects.push(objectId); });
const errataEvents: unknown[] = [];
eventBus.on("markdown-errata-changed", (detail) => { errataEvents.push(detail); });
const errataStore = {
  items: [] as any[],
  seq: 1,
};
const unitMarkers: any[] = [];
let canonicalProjection: any = null;
// Extra physical Markdown snapshots (e.g. the segments of a canonical view).
const markdownSnapshots: Record<string, AnchoredMarkdown> = {};
function setMarkdownSnapshot(id: string, doc: AnchoredMarkdown): void {
  markdownSnapshots[id] = doc;
  files[id] = { media: "text/markdown", bytes: new TextEncoder().encode(doc.source) };
}
const backlinkRequests: unknown[] = [];
const behavior = { pdfRangeFailure: false, selectionDelay: 0, discoveryDelay: 0, failSelection: false, admission: "review_before_publish", discoveryStatus: "completed", pendingPatchStatus: null as string | null, documentHash: "test-hash", markdownOnlyReason: null as string | null };
const byteOffset = (text: string): number => new TextEncoder().encode(markdown.slice(0, markdown.indexOf(text))).length;
function patchReview(status: string): unknown {
  const start = byteOffset("x^2");
  return {
    run_id: "mathscape:discovery-run/browser-fixture", patch_set_id: "mathscape:markdown-patch-set/fixture",
    input_fingerprint: "test-input", status, provider_model: null, message: null,
    patch_set: {
      version: "mathscape-markdown-patch-set-1", run_id: "mathscape:discovery-run/browser-fixture",
      base_snapshot: anchored("conversion").snapshot, base_input_fingerprint: "test-input",
      fingerprint: "sha256:" + "3".repeat(64),
      patches: [{
        patch_ref: "p1", issue_ref: "a-i1", range: { start, end: start + 3 },
        old_text: "x^2", old_text_hash: "h-old", replacement: "x^3", replacement_hash: "h-new",
        rationale: "指数被识别错误", status: "proposed",
      }],
    },
  };
}
// Predetermined IPC double, not a second implementation of backend segmentation.
function anchored(conversion: string): AnchoredMarkdown {
  const starts = ["# Math reader", "Inline $", "$$\n", "\\[\n", "![Diagram]", "<script>", "~~~tex"].map(marker => markdown.indexOf(marker));
  return {
    snapshot: { artifact_id: "md", conversion_id: conversion, content_hash: "test-hash", anchor_scheme: "mathscape-markdown-blocks-1", fingerprint: "test-" + conversion },
    source: markdown,
    blocks: starts.map((start, i) => ({ id: "test-block-" + i, kind: i === ink.pageMarkerBlock ? "page_marker" : "text", range: {
      start: new TextEncoder().encode(markdown.slice(0, start)).length,
      end: new TextEncoder().encode(markdown.slice(0, starts[i + 1] ?? markdown.length)).length,
    } })),
  };
}
function handle(command: string, payload: any): any {
  if (command === "get_ink_markdown_links") {
    const aid = String(payload.artifactId);
    const list = ink.linksByArtifact[aid] ?? ink.links;
    const uList = ink.unitLinksByArtifact[aid] ?? ink.unitLinks;
    return { artifact_id: aid, links: structuredClone(list), unit_links: structuredClone(uList), original_note_count: ink.originalNoteCount };
  }
  if (command === "get_markdown_unit_markers") {
    const wanted = new Set<string>(payload.artifactIds ?? []);
    return structuredClone(unitMarkers.filter(m => wanted.has(m.artifact_id)));
  }
  if (command === "get_canonical_reader_projection") {
    if (!canonicalProjection) throw new Error("规范解析正文未登记");
    return structuredClone(canonicalProjection);
  }
  if (command === "list_ink_documents") return [{ document_id: "note-old", title: "旧手稿", created_at: "unix:1", head_seq: 0, active_session_id: null, updated_at: "unix:1", lifecycle: "active", last_used_at: 5 }, { document_id: "note-gone", title: "已删手稿", created_at: "unix:1", head_seq: 0, active_session_id: null, updated_at: "unix:1", lifecycle: "trashed", last_used_at: 9 }];
  if (command === "list_ink_note_links") return [];
  if (command === "link_ink_note") {
    ink.linkRequests.push(structuredClone(payload.request));
    return { request_id: payload.request.request_id, kind: "link", note: { document_id: payload.request.document_id, title: "旧手稿" }, link: null, existing: false, title_changed: false, replayed: false };
  }
  if(command==="list_discovery_runs")return persistedOutcome?[{
    run_id:persistedOutcome.run.run_id,artifact_id:"md",artifact_name:"数学笔记.md",conversion_id:"conversion",input_fingerprint:"test-input",result_fingerprint:persistedOutcome.review.result_fingerprint,pipeline_version:"test",status:"completed",review_status:disposition?.disposition==="discarded"?"discarded":"reviewable",profile_id:"test",unit_count:2,fact_count:2,pending_task_count:0,unresolved_reference_count:1,relation_count:1,created_at:"test",updated_at:"test",registrations:[],disposition,
  }]:[];
  if(command==="list_discovery_registrations")return [];
  if(command==="get_discovery_review") {
    const outcome=structuredClone(persistedOutcome);
    if(disposition?.disposition==="discarded") {outcome.review.acceptance.state="blocked";outcome.review.units.forEach((u:any)=>u.acceptance="blocked");}
    return {run_id:outcome.run.run_id,review_status:disposition?.disposition==="discarded"?"discarded":"reviewable",created_at:"test",updated_at:"test",outcome,registrations:[],disposition};
  }
  if(command==="discard_discovery_run"||command==="restore_discovery_review") {
    const r=payload.request;dispositionRequests.push({command,request:r});
    if(command==="restore_discovery_review"&&restoreFailure)throw new Error("原文不可读取，恢复验证失败");
    if(command==="restore_discovery_review"&&restoredRequest===r.request_id)return disposition;
    if(r.expected_disposition_request_id!==(disposition?.request_id??null))throw new Error("disposition_conflict");
    disposition={run_id:r.run_id,request_id:r.request_id,disposition:command==="restore_discovery_review"?"restored":"discarded",actor:"test",reason:r.reason,decided_at:"test",previous_request_id:r.expected_disposition_request_id};
    if(command==="restore_discovery_review") {restoredRequest=r.request_id;if(restoreLostResponse){restoreLostResponse=false;throw new Error("响应丢失");}}
    return disposition;
  }
  if (command === "get_pending_markdown_patch_review") {
    return behavior.pendingPatchStatus === null ? null : patchReview(behavior.pendingPatchStatus);
  }
  if (command === "get_markdown_patch_review") return patchReview(behavior.pendingPatchStatus ?? "proposed");
  if (command === "preview_markdown_patch") return {
    run_id: "mathscape:discovery-run/browser-fixture", patch_set_id: "mathscape:markdown-patch-set/fixture",
    patch_set_fingerprint: "sha256:" + "3".repeat(64), original: markdown, revised: markdown.replace("x^2", "x^3"),
    safety: { status: "safe_for_b", diagnostics: [] },
  };
  if (command === "submit_markdown_patch_decision") {
    const request = (payload as any).request;
    patchDecisions.push(request);
    behavior.pendingPatchStatus = request.accepted_patch_refs.length > 0 ? "applied" : "rejected";
    return { status: behavior.pendingPatchStatus, patched_artifact_id: null, conversion_id: null, safety: { status: "safe_for_b", diagnostics: [] } };
  }
  if (command === "resume_discovery_after_patch") {
    resumeRequests.push((payload as any).request);
    behavior.pendingPatchStatus = null;
    behavior.discoveryStatus = "completed";
    const selection = discoveryRequests.at(-1) ?? { snapshot: anchored("conversion").snapshot, admission: "review_before_publish" };
    return handle("run_discovery", { request: { selection }, onProgress: (payload as any).onProgress });
  }
  if (command === "get_workspace_settings") return {
    version: "mathscape-workspace-settings-1",
    discovery_admission: behavior.admission,
    discovery_timeout_seconds: 600,
    connect_timeout_seconds: 15,
  };
  if (command === "get_active_discovery") return null;
  if (command === "cancel_discovery") return true;
  if (command === "accept_discovery_result") {
    const request = (payload as any).request;
    acceptanceRequests.push(request);
    return {
      state: "registered", decision_id: "acceptance-1", run_id: request.run_id,
      result_fingerprint: request.expected_result_fingerprint, plan_fingerprint: "plan-1",
      selected_units: request.selection.unit_refs, partial: behavior.discoveryStatus !== "completed",
      span_count: request.selection.unit_refs.length, unit_count: request.selection.unit_refs.length,
      reference_count: 1, relation_count: 0, pending_task_count: 0, already_accepted: false,
      commit_id: "sha256:" + "2".repeat(64), created_object_count: request.selection.unit_refs.length * 2,
      reused_object_count: 0, already_registered: false,
    };
  }
  if (command === "run_discovery") {
    const channel = (payload as any).onProgress;
    channel?.onmessage({ kind: "started", profile: "测试配置", model: "test-model" });
    channel?.onmessage({ kind: "workflow_event", event: { kind: "attempt_started", attempt: 1, stage: "unit-discovery", batch_index: null, batch_total: null, repair: false } });
    channel?.onmessage({ kind: "stream_delta", attempt: 1, stage: "unit-discovery", batch_index: null, batch_total: null, stream: "reasoning", text: "供应商公开摘要" });
    channel?.onmessage({ kind: "stream_delta", attempt: 1, stage: "unit-discovery", batch_index: null, batch_total: null, stream: "text", text: '{"units":' });
    const request = (payload as { request: { selection: SelectionRequest } }).request.selection;
    discoveryRequests.push(request);
    const doc = anchored(request.snapshot.conversion_id!);
    const unitBlock = doc.blocks[1];
    const anchor = {
      start: unitBlock.range.start, end: unitBlock.range.end,
      block_ids: [unitBlock.id], excerpt: "Inline $x^2$ and \\(a_n\\to a\\).",
    };
    // A strict sub-range of the unit block: the formula `$x^2$`.
    const formula = { start: byteOffset("$x^2$"), end: byteOffset("$x^2$") + 5, block_ids: [unitBlock.id], excerpt: "$x^2$" };
    const targetBlock = doc.blocks[2];
    const target = { start: targetBlock.range.start, end: targetBlock.range.end, block_ids: [targetBlock.id], excerpt: "\\frac{1}{n}" };
    const reviewAnchor = (purpose: string, a: typeof anchor) => ({ purpose, range: { start: a.start, end: a.end }, block_ids: a.block_ids, excerpt: a.excerpt });
    return new Promise(resolve => setTimeout(() => resolve({
      outcome: behavior.discoveryStatus,
      view: {
        version: "mathscape-desktop-discovery-view-1", status: behavior.discoveryStatus, complete: behavior.discoveryStatus === "completed",
        source: { artifact_id: "md", conversion_id: request.snapshot.conversion_id, content_hash: "test-hash", input_fingerprint: "test-input", requested_range: { start: unitBlock.range.start, end: unitBlock.range.end } },
        units: [{
          unit_ref: "u1", unit_kind: "statement", title: "测试命题", display_title: "测试命题",
          parent: null, completeness: "complete", fact_status: behavior.discoveryStatus === "completed" ? "processed" : "pending",
          source_excerpt: anchor.excerpt, anchors: [anchor], source_labels: [], source_title: null,
        }, {
          unit_ref: "u2", unit_kind: "statement", title: "被证命题", display_title: "被证命题",
          parent: null, completeness: "complete", fact_status: "processed",
          source_excerpt: target.excerpt, anchors: [target], source_labels: [], source_title: null,
        }],
        references: [{ owner: "u1", text: "定义 2", anchor: formula, status: "unresolved", candidates: [], reason: "no local declaration", origins: ["t1:r1"] }],
        relations: [{ source: "u1", target: "u2", relation: "proves", evidence: [anchor], bases: ["原文明示证明关系"], origins: ["t1:e1"] }],
        issues: [{ issue_ref: "t1:i1", stage: "source_facts", owner: "u1", kind: "missing_context", observation: "定义 2 不在选区中", requires_unit_revision: false, evidence: [anchor] }],
        pending_tasks: behavior.discoveryStatus === "completed" ? [] : [{ task_ref: "t1", owner: "u1" }], diagnostics: [],
        attempts: [{ attempt: 1, stage: "unit-discovery", repair: false, status: "completed", error: null, total_tokens: 150, elapsed_ms: 20 }],
      },
      review: behavior.discoveryStatus === "awaiting_patch_review" ? null : {
        version: "mathscape-source-review-projection-1", run_id: "mathscape:discovery-run/browser-fixture",
        result_fingerprint: "sha256:" + "1".repeat(64), status: behavior.discoveryStatus,
        complete: behavior.discoveryStatus === "completed",
        source: { artifact_id: "md", conversion_id: request.snapshot.conversion_id, content_hash: "test-hash", input_fingerprint: "test-input" },
        units: [{
          unit_ref: "u1", unit_kind: "statement", display_title: "测试命题", source_title: null,
          source_labels: [], parent: null, completeness: "complete",
          fact_status: behavior.discoveryStatus === "completed" ? "processed" : "pending",
          acceptance: "selectable", anchors: [{ purpose: "unit", range: { start: anchor.start, end: anchor.end }, block_ids: anchor.block_ids, excerpt: anchor.excerpt }],
          source_excerpt: anchor.excerpt,
        }, {
          unit_ref: "u2", unit_kind: "statement", display_title: "被证命题", source_title: null,
          source_labels: [], parent: null, completeness: "complete", fact_status: "processed",
          acceptance: "selectable", anchors: [reviewAnchor("unit", target)], source_excerpt: target.excerpt,
        }],
        references: [{ reference_ref: "r1", owner: "u1", text: "定义 2", anchor: reviewAnchor("reference", formula), status: "unresolved", candidates: [], reason: "no local declaration", origins: ["t1:r1"] }],
        structure_relations: [],
        relations: [{ relation_ref: "e1", source: "u1", target: "u2", relation: "proves", evidence: [reviewAnchor("relation_evidence", anchor), reviewAnchor("relation_evidence", formula)], bases: ["原文明示证明关系"], origins: ["t1:e1"] }],
        diagnostics: [{ diagnostic_ref: "d1", severity: "warning", stage: "source_facts", code: "warning_context", path: "/tasks/0", message: "上下文不足", checked: true, related_units: ["u1"] }],
        pending_tasks: behavior.discoveryStatus === "completed" ? [] : [{ task_ref: "t1", owner: "u1", reason: "待处理" }],
        acceptance: { state: "selectable", selected_units: [], required_units: [], missing_targets: [], blockers: [], allowed_scopes: [{ kind: "all_validated" }] },
      },
      run: {
        run_id: "mathscape:discovery-run/browser-fixture",
        profile_id: "desktop-default", provider: "test", requested_model: "test-model",
        admission: request.admission,
        request_timeout_seconds: 600,
        connect_timeout_seconds: 15,
        attempts: 1, input_tokens: 100, output_tokens: 50, total_tokens: 150, elapsed_ms: 20,
      },
    }), behavior.discoveryDelay));
  }
  if (command === "prepare_discovery_input") {
    const request = (payload as { request: SelectionRequest }).request;
    selectionRequests.push(request);
    const doc = anchored(request.snapshot.conversion_id!);
    const selector = request.selector;
    const first = selector.kind === "whole_document" ? 0 : doc.blocks.findIndex(b => b.id === selector.first);
    const last = selector.kind === "whole_document" ? doc.blocks.length - 1 : doc.blocks.findIndex(b => b.id === selector.last);
    const range = { start: doc.blocks[first].range.start, end: doc.blocks[last].range.end };
    return new Promise((resolve, reject) => setTimeout(() => {
      if (behavior.failSelection) return reject("测试：输入快照已改变");
      resolve({
        version: "mathscape-discovery-input-2", snapshot: request.snapshot,
        selection_fingerprint: "test-selection", fingerprint: "test-input",
        requested: { source_range: range, segments: doc.blocks.slice(first, last + 1).map(block => ({
          segment_kind: "block", block_id: block.id, structural_kind: block.kind,
          physical_range: block.range, source_range: block.range,
          text: new TextDecoder().decode(files.md.bytes.slice(block.range.start, block.range.end)),
        })) },
        context: [], boundary: { at_document_start: range.start === 0, at_document_end: range.end === files.md.bytes.length },
      });
    }, behavior.selectionDelay));
  }
  if (command === "list_markdown_errata") {
    const aid = (payload as { artifactId: string }).artifactId;
    return errataStore.items.filter((e) => e.artifact_id === aid && e.status === "active");
  }
  if (command === "save_markdown_errata") {
    const req = (payload as { request: any }).request;
    const blocks = req.artifact_id === "md"
      ? anchored("conversion").blocks
      : canonicalProjection?.pieces?.find((p: any) => p.kind === "segment" && p.artifact_id === req.artifact_id)?.blocks ?? [];
    const sourceText = req.artifact_id === "md"
      ? markdown
      : canonicalProjection?.pieces?.find((p: any) => p.kind === "segment" && p.artifact_id === req.artifact_id)?.markdown ?? "";
    const byteStartOffset = req.artifact_id === "md"
      ? 0
      : canonicalProjection?.pieces?.find((p: any) => p.kind === "segment" && p.artifact_id === req.artifact_id)?.byte_start ?? 0;
    const firstIdx = blocks.findIndex((b: any) => b.id === req.first_block_id);
    const lastIdx = blocks.findIndex((b: any) => b.id === req.last_block_id);
    if (firstIdx < 0 || lastIdx < firstIdx) throw new Error("所选块不存在");
    const slice = blocks.slice(firstIdx, lastIdx + 1);
    if (slice.some((b: any) => b.kind === "page_marker")) throw new Error("所选区间包含 PDF 分页标记");
    const baseStart = slice[0].range.start;
    const baseEnd = slice[slice.length - 1].range.end;
    const srcBytes = new TextEncoder().encode(sourceText);
    const originalText = new TextDecoder().decode(srcBytes.subarray(Math.max(0, baseStart - byteStartOffset), Math.max(0, baseEnd - byteStartOffset)));
    const newId = `mathscape:markdown-errata/test-${errataStore.seq++}`;
    const superseded_ids: string[] = [];
    for (const prev of errataStore.items) {
      if (prev.artifact_id === req.artifact_id && prev.status === "active" && prev.base_byte_start < baseEnd && prev.base_byte_end > baseStart) {
        prev.status = "superseded";
        prev.superseded_by = newId;
        superseded_ids.push(prev.errata_id);
      }
    }
    const created = {
      errata_id: newId,
      artifact_id: req.artifact_id,
      base_content_hash: req.base_content_hash ?? behavior.documentHash,
      first_block_id: req.first_block_id,
      last_block_id: req.last_block_id,
      covered_block_ids: slice.map((b: any) => b.id),
      base_byte_start: baseStart,
      base_byte_end: baseEnd,
      base_text_hash: "sha256:fixture",
      original_text: originalText,
      corrected_text: req.corrected_text,
      status: "active",
      source_kind: "manual",
      actor: "mathscape:actor/desktop-user",
      note: req.note ?? null,
      verified_pdf_page: req.verified_pdf_page ?? null,
      created_at: "2026-11-19T11:00:00+08:00",
      updated_at: "2026-11-19T11:00:00+08:00",
    };
    errataStore.items.push(created);
    const affected_unit_ids = unitMarkers
      .filter((u: any) => u.artifact_id === req.artifact_id && u.byte_range.start < baseEnd && u.byte_range.end > baseStart)
      .map((u: any) => u.unit_id);
    return { errata: created, superseded_ids, affected_unit_ids };
  }
  if (command === "revert_markdown_errata") {
    const eid = (payload as { errataId: string }).errataId;
    const item = errataStore.items.find((e) => e.errata_id === eid);
    if (!item) throw new Error("未找到勘误记录");
    item.status = "reverted";
    item.updated_at = "2026-11-19T11:01:00+08:00";
    return { errata: item, affected_unit_ids: [] };
  }
  if (command === "get_markdown_document") {
    const extra = markdownSnapshots[(payload as { artifactId: string }).artifactId];
    if (extra) return structuredClone(extra);
    const doc = anchored((payload as { conversionId: string }).conversionId);
    doc.snapshot.content_hash = behavior.documentHash;
    return doc;
  }
  if (command === "get_source_backlink") {
    backlinkRequests.push((payload as { request: unknown }).request);
    if (behavior.markdownOnlyReason) {
      return { kind: "markdown_only", reason: behavior.markdownOnlyReason };
    }
    return { kind: "exact_pdf_pages", pdf_artifact_id: "pdf", pdf_page_indices: [1], marker_page_ids: [1] };
  }
  const args = payload as { artifactId: string; offset: number; length: number };
  const file = files[args.artifactId];
  if (!file) throw new Error("Artifact 不存在");
  if (command === "get_artifact_content_info") return {
    artifact_id: args.artifactId, media_type: file.media, byte_size: file.bytes.length,
    contexts: args.artifactId === "md" ? ["conversion", "conversion-two"].map(conversion_id => ({ conversion_id, paths: [
      { path: "book/book.md", artifact_id: "md" },
      { path: "book/images/plot.png", artifact_id: "png" },
    ] })) : [],
  } satisfies ContentInfo;
  if (command === "read_artifact_bytes") {
    reads.push({ id: args.artifactId, offset: args.offset, length: args.length });
    if (behavior.pdfRangeFailure && args.artifactId === "pdf" && args.offset > 0) throw new Error("test original IPC failure");
    return file.bytes.slice(args.offset, args.offset + args.length).buffer;
  }
  throw new Error("Unexpected IPC: " + command);
}
mockIPC(handle);
// Renders one block with `selected` (its first occurrence) as the anchor.
function fragment(text: string, kind: string, selected: string) {
  const encoder = new TextEncoder();
  const bytes = encoder.encode(text);
  const start = encoder.encode(text.slice(0, text.indexOf(selected))).length;
  const anchor = { start, end: start + encoder.encode(selected).length, block_ids: ["b"], excerpt: selected, purpose: "unit" };
  const doc: AnchoredMarkdown = { ...anchored("conversion"), source: text, blocks: [{ id: "b", kind, range: { start: 0, end: bytes.length } }] };
  const node = renderAnchorFragment(doc, bytes, anchor, doc.blocks);
  return {
    marks: node.querySelectorAll(".review-anchor-highlight").length,
    whole: node.querySelector(".is-anchor-block") !== null,
    heading: node.querySelector("h2") !== null,
    items: node.querySelectorAll("li").length,
    table: node.querySelector("table") !== null,
    leaked: (node.textContent ?? "").includes("\uE000"),
  };
}
const discoveryOutcome = () => handle("run_discovery", {
  request: { selection: { snapshot: anchored("conversion").snapshot, admission: "review_before_publish" } },
});

const reader = new ArtifactReader(document.querySelector<HTMLElement>("#reader")!);
mountReviewWorkspace(document.querySelector<HTMLElement>("#review-workspace")!);
const ledger=new DiscoveryLedgerController(document.querySelector<HTMLElement>("#discovery-ledger")!,document.querySelector<HTMLButtonElement>("#refresh-ledger")!,{getSnapshot:()=>undefined,showToast:(text)=>document.querySelector("#ledger-toast")!.textContent=text});
async function prepareRecovery():Promise<void> {
  persistedOutcome=await discoveryOutcome();
  disposition={run_id:persistedOutcome.run.run_id,request_id:"mathscape:test/discarded",disposition:"discarded",actor:"test",reason:"放弃",decided_at:"test",previous_request_id:null};
  await ledger.loadDiscoveryRuns();
}
window.addEventListener("mathscape:open-source", event => {
  const detail = (event as CustomEvent).detail;
  void reader.open(detail.artifactId, detail.title, {
    focus: detail.focus, conversionId: detail.conversionId,
    patch: detail.patchRunId ? { runId: detail.patchRunId, mode: detail.patchMode ?? "workflow" } : undefined,
  });
});
Object.assign(window, {
  harness: { ink, locatedObjects, errataStore, errataEvents, unitMarkers, setCanonicalProjection: (p: any) => { canonicalProjection = p; }, setMarkdownSnapshot, backlinkRequests, overlappingNotes, markdownPreviewLabel, ArtifactPdfTransport, reader, reads, resolveProductPath, readRange, selectionRequests, discoveryRequests, acceptanceRequests, dispositionRequests, prepareRecovery, setRestoreFailure:(value:boolean)=>restoreFailure=value,setRestoreLostResponse:(value:boolean)=>restoreLostResponse=value,patchDecisions, resumeRequests, behavior, discoveryOutcome, fragment, render: (source: string) => markdownRenderer().render(source) },
});
