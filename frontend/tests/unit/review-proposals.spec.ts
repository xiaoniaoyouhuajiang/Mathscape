import { describe, expect, it } from "vitest";
import {
  defaultSelection, dependencyGroups, deselect, discardImpact, discardPreview, impactSummary, keyOf,
  proposalDiscoveryOutcome, proposalReviewProjection, select, selectionRefs, type PendingChangeSet,
} from "../../src/review/proposal-model";

function item(id: string, deps: string[] = [], state = "staged", createdAt = id): PendingChangeSet {
  return {
    changeSetId: id, version: 1, state, investigationId: "sandbox:s", roundId: "r", proposedBy: "agent:codex",
    createdAt, proposalFingerprint: `fp-${id}`, baseCommit: null,
    proposal: { rationale: id },
    result: {},
    dependencies: deps.map(dep => ({ changeSetId: dep, version: 1, referencedIds: [] })),
  };
}

// A ← B ← C (chain), D independent, E blocked depending on D.
const items = [item("A"), item("B", ["A"]), item("C", ["B"]), item("D"), item("E", ["D"], "blocked")];

describe("proposal batch selection", () => {
  it("defaults to every staged version", () => {
    expect([...defaultSelection(items)].sort()).toEqual(["A@1", "B@1", "C@1", "D@1"]);
  });

  it("unchecking cascades through every level of dependents", () => {
    const next = deselect(items, defaultSelection(items), "A@1");
    expect([...next].sort()).toEqual(["D@1"]);
    expect([...deselect(items, defaultSelection(items), "B@1")].sort()).toEqual(["A@1", "D@1"]);
  });

  it("checking pulls in dependencies and refuses blocked ones", () => {
    expect([...select(items, new Set(), "C@1")!].sort()).toEqual(["A@1", "B@1", "C@1"]);
    expect(select(items, new Set(), "E@1")).toBeNull();
  });

  it("groups by dependency component, dependencies first", () => {
    const groups = dependencyGroups([items[2], items[0], items[4], items[1], items[3]]);
    expect(groups.map(group => group.map(keyOf))).toEqual([["A@1", "B@1", "C@1"], ["D@1", "E@1"]]);
  });

  it("orders like the backend batch: by (change_set_id, version), never createdAt", () => {
    const late = [item("A", [], "staged", "2026-11-10T09:00"), item("B", ["A"], "staged", "2026-11-09T01:00"), item("D", [], "staged", "2026-11-01T00:00")];
    expect(dependencyGroups(late).map(group => group.map(keyOf))).toEqual([["A@1", "B@1"], ["D@1"]]);
    const reversed = late.map(entry => ({ ...entry, createdAt: entry.createdAt.split("").reverse().join("") }));
    expect(dependencyGroups(reversed).map(group => group.map(keyOf))).toEqual([["A@1", "B@1"], ["D@1"]]);
  });

  it("previews who a discard touches before anything happens", () => {
    const preview = discardPreview(items, ["A@1"]);
    expect(preview.roots.map(keyOf)).toEqual(["A@1"]);
    expect(preview.dependents.map(keyOf)).toEqual(["B@1", "C@1"]);
    expect(discardPreview(items, ["D@1"]).dependents.map(keyOf)).toEqual(["E@1"]);
    expect(discardPreview(items, ["C@1"]).dependents).toEqual([]);
  });

  it("mirrors the backend discard rule", () => {
    const blocked = discardImpact(items, ["A@1"], false);
    expect([...blocked.discarded]).toEqual(["A@1"]);
    expect([...blocked.blocked].sort()).toEqual(["B@1", "C@1"]);
    const cascade = discardImpact(items, ["A@1"], true);
    expect([...cascade.discarded].sort()).toEqual(["A@1", "B@1", "C@1"]);
    expect(cascade.blocked.size).toBe(0);
  });

  it("sends the proposal fingerprints the backend issued", () => {
    expect(selectionRefs(items, new Set(["B@1"]))).toEqual([{ change_set_id: "B", version: 1, proposal_fingerprint: "fp-B" }]);
  });
});

describe("proposal review projection", () => {
  it("projects units, anchors and relations per Markdown snapshot", () => {
    const proposal = item("P");
    proposal.proposal.proposed_objects = [
      { local_ref: "s1", kind: "source_span", op: "create", payload: { artifact_id: "md-1", md_byte_range: { start: 0, end: 5 }, granularity: "inline", quote: "定理" } },
      { local_ref: "s2", kind: "source_span", op: "create", payload: { artifact_id: "md-2", md_byte_range: { start: 3, end: 9 }, granularity: "block", quote: "证明" } },
      { local_ref: "u1", kind: "source_unit", op: "create", payload: { kind: "statement", display_title: "定理 1", parent: null, source_spans: ["$new:s1"], completeness: "complete" } },
      { local_ref: "u2", kind: "source_unit", op: "create", payload: { kind: "proof", display_title: "证明", parent: "$new:u1", source_spans: ["$new:s2"] } },
    ];
    proposal.proposal.proposed_relations = [{ local_ref: "r1", kind: "source_relation", op: "create", relation: "proves", source: "$new:u2", target: "$new:u1", evidence_spans: ["$new:s2"] }];
    proposal.result.diagnostics = [{ level: "info", code: "span_already_exists", message: "已存在", local_ref: "s1" }];
    const groups = proposalReviewProjection(proposal);
    expect(groups.map(group => group.source.artifact_id)).toEqual(["md-1", "md-2"]);
    expect(groups[0].units[0].anchors[0].range).toEqual({ start: 0, end: 5 });
    expect(groups[0].diagnostics[0].code).toBe("span_already_exists");
    expect(groups[1].units[0].parent).toBe("u1");
    expect(groups[1].structure_relations).toEqual([{ relation_ref: "u2:parent", parent: "u1", child: "u2" }]);
    expect(groups[1].relations[0]).toMatchObject({ source: "u2", target: "u1", relation: "proves" });
  });

  it("opens a snapshot group in the discovery workspace shape, read-only", () => {
    const proposal = item("P");
    proposal.proposal.proposed_objects = [
      { local_ref: "s1", kind: "source_span", op: "create", payload: { artifact_id: "md-1", md_byte_range: { start: 0, end: 6 }, granularity: "inline", quote: "定理" } },
      { local_ref: "u1", kind: "source_unit", op: "create", payload: { kind: "statement", display_title: "定理 1", parent: null, source_spans: ["$new:s1"], completeness: "complete" } },
    ];
    const [group] = proposalReviewProjection(proposal);
    const outcome = proposalDiscoveryOutcome(proposal, group, "sha256:doc");
    expect(outcome.view.source).toMatchObject({ artifact_id: "md-1", content_hash: "sha256:doc", conversion_id: null });
    expect(outcome.view.units[0]).toMatchObject({ unit_ref: "u1", title: "定理 1", anchors: [{ start: 0, end: 6, excerpt: "定理" }] });
    expect(outcome.review?.acceptance.selected_units).toEqual([]);
    expect(outcome.review?.result_fingerprint).toBe("fp-P");
    expect(outcome.run.run_id).toBe("proposal:P@1:md-1");
    const blocked = proposalDiscoveryOutcome({ ...proposal, state: "blocked" }, group, "sha256:doc");
    expect(blocked.review?.units[0].acceptance).toBe("blocked");
  });

  it("summarizes the impact plan", () => {
    expect(impactSummary({ plan_fingerprint: "f", status: "ready", base_commit: null,
      direct_changes: [{ object_id: "a", kind: "SourceSpan", action: "create" }, { object_id: "b", kind: "SourceSemanticUnit", action: "revise" }],
      affected_objects: [], diagnostics: [] })).toBe("新建 1 · 修改 1");
    expect(impactSummary({ plan_fingerprint: "f", status: "blocked", base_commit: null, direct_changes: [], affected_objects: [],
      diagnostics: [{ code: "dangling_staged_ref", message: "x" }] })).toBe("阻塞：引用的对象不存在");
  });
});
