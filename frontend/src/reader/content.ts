import { invoke } from "@tauri-apps/api/core";
import type { SourceBacklinkView } from "./backlink";

export interface ProductPath { path: string; artifact_id: string }
export interface ContentInfo {
  artifact_id: string;
  media_type: string;
  ink_snapshot_id?: string;
  byte_size: number;
  contexts: { conversion_id: string; paths: ProductPath[] }[];
}

export interface SourceErrataReviewItem {
  errata_id: string;
  artifact_id: string;
  first_block_id: string;
  last_block_id: string;
  original_text: string;
  corrected_text: string;
  source_kind: string;
  updated_at: string;
  sent_to_sandbox: boolean;
}

export interface SourceUnitErrataReview {
  unit_id: string;
  items: SourceErrataReviewItem[];
}

export interface SourceNodeDetail {
  node_id: string;
  kind: string;
  display_title: string | null;
  unit_kind: string | null;
  parent: string | null;
  artifact_id: string | null;
  byte_range: { start: number; end: number } | null;
  excerpt: string | null;
  markdown_block_ids: string[];
  payload: unknown;
  ink?: { snapshot_id: string; document_id: string; page_id: string; stroke_ids: string[] };
  pdf_backlink?: SourceBacklinkView;
  errata_review?: SourceUnitErrataReview;
}
export function sourceNodeDetail(nodeId: string): Promise<SourceNodeDetail> { return invoke("get_source_node_detail", { nodeId }); }

/**
 * Reader focus for a source unit / span: its block in the physical snapshot,
 * plus what the 「规范解析正文」 view needs to land on the same place (owning
 * snapshot, PDF page) and to name it on the 「回到出发处」 chip.
 */
export function readingFocusOf(detail: SourceNodeDetail, label?: string | null): {
  start: number; end: number; blockIds: string[]; artifactId?: string; pdfPageIndex?: number; label?: string;
} | undefined {
  if (!detail.byte_range) return undefined;
  const backlink = detail.pdf_backlink && detail.pdf_backlink.kind !== "markdown_only" ? detail.pdf_backlink : undefined;
  return {
    ...detail.byte_range,
    blockIds: detail.markdown_block_ids,
    artifactId: detail.artifact_id ?? undefined,
    pdfPageIndex: backlink?.marker_page_ids?.[0] ?? backlink?.pdf_page_indices?.[0],
    label: (label ?? detail.display_title) || undefined,
  };
}

export interface MarkdownUnitMarker {
  unit_id: string;
  title: string;
  unit_kind: string;
  artifact_id: string;
  byte_range: { start: number; end: number };
  block_ids: string[];
}

export async function getMarkdownUnitMarkers(artifactIds: string[]): Promise<MarkdownUnitMarker[]> {
  if (!artifactIds.length) return [];
  try {
    const markers = await invoke<MarkdownUnitMarker[] | undefined>("get_markdown_unit_markers", { artifactIds });
    return Array.isArray(markers) ? markers : [];
  } catch {
    return [];
  }
}

export function contentInfo(id: string): Promise<ContentInfo> {
  return invoke("get_artifact_content_info", { artifactId: id });
}

export async function readRange(id: string, offset: number, length: number): Promise<Uint8Array> {
  const result = new Uint8Array(length);
  for (let position = 0; position < length;) {
    const count = Math.min(4 * 1024 * 1024, length - position);
    const data = await invoke<ArrayBuffer>("read_artifact_bytes", { artifactId: id, offset: offset + position, length: count });
    const chunk = new Uint8Array(data);
    if (chunk.byteLength !== count) throw new Error("文件长度已改变，请重新打开。");
    result.set(chunk, position);
    position += count;
  }
  return result;
}

export async function readSmallFile(info: ContentInfo): Promise<Uint8Array> {
  if (info.byte_size > 16 * 1024 * 1024) throw new Error("文本或图片超过 16 MiB，当前阅读视图暂不加载。");
  return readRange(info.artifact_id, 0, info.byte_size);
}

// Resolve against the conversion's virtual root, never against a host path.
export function resolveProductPath(base: string, reference: string): string | null {
  let ref: string;
  try { ref = decodeURIComponent(reference.split(/[?#]/)[0]); } catch { return null; }
  if (!ref || /^[a-z][a-z0-9+.-]*:/i.test(ref) || ref.startsWith("/") || ref.includes("\\") || /[\u0000-\u001f]/.test(ref)) return null;
  const parts = base.split("/").slice(0, -1);
  for (const part of ref.split("/")) {
    if (part === "..") { if (!parts.length) return null; parts.pop(); }
    else if (part && part !== ".") parts.push(part);
  }
  return parts.join("/");
}
