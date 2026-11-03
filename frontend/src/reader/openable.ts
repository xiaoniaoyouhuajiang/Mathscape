import type { IncidenceNode } from "../graph/projection";

/**
 * Map node → physical artifact the reader opens.
 *
 * A SourceArtifact node's `domain_id` is usually a physical artifact id, but a
 * folded canonical markdown node (「… · 规范解析正文」) carries a *logical*
 * canonical id that is not a row in the artifact catalog. The landscape
 * projection exposes its head snapshot as `openable_artifact_id`; open that
 * instead. Map jumps / cite chips keep using `domain_id`.
 *
 * `get_artifact_content_info` stays physical-id only, so a logical id with no
 * head fails here with a clear message instead of 「Artifact 不存在」.
 */
export const CANONICAL_MARKDOWN_ID_PREFIX = "mathscape:canonical-markdown/";

type OpenableNode = Pick<IncidenceNode, "domain_kind" | "domain_id"> & { openable_artifact_id?: string | null };

export class NotOpenableError extends Error {}

export function isCanonicalMarkdownId(id: string): boolean {
  return id.startsWith(CANONICAL_MARKDOWN_ID_PREFIX);
}

/** Physical artifact id for a SourceArtifact node; throws when it has none. */
export function openableArtifactId(node: OpenableNode): string {
  if (node.domain_kind !== "SourceArtifact" || !node.domain_id) {
    throw new NotOpenableError("该节点不是可阅读的文件");
  }
  if (node.openable_artifact_id) return node.openable_artifact_id;
  if (isCanonicalMarkdownId(node.domain_id)) {
    throw new NotOpenableError("规范解析正文尚无可读的 head 快照（解析未完成或未同步），请稍后刷新地图");
  }
  return node.domain_id;
}

/** Like {@link openableArtifactId} but `undefined` instead of throwing. */
export function tryOpenableArtifactId(node: OpenableNode): string | undefined {
  try { return openableArtifactId(node); } catch { return undefined; }
}

/**
 * What 「阅读文件」 / 并排阅读 open for a SourceArtifact node. A folded canonical
 * node opens its unified read-only view under the logical canonical id
 * (`ArtifactReader` stitches every coverage segment in page order; see
 * docs/canonical-markdown.md §「统一阅读投影」). Physical nodes open by
 * domain_id. Ink notes, cite chips and discoveries keep using the physical
 * {@link openableArtifactId}.
 */
export function readerTargetId(node: OpenableNode): string {
  const physical = openableArtifactId(node);
  return isCanonicalMarkdownId(node.domain_id!) ? node.domain_id! : physical;
}

/**
 * The map node standing for a physical artifact: the node itself, or the
 * folded canonical node whose head it is. Used for labels and 「在地图中定位」.
 */
export function findSourceArtifactNode<T extends OpenableNode>(nodes: readonly T[], artifactId: string): T | undefined {
  const sources = nodes.filter(node => node.domain_kind === "SourceArtifact");
  return sources.find(node => node.domain_id === artifactId)
    ?? sources.find(node => node.openable_artifact_id === artifactId);
}

/**
 * 并排阅读 source picker: PDFs first, then every openable SourceArtifact node.
 * Folded canonical nodes list their unified view (canonical id) under the node
 * label; a canonical node without a head is left out rather than listed
 * unopenable.
 */
export function splitSourceItemsFrom(
  pdfs: readonly { artifact_id: string; filename?: string | null }[],
  nodes: readonly (OpenableNode & { label: string })[],
): { artifactId: string; label: string }[] {
  const items: { artifactId: string; label: string }[] = [];
  const seen = new Set<string>();
  for (const pdf of pdfs) {
    if (seen.has(pdf.artifact_id)) continue;
    seen.add(pdf.artifact_id);
    items.push({ artifactId: pdf.artifact_id, label: pdf.filename || pdf.artifact_id });
  }
  for (const node of nodes) {
    if (node.domain_kind !== "SourceArtifact" || !node.domain_id) continue;
    let artifactId: string;
    try { artifactId = readerTargetId(node); } catch { continue; }
    if (seen.has(artifactId)) continue;
    seen.add(artifactId);
    items.push({ artifactId, label: node.label || node.domain_id });
  }
  return items;
}
