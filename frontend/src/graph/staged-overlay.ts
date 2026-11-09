/**
 * Phase2 「未审批」 overlay for the map.
 *
 * The backend projects the investigation (scope=investigation): staged
 * objects are already in `landscape_graph`, staged edits replace the accepted
 * version, and `investigation.staged` / `investigation.conflicts` carry the
 * marks by domain object id. The frontend only styles what it is given; it
 * never assembles staged nodes itself.
 */
import type { Core } from "cytoscape";
import type { InvestigationSnapshot, StagedMark } from "../artifact/types";

export const STAGED_CLASS = "is-staged";
export const STAGED_CONFLICT_CLASS = "is-staged-conflict";

/** Domain ids the element stands for (node: domainId, edge/junction: relationId or domainId). */
function domainIds(data: Record<string, unknown>): string[] {
  return [data.domainId, data.relationId].filter((value): value is string => typeof value === "string");
}

export function stagedMark(investigation: InvestigationSnapshot | null | undefined, domainId: string | null | undefined): StagedMark | null {
  if (!investigation || !domainId) return null;
  return investigation.staged[domainId] ?? null;
}

export function applyStagedOverlay(core: Core, investigation: InvestigationSnapshot | null | undefined): void {
  const staged = investigation?.staged ?? {};
  const conflicts = new Set((investigation?.conflicts ?? []).map(conflict => conflict.object_id));
  core.batch(() => {
    core.elements().forEach(element => {
      const ids = domainIds(element.data() as Record<string, unknown>);
      const mark = ids.map(id => staged[id]).find(Boolean);
      element.toggleClass(STAGED_CLASS, !!mark);
      element.toggleClass(STAGED_CONFLICT_CLASS, ids.some(id => conflicts.has(id)));
      if (element.isNode()) {
        const base = String(element.data("label") ?? "").replace(/^〔未审批〕/, "");
        element.data("label", mark && element.data("elementKind") !== "junction" ? `〔未审批〕${base}` : base);
      }
    });
  });
}
