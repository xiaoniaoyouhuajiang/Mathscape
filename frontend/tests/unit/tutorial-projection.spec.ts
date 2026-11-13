import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { toCytoscapeElements } from "../../src/graph/cytoscape";
import { alignmentPlainLabel } from "../../src/semantic/labels";
import { TUTORIAL_PROJECTION_SCHEMA, tutorialGraph, type TutorialProjection } from "../../src/tutorial/projection";
import { inlineMathHtml, markdownHtml } from "../../src/tutorial/math";
import { MATHLIB_WIKI_URL, buildTutorialShots, debtAxisLine, debtsAttachedTo, fib, shotText } from "../../src/tutorial/script";
import { formatMergedUnitDetail, mergeLearningUnits } from "../../src/semantic/learning-view";

const projection = JSON.parse(
  readFileSync(resolve(__dirname, "../../../fixtures/emir/fibonacci-two-books/tutorial-projection.json"), "utf8"),
) as TutorialProjection;
const BANNED = ["平移", "错开", "滑动"];
/** 面向软件用户：不出现开发视角的说法和代号（书名「星港」里的“星”除外）。 */
const USER_BANNED = [/星(?!港)/, /甲/, /乙/, /虚构/, /示例材料/, /还没在本机核对/];

describe("tutorial projection (fixtures/emir/fibonacci-two-books)", () => {
  it("is the read-only projection of the six accepted change sets", () => {
    expect(projection.schema).toBe(TUTORIAL_PROJECTION_SCHEMA);
    expect(projection.label).toBe("两本数学书（教程夹具）");
    expect(projection.revisions?.length).toBe(2);
    const kinds = Object.values(projection.objects).map((o) => o.kind);
    expect(kinds.filter((k) => k === "alignment")).toHaveLength(9);
    expect(kinds.filter((k) => k === "debt")).toHaveLength(4);
    expect(kinds.filter((k) => k === "dialect_scope")).toHaveLength(2);
    expect(kinds.filter((k) => k === "semantic_entity")).toHaveLength(6);
  });

  it("keeps agent statuses and never raises trust on accept", () => {
    for (const object of Object.values(projection.objects)) {
      const p = object.payload as { trust?: { proof?: string; alignment?: string }; status?: string; mathlib_baseline?: { status?: string } | null };
      if (object.kind === "alignment") {
        expect(p.trust?.proof).toBe("none");
        expect(["retrieved_candidate", "context_checked", "partially_aligned"]).toContain(p.status);
      }
      if (object.kind === "debt") expect(p.status).toBe("open");
      if (object.kind === "semantic_entity" && p.mathlib_baseline) expect(p.mathlib_baseline.status).toBe("unchecked");
    }
  });

  it("labels every alignment exactly like the frontend template (Rust ↔ TS parity)", () => {
    const relations = projection.graph.nodes.filter((n) => n.element_kind === "relation" && n.domain_kind === "Alignment");
    expect(relations).toHaveLength(9);
    for (const node of relations) {
      const object = Object.values(projection.objects).find((o) => o.id === node.domain_id)!;
      expect(node.label).toBe(alignmentPlainLabel(object.payload));
    }
    const all = projection.graph.nodes.map((n) => `${n.label} ${n.summary ?? ""}`).join("\n");
    for (const word of BANNED) expect(all).not.toContain(word);
    expect(all).toContain("特例：灯塔的第 n 条 ＝ 共享的第 n+1 条");
    expect(all).toContain("部分重合：灯塔没有第 0 项");
  });

  it("shows users no developer wording: map labels, file names, learning view", () => {
    const visible = [
      ...projection.graph.nodes.map((n) => `${n.label} ${n.summary ?? ""}`),
      ...projection.learning.materials.flatMap((m) => [
        ...m.units.flatMap((u) => [u.unit_title, u.entity_label, ...u.only_differs_by]),
        ...m.new_conventions.flatMap((c) => [c.scope_label, c.text]),
        ...m.open_debts.map((d) => d.obligation),
      ]),
      ...Object.values(projection.objects).flatMap((o) => {
        const p = o.payload as Record<string, unknown>;
        return [p.plain_label, p.plain_note, p.map_label, p.label, p.obligation, p.failure_note, o.quote].filter((v): v is string => typeof v === "string");
      }),
    ].join("\n");
    for (const word of USER_BANNED) expect(visible).not.toMatch(word);
    const files = projection.graph.nodes.filter((n) => n.domain_kind === "SourceArtifact").map((n) => n.label).sort();
    expect(files).toEqual(["星港数列讲义.md", "灯塔组合数学初步.md"].sort());
  });

  it("renders on the main-map stylesheet without source spans", () => {
    const graph = tutorialGraph(projection);
    expect(graph.nodes.some((n) => n.domain_kind === "SourceSpan")).toBe(false);
    const elements = toCytoscapeElements(graph, "semantic");
    const types = new Set(elements.map((e) => e.data.nodeType).filter(Boolean));
    for (const type of ["shared_entity", "shared_theory", "debt", "dialect_scope", "semantic_unit"]) expect(types).toContain(type);
    expect(elements.some((e) => String(e.classes ?? "").includes("alignment-edge"))).toBe(true);
  });
});

describe("tutorial script", () => {
  const shots = buildTutorialShots(projection);

  it("has the thirteen storyboard shots, every reveal resolvable", () => {
    expect(shots.map((s) => s.id)).toEqual([
      "intro", "spans", "units", "same-concept", "declaration", "index-zero", "cassini", "matrix", "revision", "plain-labels", "debts", "accept", "recap",
    ]);
    expect(shots.reduce((sum, s) => sum + s.seconds, 0)).toBe(144);
    for (const shot of shots) {
      for (const local of [...shot.reveal, ...shot.focus]) {
        const resolved = local.startsWith("artifact:") ? projection.artifacts[local.slice(9)]
          : local.startsWith("derived:") ? projection.artifacts[local.slice(8)] : projection.refs[local];
        expect(resolved, `${shot.id}: ${local}`).toBeTruthy();
      }
    }
  });

  it("never uses the banned words or leaks raw data", () => {
    for (const shot of shots) {
      for (const line of shotText(shot)) {
        for (const word of BANNED) expect(line, shot.id).not.toContain(word);
        for (const word of USER_BANNED) expect(line, shot.id).not.toMatch(word);
        expect(line, shot.id).not.toMatch(/mathscape:|undefined|\[object/);
      }
    }
  });

  it("derives the numbers of shots 5 and 6 from the definition", () => {
    expect([0, 1, 2, 3, 4, 5, 6].map(fib)).toEqual([0, 1, 1, 2, 3, 5, 8]);
    const table = shots[5].extra!;
    expect(table.kind).toBe("sequence");
    if (table.kind === "sequence") expect(table.rows[1].cells[0]).toBeNull();
    const numbering = shots[6].extra!;
    expect(numbering.kind).toBe("numbering");
    if (numbering.kind === "numbering") {
      expect(numbering.shift).toBe(1);
      expect(numbering.sequenceNote).toBe("数列没有动：f_k = F_k");
      expect(numbering.shared.map((s) => s.text)).toEqual(["共享第 1 条 = -1", "共享第 2 条 = 1", "共享第 3 条 = -1", "共享第 4 条 = 1"]);
      expect(numbering.yi.map((s) => s.text)).toEqual(["灯塔第 1 条 = 1", "灯塔第 2 条 = -1", "灯塔第 3 条 = 1"]);
      expect(numbering.formulas.map((f) => f.tex)).toEqual(["F_{n+1}F_{n-1} - F_n^2 = (-1)^n", "f_n f_{n+2} - f_{n+1}^2 = (-1)^{n+1}"]);
      expect(numbering.explain.join("")).toContain("灯塔的第 n 条对应星港的第 n+1 条");
    }
  });

  it("introduces every learning label and type before the recap", () => {
    const firstTag = (tag: string) => shots.findIndex((s) => s.tags?.some((t) => t.tag === tag));
    expect(firstTag("别的材料里见过")).toBe(3);
    expect(firstTag("新约定")).toBe(shots.findIndex((s) => s.reveal.includes("ds.yi")));
    const accept = shots.find((s) => s.id === "accept")!;
    expect(accept.extra?.kind === "accept" && accept.extra.banner).toBe("接受只是认可这条对应，不会把可信度变成“已证明”");
    const types = shots.flatMap((s) => s.types.map((t) => t.code));
    for (const code of ["SourceArtifact", "SourceSemanticUnit", "concept", "declaration", "theory", "problem_schema", "Alignment", "DialectScope", "Debt", "expresses", "overlaps_with", "specializes"]) {
      expect(types, code).toContain(code);
    }
    const declaration = shots.find((s) => s.id === "declaration")!;
    expect(declaration.card.lines.join("\n")).toContain(`[mathlib](${MATHLIB_WIKI_URL})`);
    expect(declaration.card.lines.join("\n")).toContain("不代表哪本书更正统");
  });

  it("tells the revision story v1 → feedback → v2 from the fixture", () => {
    const revision = shots.find((s) => s.id === "revision")!.extra!;
    expect(revision.kind).toBe("revision");
    if (revision.kind === "revision") {
      expect(revision.v1.relation).toBe("definitionally_equivalent");
      expect(revision.v2.relation).toBe("overlaps_with");
      expect(revision.v2.plain).toBe("部分重合");
      expect(revision.feedback).toContain("灯塔没有第 0 项");
      expect(revision.v2.lost).toContain("F₀");
    }
  });

  it("ends with a recap of the learning labels of 灯塔", () => {
    const last = shots[12];
    const extra = last.extra!;
    expect(extra.kind).toBe("learning");
    if (extra.kind === "learning") {
      expect(extra.units.map((u) => u.title)).toEqual([
        "习题 2.4.7 剪拼悖论",
        "命题 2.4.2 Mⁿ⁺¹ 的矩阵形式",
        "命题 2.4.3 Cassini（灯塔的写法）",
        "定义 2.4.1 兔子数列（f₁=f₂=1，无第 0 项）",
      ]);
      const definition = extra.units.find((u) => u.title.includes("定义 2.4.1"))!;
      expect(definition.label).toBe("别的材料里见过");
      expect(definition.detail).toContain("见过：");
      expect(definition.detail).toContain("Fibonacci 数列");
      expect(definition.detail).toContain("只差：灯塔没有第 0 项");
      const exercise = extra.units.find((u) => u.title.includes("习题 2.4.7"))!;
      expect(exercise.label).toBe("新的");
      expect(exercise.note).toContain("习题 2.4.7 的提示");
      expect(exercise.focus).toEqual(expect.arrayContaining(["yi.u.ex", "yi.u.hint", "e.puzzle"]));
      expect(extra.units.filter((u) => u.note).map((u) => u.title)).toEqual(["习题 2.4.7 剪拼悖论"]);
      expect(extra.units.filter((u) => u.title.includes("定义 2.4.1"))).toHaveLength(1);
      expect(extra.summary).toBe("新的 1 · 别的材料里见过 3 · 新约定 2 · 未还清的账 4");
    }
    expect(last.focus).toEqual(expect.arrayContaining(["yi.u.def", "yi.u.lem", "yi.u.thm", "yi.u.ex", "yi.u.hint"]));
    expect(last.seen).toEqual(expect.arrayContaining(["yi.u.def", "yi.u.lem", "yi.u.thm"]));
    expect(last.fresh).toEqual(expect.arrayContaining(["yi.u.ex", "yi.u.hint"]));
    expect(last.accent).toEqual(expect.arrayContaining(["e.fib", "e.fib_def", "e.qpow", "e.cassini", "e.puzzle", "ds.yi", ...["d.yi.f_eq_F", "d.yi.lem.forward", "d.yi.thm.forward", "d.puzzle.all_k"]]));
    expect(last.legendNote).toBe("绿框＝别的材料里见过　黄框＝新的");
  });

  it("counts only the open debts attached to the accepted alignment", () => {
    const attached = debtsAttachedTo(projection.objects, projection.refs["a.yi.thm"]);
    expect(attached).toHaveLength(1);
    expect(attached[0].key).toBe("d.yi.thm.forward");
    expect(attached[0].shortLabel).toBe("前向证书");
    expect(debtAxisLine(attached)).toBe("这条线上的账：1 笔还欠着（前向证书）");
    expect(debtsAttachedTo(projection.objects, projection.refs["a.yi.def"]).map((d) => d.key)).toEqual(["d.yi.f_eq_F"]);
    const accept = shots.find((s) => s.id === "accept")!;
    expect(accept.focus).toEqual(expect.arrayContaining(["yi.u.thm", "e.cassini", "d.yi.thm.forward"]));
    expect(accept.narration).toContain("这条线上的那一笔账也还挂着");
    expect(accept.narration).not.toContain("那几笔账");
    if (accept.extra?.kind === "accept") {
      expect(accept.extra.button).toBe("接受这条对齐");
      expect(accept.extra.axes).toContain("这条线上的账：1 笔还欠着（前向证书）");
    }
  });
});

describe("tutorial text rendering", () => {
  it("renders Markdown emphasis and TeX instead of showing raw syntax", () => {
    const html = markdownHtml("**定义 2.4.1**　兔子数列 $(f_n)_{n \\ge 1}$ 由 $f_1 = f_2 = 1$ 给出。");
    expect(html).toContain("<strong>定义 2.4.1</strong>");
    expect(html).toContain("katex");
    expect(html).not.toContain("**");
    expect(html).not.toContain("$");
  });

  it("renders the materials quote of the fixture without raw markdown", () => {
    const quote = projection.objects["yi.s.def"].quote!;
    expect(quote).toContain("**定义 2.4.1**");
    const html = markdownHtml(quote);
    expect(html).toContain("<strong>定义 2.4.1</strong>");
    expect(html).not.toContain("**");
  });

  it("only turns https links into external links and escapes raw HTML", () => {
    const html = inlineMathHtml(`[mathlib](${MATHLIB_WIKI_URL}) <b>x</b> [bad](javascript:alert(1))`);
    expect(html).toContain('data-external="true"');
    expect(html).toContain('href="https://en.wikipedia.org/wiki/Lean_(proof_assistant)#Libraries"');
    expect(html).not.toContain("<b>");
    expect(html).not.toContain('href="javascript');
  });
});
