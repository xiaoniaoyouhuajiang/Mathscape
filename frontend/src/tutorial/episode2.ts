/**
 * Tutorial Episode 2 (10 storyboard shots, read-only projection from
 * `fixtures/emir/generating-functions-two-books/tutorial-projection.json`).
 *
 * Pure function: derives every shot from the read-only projection plus
 * localized copy in `src/i18n/tutorial-ep2-copy.ts`. Never invokes Tauri
 * commands or mutates workspace state.
 */
import { hasMessage, t } from "../i18n";
import { EP2_META, EP2_SHOTS, EP2_TAG_NOTES, EP2_TYPE_NOTES } from "../i18n/tutorial-ep2-copy";
import { debtStatusLabel } from "../semantic/labels";
import { formatMergedUnitDetail, materialSummary, mergeLearningUnits } from "../semantic/learning-view";
import type { TutorialProjection } from "./projection";
import { graphLabel, objectOf, payloadOf } from "./projection";
import type { BindingComparePanel, BranchesPanel, SymbolTablePanel, TutorialShot } from "./script";
import { MATHLIB_WIKI_URL, TYPE_NOTES, debtsAttachedTo, relationNote, relationPlain, stripTypePrefix } from "./script";

export const TUTORIAL_EP2_TITLE = EP2_META.title;

const str = (value: unknown): string => (typeof value === "string" ? value : "");
const list = (value: unknown): Record<string, unknown>[] =>
  Array.isArray(value) ? (value as Record<string, unknown>[]) : [];
const record = (value: unknown): Record<string, unknown> =>
  value && typeof value === "object" ? (value as Record<string, unknown>) : {};

function localOf(projection: TutorialProjection, domainId: string): string | undefined {
  return Object.entries(projection.refs).find(([, id]) => id === domainId)?.[0];
}

function symbolRoleLabel(role: string): string {
  const key = `semantic.s3.symbolRole.${role}`;
  return hasMessage(key) ? t(key) : role;
}

function exclusionKindLabel(kind: string): string {
  const key = `semantic.s2.exclusion.${kind}`;
  return hasMessage(key) ? t(key) : kind;
}

/**
 * Pure finite geometric sum evaluator matching `e.geom_fin`:
 * switches into `constant_sum` when `x === 1` (where `(1 - x^N) / (1 - x)` is `0 / 0`).
 */
export function geomFiniteSum(
  n: number,
  x: number,
): { branchId: "fraction_form" | "constant_sum"; value: number; fractionDefined: boolean } {
  if (x === 1) {
    return { branchId: "constant_sum", value: n, fractionDefined: false };
  }
  return { branchId: "fraction_form", value: (1 - x ** n) / (1 - x), fractionDefined: true };
}

function buildSymbolsPanel(projection: TutorialProjection): SymbolTablePanel {
  const buildScope = (scopeLocal: string) => {
    const payload = payloadOf(projection, scopeLocal);
    return {
      scopeLocal,
      title: str(payload.label),
      conventions: list(payload.conventions).map((c) => str(c.text)),
      rows: list(payload.local_symbols).map((sym) => ({
        symbol: `$${str(sym.symbol)}$`,
        role: symbolRoleLabel(str(sym.role)),
        domain: str(record(sym.domain).text),
        meaning: str(sym.meaning),
      })),
    };
  };
  return {
    kind: "symbols",
    scopes: [buildScope("ds.jia"), buildScope("ds.yi")],
  };
}

function buildBindingsPanel(
  projection: TutorialProjection,
  coreLocal: string,
  jiaBinding: string,
  yiBinding: string,
): BindingComparePanel {
  const corePayload = payloadOf(projection, coreLocal);
  const operations = list(corePayload.operations).map((op) => {
    const sig = str(record(op.signature).text);
    const note = str(op.note);
    return note ? `\`${str(op.name)}\`（${note}）：${sig}` : `\`${str(op.name)}\`：${sig}`;
  });
  const buildItem = (bindingLocal: string, book: string) => {
    const p = payloadOf(projection, bindingLocal);
    const attached = debtsAttachedTo(projection.objects, projection.refs[bindingLocal]);
    const debt = attached[0]
      ? `${attached[0].obligation}（${debtStatusLabel(attached[0].status)}）`
      : undefined;
    return {
      bindingLocal,
      book,
      mapLabel: str(p.map_label),
      mappings: list(p.symbol_map).map((entry) => ({
        symbol: `$${str(entry.local_symbol)}$`,
        targetExpr: str(record(entry.target_expr).text),
        note: str(entry.note),
      })),
      adaptations: list(p.index_and_domain_adaptations).map((a) => str(a.text)),
      lost: list(p.lost_information).map((item) => str(item.short) || str(item.claim)),
      debt,
    };
  };
  return {
    kind: "bindings",
    coreTitle: stripTypePrefix(graphLabel(projection, coreLocal)),
    carrier: str(record(corePayload.carrier).text),
    operations,
    items: [
      buildItem(jiaBinding, EP2_META.jiaShort),
      buildItem(yiBinding, EP2_META.yiShort),
    ],
  };
}

function buildBranchesPanel(
  projection: TutorialProjection,
  entityLocal: string,
  withEvaluations: boolean,
): BranchesPanel {
  const decl = record(payloadOf(projection, entityLocal).declaration);
  const guardsById = new Map<string, { label: string; predicate: string }>();
  for (const g of list(decl.guards)) {
    guardsById.set(str(g.guard_id), {
      label: str(g.label),
      predicate: str(record(g.predicate).text),
    });
  }
  const branches = list(decl.branches).map((b) => {
    const guardIds = Array.isArray(b.guards) ? (b.guards as unknown[]).map(str) : [];
    const firstGuard = guardsById.get(guardIds[0] ?? "");
    return {
      branchId: str(b.branch_id),
      label: str(b.label),
      guardPredicate: firstGuard?.predicate ?? "",
      guardLabel: firstGuard?.label ?? "",
      conclusion: str(record(b.conclusion).text),
      isDegenerate: Boolean(b.is_degenerate),
    };
  });
  const exclusions = list(decl.exclusions).map((ex) => ({
    kindLabel: exclusionKindLabel(str(ex.kind)),
    caseText: str(record(ex.case).text),
    reason: str(ex.reason),
  }));
  const partitionNote = str(record(decl.guard_partition).note) || undefined;
  if (!withEvaluations) {
    return { kind: "branches", partitionNote, branches, exclusions };
  }
  const reg = geomFiniteSum(4, 2);
  const deg = geomFiniteSum(4, 1);
  return {
    kind: "branches",
    partitionNote,
    branches,
    exclusions,
    evaluations: [
      {
        label: EP2_SHOTS.branches.evalRegularLabel,
        formula: EP2_SHOTS.branches.evalRegularFormula,
        note: `${EP2_SHOTS.branches.evalRegularNote}（= ${reg.value}）`,
      },
      {
        label: EP2_SHOTS.branches.evalDegenerateLabel,
        formula: EP2_SHOTS.branches.evalDegenerateFormula,
        note: `${EP2_SHOTS.branches.evalDegenerateNote}（= ${deg.value}）`,
      },
    ],
  };
}

export function buildEpisode2Shots(projection: TutorialProjection): TutorialShot[] {
  const label = (local: string) => graphLabel(projection, local);
  const quote = (local: string) => str(objectOf(projection, local).quote);
  const jiaMarkdown = ["jia.s.sec", "jia.s.def", "jia.s.thm", "jia.s.cor"].map(quote).join("\n\n");
  const yiMarkdown = ["yi.s.sec", "yi.s.def", "yi.s.prop", "yi.s.rem"].map(quote).join("\n\n");

  const fpsBaseline = record(payloadOf(projection, "r.fps").mathlib_baseline);
  const jiaBranches = (payloadOf(projection, "a.jia.thm").target_branches as string[] | undefined) ?? [];
  const yiBranches = (payloadOf(projection, "a.yi.prop").target_branches as string[] | undefined) ?? [];
  const yiPropLost = str(list(payloadOf(projection, "a.yi.prop").lost_information)[0]?.short);

  const rev = record(projection.revisions?.[0]);
  const revDiff = record(list(rev.diff_v1_to_v2)[0]);
  const v1Rel = str(record(revDiff.v1).relation);
  const v2Rel = str(record(revDiff.v2).relation);
  const v2Lost = str(list(record(revDiff.v2).lost_information)[0]?.claim);

  const yiMaterial = projection.learning.materials.find((m) => m.artifact_id === projection.artifacts.yi);
  const real = EP2_META.provenanceReal;
  const derived = EP2_META.provenanceDerived;
  const demo = EP2_META.provenanceDemo;

  return [
    {
      id: "intro",
      title: EP2_SHOTS.intro.title,
      seconds: 11,
      provenance: [real],
      reveal: ["artifact:jia", "artifact:yi", "e.theory", "e.ogf_concept"],
      focus: ["artifact:jia", "artifact:yi", "e.theory", "e.ogf_concept"],
      caption: EP2_SHOTS.intro.caption,
      narration: EP2_SHOTS.intro.narration,
      card: { title: EP2_SHOTS.intro.cardTitle, lines: [...EP2_SHOTS.intro.lines] },
      types: [TYPE_NOTES.material, TYPE_NOTES.concept, TYPE_NOTES.theory],
    },
    {
      id: "spans",
      title: EP2_SHOTS.spans.title,
      seconds: 13,
      provenance: [real],
      reveal: [],
      focus: ["artifact:jia", "artifact:yi"],
      caption: EP2_SHOTS.spans.caption,
      narration: EP2_SHOTS.spans.narration,
      card: { title: EP2_SHOTS.spans.cardTitle, lines: [] },
      types: [TYPE_NOTES.span],
      extra: {
        kind: "materials",
        items: [
          { file: EP2_META.jiaFile, book: EP2_META.jiaBook, markdown: jiaMarkdown },
          { file: EP2_META.yiFile, book: EP2_META.yiBook, markdown: yiMarkdown },
        ],
      },
    },
    {
      id: "local-symbols",
      title: EP2_SHOTS.localSymbols.title,
      seconds: 12,
      provenance: [real],
      reveal: ["jia.u.sec", "jia.u.def", "ds.jia", "yi.u.sec", "yi.u.def", "ds.yi", "derived:jia", "derived:yi"],
      focus: ["ds.jia", "ds.yi"],
      accent: ["jia.u.def", "yi.u.def"],
      caption: EP2_SHOTS.localSymbols.caption,
      narration: EP2_SHOTS.localSymbols.narration,
      card: { title: EP2_SHOTS.localSymbols.cardTitle, lines: [...EP2_SHOTS.localSymbols.lines] },
      tags: [EP2_TAG_NOTES.newConvention],
      types: [TYPE_NOTES.unit, TYPE_NOTES.derived, TYPE_NOTES.dialect],
      extra: buildSymbolsPanel(projection),
    },
    {
      id: "rep-seq",
      title: EP2_SHOTS.repSeq.title,
      seconds: 12,
      provenance: [real],
      reveal: ["r.seq", "rb.jia.seq", "rb.yi.seq"],
      focus: ["r.seq"],
      accent: ["ds.jia", "ds.yi"],
      caption: EP2_SHOTS.repSeq.caption,
      narration: EP2_SHOTS.repSeq.narration,
      card: { title: EP2_SHOTS.repSeq.cardTitle, lines: [...EP2_SHOTS.repSeq.lines] },
      types: [EP2_TYPE_NOTES.representationCore, EP2_TYPE_NOTES.representationBinding],
      extra: buildBindingsPanel(projection, "r.seq", "rb.jia.seq", "rb.yi.seq"),
    },
    {
      id: "rep-fps",
      title: EP2_SHOTS.repFps.title,
      seconds: 13,
      provenance: [real],
      reveal: ["r.fps", "rb.jia.fps", "rb.yi.fps", "d.yi.analytic_radius"],
      focus: ["r.fps", "d.yi.analytic_radius"],
      accent: ["ds.jia", "ds.yi", "e.ogf_concept"],
      caption: EP2_SHOTS.repFps.caption,
      narration: EP2_SHOTS.repFps.narration,
      card: {
        title: EP2_SHOTS.repFps.cardTitle,
        lines: fpsBaseline.decl
          ? [EP2_SHOTS.repFps.mathlibLine.replace("{url}", MATHLIB_WIKI_URL)]
          : [],
      },
      types: [TYPE_NOTES.debt],
      extra: buildBindingsPanel(projection, "r.fps", "rb.jia.fps", "rb.yi.fps"),
    },
    {
      id: "branches",
      title: EP2_SHOTS.branches.title,
      seconds: 13,
      provenance: [real, derived],
      reveal: ["jia.u.thm", "yi.u.prop", "e.geom_fin"],
      focus: ["e.geom_fin"],
      accent: ["jia.u.thm", "yi.u.prop"],
      caption: EP2_SHOTS.branches.caption,
      narration: EP2_SHOTS.branches.narration,
      card: {
        title: EP2_SHOTS.branches.cardTitle,
        lines: [str(record(payloadOf(projection, "e.geom_fin").declaration).statement && record(record(payloadOf(projection, "e.geom_fin").declaration).statement).text)],
      },
      types: [TYPE_NOTES.declaration, EP2_TYPE_NOTES.branches],
      extra: buildBranchesPanel(projection, "e.geom_fin", true),
    },
    {
      id: "branch-align",
      title: EP2_SHOTS.branchAlign.title,
      seconds: 11,
      provenance: [real],
      reveal: ["a.jia.thm", "a.yi.prop"],
      focus: ["jia.u.thm", "yi.u.prop", "e.geom_fin"],
      caption: EP2_SHOTS.branchAlign.caption,
      narration: EP2_SHOTS.branchAlign.narration,
      card: {
        title: EP2_SHOTS.branchAlign.cardTitle,
        lines: [
          EP2_SHOTS.branchAlign.jiaCoverFmt.replace("{branches}", jiaBranches.join(", ")),
          EP2_SHOTS.branchAlign.yiCoverFmt
            .replace("{branches}", yiBranches.join(", "))
            .replace("{lost}", yiPropLost),
        ],
      },
      tags: [EP2_TAG_NOTES.seenElsewhere],
      types: [TYPE_NOTES.alignment, relationNote("expresses"), relationNote("specializes")],
    },
    {
      id: "revision",
      title: EP2_SHOTS.revision.title,
      seconds: 12,
      provenance: [real, demo],
      reveal: [],
      focus: ["yi.u.prop", "e.geom_fin"],
      caption: EP2_SHOTS.revision.caption,
      narration: EP2_SHOTS.revision.narration,
      card: { title: EP2_SHOTS.revision.cardTitle, lines: [] },
      types: [],
      extra: {
        kind: "revision",
        v1: {
          relation: v1Rel,
          plain: relationPlain(v1Rel),
          note: EP2_SHOTS.revision.v1Note,
        },
        feedback: str(rev.user_feedback),
        v2: {
          relation: v2Rel,
          plain: relationPlain(v2Rel),
          lost: v2Lost,
          note: EP2_SHOTS.revision.v2Note,
        },
        why: EP2_SHOTS.revision.why,
      },
    },
    {
      id: "exclusions",
      title: EP2_SHOTS.exclusions.title,
      seconds: 12,
      provenance: [real],
      reveal: ["jia.u.cor", "jia.r.cor_explains", "yi.u.rem", "yi.r.rem_explains", "e.geom_inf_fps", "a.jia.cor", "a.yi.rem"],
      focus: ["e.geom_inf_fps"],
      accent: ["jia.u.cor", "yi.u.rem", "d.yi.analytic_radius"],
      caption: EP2_SHOTS.exclusions.caption,
      narration: EP2_SHOTS.exclusions.narration,
      card: {
        title: EP2_SHOTS.exclusions.cardTitle,
        lines: [
          str(record(record(payloadOf(projection, "e.geom_inf_fps").declaration).statement).text),
          EP2_SHOTS.exclusions.jiaCorLine,
          EP2_SHOTS.exclusions.yiRemLine,
        ],
      },
      types: [TYPE_NOTES.relation, relationNote("overlaps_with"), EP2_TYPE_NOTES.exclusions],
      extra: buildBranchesPanel(projection, "e.geom_inf_fps", false),
    },
    (() => {
      const merged = mergeLearningUnits(yiMaterial?.units ?? []);
      const recapUnits = merged.map((unit) => {
        const unitLocal = localOf(projection, unit.unit_id);
        const focus = [
          unitLocal,
          ...unit.targets.flatMap((target) => [localOf(projection, target.entity_id), localOf(projection, target.alignment_id)]),
        ].filter((local): local is string => Boolean(local));
        return {
          title: unit.unit_title,
          label: unit.label,
          detail: formatMergedUnitDetail(unit, (s) => stripTypePrefix(label(localOf(projection, s) ?? s))),
          focus,
        };
      });
      const seenUnits = recapUnits.flatMap((u) => u.focus.filter((local) => local.startsWith("yi.u.")));
      const conventionLocals = [...new Set((yiMaterial?.new_conventions ?? []).map((c) => localOf(projection, c.scope_id)).filter((local): local is string => Boolean(local)))];
      const debtLocals = (yiMaterial?.open_debts ?? []).map((debt) => localOf(projection, debt.debt_id)).filter((local): local is string => Boolean(local));
      return {
        id: "recap",
        title: EP2_SHOTS.recap.title,
        seconds: 12,
        provenance: [derived],
        reveal: [],
        focus: ["yi.u.def", "yi.u.prop", "yi.u.rem"],
        seen: seenUnits,
        accent: [
          "r.seq",
          "r.fps",
          "e.ogf_concept",
          "e.geom_fin",
          "e.geom_inf_fps",
          ...conventionLocals,
          ...debtLocals,
          "a.yi.prop",
          "a.yi.rem",
          "rb.yi.seq",
          "rb.yi.fps",
        ],
        legendNote: EP2_META.recapLegendNote,
        caption: EP2_SHOTS.recap.caption,
        narration: EP2_SHOTS.recap.narration,
        card: { title: EP2_SHOTS.recap.cardTitle, lines: [] },
        types: [],
        extra: {
          kind: "learning" as const,
          units: recapUnits,
          conventions: (yiMaterial?.new_conventions ?? []).map((c) => ({
            text: c.text,
            focus: [localOf(projection, c.scope_id)].filter((local): local is string => Boolean(local)),
          })),
          summary: yiMaterial ? materialSummary(yiMaterial) : "",
          summaryFocus: debtLocals,
          footerNote: t("tutorial.extra.stage3Preview"),
        },
      };
    })(),
  ];
}
