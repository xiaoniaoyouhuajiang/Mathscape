import cytoscape from "cytoscape";
import { describe, expect, it } from "vitest";
import { applyStagedOverlay, stagedConflict, stagedMark } from "../../src/graph/staged-overlay";
import type { InvestigationSnapshot } from "../../src/artifact/types";

const investigation: InvestigationSnapshot = {
  investigation_id: "sandbox:s",
  staged: { "unit-new": { staged: true, change_set_id: "cs-1", change_set_version: 2, change_sets: ["cs-1@2"], replaces_revision: null } },
  conflicts: [{ object_id: "unit-old", change_sets: ["cs-2@1", "cs-3@1"] }],
};

describe("staged map overlay", () => {
  it("marks staged and conflicting elements from backend marks only", () => {
    const core = cytoscape({ headless: true, elements: [
      { data: { id: "n1", domainId: "unit-new", label: "定理", elementKind: "entity" } },
      { data: { id: "n2", domainId: "unit-old", label: "引理", elementKind: "entity" } },
      { data: { id: "e1", source: "n1", target: "n2", relationId: "rel-x" } },
    ] });
    applyStagedOverlay(core, investigation);
    expect(core.getElementById("n1").hasClass("is-staged")).toBe(true);
    expect(core.getElementById("n1").data("label")).toBe("〔未审批〕定理");
    expect(core.getElementById("n2").hasClass("is-staged-conflict")).toBe(true);
    expect(core.getElementById("n2").data("label")).toBe("〔提案冲突〕引理");
    expect(core.getElementById("e1").hasClass("is-staged")).toBe(false);
    applyStagedOverlay(core, investigation);
    expect(core.getElementById("n1").data("label")).toBe("〔未审批〕定理");
    applyStagedOverlay(core, null);
    expect(core.getElementById("n1").hasClass("is-staged")).toBe(false);
    expect(core.getElementById("n1").data("label")).toBe("定理");
    expect(core.getElementById("n2").data("label")).toBe("引理");
  });

  it("looks up marks by domain id", () => {
    expect(stagedMark(investigation, "unit-new")?.change_set_version).toBe(2);
    expect(stagedMark(investigation, "unit-old")).toBeNull();
    expect(stagedMark(undefined, "unit-new")).toBeNull();
    expect(stagedConflict(investigation, "unit-old")).toEqual(["cs-2@1", "cs-3@1"]);
    expect(stagedConflict(investigation, "unit-new")).toBeNull();
  });
});
