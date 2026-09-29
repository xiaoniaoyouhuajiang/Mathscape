import type { IncidenceGraph } from "../graph/projection";

export interface DesktopSnapshot {
  workspace_id: string;
  workspace_root: string;
  artifact_count: number;
  conversion_count: number;
  marker: MarkerRuntimeStatus;
  pdf_artifacts: PdfArtifactSummary[];
  landscape_graph: IncidenceGraph;
}

export interface MarkerRuntimeStatus {
  configured: boolean;
  ready: boolean;
  executable: string | null;
  version: string | null;
  problem: string | null;
}

export interface PdfArtifactSummary {
  artifact_id: string;
  filename: string;
  page_count: number | null;
}

export type PdfImportOutcome = "inserted" | "deduplicated";

export interface PdfImportResult {
  artifact_id: string;
  content_hash: string;
  byte_size: number;
  filename: string;
  page_count: number;
  document_title: string | null;
  content_kind: "text_based" | "scanned" | "image_based" | "mixed";
  classification_confidence: number;
  sampled_ocr_page_count: number;
  outcome: PdfImportOutcome;
  snapshot: DesktopSnapshot;
}

export type MarkerProfileChoice = "fast_native_v1" | "math_ocr_v1";

export interface ParseRequest {
  artifact_id: string;
  first_page: number;
  last_page: number;
  profile: MarkerProfileChoice;
}

export interface ParsedArtifactOutput {
  artifact_id: string;
  role: string;
  relative_path: string;
  byte_size: number;
}

export interface PdfParseResult {
  conversion_id: string;
  input_artifact_id: string;
  first_page: number;
  last_page: number;
  profile: MarkerProfileChoice;
  marker_version: string;
  elapsed_ms: number;
  outputs: ParsedArtifactOutput[];
  cleanup_warning: string | null;
  snapshot: DesktopSnapshot;
}
