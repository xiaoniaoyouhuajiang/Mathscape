import { mockIPC } from "@tauri-apps/api/mocks";
import { ArtifactReader } from "../src/reader/reader";
import { ArtifactPdfTransport } from "../src/reader/pdf-transport";
import { markdownRenderer } from "../src/reader/markdown";
import { resolveProductPath, readRange, type ContentInfo } from "../src/reader/content";
import type { AnchoredMarkdown, SelectionRequest } from "../src/reader/selection";

// Tiny reproducible PDF: two pages and one outline entry, no copyrighted assets.
function samplePdf(): Uint8Array {
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R /Outlines 7 0 R >>",
    "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Contents 5 0 R >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Contents 6 0 R >>",
    "<< /Length 25 >>\nstream\n1 0 0 rg 0 0 100 100 re f\n\nendstream",
    "<< /Length 25 >>\nstream\n0 0 1 rg 0 0 200 200 re f\n\nendstream",
    "<< /Type /Outlines /First 8 0 R /Last 8 0 R /Count 1 >>",
    "<< /Title (Second page) /Parent 7 0 R /Dest [4 0 R /Fit] >>",
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
const patchDecisions: any[] = [];
const resumeRequests: unknown[] = [];
const behavior = { pdfRangeFailure: false, selectionDelay: 0, discoveryDelay: 0, failSelection: false, admission: "review_before_publish", discoveryStatus: "completed", pendingPatchStatus: null as string | null };
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
    blocks: starts.map((start, i) => ({ id: "test-block-" + i, kind: "text", range: {
      start: new TextEncoder().encode(markdown.slice(0, start)).length,
      end: new TextEncoder().encode(markdown.slice(0, starts[i + 1] ?? markdown.length)).length,
    } })),
  };
}
mockIPC(function handle(command, payload): any {
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
    return new Promise(resolve => setTimeout(() => resolve({
      outcome: behavior.discoveryStatus,
      view: {
        version: "mathscape-desktop-discovery-view-1", status: behavior.discoveryStatus, complete: behavior.discoveryStatus === "completed",
        source: { artifact_id: "md", conversion_id: request.snapshot.conversion_id, content_hash: "test-hash", input_fingerprint: "test-input" },
        units: [{
          unit_ref: "u1", unit_kind: "statement", title: "测试命题", display_title: "测试命题",
          parent: null, completeness: "complete", fact_status: behavior.discoveryStatus === "completed" ? "processed" : "pending",
          source_excerpt: anchor.excerpt, anchors: [anchor], source_labels: [], source_title: null,
        }],
        references: [{ owner: "u1", text: "定义 2", anchor, status: "unresolved", candidates: [], reason: "no local declaration", origins: ["t1:r1"] }],
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
        }],
        references: [], structure_relations: [], relations: [], diagnostics: [],
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
  if (command === "get_markdown_document") return anchored((payload as { conversionId: string }).conversionId);
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
});

const reader = new ArtifactReader(document.querySelector<HTMLElement>("#reader")!);
Object.assign(window, {
  harness: { ArtifactPdfTransport, reader, reads, resolveProductPath, readRange, selectionRequests, discoveryRequests, acceptanceRequests, patchDecisions, resumeRequests, behavior, render: (source: string) => markdownRenderer().render(source) },
});
