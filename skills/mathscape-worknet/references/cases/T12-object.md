# 对象声明 · T12 KL 散度：非负性（Gibbs）与 Donsker–Varadhan 变分表示 · v1.3（round4 补丁，依据 layer-a/round4-astra.md §2、§7 `patch_checklist_r4[]` 与准则 v0.5.1；v1.2 备份 object-v1.2.md）· 前版 v1.2（round3 补丁，依据 layer-a/round3-astra.md §2 R3-C01/C07、§6、§7 `patch_checklist_r3[]`；v1.1 备份 object-v1.1.md；v1 备份 object-v1.md；v0 备份 object-v0.md）
- 材料源：/workspace/mathscape/parts/04_tools_b.md §T12（与 report.md §T12 同文）；02_papers.md 中 P6 式(11)、P7 定理 2.3 的出现位置
- 准则：/workspace/rdtx-criteria/criteria/v0.5.md（v0.4 及更早版本保留）
- **全局（v1.2 改写，R3-C01）**：标量为 $\mathbb R$；$(\Omega,\mathcal F)$ 是任意可测空间；$\mu,\lambda$ 是 $(\Omega,\mathcal F)$ 上的**有限正测度**，即 $0\le\mu(\Omega)<\infty$、$0\le\lambda(\Omega)<\infty$。各分支的量词域在下文「分支量词域」中**逐分支正式给出**；「$\mu,\lambda$ 均为概率测度」是对全部分支同时成立的**充分条件**（I2(e)），不再是全局量词。约定 $\log0=-\infty$，$0\log0=0$，$\log(+\infty)=+\infty$，$0\cdot(\pm\infty)=0$。

## 工具
KL 散度衡量用 λ 去近似 μ 的「信息代价」。非负性是信息论的基本不等式；DV 变分式把 KL 写成对检验函数 Φ 的上确界，是 PAC-Bayes 换测度论证的核心。

## 分支量词域（v1.2 新增；I2(d)/I2(e)/I7，v0.5【放宽域有限质量】）
| 域记号 | 分支 | $\mu$ 的域 | $\lambda$ 的域 | 与「概率对」充分条件的关系 |
|---|---|---|---|---|
| $\mathcal D_A$ | A | $\mu(\Omega)=1$ | $\lambda(\Omega)\le1$（λ 为正测度） | 概率对 ⊂ $\mathcal D_A$ |
| $\mathcal D_{lo}$ | B-lo、B-att | $\mu(\Omega)=1$ | $0<\lambda(\Omega)<\infty$ | 概率对 ⊂ $\mathcal D_{lo}$ |
| $\mathcal D_{up}$ | B-up | $0<\mu(\Omega)<\infty$ | $0<\lambda(\Omega)<\infty$ | 概率对 ⊂ $\mathcal D_{lo}\subset\mathcal D_{up}$ |
| $\mathcal D_B:=\mathcal D_{lo}\cap\mathcal D_{up}=\mathcal D_{lo}$ | B（等式形式） | $\mu(\Omega)=1$ | $0<\lambda(\Omega)<\infty$ | 概率对 ⊂ $\mathcal D_B$ |

- $\mathcal D_{up}$ 中 $\mu(\Omega)<\infty$ 对应 statement-Debt `Debt-μfin-up`；$\mu(\Omega)>0$ 对应 route-Debt `RD-μpos`（B-up 陈述在 $\mu=0$ 时仍成立，只有本路线的平移构造失效；见网）。**v1.3（R4-C02，v0.5.1 I2【route-Debt 见证域】）**：RD-μpos 的见证 W-μ0 落在正式 $\mathcal D_{up}$ 之外；因 RD-μpos 本身就是被移除的域条件 $\mu(\Omega)>0$，按 v0.5.1 保留为核心 route-Debt，其 scope 显式登记扩展测试域 $\mathcal D_{up}^{0}=\{0\le\mu(\Omega)<\infty,\ 0<\lambda(\Omega)<\infty\}$（只去掉 $\mu(\Omega)>0$，其余结论前提 Debt-μfin-up、Debt-λpos、Debt-λfin-up、Debt-bdd、guard 保持不变）。二者合起来就是 B-up 的显式有限质量条件 $0<\mu(\Omega)<\infty$。**本对象不使用「按有限正测度理解」之类的非量词说法**；上表即为量词。
- $\mathcal D_A$、$\mathcal D_{lo}$ 中 $\mu(\Omega)=1$ 即 theorem-Debt `Debt-μP`（scope：A、B-lo、B-att；**不含 B-up**）。

## 记号与类型（I7；v1.2 逐分支）
- $\mu\ll\lambda$ 时，$f:=d\mu/d\lambda$ 取 $\mathcal F$-可测、$[0,\infty)$ 值的固定版本（λ-a.e. 唯一），$\int_\Omega f\,d\lambda=\mu(\Omega)$（T1，有限正测度域，见网）。
- $\mathrm{KL}(\mu\|\lambda):=\int_\Omega\log f\,d\mu$（$\mu\ll\lambda$）；$:=+\infty$（$\mu\not\ll\lambda$）。**逐分支类型**：在任一 $\mu(\Omega)<\infty$、$\lambda(\Omega)<\infty$ 的域上，$\mathrm{KL}\in[-e^{-1}\lambda(\Omega),+\infty]$ 良定（网 L1）；在 $\mathcal D_A$ 上由 (A) 得 $\mathrm{KL}\in[0,+\infty]$；在 $\mathcal D_{lo}$ 上 $\mathrm{KL}\in[-\log\lambda(\Omega),+\infty]$；在 $\mathcal D_{up}$ 上 $\mathrm{KL}\in[-e^{-1}\lambda(\Omega),+\infty]$。
- $\mathcal B:=\{\Phi:\Omega\to\mathbb R\ \mathcal F\text{-可测且有界}\}$；$J_\mu(\Phi):=\int\Phi\,d\mu-\log\int e^\Phi d\lambda$。
- **$J$ 的逐分支类型条件（v1.2，R3 §1 第 13 行）**：$J_\mu(\Phi)\in\mathbb R$（$\forall\Phi\in\mathcal B$）当且仅当所用的域满足 $\mu(\Omega)<\infty$ 且 $0<\lambda(\Omega)<\infty$（充分性见网 L2；必要性的失败见证见网 W-μ∞、W-λ0、W-λ∞）。因此：在 $\mathcal D_{lo}$（B-lo、B-att）上由 `Debt-μP`、`Debt-λpos`、`Debt-λfin-stmt`、`Debt-bdd` 保证；在 $\mathcal D_{up}$（B-up）上由 `Debt-μfin-up`、`Debt-λpos`、`Debt-λfin-up`、`Debt-bdd` 保证。$J\notin\mathbb R$ 时按 v0.4 I6 记 `statement_applicability / premise_inapplicable`。
- **B-up 的扩展值许可（v1.2 显式化，对应 R2 E1）**：仅为评估 `Debt-λfin-up` 的失败见证（$\lambda(\Omega)=+\infty$），对象**明确允许** B-up 使用 $J$ 的自然扩展 $J_\mu(\Phi)=\int\Phi\,d\mu-\log\int e^\Phi d\lambda\in[-\infty,+\infty)$ 与 $\sup$ 在 $[-\infty,+\infty]$ 中比较；这是 v0.4 I6「对象明确允许扩展值」的情形，故该见证在 B-up 记为 `theorem_conclusion`。B-lo、B-att 不享有此许可（同一见证在其上记 `statement_applicability`）。**v1.3（R4 §2 接受、§7 object 第 2 项）**：此许可仅限评估 W-λ∞，范围局部；不扩展成一般 B-up 结论，也不改变 B-up 的一般 J 类型（$J\in\mathbb R$）。按 R4 裁决与 v0.5.1 I6【扩展值与 normalizer_invalid】，该见证记 `failure_target=theorem_conclusion`、`failure_type=branch_conclusion_mismatch`、`failure_subtype=normalizer_invalid`（根因 $Z=+\infty$）。
- 「log f 本质有界」指：存在 $M<\infty$ 使 $e^{-M}\le f\le e^M$ λ-a.e.
- **零集代表元与 a.e. 基准（v0.3 I7）**：f 取处处有定义、$[0,\infty)$ 值的固定版本。$\log f$ 在 $\{f=0\}$ 上按全局约定取 $-\infty$（处处有定义，$[-\infty,\infty)$ 值可测）。只在 $f>0$ 上直接定义的表达式一律做可测延拓：$Y_0:=\mathbf 1_{\{f>0\}}/f$、$Y_\Phi:=\mathbf 1_{\{f>0\}}e^\Phi/f$（在 $\{f=0\}$ 上取 0），并且只在 μ-a.e. 意义下使用（$\mu(f=0)=0$）。每个等式都标明基准：涉及 μ 积分的写「μ-a.e.」，涉及 λ 积分的写「λ-a.e.」。
- **参数域与极限次序（I7）**：截断序列的下标 $n\in\mathbb N_1=\{1,2,\dots\}$，只有单一极限 $n\to\infty$；$\Phi_n$ 处处有定义且有界；平移常数 $c_m$（$m=\mu(\Omega)\in(0,\infty)$）与 n 无关，先固定 μ、λ，再取 $n\to\infty$。

## 形式结论（分支；v1.2 量词与上表一致）
- **(A) Gibbs 非负性**：∀(Ω,𝓕)、∀$(\mu,\lambda)\in\mathcal D_A$：$\mathrm{KL}(\mu\|\lambda)\ge0$，且等号成立 ⇔ μ=λ。
- **(B) DV 变分表示**：∀(Ω,𝓕)、∀$(\mu,\lambda)\in\mathcal D_B$：$\mathrm{KL}(\mu\|\lambda)=\sup_{\Phi\in\mathcal B}J_\mu(\Phi)$（等式在 $(-\infty,+\infty]$ 中成立；概率对时两边在 $[0,+\infty]$ 中）。它由下面两个分支在 $\mathcal D_B=\mathcal D_{lo}\cap\mathcal D_{up}$ 上合成：
  - (B-lo) ∀$(\mu,\lambda)\in\mathcal D_{lo}$、∀Φ∈𝓑：$J_\mu(\Phi)\le\mathrm{KL}(\mu\|\lambda)$；
  - (B-up) ∀$(\mu,\lambda)\in\mathcal D_{up}$：存在 $(\Phi_n)_{n\in\mathbb N_1}\subset\mathcal B$，$J_\mu(\Phi_n)\to\mathrm{KL}(\mu\|\lambda)$（KL=+∞ 时 → +∞）。B-up 在比 $\mathcal D_B$ 更大的 $\mathcal D_{up}$ 上单独成立（此时 sup 可以大于 KL，B-up 不断言 sup=KL）。
- **(B-att) 可达性**：∀$(\mu,\lambda)\in\mathcal D_{lo}$：上确界被某个 Φ∈𝓑 取到 ⇔ $\mu\ll\lambda$ 且 log f 本质有界（后者蕴含 f>0 λ-a.e.，即 $\lambda\ll\mu$）；此时取到的 Φ 恰为 $\log f+c$（c∈ℝ，λ-a.e.）。

## 分支守卫（guard_registry 见两张网，内容相同；β 另有实例 guard）
G-nac：$\mu\not\ll\lambda$；G-fin：$\mu\ll\lambda$ 且 KL<∞；G-inf：$\mu\ll\lambda$ 且 KL=+∞。三者在每个分支量词域上两两互斥并覆盖全部 (μ,λ)。G-eb：$\mu\ll\lambda$ 且 log f 本质有界，G-eb ⊂ G-fin（overlap）。守卫只用于分情形，不是 Debt。

## 最小 Debt、充分条件与放宽域失败见证的适用范围（I2(e) / v0.4；v1.2 改写）
「μ、λ 为概率测度」是对象的**充分条件**声明（适用域：全部分支）。各分支的正式量词域即上表 $\mathcal D_A,\mathcal D_{lo},\mathcal D_{up}$；网中的**分支最小 Debt**按分支与 `debt_kind` 原子化：
- $\mu(\Omega)=1$（Debt-μP）：theorem-Debt，scope = A、B-lo、B-att。**B-up 无失败见证，故不在该 scope/consumer**（v1.2 的 B-up 链 D-Bup2 不消费 B-lo，也不消费 $\int f\,d\lambda=1$；W-μ 在 $\mathcal D_{up}$ 内且 B-up 成立）。
- $\mu(\Omega)<\infty$（Debt-μfin-up）：statement-Debt，scope = B-up（失败见证 W-μ∞：$\int\Phi\,d\mu$ 为 $\infty-\infty$）。
- $\mu(\Omega)>0$（RD-μpos）：route-Debt，只属 B-up 的平移截断路线；陈述本身在 $\mu=0$ 时成立。v1.3：核心 route-Debt，测试域为扩展测试域 $\mathcal D_{up}^{0}=\{0\le\mu(\Omega)<\infty,\ 0<\lambda(\Omega)<\infty\}$（R4-C02；v0.5.1 I2【route-Debt 见证域】）。
- $\lambda(\Omega)\le1$（Debt-λ≤1）：theorem-Debt，只 A。
- $\lambda(\Omega)<\infty$：B-up 为 theorem-Debt（Debt-λfin-up，依赖上文扩展值许可）；B-lo/B-att 为 statement-Debt（Debt-λfin-stmt，因 $J\notin\mathbb R$）。
- $\lambda(\Omega)>0$：statement-Debt（Debt-λpos），全 B（归一化常数 $Z=0$，subtype `normalizer_invalid`）。

**放宽域失败见证的适用范围（R2-C06 / R3 §7 object 第 2 项）**：每个失败见证都声明它落在哪个域：W-μ ∈ 「$\mathcal D_A$/$\mathcal D_{lo}$ 去掉 $\mu(\Omega)=1$、保留有限正质量」；W-μ∞ ∈ 「$\mathcal D_{up}$ 去掉 $\mu(\Omega)<\infty$」；W-μ0 ∈「$\mathcal D_{up}$ 去掉 $\mu(\Omega)>0$」，即扩展测试域 $\mathcal D_{up}^{0}=\{0\le\mu(\Omega)<\infty,\ 0<\lambda(\Omega)<\infty\}$（v1.3 显式登记）；W-λ1 ∈「$\mathcal D_A$ 去掉 $\lambda(\Omega)\le1$」；W-λ∞、W-λ0 ∈「$\mathcal D_{lo}$/$\mathcal D_{up}$ 去掉对应 λ 条件」。没有任何见证被用来否定「概率对」这一充分条件本身。

**T1 的适用域（v1.2 正式量词，R3 §7 object 第 3 项）**：T1 的量词为「∀(Ω,𝓕)，∀ 有限正测度 μ、λ（$0<\mu(\Omega)<\infty$、$0<\lambda(\Omega)<\infty$），$\mu\ll\lambda$」；(iv) 写成 $\int f\,d\lambda=\mu(\Omega)$，不再内含 $\mu(\Omega)=1$。需要「$=1$」的边自行消费 Debt-μP。四个分支量词域中 T1 被调用的 (μ,λ) 均落在此域内（$\mathcal D_A$ 中由 $\mu\ll\lambda$、$\mu(\Omega)=1$ 推出 $\lambda(\Omega)>0$）。

## 刻意排除
- σ-有限但无限质量的测度（$\mu(\Omega)=\infty$ 或 $\lambda(\Omega)=\infty$）作为量词域；它们只作为失败见证出现（W-μ∞、W-λ∞）
- f-散度的一般理论；Rényi 散度
- Pinsker（T13）、高斯闭式、链式法则、PAC-Bayes 界本身：只作 App 或 view
- Φ 取无界函数时的 DV 变体（这时 $\int\Phi d\mu$ 可能无定义）

## 路线摘要（I3·v0.2 等价关系）
- 候选：α（Jensen 路线：在 μ 下对 $Y_\Phi=\mathbf 1_{\{f>0\}}e^\Phi/f$ 用 Jensen，work-net.md）与 β（逐点不等式 + 倾斜测度路线，work-net-beta.md）。两者 v1.2 共用同一条 B-up 独立链（截断 + 常数平移，不消费 B-lo）。
- β 另有由 X-tilt 生成、被核心边消费的中间载体 $R_{\rm tilt}$（倾斜测度 $\lambda_\Phi$）；β 对测度对 $(\mu,\lambda_\Phi)$ 使用 T1，登记为 T1 的第二个实例 `T1@λΦ`，不是新的硬 T 类型（round1b D2）。
- 有界检验函数类 𝓑 保持不变；无界 Φ 的版本不进入本对象（若写，只作 route-extension note，round1b D3）。
- 对象级只记录「存在合格路线」。α、β 是否属于同一路线等价类，按 I3【v0.2·等价关系】判定，**作者不预设**。

## 锚点
- MSE 2031062（leonbloy：对 Y=q/p 用 Jensen，f=−log）
- MSE 3640450（Raghav：Jensen 给出 DV 下界；dμ/dλ 有界时取 Φ=log dμ/dλ 取等；一般情形截断逼近）
- P7 定理 2.3（PAC-Bayes 中的 KL(Q‖P)）；P6 式(11)（r_n 收敛到 KL 差）

## changelog（v1.1 → v1.2）
- R3-C01：全局改为有限正测度；新增「分支量词域」表，B-up 显式 $0<\mu(\Omega)<\infty$、全 B 显式 $0<\lambda(\Omega)<\infty$；J 的逐分支类型条件；形式结论量词改为 $\mathcal D_*$；T1 量词正式化为有限正测度域，(iv) 改为 $\int f\,d\lambda=\mu(\Omega)$。
- R3-C02（对象侧）：Debt-μP 不含 B-up 的理由改为「B-up 独立链 + 无失败见证」；新增 Debt-μfin-up（statement）与 RD-μpos（route）；RD-μnorm 撤销（新链不依赖 $\int f=1$）。
- R3-C07：Debt-λpos / Debt-λfin-stmt 的 subtype 指向 `normalizer_invalid`。
- 执行方补充（交 R4 核）：B-up 的「扩展值许可」显式化，使 R2 E1 的 `Debt-λfin-up`=theorem-Debt 与 v0.4 I6 的 J∉ℝ 规则一致。

## changelog（v1.2 → v1.3；R4 patch_checklist_r4 object 组）
- R4-C02 / checklist object 第 1、3 项：RD-μpos 采用「核心 route-Debt + 显式扩展测试域 $\mathcal D_{up}^{0}$」（R4 给出的两种落实方式之一，依据 v0.5.1 I2【route-Debt 见证域】的「该 Debt 本身就是被移除的域条件」分支；待 R5 确认）。
- R4 §2 / checklist object 第 2 项：B-up 扩展值许可写明仅限 W-λ∞、不扩展一般 J 类型；W-λ∞@B-up 的 subtype 改为 `normalizer_invalid`。
