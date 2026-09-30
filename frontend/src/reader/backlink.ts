import { invoke } from "@tauri-apps/api/core";

export interface SourceBacklinkView {
  kind: "exact_pdf_pages" | "candidate_pdf_pages" | "markdown_only";
  pdf_artifact_id?: string;
  pdf_page_indices?: number[];
  marker_page_ids?: number[];
  reason?: string;
}

export interface SourceBacklinkRequest {
  artifact_id: string;
  conversion_id: string | null;
  start: number;
  end: number;
  block_ids: string[];
}

export function getSourceBacklink(request: SourceBacklinkRequest): Promise<SourceBacklinkView> {
  return invoke("get_source_backlink", { request });
}
