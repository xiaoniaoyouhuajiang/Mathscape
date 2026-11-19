/**
 * Non-blocking map overlay for `SourceSemanticUnit` nodes whose anchored
 * Markdown blocks have unreviewed manual/patch errata.
 *
 * Synthesized purely in Cytoscape (never persisted into map commits), always
 * cleared before `ink-overlay.ts` unwinds and applied right after so base
 * labels stack and unwind in exact LIFO order.
 */
import type { Core } from "cytoscape";

export const ERRATA_REVIEW_CLASS = "has-errata-review";

export function clearErrataReviewOverlay(core: Core): void {
  core.nodes(`.${ERRATA_REVIEW_CLASS}`).forEach((node) => {
    const base = node.data("errataBaseLabel");
    if (typeof base === "string") node.data("label", base);
    node.removeData("errataBaseLabel errataUnreviewedCount");
    node.removeClass(ERRATA_REVIEW_CLASS);
  });
}

export function applyErrataReviewOverlay(
  core: Core,
  unreviewedByUnitId: Readonly<Record<string, number>> | undefined,
): number {
  clearErrataReviewOverlay(core);
  if (!unreviewedByUnitId) return 0;
  const entries = Object.entries(unreviewedByUnitId).filter(([, count]) => count > 0);
  if (!entries.length) return 0;
  const lookup = new Map<string, number>(entries);
  let marked = 0;
  core.batch(() => {
    core.nodes().forEach((node) => {
      const domainId = node.data("domainId");
      if (typeof domainId !== "string") return;
      if (node.data("elementKind") && node.data("elementKind") !== "entity") return;
      const count = lookup.get(domainId);
      if (!count) return;
      const base = String(node.data("label") ?? "");
      node.data("errataBaseLabel", base);
      node.data("errataUnreviewedCount", count);
      if (!base.endsWith(" · 已校对")) node.data("label", `${base} · 已校对`);
      node.addClass(ERRATA_REVIEW_CLASS);
      marked += 1;
    });
  });
  return marked;
}
