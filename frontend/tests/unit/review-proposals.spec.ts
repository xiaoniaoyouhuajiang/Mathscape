import { describe, expect, it } from "vitest";
import {
  debtLabel, debtView, defaultSelection, dependencyGroups, deselect, discardImpact, discardPreview, impactSummary, keyOf,
  proposalDiscoveryOutcome, proposalOverview, proposalReviewProjection, select, selectionRefs, targetDiagnosticGroups,
  type PendingChangeSet, type ProposalAnchor,
} from "../../src/review/proposal-model";

function anchor(spanRef: string, artifact: string, start: number, end: number, extra: Partial<ProposalAnchor> = {}): ProposalAnchor {
  return {
    span_ref: spanRef, span_id: `mathscape:source-span/${spanRef}`, origin: "new", artifact, conversion: `conv-${artifact}`,
    range: { start, end }, block_ids: [`block:${spanRef}`], page: { pdf_page_index: 2, page_number: 3 },
    granularity: "inline", excerpt: `text-${spanRef}`, verified: true, compiled: true, ...extra,
  };
}

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
  it("projects units, anchors and relations per Markdown snapshot from the server anchors", () => {
    const proposal = item("P");
    proposal.proposal.proposed_objects = [
      // The raw payload ranges are deliberately different: only server anchors count.
      { local_ref: "s1", kind: "source_span", op: "create", payload: { artifact_id: "md-1", md_byte_range: { start: 99, end: 100 }, granularity: "inline", quote: "raw" } },
      { local_ref: "s2", kind: "source_span", op: "create", payload: { artifact_id: "md-2", md_byte_range: { start: 3, end: 9 }, granularity: "block", quote: "证明" } },
      { local_ref: "u1", kind: "source_unit", op: "create", payload: { kind: "statement", display_title: "定理 1", parent: null, source_spans: ["$new:s1"], completeness: "complete" } },
      { local_ref: "u2", kind: "source_unit", op: "create", payload: { kind: "proof", display_title: "证明", parent: "$new:u1", source_spans: ["$new:s2"] } },
    ];
    proposal.proposal.proposed_relations = [{ local_ref: "r1", kind: "source_relation", op: "create", relation: "proves", source: "$new:u2", target: "$new:u1", evidence_spans: ["$new:s2"] }];
    proposal.result.diagnostics = [{ level: "info", code: "span_already_exists", message: "已存在", local_ref: "s1" }];
    proposal.anchors = [anchor("s1", "md-1", 0, 5), anchor("s2", "md-2", 3, 9)];
    const groups = proposalReviewProjection(proposal);
    expect(groups.map(group => group.source)).toEqual([
      { artifact_id: "md-1", conversion_id: "conv-md-1" }, { artifact_id: "md-2", conversion_id: "conv-md-2" },
    ]);
    expect(groups[0].units[0].anchors[0]).toEqual({ purpose: "unit", range: { start: 0, end: 5 }, block_ids: ["block:s1"], excerpt: "text-s1" });
    expect(groups[0].diagnostics[0].code).toBe("span_already_exists");
    expect(groups[1].units[0].parent).toBe("u1");
    expect(groups[1].structure_relations).toEqual([{ relation_ref: "u2:parent", parent: "u1", child: "u2" }]);
    expect(groups[1].relations[0]).toMatchObject({ source: "u2", target: "u1", relation: "proves" });
    expect(groups[1].relations[0].evidence[0].block_ids).toEqual(["block:s2"]);
  });

  it("resolves references to existing spans through the server anchors", () => {
    const proposal = item("P");
    const existing = "mathscape:source-span/old";
    proposal.proposal.proposed_objects = [
      { local_ref: "u1", kind: "source_unit", op: "create", payload: { kind: "statement", display_title: "引理", parent: null, source_spans: [existing] } },
      { local_ref: "u2", kind: "source_unit", op: "edit", target_id: "mathscape:source-unit/x", changes: { source_spans: { action: "set", value: [existing] } } },
    ];
    proposal.anchors = [anchor(existing, "md-9", 10, 20, { origin: "head", span_id: existing })];
    const [group] = proposalReviewProjection(proposal);
    expect(group.source.artifact_id).toBe("md-9");
    expect(group.units.map(unit => unit.anchors[0]?.range)).toEqual([{ start: 10, end: 20 }, { start: 10, end: 20 }]);
  });

  it("groups diagnostics without a local ref by the existing object they name", () => {
    const proposal = item("P");
    proposal.result.diagnostics = [
      { level: "warn", code: "head_integrity", message: "a", target_id: "mathscape:source-unit/b" },
      { level: "warn", code: "head_integrity", message: "b", target_id: "mathscape:source-unit/a" },
      { level: "warn", code: "head_integrity", message: "c", target_id: "mathscape:source-unit/a" },
      { level: "error", code: "dependency_blocked", message: "d" },
      { level: "error", code: "quote_mismatch", message: "e", local_ref: "s1" },
    ];
    proposal.targetTitles = { "mathscape:source-unit/a": "定理 A" };
    const groups = targetDiagnosticGroups(proposal);
    expect(groups.map(group => [group.targetId, group.title, group.diagnostics.length])).toEqual([
      ["mathscape:source-unit/a", "定理 A", 2], ["mathscape:source-unit/b", null, 1], [null, null, 1],
    ]);
  });

  it("shows structured Debt as text, never JSON", () => {
    const declared = debtView({ kind: "unresolved", value: { id: "chapter-pages-1-7", kind: "locator", summary: "缺少分页标记" } });
    expect(declared).toEqual({ origin: "未解决", tag: "locator", body: "缺少分页标记", id: "chapter-pages-1-7" });
    const nested = debtView({ kind: "agent_declared", value: { kind: "gap", detail: { page: 3 } } });
    expect(nested.body).toBe("detail：page：3");
    expect(debtLabel({ kind: "agent_declared", value: { a: [1, 2] } })).not.toMatch(/[{}[\]"]/);
    expect(debtView({ kind: "validation", code: "invalid_locator", message: "缺少物理定位", debt_id: "validation:invalid_locator" } as never))
      .toEqual({ origin: "校验", tag: "原文证据缺少物理定位", body: "缺少物理定位", id: "validation:invalid_locator" });
  });

  it("opens a snapshot group in the discovery workspace shape", () => {
    const proposal = item("P");
    proposal.proposal.proposed_objects = [
      { local_ref: "s1", kind: "source_span", op: "create", payload: { artifact_id: "md-1", md_byte_range: { start: 0, end: 6 }, granularity: "inline", quote: "定理" } },
      { local_ref: "u1", kind: "source_unit", op: "create", payload: { kind: "statement", display_title: "定理 1", parent: null, source_spans: ["$new:s1"], completeness: "complete" } },
    ];
    proposal.anchors = [anchor("s1", "md-1", 0, 6, { excerpt: "定理" })];
    const [group] = proposalReviewProjection(proposal);
    const outcome = proposalDiscoveryOutcome(proposal, group, "sha256:doc");
    expect(outcome.view.source).toMatchObject({ artifact_id: "md-1", content_hash: "sha256:doc", conversion_id: "conv-md-1" });
    expect(outcome.view.units[0]).toMatchObject({ unit_ref: "u1", title: "定理 1", anchors: [{ start: 0, end: 6, excerpt: "定理", block_ids: ["block:s1"] }] });
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

/** Three snapshots; relations inside one, across two, and to existing map objects. */
function multiSnapshotProposal(): PendingChangeSet {
  const proposal = item("M");
  const span = (ref: string, artifact: string) => ({ local_ref: ref, kind: "source_span", op: "create", payload: { artifact_id: artifact, md_byte_range: { start: 0, end: 4 }, granularity: "inline", quote: ref } });
  const unit = (ref: string, title: string, spanRef: string) => ({ local_ref: ref, kind: "source_unit", op: "create", payload: { kind: "statement", display_title: title, parent: null, source_spans: [`$new:${spanRef}`] } });
  const rel = (ref: string, relation: string, source: string, target: string, evidence: string[] = []) => ({ local_ref: ref, kind: "source_relation", op: "create", relation, source, target, evidence_spans: evidence });
  proposal.proposal.proposed_objects = [
    span("sa", "md-1"), span("sb", "md-1"), span("sc", "md-2"), span("sd", "md-3"), span("se", "md-3"),
    unit("a", "定义 1", "sa"), unit("b", "定理 2", "sb"), unit("c", "例 3", "sc"), unit("d", "引理 4", "sd"), unit("e", "推论 5", "se"),
  ];
  proposal.proposal.proposed_relations = [
    rel("r_ab", "explains", "$new:a", "$new:b", ["$new:sb"]),
    rel("r_bc", "applies_method", "$new:b", "$new:c", ["$new:sc"]),
    rel("r_ca", "cites", "$new:c", "$new:a"),
    rel("r_xd", "explains", "mathscape:source-unit/x", "$new:d", ["$new:sd"]),
    rel("r_ey", "applies_method", "$new:e", "mathscape:source-unit/y"),
  ];
  proposal.anchors = [anchor("sa", "md-1", 0, 4), anchor("sb", "md-1", 5, 9), anchor("sc", "md-2", 0, 4), anchor("sd", "md-3", 0, 4), anchor("se", "md-3", 5, 9)];
  proposal.result.debts = [{ kind: "unresolved", value: "页码待核" }];
  proposal.targetTitles = { "mathscape:source-unit/x": "已有定义 X" };
  return proposal;
}

describe("proposal overview across snapshots", () => {
  it("counts the whole change set, not the shown snapshot", () => {
    const overview = proposalOverview(multiSnapshotProposal());
    expect([overview.units, overview.relations, overview.spans, overview.debts]).toEqual([5, 5, 5, 1]);
    expect(overview.groups.map(group => [group.artifact, group.units, group.relations, group.crossRelations])).toEqual([
      ["md-1", 2, 3, 2], ["md-2", 1, 2, 2], ["md-3", 2, 2, 0],
    ]);
    expect(overview.defaultGroup).toBe(0);
    expect(overview.unitList.map(unit => [unit.ref, unit.group])).toEqual([["a", 0], ["b", 0], ["c", 1], ["d", 2], ["e", 2]]);
  });

  it("lists every relation with its ends, flagging cross-snapshot ones and existing objects", () => {
    const overview = proposalOverview(multiSnapshotProposal());
    expect(overview.relationList.map(r => [r.ref, r.source.title, r.target.title, r.crossSnapshot, r.source.origin, r.target.origin])).toEqual([
      ["r_ab", "定义 1", "定理 2", false, "new", "new"],
      ["r_bc", "定理 2", "例 3", true, "new", "new"],
      ["r_ca", "例 3", "定义 1", true, "new", "new"],
      ["r_xd", "已有定义 X", "引理 4", false, "existing", "new"],
      ["r_ey", "推论 5", "mathscape:source-unit/y", false, "new", "existing"],
    ]);
    expect(overview.groupOf("c")).toBe(1);
    expect(overview.groupOf("mathscape:source-unit/x")).toBeNull();
  });

  it("keeps cross-snapshot relations visible in both snapshots", () => {
    const groups = proposalReviewProjection(multiSnapshotProposal());
    expect(groups[0].relations.map(r => r.relation_ref)).toEqual(["r_ab", "r_bc", "r_ca"]);
    expect(groups[1].relations.map(r => r.relation_ref)).toEqual(["r_bc", "r_ca"]);
    // A relation from an existing object lives with its new target.
    expect(groups[2].relations.map(r => r.relation_ref)).toEqual(["r_xd", "r_ey"]);
    // Evidence is shown only against the snapshot it points into.
    expect(groups[0].relations[1].evidence).toEqual([]);
    expect(groups[1].relations[0].evidence[0].block_ids).toEqual(["block:sc"]);
  });

  it("defaults to the snapshot with the most units and relations", () => {
    const proposal = multiSnapshotProposal();
    proposal.proposal.proposed_relations = proposal.proposal.proposed_relations!.slice(3);
    expect(proposalOverview(proposal).defaultGroup).toBe(2);
  });

  it("projects semantic nodes, alignments, debts and referenced head spans without phantom null snapshots", () => {
    const proposal = item("S");
    proposal.proposal.proposed_objects = [
      { local_ref: "s1", kind: "source_span", op: "create", payload: { artifact_id: "md-1", md_byte_range: { start: 0, end: 5 }, granularity: "page", quote: "节标题" } },
      { local_ref: "sec2", kind: "source_unit", op: "create", payload: { kind: "section", display_title: "二、形式幂级数", parent: null, source_spans: ["$new:s1"] } },
      { local_ref: "t1", kind: "semantic_entity", op: "create", payload: { semantic_kind: "theory", plain_label: "理论：组合计数与母函数", preferred_labels: [{ lang: "zh", label: "组合计数与母函数" }] } },
      { local_ref: "d_thm1", kind: "semantic_entity", op: "create", payload: {
        semantic_kind: "declaration", home_theory: "$new:t1", plain_label: "精确说法：定理 1",
        preferred_labels: [{ lang: "zh", label: "定理 1" }],
        declaration: { declaration_kind: "theorem", statement: { text: "展开式", evidence_spans: ["mathscape:source-span/head3"] } },
      } },
      { local_ref: "scope2", kind: "dialect_scope", op: "create", payload: {
        label: "第二章约定", anchor_unit: "$new:sec2", applies_to: "anchor_subtree",
        conventions: [{ key: "terminology", subject: "相等", value: "coeff", text: "系数逐项相等", evidence_spans: ["$new:s1"] }],
      } },
      { local_ref: "a_thm1", kind: "alignment", op: "create", payload: {
        source: "mathscape:source-unit/head_thm1", target: "$new:d_thm1", relation: "expresses",
        source_context: { binding: "uncontextualized", reason: "跨快照" },
        evidence_spans: ["mathscape:source-span/head3"], status: "partially_aligned",
      } },
      { local_ref: "debt_1", kind: "debt", op: "create", payload: {
        debt_kind: "proof_debt", attached_to: "$new:a_thm1", status: "open",
        obligation: "核对 n=0", quantifier: "∀ n", failure_mode: "statement",
        evidence_spans: ["mathscape:source-span/head4"],
      } },
    ];
    proposal.anchors = [
      anchor("s1", "md-1", 0, 5, { page: { pdf_page_index: 7, page_number: 8 } }),
      anchor("mathscape:source-span/head3", "md-3", 10, 30, { origin: "head", span_id: "mathscape:source-span/head3", page: { pdf_page_index: 12, page_number: 13 } }),
      anchor("mathscape:source-span/head4", "md-4", 40, 60, { origin: "head", span_id: "mathscape:source-span/head4", page: { pdf_page_index: 32, page_number: 33 } }),
    ];
    proposal.result.diagnostics = [
      { level: "info", code: "uncontextualized", message: "没有约定卡", local_ref: "a_thm1" },
    ];
    proposal.targetTitles = { "mathscape:source-unit/head_thm1": "定理 1（原文单元）" };

    const groups = proposalReviewProjection(proposal);
    // Only the 3 real Markdown snapshots exist — no phantom `artifact_id: null` 4th group!
    expect(groups.map(g => g.source.artifact_id)).toEqual(["md-1", "md-3", "md-4"]);
    // Snapshots md-3 and md-4 (which have 0 new source_units) still populate units & relations from semantic objects!
    expect(groups[1].units.map(u => u.unit_ref)).toContain("d_thm1");
    expect(groups[1].units.find(u => u.unit_ref === "d_thm1")!.anchors[0]?.range).toEqual({ start: 10, end: 30 });
    expect(groups[1].relations.map(r => r.relation_ref)).toContain("a_thm1");
    expect(groups[2].units.map(u => u.unit_ref)).toContain("debt_1");
    expect(groups[2].units.find(u => u.unit_ref === "debt_1")!.anchors[0]?.range).toEqual({ start: 40, end: 60 });

    const overview = proposalOverview(proposal, groups);
    expect(overview.semanticNodes).toBe(4);
    expect(overview.relations).toBeGreaterThanOrEqual(4); // in_theory, scope_of, a_thm1, debt_on
    expect(overview.relationList.find(r => r.ref === "a_thm1")?.source.title).toBe("定理 1（原文单元）");
    expect(overview.groups.map(g => g.pages)).toEqual([{ from: 8, to: 8 }, { from: 13, to: 13 }, { from: 33, to: 33 }]);
  });
});

