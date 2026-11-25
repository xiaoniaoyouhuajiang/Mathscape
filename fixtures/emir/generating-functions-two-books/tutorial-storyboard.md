# 教程第 2 集分镜：退化边界与换一种表示

> 数据：`fixtures/emir/generating-functions-two-books/objects.json` / `tutorial-projection.json`  
> 主题：阶段二（S2 内聚多分支/守卫/排除边界 + S3 前半方言符号表、表示核 $R$ 与表示绑定）  
> 只读承诺：播放器只读取编译期打包的 `tutorial-projection.json`，不调用任何写入命令，不触碰用户工作区。

## 镜头列表（10 镜）

1. **`intro` — 同叫「母函数」，两本书看的不是同一个东西**
   - 出场：`artifact:jia`（《星港母函数讲义》）、`artifact:yi`（《灯塔母函数初步》）、`e.theory`（「母函数与形式幂级数」）、`e.ogf_concept`（「普通母函数」）。
   - 核心提问：《星港》把母函数看作不问收敛性的形式幂级数符号串；《灯塔》把它看作开区间 $(-r, r)$ 上绝对收敛的实函数。它们怎样连到同一张地图上而不混淆条件？
2. **`spans` — 先读两本书的原话**
   - 出场：展示两本书关于普通母函数/生成函数、有限等比和与无穷级数收敛边界的 Markdown + TeX 原文。
3. **`local-symbols` — 约定卡升级：局部符号表**
   - 出场：`jia.u.sec`、`jia.u.def`、`ds.jia`、`yi.u.sec`、`yi.u.def`、`ds.yi` 及两条派生虚线。
   - 面板对比：《星港》声明 $a_n$（0 起算）、$A(x) \in R[[x]]$、系数提取 $[x^n]$；《灯塔》声明 $b_m$（1 起算）、实函数 $g(t)$、正收敛半径 $r > 0$。
4. **`rep-seq` — 第一个表示核 $R_{\text{seq}}$：统一离散数列的承载空间**
   - 出场：六边形节点 `r.seq`（「自然数下标数列表示核 $\mathbb{N} \to R$」）及紫色点划线 `rb.jia.seq`、`rb.yi.seq`（`binds_representation`）。
   - 说明：《星港》0 起算直连 $f(n)$；《灯塔》1 起算在绑定线上注明首项补零 $f(0) = 0$。
5. **`rep-fps` — 第二个表示核 $R_{\text{fps}}$：剥离收敛半径，把欠账挂在绑定线上**
   - 出场：`r.fps`（「形式幂级数表示核 $R[[X]]$」，含算子 `coeff`、`mul`）、`rb.jia.fps`、`rb.yi.fps`、`d.yi.analytic_radius`。
   - 说明：《灯塔》的 $g(t)$ 升格为形式幂级数时剥离了 $(-r, r)$ 绝对收敛条件，因此把 `d.yi.analytic_radius` 直接挂在 `rb.yi.fps` 上——以后若想代回实数求值，这笔账会提醒先验 $|t| < r$。
6. **`branches` — 内聚多分支与守卫：$x \neq 1$ 与 $x = 1$ 写在同一个定理里**
   - 出场：`jia.u.thm`、`yi.u.prop`、`e.geom_fin`（「有限几何级数求和公式」）。
   - 面板：展示 `guards`（常规 $x \neq 1$ / 退化 $x = 1$）、`guard_partition: partition`（互斥且穷尽）、两条 `branches`（`fraction_form` 分式与标记为 `[退化分支]` 的 `constant_sum` $= N$）及 `undefined` 分母排除项，不把主地图拆成细碎分支节点。
7. **`branch-align` — 分支感知对齐：谁覆盖了退化分支，一眼看清**
   - 出场：`a.jia.thm`（指向双分支 `["fraction_form", "constant_sum"]`）与 `a.yi.prop`（`specializes`，仅指向 `["fraction_form"]`）。
8. **`revision` — 漏填分支覆盖时的审核提醒与 v1 → v2 修订**
   - 面板：演示 v1 将《灯塔》命题 1.3.2 标为 `expresses` 且漏写 `target_branches`，触发 `branch_coverage_unspecified` 提醒；点击「反馈给 agent」后，v2 缩窄为 `specializes` 并写清漏掉 $t = 1$ 退化分支。
9. **`exclusions` — 形式逆元 vs 实数发散禁区：`exclusions` 不是第二分支**
   - 出场：`jia.u.cor`、`jia.r.cor_explains`、`yi.u.rem`、`yi.r.rem_explains`、`e.geom_inf_fps`、`a.jia.cor`、`a.yi.rem`。
   - 说明：在 $R[[X]]$ 中 $(1-X)\sum X^n = 1$ 无条件成立；而在实变数下 $|x| \ge 1$ 级数发散无定义——这是被排除在外的禁区（`exclusions: diverges`），不能伪装成定理的一条“发散分支”。
10. **`recap` — 全图回顾：两个表示核就位，等待下一阶段的「跨表示运输 $T$」**
    - 面板：读《灯塔母函数初步》时的学习视图汇总（已见过内容、退化分支差异提示与绑定线上的未还清账），并预告阶段三将在 $R_{\text{seq}}$ 与 $R_{\text{fps}}$ 之间架设双向运输边 $T$。
