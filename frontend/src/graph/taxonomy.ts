import { FOCUS_CONTRACTION_KIND } from "./focus";
import type { IncidenceNode } from "./projection";

/**
 * Single source of truth for user-visible type names and visual encoding.
 *
 * Cytoscape styles, the canvas legend and the Inspector all read from here so
 * that one concept never appears under two different Chinese names.
 */
export type NodeTypeKey =
  | "pdf"
  | "markdown"
  | "parse_metadata"
  | "ink_snapshot"
  | "parse_image"
  | "attachment"
  | "artifact"
  | "semantic_unit"
  | "source_span"
  | "conversion_run"
  | "source_relation"
  | "evidence_relation"
  | "focus_contraction"
  | "shared_entity"
  | "shared_theory"
  | "dialect_scope"
  | "debt"
  | "alignment_relation"
  | "semantic_link"
  | "unknown_entity"
  | "unknown_relation";

export type NodeShape =
  | "round-rectangle"
  | "cut-rectangle"
  | "hexagon"
  | "barrel"
  | "rectangle"
  | "ellipse"
  | "diamond"
  | "star"
  | "round-octagon"
  | "tag";

export type BorderStyle = "solid" | "dashed";

export interface NodeTypeVisual {
  label: string;
  shape: NodeShape;
  width: number;
  height: number;
  background: string;
  border: string;
  borderStyle: BorderStyle;
  backgroundOpacity: number;
  fontSize: number;
}

/** Declaration order doubles as legend order. */
export const NODE_TYPES: Record<NodeTypeKey, NodeTypeVisual> = {
  pdf: {
    label: "PDF",
    shape: "round-rectangle",
    width: 31,
    height: 20,
    background: "var(--ms-accent)",
    border: "var(--ms-accent)",
    borderStyle: "solid",
    backgroundOpacity: 0.25,
    fontSize: 11,
  },
  markdown: {
    label: "Markdown 正文",
    shape: "cut-rectangle",
    width: 30,
    height: 18,
    background: "var(--ms-accent)",
    border: "var(--ms-accent)",
    borderStyle: "solid",
    backgroundOpacity: 0.25,
    fontSize: 11,
  },
  parse_metadata: {
    label: "解析元数据",
    shape: "hexagon",
    width: 21,
    height: 16,
    background: "var(--ms-accent)",
    border: "var(--ms-accent)",
    borderStyle: "solid",
    backgroundOpacity: 0.25,
    fontSize: 9,
  },
  ink_snapshot: {
    label: "手稿快照", shape: "round-rectangle", width: 30, height: 20,
    background: "var(--ms-candidate)", border: "var(--ms-candidate)", borderStyle: "solid", backgroundOpacity: 0.25, fontSize: 10,
  },
  parse_image: {
    label: "解析图片",
    shape: "barrel",
    width: 22,
    height: 17,
    background: "var(--ms-accent)",
    border: "var(--ms-accent)",
    borderStyle: "solid",
    backgroundOpacity: 0.25,
    fontSize: 9,
  },
  attachment: {
    label: "附件",
    shape: "rectangle",
    width: 21,
    height: 16,
    background: "var(--ms-accent)",
    border: "var(--ms-accent)",
    borderStyle: "solid",
    backgroundOpacity: 0.25,
    fontSize: 9,
  },
  artifact: {
    label: "材料文件",
    shape: "round-rectangle",
    width: 29,
    height: 18,
    background: "var(--ms-accent)",
    border: "var(--ms-accent)",
    borderStyle: "solid",
    backgroundOpacity: 0.25,
    fontSize: 11,
  },
  semantic_unit: {
    label: "内容单元",
    shape: "ellipse",
    width: 22,
    height: 22,
    background: "var(--ms-accent)",
    border: "var(--ms-accent)",
    borderStyle: "solid",
    backgroundOpacity: 0.25,
    fontSize: 11,
  },
  source_span: {
    label: "原文证据",
    shape: "round-rectangle",
    width: 20,
    height: 14,
    background: "var(--ms-accent)",
    border: "var(--ms-accent)",
    borderStyle: "dashed",
    backgroundOpacity: 0.25,
    fontSize: 9,
  },
  conversion_run: {
    label: "转换任务",
    shape: "diamond",
    width: 22,
    height: 22,
    background: "var(--ms-accent)",
    border: "var(--ms-accent)",
    borderStyle: "solid",
    backgroundOpacity: 0.25,
    fontSize: 10,
  },
  source_relation: {
    label: "来源关系",
    shape: "diamond",
    width: 21,
    height: 21,
    background: "var(--ms-accent)",
    border: "var(--ms-accent)",
    borderStyle: "solid",
    backgroundOpacity: 0.25,
    fontSize: 10,
  },
  evidence_relation: {
    label: "证据关系",
    shape: "diamond",
    width: 18,
    height: 18,
    background: "var(--ms-accent)",
    border: "var(--ms-accent)",
    borderStyle: "dashed",
    backgroundOpacity: 0.25,
    fontSize: 9,
  },
  focus_contraction: {
    label: "已折叠的派生路径",
    shape: "diamond",
    width: 17,
    height: 17,
    background: "var(--ms-accent)",
    border: "var(--ms-accent)",
    borderStyle: "dashed",
    backgroundOpacity: 0.25,
    fontSize: 9,
  },
  // 语义层（docs/semantic-write-contract.md S9）：白话名字只在这里定义。
  shared_entity: {
    label: "共享对象",
    shape: "star",
    width: 26,
    height: 26,
    background: "var(--ms-map-node-core)",
    border: "var(--ms-star-gold)",
    borderStyle: "solid",
    backgroundOpacity: 0.95,
    fontSize: 11,
  },
  shared_theory: {
    label: "理论",
    shape: "round-octagon",
    width: 30,
    height: 26,
    background: "var(--ms-accent)",
    border: "var(--ms-accent)",
    borderStyle: "solid",
    backgroundOpacity: 0.25,
    fontSize: 11,
  },
  dialect_scope: {
    label: "约定卡",
    shape: "rectangle",
    width: 26,
    height: 17,
    background: "var(--ms-accent)",
    border: "var(--ms-accent)",
    borderStyle: "solid",
    backgroundOpacity: 0.25,
    fontSize: 10,
  },
  debt: {
    label: "一笔账",
    shape: "tag",
    width: 20,
    height: 14,
    background: "var(--ms-warning)",
    border: "var(--ms-warning)",
    borderStyle: "dashed",
    backgroundOpacity: 0.08,
    fontSize: 9,
  },
  alignment_relation: {
    label: "对齐线",
    shape: "diamond",
    width: 18,
    height: 18,
    background: "var(--ms-star-gold)",
    border: "var(--ms-star-gold)",
    borderStyle: "solid",
    backgroundOpacity: 0.25,
    fontSize: 10,
  },
  semantic_link: {
    label: "语义连接",
    shape: "diamond",
    width: 14,
    height: 14,
    background: "var(--ms-accent)",
    border: "var(--ms-accent)",
    borderStyle: "dashed",
    backgroundOpacity: 0.25,
    fontSize: 9,
  },
  unknown_entity: {
    label: "未分类对象",
    shape: "ellipse",
    width: 21,
    height: 21,
    background: "var(--ms-accent)",
    border: "var(--ms-accent)",
    borderStyle: "solid",
    backgroundOpacity: 0.25,
    fontSize: 11,
  },
  unknown_relation: {
    label: "未分类关系",
    shape: "diamond",
    width: 20,
    height: 20,
    background: "var(--ms-accent)",
    border: "var(--ms-accent)",
    borderStyle: "solid",
    backgroundOpacity: 0.25,
    fontSize: 10,
  },
};

export const NODE_TYPE_ORDER = Object.keys(NODE_TYPES) as NodeTypeKey[];

export type EdgeTypeKey =
  | "relation"
  | "alignment"
  | "debt"
  | "evidence"
  | "contracted"
  | "incidence";

export interface EdgeTypeVisual {
  label: string;
  color: string;
  dashed: boolean;
}

export const EDGE_TYPES: Record<EdgeTypeKey, EdgeTypeVisual> = {
  relation: { label: "关系（箭头表示方向）", color: "var(--ms-map-edge-secondary)", dashed: false },
  alignment: { label: "对齐线（教材内容 → 共享对象）", color: "var(--ms-map-edge-primary)", dashed: false },
  debt: { label: "挂账（还欠的检查）", color: "var(--ms-warning)", dashed: true },
  evidence: { label: "原文证据链接", color: "var(--ms-text-muted)", dashed: true },
  contracted: { label: "派生自（路径已折叠）", color: "var(--ms-map-edge-secondary)", dashed: true },
  incidence: { label: "超边端点连接", color: "var(--ms-map-edge-secondary)", dashed: false },
};

/** Chinese names for raw `domain_kind` values, used when no finer type applies. */
const DOMAIN_KIND_LABELS: Record<string, string> = {
  SourceArtifact: "材料文件",
  ConversionRun: "转换任务",
  SourceSpan: "原文证据",
  SourceSemanticUnit: "内容单元",
  SourceUnitRelation: "来源关系",
  Evidence: "证据关系",
  FocusContraction: "已折叠的派生路径",
  SemanticEntity: "共享对象",
  Alignment: "对齐线",
  Debt: "一笔账",
  DialectScope: "约定卡",
  InkNote: "手稿",
};

export function domainKindLabel(kind: string | null): string {
  if (kind === null) return "—";
  return DOMAIN_KIND_LABELS[kind] ?? kind;
}

export function nodeTypeKey(node: IncidenceNode): NodeTypeKey {
  if (node.element_kind === "relation") {
    switch (node.domain_kind) {
      case "ConversionRun":
        return "conversion_run";
      case "SourceUnitRelation":
        return "source_relation";
      case "Evidence":
        return "evidence_relation";
      case FOCUS_CONTRACTION_KIND:
        return "focus_contraction";
      case "Alignment":
        return "alignment_relation";
      case "SemanticEntity":
      case "Debt":
      case "DialectScope":
        return "semantic_link";
      default:
        return "unknown_relation";
    }
  }
  switch (node.domain_kind) {
    case "SourceSemanticUnit":
      return "semantic_unit";
    case "SourceSpan":
      return "source_span";
    case "SourceArtifact":
      return artifactTypeKey(node.summary);
    case "SemanticEntity":
      return node.summary?.startsWith("理论") ? "shared_theory" : "shared_entity";
    case "DialectScope":
      return "dialect_scope";
    case "Debt":
      return "debt";
    default:
      return "unknown_entity";
  }
}

export function nodeTypeLabel(node: IncidenceNode): string {
  return NODE_TYPES[nodeTypeKey(node)].label;
}

/**
 * PDF and its parsed Markdown share `domain_kind = "SourceArtifact"`, so the
 * media type has to come from the summary that `project_artifact_catalog`
 * writes as `"{media_type} · {bytes} bytes · {status}"`. If the projection ever
 * exposes the media type as its own field, read that field instead.
 */
function artifactTypeKey(summary: string | null): NodeTypeKey {
  const mediaType = summary === null ? "" : summary.split(" · ", 1)[0];
  if (!mediaType.includes("/")) return "artifact";
  if (mediaType === "application/pdf") return "pdf";
  if (mediaType === "text/markdown") return "markdown";
  if (mediaType === "application/json") return "parse_metadata";
  if (mediaType === "application/vnd.mathscape.ink-snapshot+json") return "ink_snapshot";
  if (mediaType.startsWith("image/")) return "parse_image";
  return "attachment";
}
