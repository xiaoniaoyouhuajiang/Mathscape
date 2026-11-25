/**
 * Localized narrative, type explanations, and card copy for Tutorial Episode 2
 * ("教程（二）：退化边界与换一种表示").
 *
 * Lives under `src/i18n/` so CJK UI/tutorial copy stays centralized in the i18n
 * layer rather than scattering raw Chinese strings across `src/tutorial/`.
 */

export const EP2_META = {
  title: "教程（二）：退化边界与换一种表示",
  jiaFile: "星港母函数讲义.md",
  yiFile: "灯塔母函数初步.md",
  jiaBook: "《星港母函数讲义》§2.1",
  yiBook: "《灯塔母函数初步》§1.3",
  jiaShort: "星港",
  yiShort: "灯塔",
  provenanceReal: "真实" as const,
  provenanceDerived: "派生" as const,
  provenanceDemo: "演示" as const,
  recapLegendNote: "绿框＝别的材料里见过（线上写清漏掉的退化分支或实收敛域限制）",
};

export const EP2_TYPE_NOTES = {
  representationCore: {
    name: "表示核",
    code: "RepresentationCore",
    plain: "把一个数学概念落到具体的承载空间（carrier）与许可算子（operations）上，例如用系数函数 $\\mathbb{N} \\to R$ 承载数列，或用形式幂级数环 $R[[X]]$ 承载母函数。",
  },
  representationBinding: {
    name: "表示绑定",
    code: "RepresentationBinding",
    plain: "把一本书的约定卡（局部符号表）连到共享表示核上，逐个说明原书符号对应哪个共享表达式、首项怎么补齐、剥离了什么分析条件。",
  },
  branches: {
    name: "内聚多分支与守卫",
    code: "guards + branches",
    plain: "把同一条定理的常规情形（如 $x \\neq 1$）与退化边界（如 $x = 1$）收进同一个精确说法里，用守卫划分标明互斥完备，不把地图拆成细碎节点。",
  },
  exclusions: {
    name: "排除边界",
    code: "exclusions",
    plain: "明确记录使公式无定义或级数发散的禁区（如 $x = 1$ 代入分母为零的分式、$|x| \\ge 1$ 实级数发散），绝不把禁区伪装成定理的另一条结论分支。",
  },
} as const;

export const EP2_TAG_NOTES = {
  newConvention: {
    tag: "新约定",
    text: "阶段二的约定卡升级了「局部符号表」：把这节里的每个核心记号、角色和取值空间都登记清楚，后面的表示绑定才能逐号对照。",
  },
  seenElsewhere: {
    tag: "别的材料里见过",
    text: "就算两本书一条写了退化边界、一条漏了退化边界，学习视图也能认出它们讲的是同一条求和公式，并直接标出漏掉了哪一支。",
  },
} as const;

export const EP2_SHOTS = {
  intro: {
    title: "同叫「母函数」，看的不是同一个东西",
    caption: "一门讲形式符号串，一门讲收敛区间上的实函数",
    narration: "欢迎来到教程第二集。这次我们打开两本讲母函数的书：《星港母函数讲义》把母函数看作只比系数的形式幂级数；《灯塔母函数初步》却要求它在实开区间 $(-r, r)$ 内绝对收敛。连同后面的等比求和退化边界，地图怎样把它们连起来而不混淆前提？",
    cardTitle: "阶段二要解决的两件事",
    lines: [
      "《星港母函数讲义》：形式幂级数 $A(x) \\in R[[x]]$，只按系数相等，不谈实数代入收敛",
      "《灯塔母函数初步》：开区间 $(-r, r)$ 上绝对收敛的实函数 $g(t) = \\sum_{m=1}^{\\infty} b_m t^m$",
      "同一个概念「普通母函数」可以落到不同的承载表示核上；同一条求和公式也要分清常规分支与退化边界",
    ],
  },
  spans: {
    title: "先读两本书的原话",
    caption: "原文片段：形式幂级数、有限等比和与收敛边界",
    narration: "先看两本书这一节的原话。留意三处差别：第一，数列从第 0 项还是第 1 项起算；第二，母函数要不要收敛半径 $r > 0$；第三，有限等比求和有没有把公比等于 $1$ 的退化情形写出来。",
    cardTitle: "两本书各一节的原文（Markdown + TeX）",
  },
  localSymbols: {
    title: "约定卡升级：局部符号表",
    caption: "把每本书的记号、角色和定义域先立好档案",
    narration: "切出两节的定义后，两张约定卡先登场。和第一集相比，约定卡里多了一张「局部符号表」：星港登记了 $a_n$、$A(x)$ 和提取系数的算子 $[x^n]$；灯塔登记了从 1 起算的 $b_m$、实函数 $g(t)$ 和收敛半径 $r > 0$。",
    cardTitle: "两张约定卡与局部符号表",
    lines: [
      "星港约定卡：$(a_n)_{n \\ge 0}$ 从第 0 项起算，$A(x) \\in R[[x]]$ 不讨论实数收敛性",
      "灯塔约定卡：$(b_m)_{m \\ge 1}$ 从第 1 项起算，$g(t)$ 定义在开区间 $(-r, r)$ 上",
    ],
  },
  repSeq: {
    title: "第一个表示核：统一数列的承载空间",
    caption: "表示核（六边形）：把 0 起算和 1 起算放进同一个函数空间",
    narration: "两本书还没开始算母函数，底层数列的下标就已经不一样了。地图放上第一个六边形「数列表示核 $\\mathbb{N} \\to R$」，再用两条紫色点划线把两本书的约定卡绑上去：星港从 0 起算直连 $f(n)$；灯塔从 1 起算，在线上注明首项补零 $f(0) = 0$。",
    cardTitle: "数列表示核与两条表示绑定",
    lines: [
      "星港 $a_n \\mapsto f(n)$（$n \\ge 0$ 直接恒等对应）",
      "灯塔 $b_m \\mapsto f(m)$（$m \\ge 1$），显式补齐首项 $f(0) = 0$",
    ],
  },
  repFps: {
    title: "第二个表示核：剥离收敛半径，挂上欠账",
    caption: "升格为形式幂级数时，分析前提不能悄悄丢掉",
    narration: "接着放上第二个六边形「形式幂级数表示核 $R[[X]]$」，自带提取系数 $\\mathrm{coeff}$ 和卷积乘法 $\\mathrm{mul}$。灯塔的实函数 $g(t)$ 绑上来时剥离了 $(-r, r)$ 绝对收敛条件——地图不允许悄悄丢前提，所以把一笔收敛半径的账直接挂在这条绑定线上。",
    cardTitle: "形式幂级数表示核与收敛半径挂账",
    mathlibLine: "对照名称：[mathlib]({url}) 里的 `PowerSeries`（只作命名参照，不代表哪本书更正统）",
  },
  branches: {
    title: "内聚多分支：$x \\neq 1$ 与 $x = 1$ 合住一处",
    caption: "同一个精确说法里写清守卫划分、退化分支与分母禁区",
    narration: "再看有限等比级数求和。如果把 $x \\neq 1$ 和 $x = 1$ 拆成两个孤立节点，地图很快就会碎成一地零件。阶段二把它们收进同一个「有限几何级数求和公式」里：两条守卫互斥且穷尽，$x = 1$ 标为退化边界，在 $x = 1$ 处硬代分式则记入无定义排除项。",
    cardTitle: "一条定理，两支守卫，一处无定义排除",
    evalRegularLabel: "常规分支（x = 2 ≠ 1）",
    evalRegularFormula: "$1 + 2 + 4 + 8 = 15 = \\frac{1 - 2^4}{1 - 2}$",
    evalRegularNote: "走分式闭合式 fraction_form",
    evalDegenerateLabel: "退化边界（x = 1）",
    evalDegenerateFormula: "$1 + 1 + 1 + 1 = 4 = N$",
    evalDegenerateNote: "分式 $\\frac{1 - 1^4}{1 - 1} = \\frac{0}{0}$ 无定义，必须切入退化分支 constant_sum",
  },
  branchAlign: {
    title: "分支感知对齐：谁漏了退化边界，一眼看清",
    caption: "对齐线精确指向覆盖了哪几条分支",
    narration: "有了内部的多分支结构，两本书连上来的对齐线就能精确点名分支了：星港定理 2.1.2 同时覆盖分式分支与退化常数分支，关系是「说的就是它」；灯塔命题 1.3.2 开头写了 $t \\neq 1$，只覆盖常规分式分支，因此是「特例」，线上标明漏写了 $t = 1$ 时和为 $N$。",
    cardTitle: "按分支核对两本书的覆盖范围",
    jiaCoverFmt: "星港定理 2.1.2 → `expresses`（覆盖分支：{branches}）",
    yiCoverFmt: "灯塔命题 1.3.2 → `specializes`（仅覆盖：{branches}；缺少：{lost}）",
  },
  revision: {
    title: "漏填分支覆盖时的审核提醒与修订",
    caption: "v1 笼统写成「说的就是它」→ 提醒未声明分支 → v2 缩窄为「特例」",
    narration: "如果 agent 一开始大意了，把灯塔命题 1.3.2 笼统标成「说的就是它」又没填覆盖分支，系统会自动亮起「未声明分支覆盖」审核提醒。你在审核页点「反馈给 agent」指出灯塔漏了 $t = 1$，v2 就会缩窄为只指向常规分式分支的「特例」。",
    cardTitle: "对齐线的分支修订记录（v1 → v2）",
    v1Note: "目标定理含两条分支，v1 写成完整表达且未填 target_branches，触发「未声明分支覆盖」审核提醒",
    v2Note: "v2 明确 target_branches = [fraction_form]，取代 v1，旧提案留档备查",
    why: "只写 $t \\neq 1$ 分式并不涵盖 $t = 1$ 时和等于 $N$ 的退化边界；缩窄到具体分支后，下游引用才不会在 $t = 1$ 处除零出错。",
  },
  exclusions: {
    title: "形式逆元 vs 实数发散禁区：禁区不是第二分支",
    caption: "形式环里无条件相乘为 1；实变数 $|x| \\ge 1$ 发散属于排除边界",
    narration: "最后看无穷等比级数。在形式幂级数环 $R[[X]]$ 里，$(1 - X)\\sum_{n=0}^{\\infty} X^n = 1$ 逐项相乘恒成立；但代回实数时 $|x| \\ge 1$ 级数发散。注意：发散是没有合法的实数级数和，必须记在「排除边界」里，绝不能写成一条所谓的“等于发散”的结论分支。",
    cardTitle: "形式恒等式与实变数发散禁区",
    jiaCorLine: "星港推论 2.1.3 → `expresses`：陈述 $R[[x]]$ 形式逆元，并显式排除实数 $|x| \\ge 1$ 求值",
    yiRemLine: "灯塔注 1.3.3 → `overlaps_with`：只谈 $|t| < 1$ 实级数极限，受收敛半径挂账约束",
  },
  recap: {
    title: "全图回顾：两个表示核就位，等待跨表示运输",
    caption: "读《灯塔母函数初步》时的学习视图与下一阶段预告",
    narration: "全图拼好了！现在打开《灯塔母函数初步》的学习视图：命题 1.3.2 和注 1.3.3 都标着「别的材料里见过」，并精准提醒一处漏了 $t = 1$ 退化分支、一处限于 $|t| < 1$ 实收敛域，外加绑定线上的 1 笔收敛半径账。两个六边形表示核已经就位，下一阶段就可以在它们之间架设双向运输边了。",
    cardTitle: "读《灯塔母函数初步》时的学习视图",
  },
} as const;
