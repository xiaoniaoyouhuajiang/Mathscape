/**
 * 教程「两本教材是不是同一颗星」的 11 镜（分镜见
 * fixtures/emir/fibonacci-two-books/tutorial-storyboard.md）。
 *
 * 纯函数：每一镜的文字都从只读投影里读（真实），或由投影里的对象算出来
 * （派生）；只有“合并失败”和“点接受”两处是演示。全片不出现“平移 / 错开 /
 * 滑动”这类说法：动的只是“第几条”。
 */
import { debtStatusLabel } from "../semantic/labels";
import { materialSummary } from "../semantic/learning-view";
import type { TutorialProjection } from "./projection";
import { graphLabel, payloadOf } from "./projection";

export type Provenance = "真实" | "派生" | "演示";

/** 镜 5：按项下标 k 对齐的数表。 */
export interface SequenceTable {
  kind: "sequence";
  ks: number[];
  rows: Array<{ label: string; cells: Array<number | null> }>;
  note: string;
}

/** 镜 6：“第几条”编号牌对照（数列行固定不动，只有乙的编号牌右移一格）。 */
export interface NumberingTable {
  kind: "numbering";
  sequenceNote: string;
  shared: Array<{ n: number; text: string }>;
  yi: Array<{ n: number; text: string; matches: number }>;
  missing: { n: number; text: string; note: string };
  shift: number;
}

/** 镜 8：合并失败（演示），理由来自对齐线上的“对不上的部分”。 */
export interface MergeDemo {
  kind: "merge";
  branches: Array<{ title: string; reason: string }>;
}

/** 镜 11：接受 ≠ 证明，以及读乙时的学习标签。 */
export interface LearningPanel {
  kind: "learning";
  banner: string;
  badge: string;
  units: Array<{ title: string; label: string; detail: string }>;
  conventions: string[];
  summary: string;
}

export type ShotExtra = SequenceTable | NumberingTable | MergeDemo | LearningPanel;

export interface TutorialShot {
  id: string;
  title: string;
  seconds: number;
  provenance: Provenance[];
  /** 本镜新出现在地图上的对象（夹具 local_ref）。 */
  reveal: string[];
  /** 本镜高亮的对象。 */
  focus: string[];
  caption: string;
  narration: string;
  card: { title: string; lines: string[] };
  extra?: ShotExtra;
}

const str = (value: unknown): string => (typeof value === "string" ? value : "");
const list = (value: unknown): Record<string, unknown>[] =>
  Array.isArray(value) ? (value as Record<string, unknown>[]) : [];

/** F_k（F₀ = 0, F₁ = 1）。 */
export function fib(k: number): number {
  let [a, b] = [0, 1];
  for (let i = 0; i < k; i++) [a, b] = [b, a + b];
  return a;
}

const cassiniShared = (n: number) => fib(n + 1) * fib(n - 1) - fib(n) ** 2;
const cassiniYi = (n: number) => fib(n) * fib(n + 2) - fib(n + 1) ** 2;

function convention(projection: TutorialProjection, scope: string, key: string): string {
  const found = list(payloadOf(projection, scope).conventions).find((c) => c.key === key);
  return str(found?.text);
}

function lost(projection: TutorialProjection, alignment: string): { short: string; claim: string } {
  const first = list(payloadOf(projection, alignment).lost_information)[0] ?? {};
  return { short: str(first.short), claim: str(first.claim) };
}

function shiftOf(projection: TutorialProjection, alignment: string): number {
  // 乙的第 n 条 ＝ 共享的第 n+1 条：编号牌右移的格数。
  const map = list(payloadOf(projection, alignment).parameter_map)[0];
  const match = /第\s*n\s*\+\s*(\d+)\s*条/.exec(str(map?.map));
  return match ? Number(match[1]) : 0;
}

export function buildTutorialShots(projection: TutorialProjection): TutorialShot[] {
  const label = (local: string) => graphLabel(projection, local);
  const fibDef = payloadOf(projection, "e.fib_def");
  const baseline = (fibDef.mathlib_baseline ?? {}) as Record<string, unknown>;
  const yiDefLost = lost(projection, "a.yi.def");
  const yiThmLost = lost(projection, "a.yi.thm");
  const shift = shiftOf(projection, "a.yi.thm");
  const yiArtifact = projection.artifacts.yi;
  const yiMaterial = projection.learning.materials.find((m) => m.artifact_id === yiArtifact);
  const debts = ["d.yi.f_eq_F", "d.yi.lem.forward", "d.yi.thm.forward", "d.puzzle.all_k"];
  const trust = (payloadOf(projection, "a.yi.thm").trust ?? {}) as Record<string, unknown>;
  const quote = (local: string) => str(projection.objects[local]?.quote);
  const firstDebt = payloadOf(projection, "d.yi.f_eq_F");

  const sources = [
    "jia.u.sec", "jia.u.def", "jia.u.lem", "jia.u.lem_pf", "jia.u.thm", "jia.u.thm_pf", "jia.u.rem",
    "yi.u.sec", "yi.u.def", "yi.u.lem", "yi.u.thm", "yi.u.ex", "yi.u.hint",
    "jia.r.lem_proof", "jia.r.thm_proof", "jia.r.thm_cites_lem", "jia.r.rem_explains", "yi.r.hint", "yi.r.hint_cites_thm",
  ];

  return [
    {
      id: "teaser", title: "接上预告", seconds: 7, provenance: ["演示", "真实"],
      reveal: ["artifact:jia", "artifact:yi", "e.fib"], focus: ["e.fib"],
      caption: "两本教材，是不是同一颗星？",
      narration: "上一集留了个问题：两本教材，一本从 F₀ 等于 0 开始，一本从 F₁ 等于 1 开始。它们说的是同一颗星吗？我们在地图上看一看。",
      card: { title: "两本教材，是不是同一颗星？", lines: [`星：${label("e.fib")}（共享对象）`, "两本书都是示例材料（虚构教材）"] },
    },
    {
      id: "spans", title: "两段原文", seconds: 10, provenance: ["真实"],
      reveal: [], focus: ["artifact:jia", "artifact:yi"],
      caption: "原文片段：书上这一段",
      narration: "左边是《星港数列讲义》，右边是《灯塔组合数学初步》，都是我们编的示例书。地图记下的第一件事，永远是“书上这一段在哪儿”。",
      card: { title: "原文片段：书上这一段", lines: [`甲：${quote("jia.s.def")}`, `乙：${quote("yi.s.def")}`] },
    },
    {
      id: "units", title: "来源单元", seconds: 9, provenance: ["真实"],
      reveal: sources, focus: ["jia.u.def", "jia.u.lem", "jia.u.thm", "jia.u.rem", "yi.u.def", "yi.u.lem", "yi.u.thm", "yi.u.ex"],
      caption: "来源单元：书里的一条定义、定理或习题",
      narration: "每段话变成一张来源卡片：一条定义、一条定理、一道习题。卡片永远连着它的原文。",
      card: { title: "来源单元", lines: [label("jia.u.def"), label("jia.u.thm"), label("yi.u.def"), label("yi.u.ex")] },
    },
    {
      id: "same-star", title: "指向同一颗星", seconds: 10, provenance: ["真实"],
      reveal: ["a.jia.def.fib", "a.yi.def.fib"], focus: ["e.fib", "jia.u.def", "yi.u.def"],
      caption: "对齐线：这段话说的就是那颗星",
      narration: "一本叫它 Fibonacci 数列，一本叫它兔子数列。名字不同，说的是同一个概念：同一颗星。",
      card: { title: "对齐线", lines: [
        `甲的定义 → ${label("a.jia.def.fib")}`, `乙的定义 → ${label("a.yi.def.fib")}`,
        `乙的约定卡：${convention(projection, "ds.yi", "terminology")}`,
      ] },
    },
    {
      id: "index-zero", title: "但不是同一句话：第 0 项", seconds: 12, provenance: ["真实", "派生"],
      reveal: ["e.theory", "e.fib_def", "a.jia.def", "a.yi.def", "ds.jia", "ds.yi"], focus: ["e.fib_def", "jia.u.def", "yi.u.def", "ds.yi"],
      caption: "数列逐项对齐：f_k = F_k",
      narration: "再看精确说法。从第 1 项起，两本书每一项都一样，列对列。差别只有一格：乙不用第 0 项。这条约定写在乙的约定卡上，地图读得懂它。",
      card: { title: label("e.fib_def"), lines: [
        `甲的定义 → ${label("a.jia.def")}`, `乙的定义 → ${label("a.yi.def")}`,
        `约定卡：${convention(projection, "ds.yi", "sequence_index_origin")}`,
        `参照基线：mathlib ${str(baseline.decl)}（${baseline.status === "unchecked" ? "还没在本机核对" : str(baseline.status)}）`,
        "写法跟 mathlib 一致，不代表哪本书更正统",
      ] },
      extra: {
        kind: "sequence", ks: [0, 1, 2, 3, 4, 5, 6],
        rows: [
          { label: "甲 F_k", cells: [0, 1, 2, 3, 4, 5, 6].map(fib) },
          { label: "乙 f_k", cells: [0, 1, 2, 3, 4, 5, 6].map((k) => (k === 0 ? null : fib(k))) },
        ],
        note: `从 k = 1 起每一列都相同；乙的 k = 0 是空格（${yiDefLost.short}）`,
      },
    },
    {
      id: "cassini", title: "Cassini：是第几条对第几条", seconds: 18, provenance: ["真实", "派生"],
      reveal: ["e.cassini", "a.jia.thm", "a.yi.thm"], focus: ["jia.u.thm", "yi.u.thm", "jia.u.rem", "e.cassini"],
      caption: `动的是第几条：${label("a.yi.thm").replace(/^特例：/, "")}`,
      narration: "Cassini 恒等式在两本书里长得不一样。但数列一项都没动，动的是“第几条”。乙的第 1 条，就是共享的第 2 条；乙的第 2 条，就是共享的第 3 条。这样对下去，乙只少了共享的第 1 条，而那一条恰好要用到第 0 项。所以乙说的是一个特例。",
      card: { title: label("e.cassini"), lines: [
        `甲 → ${label("a.jia.thm")}`, `乙 → ${label("a.yi.thm")}`, `对不上的部分：${yiThmLost.short}`,
      ] },
      extra: {
        kind: "numbering", sequenceNote: "数列没有动：f_k = F_k", shift,
        shared: [1, 2, 3, 4].map((n) => ({ n, text: `共享第 ${n} 条 = ${cassiniShared(n)}` })),
        yi: [1, 2, 3].map((n) => ({ n, text: `乙第 ${n} 条 = ${cassiniYi(n)}`, matches: n + shift })),
        missing: { n: 1, text: `共享第 1 条：F₂F₀ − F₁² = ${cassiniShared(1)}`, note: "乙写不出来：要用 F₀（就是甲的注 3.2.4）" },
      },
    },
    {
      id: "matrix", title: "矩阵也一样", seconds: 6, provenance: ["真实"],
      reveal: ["e.qpow", "a.jia.lem", "a.yi.lem"], focus: ["e.qpow", "jia.u.lem", "yi.u.lem"],
      caption: "矩阵公式：同样是第几条对第几条",
      narration: "矩阵那条也一样：动的是第几条，不是数列。",
      card: { title: label("e.qpow"), lines: [`甲 → ${label("a.jia.lem")}`, `乙 → ${label("a.yi.lem")}`] },
    },
    {
      id: "no-merge", title: "为什么不合并", seconds: 12, provenance: ["演示", "真实"],
      reveal: [], focus: ["jia.u.def", "yi.u.def", "ds.yi"],
      caption: "不合并，各连一条对齐线",
      narration: "那为什么不干脆合成一个？按甲合并，等于替乙补写了它没写过的第 0 项；按乙合并，甲那一条就丢了。所以不合并：两本书各自保留原样，各连一条对齐线，差别就写在线上。",
      card: { title: "不合并，各连一条对齐线", lines: ["地图里没有“合并”这个操作，也没有“同一个”（same_as）"] },
      extra: { kind: "merge", branches: [
        { title: "按甲合并 → 乙被塞进一个它没写过的 F₀", reason: yiDefLost.claim },
        { title: "按乙合并 → 甲的第 1 条没地方放", reason: yiThmLost.claim },
      ] },
    },
    {
      id: "plain-labels", title: "一眼看全：线上的白话", seconds: 8, provenance: ["真实"],
      reveal: ["e.puzzle", "a.yi.ex"], focus: ["e.puzzle", "yi.u.ex"],
      caption: "每条线都用一句白话写清楚",
      narration: "每条线都用一句白话写清楚：说的就是它，部分重合，或者特例。连那道剪拼题也认出来了。",
      card: { title: "三种白话标签", lines: ["说的就是它", "部分重合", "特例", `乙的习题 → ${label("a.yi.ex")}`] },
    },
    {
      id: "debts", title: "一笔账，挂在地图上", seconds: 10, provenance: ["真实"],
      reveal: debts, focus: debts,
      caption: "一笔账：还欠一个证明或检查",
      narration: "还没核对完的地方，不会被藏起来。它们是地图上的一笔笔账，挂在具体的线和卡片上，写清楚欠什么、对谁有影响。",
      card: { title: "一笔账", lines: [
        ...debts.map((local) => `${str(payloadOf(projection, local).obligation)}（${debtStatusLabel(str(payloadOf(projection, local).status))}）`),
        `第一笔：${str(firstDebt.quantifier)} · ${str(firstDebt.failure_note)}`,
      ] },
    },
    {
      id: "accept-and-learn", title: "接受，以及学到了什么", seconds: 10, provenance: ["演示", "真实", "派生"],
      reveal: [], focus: ["yi.u.def", "yi.u.lem", "yi.u.thm", "yi.u.ex"],
      caption: "接受只表示写入地图，不代表数学上已验证",
      narration: "接受，只是把它写进地图，不等于证明了它。真正的好处在这里：以后读乙，地图告诉你哪些在别的材料里见过，哪些才是新的，以及还有哪几笔账没还。",
      card: { title: "读乙时", lines: [] },
      extra: {
        kind: "learning",
        banner: "接受只表示写入地图，不代表数学上已验证",
        badge: trust.alignment === "candidate" ? "可信度：候选" : `可信度：${str(trust.alignment)}`,
        units: (yiMaterial?.units ?? []).map((unit) => ({
          title: unit.unit_title, label: unit.label,
          detail: unit.only_differs_by.length ? `只差：${unit.only_differs_by.join("；")}` : unit.entity_label,
        })),
        conventions: (yiMaterial?.new_conventions ?? []).map((c) => `新约定：${c.text}`),
        summary: yiMaterial ? materialSummary(yiMaterial) : "",
      },
    },
  ];
}

/** 一镜的全部文字（测试用：检查禁用词、确认没有花括号之类的原始数据）。 */
export function shotText(shot: TutorialShot): string[] {
  const out = [shot.title, shot.caption, shot.narration, shot.card.title, ...shot.card.lines];
  const extra = shot.extra;
  if (extra?.kind === "sequence") out.push(extra.note, ...extra.rows.map((row) => row.label));
  if (extra?.kind === "numbering") out.push(extra.sequenceNote, extra.missing.text, extra.missing.note, ...extra.shared.map((s) => s.text), ...extra.yi.map((s) => s.text));
  if (extra?.kind === "merge") out.push(...extra.branches.flatMap((b) => [b.title, b.reason]));
  if (extra?.kind === "learning") out.push(extra.banner, extra.badge, extra.summary, ...extra.conventions, ...extra.units.flatMap((u) => [u.title, u.label, u.detail]));
  return out;
}
