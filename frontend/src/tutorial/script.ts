/**
 * 教程「两本书讲的是不是同一个东西？」的 13 镜（分镜见
 * fixtures/emir/fibonacci-two-books/tutorial-storyboard.md）。
 *
 * 纯函数：每一镜的文字都从只读投影里读（真实），或由投影里的对象算出来
 * （派生）；只有“反馈给 agent”按钮和“点接受”两处是演示。全片不出现
 * “平移 / 错开 / 滑动”这类说法：动的只是恒等式的编号 n（第几条）。
 *
 * 面向软件用户：只用地图上真实的节点类型和说法；两本书用书名，简称
 * 「星港」「灯塔」。卡片文字是 Markdown + TeX（`$…$`），由播放器渲染。
 */
import { debtStatusLabel, relationLabel } from "../semantic/labels";
import { formatMergedUnitDetail, materialSummary, mergeLearningUnits, stripEntityTypePrefix } from "../semantic/learning-view";
import type { TutorialProjection } from "./projection";
import { graphLabel, payloadOf } from "./projection";

export type Provenance = "真实" | "派生" | "演示";

/** mathlib 的维基百科说明：维基百科没有单独的 mathlib 词条（Mathlib 重定向到这一节）。 */
export const MATHLIB_WIKI_URL = "https://en.wikipedia.org/wiki/Lean_(proof_assistant)#Libraries";

/** 说明页里的一条类型说明：类型名 + 系统里的类型代码 + 白话含义。 */
export interface TypeNote {
  name: string;
  code: string;
  plain: string;
}

/** 学习标签第一次出现时的说明。 */
export interface TagNote {
  tag: string;
  text: string;
}

/** 镜 2：两份材料里的原文片段（Markdown + TeX 原文，播放器渲染）。 */
export interface MaterialsPanel {
  kind: "materials";
  items: Array<{ file: string; book: string; markdown: string }>;
}

/** 镜 6：按项下标 k 对齐的数表。 */
export interface SequenceTable {
  kind: "sequence";
  ks: number[];
  rows: Array<{ label: string; cells: Array<number | null> }>;
  note: string;
}

/** 镜 7：两条 Cassini 公式 + “第几条”编号牌对照（数列行不动，只有灯塔的编号牌右移一格）。 */
export interface NumberingTable {
  kind: "numbering";
  formulas: Array<{ book: string; range: string; tex: string }>;
  explain: string[];
  sequenceNote: string;
  shared: Array<{ n: number; text: string }>;
  yi: Array<{ n: number; text: string; matches: number }>;
  missing: { n: number; text: string; note: string };
  shift: number;
}

/** 镜 9：对齐线的修订：v1 → 反馈给 agent → v2。 */
export interface RevisionPanel {
  kind: "revision";
  v1: { relation: string; plain: string; note: string };
  feedback: string;
  v2: { relation: string; plain: string; lost: string; note: string };
  why: string;
}

/** 镜 12：接受 ≠ 证明。 */
export interface AcceptPanel {
  kind: "accept";
  banner: string;
  badge: string;
  axes: string[];
  button: string;
}

/** 镜 13：回顾：读灯塔时的学习视图。 */
export interface LearningPanel {
  kind: "learning";
  units: Array<{ title: string; label: string; detail: string; note?: string; focus: string[] }>;
  conventions: Array<{ text: string; focus: string[] }>;
  summary: string;
  summaryFocus: string[];
}

export type ShotExtra = MaterialsPanel | SequenceTable | NumberingTable | RevisionPanel | AcceptPanel | LearningPanel;

export interface TutorialShot {
  id: string;
  title: string;
  seconds: number;
  provenance: Provenance[];
  /** 本镜新出现在地图上的对象（夹具 local_ref，或 artifact:<key>、derived:<key>）。 */
  reveal: string[];
  /** 本镜高亮的对象。 */
  focus: string[];
  /** 回顾镜：别的材料里见过的内容单元，绿色描边。 */
  seen?: string[];
  /** 回顾镜：新的内容单元，黄色描边。 */
  fresh?: string[];
  /** 一并点亮并收入镜头、但不展开成长标签的对象（约定卡、账、见过的共享对象）。 */
  accent?: string[];
  /** 本镜图例下的补充说明（例如回顾镜的绿/黄框）。 */
  legendNote?: string;
  caption: string;
  narration: string;
  card: { title: string; lines: string[] };
  /** 说明页：本镜第一次出现（或重点讲）的类型。 */
  types: TypeNote[];
  /** 本镜第一次出现的学习标签。 */
  tags?: TagNote[];
  extra?: ShotExtra;
}

const str = (value: unknown): string => (typeof value === "string" ? value : "");
const list = (value: unknown): Record<string, unknown>[] =>
  Array.isArray(value) ? (value as Record<string, unknown>[]) : [];
const record = (value: unknown): Record<string, unknown> =>
  value && typeof value === "object" ? (value as Record<string, unknown>) : {};

/** F_k（F₀ = 0, F₁ = 1）。 */
export function fib(k: number): number {
  let [a, b] = [0, 1];
  for (let i = 0; i < k; i++) [a, b] = [b, a + b];
  return a;
}

const cassiniShared = (n: number) => fib(n + 1) * fib(n - 1) - fib(n) ** 2;
const cassiniYi = (n: number) => fib(n) * fib(n + 2) - fib(n + 1) ** 2;

/** 对齐关系的白话：地图上那三种用同一份模板（说的就是它 / 特例 / 部分重合），其余用对照表。 */
export function relationPlain(relation: string): string {
  switch (relation) {
    case "expresses": return "说的就是它";
    case "specializes": return "特例";
    case "generalizes": return "推广";
    case "overlaps_with": return "部分重合";
    default: return relationLabel(relation);
  }
}

/** 地图上的节点类型名（说明页、地图标注、图例共用）。 */
export const TYPE_NOTES = {
  material: { name: "Markdown 材料", code: "SourceArtifact", plain: "导入地图的一份材料文件；这里是两本书各一节的 Markdown 正文。" },
  span: { name: "原文片段", code: "SourceSpan", plain: "书上的某一段原文。地图按位置记下它，别的对象都靠它指回原文。" },
  unit: { name: "内容单元", code: "SourceSemanticUnit", plain: "书里的一条定义、定理、习题或证明；永远连着它的原文。" },
  derived: { name: "派生（虚线）", code: "derived_from", plain: "这条内容单元是从哪份材料里切出来的。" },
  relation: { name: "书内关系", code: "SourceUnitRelation", plain: "同一本书里“这段证明那条”“这条提示那题”这类关系，箭头表示方向。" },
  concept: { name: "概念", code: "concept", plain: "大家都在讲的那个数学东西本身，不绑定哪本书的写法。" },
  alignment: { name: "对齐线", code: "Alignment", plain: "把书里的一条内容连到共享对象上，并写清是什么关系、哪里对不上。线上写关系类型和它的白话。" },
  declaration: {
    name: "精确说法", code: "declaration",
    plain: "把概念写成一句带前提、带下标范围的话。它由四部分组成：约束变量（binders，例如 $n : \\mathbb N$）、定义体或陈述（definiens / statement）、原文证据（evidence spans，指回书上的原句）和约定（conventions，例如从第 0 项起编）。",
  },
  theory: { name: "理论", code: "theory", plain: "共享对象所在的那一片数学（home theory），像一个容器；这里的概念、精确说法和题型都挂在「线性递推」下面。" },
  dialect: { name: "约定卡", code: "DialectScope", plain: "一本书在某一节里的写法约定，比如下标从几开始、把什么叫什么。地图读得懂它，并用它解释两本书的差别。" },
  problem: { name: "题型", code: "problem_schema", plain: "一族带参数的题。剪拼题是参数 $k \\ge 4$ 的一族，动画里的 8×8 是 $k = 6$ 那一张。" },
  debt: { name: "一笔账", code: "Debt", plain: "还欠的一个证明或检查，挂在具体的线或对象上；只有你能确认它还清了。" },
  trust: { name: "可信度", code: "TrustRecord", plain: "分几项记录（对齐、证明、整理……），不是一个分数；接受不会改变它。" },
} satisfies Record<string, TypeNote>;

const RELATION_MEANING: Record<string, string> = {
  expresses: "书里这段话讲的就是这个共享对象。",
  overlaps_with: "两边只有一部分重合；线上写清哪里对不上。",
  specializes: "书里这条是共享对象的一个特例；线上写清参数怎么对应、缺了哪部分。",
  definitionally_equivalent: "两边的定义展开后一模一样。",
  logically_equivalent: "两边可以互相推出。",
};

/** 说明页里的对齐关系类型：类型代码 + 地图上的白话 + 一句解释。 */
const relationNote = (relation: string): TypeNote => ({
  name: `对齐关系「${relationPlain(relation)}」`, code: relation,
  plain: RELATION_MEANING[relation] ?? relationPlain(relation),
});

function convention(projection: TutorialProjection, scope: string, key: string): string {
  const found = list(payloadOf(projection, scope).conventions).find((c) => c.key === key);
  return str(found?.text);
}

function lost(projection: TutorialProjection, alignment: string): { short: string; claim: string } {
  const first = list(payloadOf(projection, alignment).lost_information)[0] ?? {};
  return { short: str(first.short), claim: str(first.claim) };
}

function shiftOf(projection: TutorialProjection, alignment: string): number {
  // 灯塔的第 n 条 ＝ 共享的第 n+1 条：编号牌右移的格数。
  const map = list(payloadOf(projection, alignment).parameter_map)[0];
  const match = /第\s*n\s*\+\s*(\d+)\s*条/.exec(str(map?.map));
  return match ? Number(match[1]) : 0;
}

/** 去掉标签前的类型前缀（“概念：”“精确说法：”……），地图上类型另起一行标注。 */
export function stripTypePrefix(label: string): string {
  return stripEntityTypePrefix(label);
}

/** 挂在某条对齐线上、状态为还欠着的账。shortLabel 取义务原文在第一个冒号前的短名。 */
export function debtsAttachedTo(
  objects: Record<string, { payload: Record<string, unknown> }>,
  alignmentId: string,
): Array<{ key: string; obligation: string; status: string; shortLabel: string }> {
  return Object.entries(objects).flatMap(([key, object]) => {
    if (object.payload.attached_to !== alignmentId) return [];
    const obligation = typeof object.payload.obligation === "string" ? object.payload.obligation : "";
    const status = typeof object.payload.status === "string" ? object.payload.status : "";
    if (status && status !== "open") return [];
    const colon = obligation.indexOf("：");
    const shortLabel = (colon >= 0 ? obligation.slice(0, colon) : obligation).trim();
    return [{ key, obligation, status, shortLabel }];
  });
}

/** 接受面板上“这条线上的账”这一行，由挂在该对齐线上的未还清账推导，不写死笔数。 */
export function debtAxisLine(debts: Array<{ shortLabel: string }>): string {
  if (!debts.length) return "这条线上的账：没有还欠着的";
  const detail = debts.map((debt) => debt.shortLabel).filter(Boolean).join("、");
  return `这条线上的账：${debts.length} 笔还欠着${detail ? `（${detail}）` : ""}`;
}

function localOf(projection: TutorialProjection, id: string): string | undefined {
  return Object.entries(projection.refs).find(([, value]) => value === id)?.[0];
}

function hintLocalFor(projection: TutorialProjection, unitLocal: string | undefined): string | undefined {
  if (!unitLocal?.endsWith(".ex")) return undefined;
  const hint = unitLocal.replace(/\.ex$/, ".hint");
  return projection.refs[hint] ? hint : undefined;
}

export function buildTutorialShots(projection: TutorialProjection): TutorialShot[] {
  const label = (local: string) => graphLabel(projection, local);
  const fibDef = payloadOf(projection, "e.fib_def");
  const baseline = record(fibDef.mathlib_baseline);
  const declaration = record(fibDef.declaration);
  const binder = list(declaration.binders)[0] ?? {};
  const definiens = str(record(declaration.definiens).text);
  const yiDefLost = lost(projection, "a.yi.def");
  const yiThmLost = lost(projection, "a.yi.thm");
  const shift = shiftOf(projection, "a.yi.thm");
  const jiaArtifact = projection.artifacts.jia;
  const yiArtifact = projection.artifacts.yi;
  const fileOf = (artifact: string) =>
    projection.graph.nodes.find((node) => node.domain_id === artifact && node.element_kind === "entity")?.label ?? "";
  const jiaFile = fileOf(jiaArtifact);
  const yiFile = fileOf(yiArtifact);
  const yiMaterial = projection.learning.materials.find((m) => m.artifact_id === yiArtifact);
  const seenFib = yiMaterial?.units.find((unit) => unit.entity_id === projection.refs["e.fib"]);
  const debts = ["d.yi.f_eq_F", "d.yi.lem.forward", "d.yi.thm.forward", "d.puzzle.all_k"];
  const lineDebts = debtsAttachedTo(projection.objects, projection.refs["a.yi.thm"]);
  const trust = record(payloadOf(projection, "a.yi.thm").trust);
  const quote = (local: string) => str(projection.objects[local]?.quote);
  const firstDebt = payloadOf(projection, "d.yi.f_eq_F");
  const relationOf = (local: string) => str(payloadOf(projection, local).relation);
  const revision = (projection.revisions ?? [])
    .flatMap((entry) => list(entry.diff_v1_to_v2).map((diff) => ({ entry, diff })))
    .find(({ diff }) => diff.local_ref === "a.yi.def");
  const v1Relation = str(record(revision?.diff.v1).relation);
  const v2 = record(revision?.diff.v2);
  const puzzle = record(payloadOf(projection, "e.puzzle").problem_schema);
  const instance = list(puzzle.known_instances)[0] ?? {};

  const sources = [
    "jia.u.sec", "jia.u.def", "jia.u.lem", "jia.u.lem_pf", "jia.u.thm", "jia.u.thm_pf", "jia.u.rem",
    "yi.u.sec", "yi.u.def", "yi.u.lem", "yi.u.thm", "yi.u.ex", "yi.u.hint",
    "jia.r.lem_proof", "jia.r.thm_proof", "jia.r.thm_cites_lem", "jia.r.rem_explains", "yi.r.hint", "yi.r.hint_cites_thm",
  ];

  return [
    {
      id: "intro", title: "两本书", seconds: 8, provenance: ["真实"],
      reveal: ["artifact:jia", "artifact:yi", "e.fib"], focus: [],
      caption: "两本书讲的是不是同一个东西？",
      narration: "假设现在数学地图上有两本数学书：一本从 F₀ 等于 0 开始，一本从 F₁ 等于 1 开始。它们讲的是不是同一个东西？我们在地图上看一看。",
      card: { title: "地图上的两本书", lines: [
        `概念节点：${stripTypePrefix(label("e.fib"))}——两本书都在讲它，所以它是两本书共享的节点`,
        `两本书：《星港数列讲义》（\`${jiaFile}\`）和《灯塔组合数学初步》（\`${yiFile}\`）`,
      ] },
      types: [TYPE_NOTES.material, TYPE_NOTES.concept],
    },
    {
      id: "spans", title: "两段原文", seconds: 10, provenance: ["真实"],
      reveal: [], focus: ["artifact:jia", "artifact:yi"],
      caption: "原文片段：书上这一段",
      narration: "左边是《星港数列讲义》，右边是《灯塔组合数学初步》。地图记下的第一件事，永远是“书上这一段在哪儿”。",
      card: { title: "两本书里的定义", lines: [] },
      types: [TYPE_NOTES.span],
      extra: { kind: "materials", items: [
        { file: jiaFile, book: "《星港数列讲义》", markdown: quote("jia.s.def") },
        { file: yiFile, book: "《灯塔组合数学初步》", markdown: quote("yi.s.def") },
      ] },
    },
    {
      id: "units", title: "内容单元", seconds: 10, provenance: ["真实", "派生"],
      reveal: [...sources, "derived:jia", "derived:yi"], focus: ["jia.u.def", "yi.u.def", "artifact:jia", "artifact:yi"],
      caption: "内容单元：书里的一条定义、定理或习题",
      narration: "每段话变成一个内容单元：一条定义、一条定理、一道习题。两条虚线标出两条定义各自来自哪本书。",
      card: { title: "内容单元", lines: [
        `${jiaFile} ⇢ ${label("jia.u.def")}`,
        `${yiFile} ⇢ ${label("yi.u.def")}`,
        `其他内容单元：${label("jia.u.thm")}、${label("yi.u.ex")}……`,
      ] },
      types: [TYPE_NOTES.unit, TYPE_NOTES.derived, TYPE_NOTES.relation],
    },
    {
      id: "same-concept", title: "指向同一个概念", seconds: 12, provenance: ["真实", "派生"],
      reveal: ["a.jia.def.fib", "a.yi.def.fib"], focus: ["e.fib", "jia.u.def", "yi.u.def"],
      caption: `对齐线：${relationOf("a.yi.def.fib")} · ${relationPlain(relationOf("a.yi.def.fib"))}`,
      narration: "一本叫它 Fibonacci 数列，一本叫它兔子数列。名字不同，讲的是同一个概念。读灯塔时，地图会告诉你：这个概念在星港里已经见过。",
      card: { title: "对齐线", lines: [
        `星港的定义 → ${stripTypePrefix(label("e.fib"))}：\`${relationOf("a.jia.def.fib")}\`（${label("a.jia.def.fib")}）`,
        `灯塔的定义 → ${stripTypePrefix(label("e.fib"))}：\`${relationOf("a.yi.def.fib")}\`（${label("a.yi.def.fib")}）`,
      ] },
      types: [TYPE_NOTES.alignment, relationNote(relationOf("a.yi.def.fib"))],
      tags: [{
        tag: seenFib?.label ?? "别的材料里见过",
        text: "根据对齐线自动算出：灯塔的定义 2.4.1 连到的概念，在《星港数列讲义》里也连着。不用你手动标。",
      }],
    },
    {
      id: "declaration", title: "精确说法与理论", seconds: 12, provenance: ["真实"],
      reveal: ["e.theory", "e.fib_def", "a.jia.def"], focus: ["e.fib_def", "e.theory", "jia.u.def"],
      caption: "精确说法：把概念写成一句带前提的话",
      narration: "再往下一层，是精确说法：带约束变量、定义体和原文证据的那一句。它属于“线性递推”这个理论。地图用 mathlib 的统一写法作参照基线。",
      card: { title: label("e.fib_def"), lines: [
        `约束变量：$${str(binder.name)} : ${str(binder.type) === "ℕ" ? "\\mathbb N" : str(binder.type)}$`,
        `定义体：${definiens}`,
        `原文证据：星港的定义 3.2.1（\`${relationOf("a.jia.def")}\`：${label("a.jia.def")}）`,
        `所在理论：${stripTypePrefix(label("e.theory"))}`,
        `参照基线：[mathlib](${MATHLIB_WIKI_URL}) \`${str(baseline.decl)}\``,
        `前提：「精确说法」采用的是从 [mathlib](${MATHLIB_WIKI_URL}) 提取的统一约定，作为参照基线；这不代表哪本书更正统。`,
      ] },
      types: [TYPE_NOTES.declaration, TYPE_NOTES.theory],
    },
    {
      id: "index-zero", title: "但不是同一句话：第 0 项", seconds: 12, provenance: ["真实", "派生"],
      reveal: ["a.yi.def", "ds.jia", "ds.yi"], focus: ["e.fib_def", "yi.u.def", "ds.yi"],
      caption: `部分重合：$f_k = F_k$，但灯塔没有第 0 项`,
      narration: "从第 1 项起，两本书每一项都一样，列对列。差别只有一格：灯塔不用第 0 项。这条约定写在灯塔的约定卡上，地图读得懂它。",
      card: { title: "灯塔的定义 → 精确说法", lines: [
        `\`${relationOf("a.yi.def")}\`：${label("a.yi.def")}`,
        `灯塔的约定卡：${convention(projection, "ds.yi", "sequence_index_origin")}；${convention(projection, "ds.yi", "terminology")}`,
      ] },
      types: [TYPE_NOTES.dialect, relationNote(relationOf("a.yi.def"))],
      tags: [{ tag: "新约定", text: "灯塔约定卡上的这两条写法约定，在别的材料里没有。读灯塔时，学习视图把它们标成「新约定」，提醒你换了写法。" }],
      extra: {
        kind: "sequence", ks: [0, 1, 2, 3, 4, 5, 6],
        rows: [
          { label: "星港 F_k", cells: [0, 1, 2, 3, 4, 5, 6].map(fib) },
          { label: "灯塔 f_k", cells: [0, 1, 2, 3, 4, 5, 6].map((k) => (k === 0 ? null : fib(k))) },
        ],
        note: `从 k = 1 起每一列都相同；灯塔的 k = 0 是空格（${yiDefLost.short}）`,
      },
    },
    {
      id: "cassini", title: "Cassini：是第几条对第几条", seconds: 18, provenance: ["真实", "派生"],
      reveal: ["e.cassini", "a.jia.thm", "a.yi.thm"], focus: ["jia.u.thm", "yi.u.thm", "jia.u.rem", "e.cassini"],
      caption: `\`${relationOf("a.yi.thm")}\` · ${label("a.yi.thm")}`,
      narration: "Cassini 恒等式在两本书里长得不一样。但数列一项都没动，动的是恒等式的编号 n：灯塔的第 1 条，就是共享的第 2 条；灯塔的第 2 条，就是共享的第 3 条。所以右边的符号也跟着变。灯塔只少了共享的第 1 条，而那一条恰好要用到第 0 项。所以灯塔写的是一个特例。",
      card: { title: label("e.cassini"), lines: [
        `星港 → \`${relationOf("a.jia.thm")}\`（${label("a.jia.thm")}）`,
        `灯塔 → \`${relationOf("a.yi.thm")}\`（${label("a.yi.thm")}）`,
        `对不上的部分：${yiThmLost.short}`,
      ] },
      types: [relationNote(relationOf("a.yi.thm"))],
      extra: {
        kind: "numbering",
        formulas: [
          { book: "星港", range: "从第 0 项起编，$n \\ge 1$", tex: "F_{n+1}F_{n-1} - F_n^2 = (-1)^n" },
          { book: "灯塔", range: "从第 1 项起编，$n \\ge 1$", tex: "f_n f_{n+2} - f_{n+1}^2 = (-1)^{n+1}" },
        ],
        explain: [
          "两本书的数列逐项相同：$f_k = F_k$。",
          "不同的只是恒等式里的编号 n：灯塔的第 n 条对应星港的第 n+1 条。把星港式子里的 n 换成 n+1，就得到 $F_{n+2}F_n - F_{n+1}^2 = (-1)^{n+1}$，正是灯塔的写法。",
          "所以右边的符号从 $(-1)^n$ 变成了 $(-1)^{n+1}$。",
        ],
        sequenceNote: "数列没有动：f_k = F_k", shift,
        shared: [1, 2, 3, 4].map((n) => ({ n, text: `共享第 ${n} 条 = ${cassiniShared(n)}` })),
        yi: [1, 2, 3].map((n) => ({ n, text: `灯塔第 ${n} 条 = ${cassiniYi(n)}`, matches: n + shift })),
        missing: { n: 1, text: `共享第 1 条：$F_2F_0 - F_1^2 = ${cassiniShared(1)}$`, note: "灯塔写不出来：要用 F₀（就是星港的注 3.2.4）" },
      },
    },
    {
      id: "matrix", title: "矩阵也一样", seconds: 6, provenance: ["真实"],
      reveal: ["e.qpow", "a.jia.lem", "a.yi.lem"], focus: ["e.qpow", "jia.u.lem", "yi.u.lem"],
      caption: "矩阵公式：同样是第几条对第几条",
      narration: "矩阵那条也一样：动的是第几条，不是数列。",
      card: { title: label("e.qpow"), lines: [
        `星港 → \`${relationOf("a.jia.lem")}\`（${label("a.jia.lem")}）`,
        `灯塔 → \`${relationOf("a.yi.lem")}\`（${label("a.yi.lem")}）`,
      ] },
      types: [],
    },
    {
      id: "revision", title: "连错了怎么办：修订", seconds: 14, provenance: ["真实", "演示"],
      reveal: [], focus: ["yi.u.def", "e.fib_def"],
      caption: "对齐线可以修订：v1 → 反馈 → v2",
      narration: "这条线一开始其实连错了。Agent 先把灯塔的定义写成“定义等价”。你点“反馈给 agent”，告诉它灯塔没有第 0 项；Agent 提交第二版，改成“部分重合”，并写清对不上的部分。两本书都保持原样，差别写在线上。",
      card: { title: "灯塔的定义 → 精确说法", lines: [] },
      types: [relationNote(v1Relation || "definitionally_equivalent")],
      extra: {
        kind: "revision",
        v1: { relation: v1Relation, plain: relationPlain(v1Relation), note: "Agent 的第一版：说两边定义展开后一样" },
        feedback: str(revision?.entry.user_feedback),
        v2: { relation: str(v2.relation), plain: relationPlain(str(v2.relation)), lost: yiDefLost.claim, note: `替换第一版（${label("a.yi.def")}）` },
        why: str(revision?.diff.why),
      },
    },
    {
      id: "plain-labels", title: "一眼看全：类型和白话", seconds: 12, provenance: ["真实"],
      reveal: ["e.puzzle", "a.yi.ex"], focus: ["e.puzzle", "yi.u.ex"],
      caption: "每条线都写着关系类型和一句白话",
      narration: "每条线上都写着关系类型和一句白话：说的就是它，部分重合，或者特例。灯塔的那道剪拼题，也认出来了：它是一个题型，一族带参数的题。",
      card: { title: "线上的关系类型", lines: [
        ...["expresses", "overlaps_with", "specializes"].map((r) => `\`${r}\` · ${relationPlain(r)}`),
        `灯塔的习题 2.4.7 → ${stripTypePrefix(label("e.puzzle"))}：\`${relationOf("a.yi.ex")}\`（${label("a.yi.ex")}）`,
        `题型的参数：$k \\ge 4$；动画里的那一张：$k = ${String(instance.k ?? 6)}$（${str(instance.label)}）`,
      ] },
      types: [TYPE_NOTES.problem],
    },
    {
      id: "debts", title: "一笔账，挂在地图上", seconds: 10, provenance: ["真实"],
      reveal: debts, focus: debts,
      caption: "一笔账：还欠一个证明或检查",
      narration: "还没核对完的地方，不会被藏起来。它们是地图上的一笔笔账，挂在具体的线和对象上，写清楚欠什么、对谁有影响。",
      card: { title: "一笔账", lines: [
        ...debts.map((local) => `${str(payloadOf(projection, local).obligation)}（${debtStatusLabel(str(payloadOf(projection, local).status))}）`),
        `第一笔：${str(firstDebt.quantifier)} · ${str(firstDebt.failure_note)}`,
      ] },
      types: [TYPE_NOTES.debt],
    },
    {
      id: "accept", title: "接受 ≠ 证明", seconds: 10, provenance: ["演示", "真实"],
      reveal: [], focus: ["yi.u.thm", "e.cassini", ...lineDebts.map((debt) => debt.key)],
      caption: "接受只是认可这条对应",
      narration: `最后，你在审核页点接受。接受只是认可这条对应，把它写进地图；可信度不会因此变成“已证明”，这条线上的${lineDebts.length === 1 ? "那一笔账" : ` ${lineDebts.length} 笔账`}也还挂着。`,
      card: { title: "点「接受」以后", lines: [] },
      types: [TYPE_NOTES.trust],
      extra: {
        kind: "accept",
        banner: "接受只是认可这条对应，不会把可信度变成“已证明”",
        badge: trust.alignment === "candidate" ? "可信度：候选" : `可信度：${str(trust.alignment)}`,
        button: "接受这条对齐",
        axes: [
          `对齐：${trust.alignment === "candidate" ? "候选" : str(trust.alignment)}`,
          `证明：${trust.proof === "none" ? "还没有" : str(trust.proof)}`,
          debtAxisLine(lineDebts),
        ],
      },
    },
    (() => {
      const merged = mergeLearningUnits(yiMaterial?.units ?? []);
      const recapUnits = merged.map((unit) => {
        const unitLocal = localOf(projection, unit.unit_id);
        const hint = hintLocalFor(projection, unitLocal);
        const focus = [
          unitLocal,
          hint,
          ...unit.targets.flatMap((target) => [localOf(projection, target.entity_id), localOf(projection, target.alignment_id)]),
        ].filter((local): local is string => Boolean(local));
        return {
          title: unit.unit_title,
          label: unit.label,
          detail: formatMergedUnitDetail(unit, stripTypePrefix),
          note: hint ? `含「${stripTypePrefix(label(hint))}」` : undefined,
          focus,
        };
      });
      const seenUnits = recapUnits.filter((unit) => unit.label === "别的材料里见过").flatMap((unit) => unit.focus.filter((local) => local.startsWith("yi.u.")));
      const freshUnits = recapUnits.filter((unit) => unit.label === "新的").flatMap((unit) => unit.focus.filter((local) => local.startsWith("yi.u.")));
      const sharedAndAlignments = recapUnits.flatMap((unit) => unit.focus.filter((local) => !local.startsWith("yi.u.")));
      const conventionLocals = [...new Set((yiMaterial?.new_conventions ?? []).map((c) => localOf(projection, c.scope_id)).filter((local): local is string => Boolean(local)))];
      const debtLocals = (yiMaterial?.open_debts ?? []).map((debt) => localOf(projection, debt.debt_id)).filter((local): local is string => Boolean(local));
      return {
        id: "recap", title: "回顾：读灯塔时", seconds: 10, provenance: ["派生"] as const,
        reveal: [] as string[],
        focus: ["yi.u.def", "yi.u.lem", "yi.u.thm", "yi.u.ex", "yi.u.hint"],
        seen: seenUnits,
        fresh: freshUnits,
        accent: [...new Set([...sharedAndAlignments, ...conventionLocals, ...debtLocals])],
        legendNote: "绿框＝别的材料里见过　黄框＝新的",
        caption: "回顾：地图告诉你哪些见过、哪些是新的",
        narration: "回顾一下：以后读灯塔，地图告诉你哪些在别的材料里见过、只差在哪儿，哪些才是新的，有哪些新约定，以及还有哪几笔账没还。",
        card: { title: "读《灯塔组合数学初步》时", lines: [] as string[] },
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
        },
      };
    })(),
  ];
}

/** 一镜的全部文字（测试用：检查禁用词、确认没有花括号之类的原始数据）。 */
export function shotText(shot: TutorialShot): string[] {
  const out = [shot.title, shot.caption, shot.narration, shot.card.title, ...shot.card.lines];
  out.push(...shot.types.flatMap((t) => [t.name, t.plain]), ...(shot.tags ?? []).flatMap((t) => [t.tag, t.text]));
  const extra = shot.extra;
  if (extra?.kind === "materials") out.push(...extra.items.flatMap((i) => [i.file, i.book, i.markdown]));
  if (extra?.kind === "sequence") out.push(extra.note, ...extra.rows.map((row) => row.label));
  if (extra?.kind === "numbering") out.push(extra.sequenceNote, extra.missing.text, extra.missing.note, ...extra.explain, ...extra.formulas.flatMap((f) => [f.book, f.range]), ...extra.shared.map((s) => s.text), ...extra.yi.map((s) => s.text));
  if (extra?.kind === "revision") out.push(extra.v1.plain, extra.v1.note, extra.feedback, extra.v2.plain, extra.v2.lost, extra.v2.note, extra.why);
  if (shot.legendNote) out.push(shot.legendNote);
  if (extra?.kind === "accept") out.push(extra.banner, extra.badge, extra.button, ...extra.axes);
  if (extra?.kind === "learning") {
    out.push(extra.summary, ...extra.conventions.map((c) => c.text), ...extra.units.flatMap((u) => [u.title, u.label, u.detail, u.note ?? ""]));
  }
  return out;
}
