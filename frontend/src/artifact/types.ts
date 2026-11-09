import type { IncidenceGraph } from "../graph/projection";

export interface DesktopSnapshot {
  workspace_id: string;
  workspace_root: string;
  artifact_count: number;
  conversion_count: number;
  marker: MarkerRuntimeStatus;
  lean: LeanRuntimeStatus;
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

export type LeanProfileRootSource =
  | "environment"
  | "workspace"
  | "workspace_parent"
  | "current_directory"
  | "platform_data";

export type LeanProfileState = "ready" | "incomplete" | "unverified" | "mismatch";
export type LeanFingerprintStatus = "verified" | "unverified" | "incomplete";

/** Lean profiles visible to the workspace: an environment / external library. */
export interface LeanRuntimeStatus {
  root: string | null;
  source: LeanProfileRootSource | null;
  root_exists: boolean;
  profiles: LeanProfileSummary[];
  problem: string | null;
}

export interface LeanProfileSummary {
  id: string;
  state: LeanProfileState | null;
  lean_toolchain: string | null;
  mathlib_reference: string | null;
  detected_mathlib_revision: string | null;
  fingerprint_status: LeanFingerprintStatus | null;
  fingerprint_digest: string | null;
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
