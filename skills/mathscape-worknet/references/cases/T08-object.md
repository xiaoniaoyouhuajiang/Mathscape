# 对象声明 · T08 Laplace 原理 / log-sum-exp（「和被最大项支配」）· v2.1

## 工具
Laplace 原理 / log-sum-exp：指数加权的和（或积分）在大参数下，由最大指数项（最大点邻域）支配。

## 形式结论（本对象必须证完的承诺）
- **(A) 离散 log-sum-exp**：取**固定**的项数 $N\in\mathbb N_{\ge1}$，实数 $a_1,\dots,a_N$，参数 $\beta>0$。记 $M=\max_i a_i$，$A^*=\{i:a_i=M\}$，$k=|A^*|\ge1$。则
  $$M+\frac{\log k}{\beta}\ \le\ \frac1\beta\log\sum_{i=1}^N e^{\beta a_i}\ \le\ M+\frac{\log N}{\beta}.$$
  在 **N 固定**的前提下令 $\beta\to\infty$，极限为 $M$，误差不超过 $(\log N)/\beta$。最大值不唯一也不影响结论（$M$ 总是存在的）。
- **(B) 测度版（扩展值，正式分成三支，每支各写前提）**：$(X,\mu)$ 为测度空间，$f$ 可测（实值或复值），$p$ 只取有限正数 $0<p<\infty$，$p\to\infty$。记 $M=\|f\|_\infty=\operatorname{ess\,sup}|f|\in[0,\infty]$。约定 $\|f\|_p=(\int|f|^pd\mu)^{1/p}\in[0,\infty]$，$\infty^{1/p}=\infty$。
  - (B-zero) 前提：$M=0$。此时 $f=0$ a.e.，对所有 p 有 $\|f\|_p=0$，结论平凡。
  - (B-fin) 前提：$0<M<\infty$，**且**存在 $0<q<\infty$ 使 $f\in L^q(\mu)$。结论：$\lim_{p\to\infty}\|f\|_p=M$。证法是夹逼：下界用水平集 $S_\delta$，上界用插值。（去掉 $L^q$ 会失败：Lebesgue 测度下 ℝ 上 f≡1。）
  - (B-inf) 前提：$M=\infty$，**只要求 f 可测，不要求任何可积性**。结论：$\lim_{p\to\infty}\|f\|_p=\infty$。证法：对每个 $K>0$ 取 $E_K=\{|f|\ge K\}$，证 $\liminf_p\|f\|_p\ge K$。
  - 三支的条件（$M=0$、$0<M<\infty$、$M=\infty$）是**分支守卫**，不是 Debt。
- **(C) 渐近积分**（有限区间 $[a,b]$，$a<b$，$n\to\infty$）
  - (C1) 对数尺度：若 φ 是实值函数且在 $[a,b]$ 上连续，则 $\frac1n\log\int_a^b e^{n\varphi(t)}dt\to\max_{[a,b]}\varphi$。**不需要**最大点唯一，也没有 g。
  - (C2) 精细主项，**只处理唯一内点最大点**：设 φ 是实值函数，在 $[a,b]$ 上连续，只在 $t_0\in(a,b)$ 处取最大值 $M=\varphi(t_0)$；φ 在 $t_0$ 处**二阶可导**，即有 Peano 形式 $\varphi(t)=M-\tfrac c2(t-t_0)^2+o((t-t_0)^2)$，其中 $c=-\varphi''(t_0)>0$。（「φ 在 $t_0$ 邻域内 $C^2$」是它的一个充分条件，不是本对象的前提。）g 在 $[a,b]$ 上可测（实值或复值），$g\in L^1[a,b]$；取在 $t_0$ 处连续的那个代表元，并且 $g(t_0)\ne0$。则
  $$\int_a^b g\,e^{n\varphi}dt\ \sim\ g(t_0)\,e^{n\varphi(t_0)}\sqrt{\frac{2\pi}{n|\varphi''(t_0)|}}.$$
  多峰求和、端点最大、退化最大（$\varphi''(t_0)=0$）都不在本对象承诺内。

## 刻意排除（可作 Debt 或延伸，不作为必须证完的结论）
- 一般大偏差理论（LDP、速率函数、Cramér/Sanov 等）
- Varadhan 引理完整陈述（(C1) 的推广方向）
- 复 Laplace / 鞍点法 / 最速下降
- (C2) 的多峰、端点、退化情形
- 应用关系 T4a（Soudry）与 T4b（Xie）在 v2.1 中改为 **App** 类型，属于非核心关系，只作 optional 的应用说明，不计入 A/B/C 的证明义务
- 附录：另有特殊函数路线（例如 MSE 197602 中的不完全 Γ 写法），本对象排除。

## 锚点
- 论文：Soudry 等 2017 式(5)；Xie 等 2021 式(1)、§3.1 式(5)(6)。
- 习题：MSE 242779（(B)）；MSE 197602（端点最大值例子。在本对象中只用来支撑「近/远切分 + 尾部估计」这一**方法**，不支撑 (C2) 的内点主项）。
- 背景：/workspace/mathscape/parts/04_tools_b.md §T08；/workspace/mathscape/report.md §3.3、§3.6、§T08。

## Changelog
- v1（2026-10-30 01:45 UTC+8）：初版，备份在 object-v1.md。
- v2（2026-10-30 02:15 UTC+8）：按 scope-decisions.md 修改。(A) 改用 N 与 β，引入 argmax 集合和 log k 下界；(B) 改为扩展值，拆成 B-fin/B-inf 两支，可积性只要求 f∈L^q；(C1) 去掉唯一性和 g；(C2) 限定唯一内点非退化最大，并写明 g 的条件；排除特殊函数路线以及多峰/端点/退化情形；应用边改为 optional；说明 197602 只支撑切分方法。逐条对照见 changelog-round1.md。
- v2.1（2026-10-30 02:40 UTC+8）：按 Astra 的 Round 2b 决定（layer-a/round2b-astra-decisions.md，P1/P3/P7）修改。(B) 正式拆成 B-zero/B-fin/B-inf 三支，各写前提：$L^q$（$0<q<\infty$）只是 B-fin 的前提，B-inf 只要求可测，p 只取有限正数，三支的条件改称分支守卫。(C1)(C2) 声明 φ 为实值。(C2) 把「$C^2$ 邻域」改为「在 $t_0$ 处二阶可导（Peano）」，$C^2$ 只作充分条件；g 的条件从 $L^\infty$ 放宽为 $L^1$，并写明取 $t_0$ 处连续的代表元。T4a/T4b 改为 App。v2 备份在 object-v2.md。
