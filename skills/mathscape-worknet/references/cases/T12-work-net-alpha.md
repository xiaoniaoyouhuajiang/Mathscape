# T12 工作网 α（Jensen 路线：在 μ 下对密度比用 Jensen）v1.3（round4 补丁 R4-P1–P4，依据 layer-a/round4-astra.md §2、§7 与准则 v0.5.1；v1.2 备份 work-net-v1.2.md）· 前版 v1.2（round3 补丁 R3-P1–P9，依据 layer-a/round3-astra.md §2、§6、§7；v1.1 备份 work-net-v1.1.md；v1 备份 work-net-v1.md；v0 备份 work-net-v0.md）
- object_ref: /workspace/rdtx-criteria/objects/T12-kl-dv/object.md (v1.3)
- **v1.3 补丁（R4 patch_checklist_r4[] work-net 组，字面落实）**：
  - **R4-P1**（R4-C01，R4 §2 裁决「修改」）：W-λ∞@B-up 改为 `theorem_conclusion / branch_conclusion_mismatch / normalizer_invalid`（v0.5.1 I6【扩展值与 normalizer_invalid】）。
  - **R4-P2**（R4-C03）：secondary feature 消费者索引按 R4 §2 点名项补齐（C1→D-A2、D-Blo1、D-Batt⇐；C2→D-Batt⇐）。
  - **R4-P3**（checklist「必要时」项）：D-Bup1 行显式重复 $\mathcal D_{up}$ 量词（D-Bup2 行原已写）。
  - **R4-P4**（R4-C02）：RD-μpos 的 scope 显式登记扩展测试域 $\mathcal D_{up}^{0}=\{0\le\mu(\Omega)<\infty,\ 0<\lambda(\Omega)<\infty\}$，保留为核心 route-Debt（v0.5.1 I2【route-Debt 见证域】；与 object v1.3 一致，待 R5 确认）。
- criteria_ref: /workspace/rdtx-criteria/criteria/v0.5.md（v0.4 及更早版本保留）
- 变体网：work-net-beta.md（逐点不等式 + 倾斜测度路线）
- 分支：A / B-lo / B-up / B-att（B-lo、B-up 合起来是 B）。B-att 与 B-lo 在同一 (μ,λ) 上同时成立（overlap，按各自前提逐一核对）
- **分支量词域（object v1.2）**：$\mathcal D_A=\{\mu(\Omega)=1,\ \lambda(\Omega)\le1\}$；$\mathcal D_{lo}=\{\mu(\Omega)=1,\ 0<\lambda(\Omega)<\infty\}$（B-lo、B-att）；$\mathcal D_{up}=\{0<\mu(\Omega)<\infty,\ 0<\lambda(\Omega)<\infty\}$（B-up）。μ、λ 均为 (Ω,𝓕) 上的有限正测度；「概率对」只是充分条件。
- 补丁索引：v1 的 P1–P8、v1.1 的 R2-P1–P6 保留（被下列条目覆盖的部分以本版为准）。R3 新增：
  - **R3-P1**（R3-C01）：全网改用 object v1.2 的分支量词域；T1 量词改为有限正测度域，(iv) 改为 $\int f\,d\lambda=\mu(\Omega)$（不再内含 Debt-μP）；R_meas/R_dens 载体同步。
  - **R3-P2**（R3-C02，**闭合方案 = B-up 独立链**）：D-Bup2 改为「截断 + 常数平移」构造 $\Psi_n=\Phi_n+c_m$，只用新引理 $L_Z^{\lim}$（$Z_n\to\mu(\Omega)$）与 L-shift；**不再消费 D-Blo0/D-Blo1，也不消费 $\int f\,d\lambda=1$**。因此 Debt-μP 不回流 B-up，RD-μnorm 撤销；新增 statement-Debt `Debt-μfin-up`（$\mu(\Omega)<\infty$）与 route-Debt `RD-μpos`（$\mu(\Omega)>0$）。
  - **R3-P3**（R3-C06）：删除 failure 表中 `Debt-μP | — | B-up | —` 空行，改为 Debt 注释「B-up 无失败见证，故不在该 scope/consumer」。
  - **R3-P4**（R3-C03）：`Debt-λfin-stmt` 的 consumer 与语义映射移除 LZup（及其后继 $L_Z^{\lim}$）；核对传递消费者。
  - **R3-P5**（R3-C05）：evidence 表按 `edge_id` 一边一行（含 guard、等级、传递等级、来源、量词、分支、边界）。
  - **R3-P6**（R3-C07）：W-λ0、W-λ∞ 的 statement 记录 subtype 改为 `normalizer_invalid`；只有 $\infty-\infty$ 用 `integral_indeterminate`。RD 记录的 `failure_target` 更正为 `proof_route`（v1.1 误写为 `route_failure`）。
  - **R3-P7**：L2 拆为 L2-lo（B-lo/B-att）与 L2-up（B-up），使 Debt consumer 与偏序一致；D-Batt 拆为 D-Batt⇐、D-Batt⇒ 两个 edge_id；D-Batt⇒ 不再跨域引用 A 分支的 D-A0（改用 KL 定义）。
  - **R3-P8**（R3 §5 建议）：R 节点的每个 secondary feature 给出核心消费者索引。
  - **R3-P9**：语义失败键映射重写（与 β 文件内的完整副本逐行一致）。

## guard_registry（v0.3 I2(c)/I4）
谓词的定义域是所在分支的量词域（$\mathcal D_A$、$\mathcal D_{lo}$ 或 $\mathcal D_{up}$）。
| guard_id | 谓词 | 覆盖分支 | 互斥性 | 覆盖性 | overlap |
|---|---|---|---|---|---|
| G-nac | $\mu\not\ll\lambda$ | A、B-lo、B-up、B-att | 与 G-fin、G-inf 互斥 | 在每个分支域上 G-nac ∪ G-fin ∪ G-inf = 全部 (μ,λ) | 无 |
| G-fin | $\mu\ll\lambda$ 且 KL<∞ | A、B-lo、B-up、B-att | 与 G-nac、G-inf 互斥 | 同上 | 包含 G-eb |
| G-inf | $\mu\ll\lambda$ 且 KL=+∞ | A、B-lo、B-up | 与 G-nac、G-fin 互斥 | 同上 | 无 |
| G-eb | $\mu\ll\lambda$ 且 log f 本质有界 | B-att(⇐) | 非划分守卫 | 不要求覆盖 | G-eb ⊂ G-fin |
守卫不是 Debt，不进入 Debt 闭包。G-fin、G-inf、G-eb 的谓词预设 f 存在（T1(i)），故以它们为 guard 的边在 I5 传递覆盖中继承 T1(i) 的等级。

## R 节点（carrier 三元组 + 多标签，v0.3 I3）
| id | 分支 | primary_class | secondary_features | base carrier | semantic payload | operator set |
|---|---|---|---|---|---|---|
| R_meas | 全部 | C0 | — | $(\Omega,\mathcal F)$ 上的有限正测度对 (μ,λ)，限于所在分支量词域 | μ、λ、可测集、绝对连续关系、检验函数 Φ∈𝓑、常数 $c\in\mathbb R$ | 集合测度、$\int\cdot\,d\mu$、$\int\cdot\,d\lambda$、ℝ 中的 log/exp、Φ↦Φ+c |
| R_dens | A、B-lo、B-up（G-fin/G-inf）、B-att | C3 | C1、C2、C4 | $L^1_+(\lambda)$ 中满足 $\int f\,d\lambda=\mu(\Omega)\in(0,\infty)$ 的固定版本函数，带基测度 λ | 密度 f、集合 {f>0}、{f=0}、$\log f$（{f=0} 上取 −∞）、延拓 $Y_0$、$Y_\Phi$、截断 $\Phi_n$ | 点态运算、$\int\cdot\,d\lambda$、换测度恒等式 $\int h\,d\mu=\int hf\,d\lambda$（T1(ii)） |
| R_app | opt | 非核心，不判 | — | — | PAC-Bayes、Pinsker、高斯闭式 | — |

primary_class 选择依据（I3 优先序 C3 > C4 > C2 > C5 > C1 > C0）：R_dens 的 f 由 RN 变换生成并被核心边作为独立语义载荷消费（C3）；载体限制在 {μ≪λ}（C4）；{f>0}/{f=0} 分解被核心边消费（C2）；(λ,f) 与 μ 一一对应（C1）。
**secondary feature 消费者索引（R3-P8；v1.3 R4-P2 补齐）**：C1（重建 $\mu=f\lambda$）→ D-A3、D-Batt⇒、$L_Z^{\lim}$（$\int f\,d\lambda=\mu(\Omega)$）、**D-A2、D-Blo1、D-Batt⇐**（v1.3）；C2（{f>0}/{f=0} 分解）→ D-A2、D-A3、D-Blo1、D-Batt⇒、D-Bup2（μ-a.e. $f>0$）、**D-Batt⇐**（v1.3）；C4（限制到 {μ≪λ}）→ 全部以 G-fin/G-inf/G-eb 为 guard 的边（D-A1–D-A3、D-Blo1、D-Bup2、D-Batt⇐、D-Batt⇒）。
隐含载荷检查：R_dens 的 payload 是 (λ, f)，μ 可由 μ=fλ 重建（T1 重建映射），不额外携带对象。

## T 边（typed certificate）
| 字段 | T1（Radon–Nikodym） |
|---|---|
| 源载体 | R_meas（μ≪λ 的有限正测度对） |
| 目标载体 | R_dens（(λ, f)） |
| 保持引理（精确陈述） | ∀(Ω,𝓕)，∀ 有限正测度 μ、λ，$0<\mu(\Omega)<\infty$、$0<\lambda(\Omega)<\infty$，μ≪λ：(i)【外部】存在 𝓕-可测 $f:\Omega\to[0,\infty)$，∀E∈𝓕：$\mu(E)=\int_Ef\,d\lambda$；f 在 λ-a.e. 意义下唯一。(ii)【由 (i) 自包含：简单函数逼近 + 单调收敛】∀ 可测 $h:\Omega\to[0,\infty]$：$\int h\,d\mu=\int hf\,d\lambda$（约定 0·∞=0）；从而 h∈L¹(μ) ⇔ hf∈L¹(λ)，且两积分相等。(iii) $\mu(\{f=0\})=\int_{\{f=0\}}f\,d\lambda=0$，即 f>0 μ-a.e.；对 E⊂{f>0}：μ(E)=0 ⇔ λ(E)=0。(iv) $\int f\,d\lambda=\mu(\Omega)$（(i) 取 E=Ω；**不消费 Debt**，需要「=1」的边自行消费 Debt-μP） |
| 量词 | ∀(Ω,𝓕)，∀ 有限正测度 μ、λ（$0<\mu(\Omega)<\infty$、$0<\lambda(\Omega)<\infty$），μ≪λ。各分支调用点均在此域内：$\mathcal D_A$ 中 μ≪λ 且 $\mu(\Omega)=1$ ⇒ $\lambda(\Omega)>0$；$\mathcal D_{lo}\subset\mathcal D_{up}$ ⊂ 本域 |
| 适用分支 | A、B-lo、B-up（G-fin、G-inf）、B-att |
| 重建映射 | $\mu(E)=\int_Ef\,d\lambda$ |
| 证据端点 | (i) Folland《Real Analysis》2nd ed. Thm 3.8（Lebesgue–Radon–Nikodym，σ-有限情形；有限正测度是其特例），**partial·待核**（未逐字核对量词、有限测度适用域与唯一性覆盖；R1 C10、1b D8、R3-C05）；(ii)–(iv) 本行内证明，非外部·自包含 |
| 判定 | 硬 T，满足最小不可消除性（R1 §2；R3 §4 维持） |

## 派生引理（L，非 Debt）
| id | 分支 | R | 内容与证明 |
|---|---|---|---|
| L1 | A、B-lo、B-up、B-att | R_dens | **KL 良定**：在 T1 的量词域内，$\int(\log f)^-d\mu=\int f(\log f)^-d\lambda\le e^{-1}\lambda(\Omega)$（T1(ii)；$t(\log t)^-\le e^{-1}$ 对 t≥0 成立，t=0 处按 0·∞=0）。故 $\mathrm{KL}=\int\log f\,d\mu\in[-e^{-1}\lambda(\Omega),+\infty]$ 良定（μ-a.e. 基准），KL<∞ ⇔ log f∈L¹(μ)。所需 $\lambda(\Omega)<\infty$ 按分支来自：A：Debt-λ≤1；B-lo/B-att：Debt-λfin-stmt；B-up：Debt-λfin-up。记作 L1[A]、L1[lo]、L1[up] |
| L2-lo | B-lo、B-att | R_meas | **J 实值（$\mathcal D_{lo}$）**：Φ∈𝓑（Debt-bdd）⇒ $|\int\Phi d\mu|\le\|\Phi\|_\infty\mu(\Omega)=\|\Phi\|_\infty$（Debt-μP），$Z=\int e^\Phi d\lambda\in[e^{-\|\Phi\|_\infty}\lambda(\Omega),e^{\|\Phi\|_\infty}\lambda(\Omega)]\subset(0,\infty)$（Debt-λpos、Debt-λfin-stmt）⇒ $J_\mu(\Phi)\in\mathbb R$ |
| L2-up | B-up | R_meas | **J 实值（$\mathcal D_{up}$）**：Φ∈𝓑（Debt-bdd）⇒ $|\int\Phi d\mu|\le\|\Phi\|_\infty\mu(\Omega)<\infty$（Debt-μfin-up），$Z\in[e^{-\|\Phi\|_\infty}\lambda(\Omega),e^{\|\Phi\|_\infty}\lambda(\Omega)]\subset(0,\infty)$（Debt-λpos、Debt-λfin-up）⇒ $J_\mu(\Phi)\in\mathbb R$ |
| L3 | A、B-lo、B-att | R_dens | **Jensen（取 −log，带等号条件，自包含）**：设 μ 为概率测度（Debt-μP）。若 Y>0 μ-a.e.、Y∈L¹(μ)、$m:=\int Y d\mu\in(0,\infty)$、log Y∈L¹(μ)，则 $\int(-\log Y)d\mu\ge-\log m$，等号 ⇔ Y=m μ-a.e.。证明：∀y>0，$-\log y\ge-\log m-(y-m)/m$，等号 ⇔ y=m（严格凸的支撑线）；两边对 μ 积分，用 μ(Ω)=1。外部 Jensen（Rudin RCA 3rd ed. Thm 3.3）本网不消费 |
| L-shift | B-up | R_meas | **常数平移（R3-P2，自包含）**：∀(μ,λ)∈$\mathcal D_{up}$、∀Φ∈𝓑、∀c∈ℝ：Φ+c∈𝓑，且 $J_\mu(\Phi+c)=J_\mu(\Phi)+c\,(\mu(\Omega)-1)$。证明：$\int(\Phi+c)d\mu=\int\Phi d\mu+c\mu(\Omega)$（Debt-μfin-up），$\int e^{\Phi+c}d\lambda=e^cZ$，两边实值由 L2-up |
| $L_Z^{\lim}$ | B-up | R_dens | **截断归一化常数的界与极限（R3-P2，取代 v1.1 的 $L_Z^{up}$）**：设 (μ,λ)∈$\mathcal D_{up}$、μ≪λ。∀n∈ℕ₁，$\Phi_n:=\max(-n,\min(n,\log f))$（处处有定义，{f=0} 上取 −n），$Z_n:=\int e^{\Phi_n}d\lambda$。则 (a) $0<e^{-n}\lambda(\Omega)\le Z_n\le\mu(\Omega)+e^{-n}\lambda(\Omega)<\infty$；(b) $Z_n\to\mu(\Omega)$（$n\to\infty$）。证明：在 $\{f\ge e^{-n}\}$ 上 $e^{\Phi_n}=\min(f,e^n)$，在 $\{f<e^{-n}\}$（含 {f=0}）上 $e^{\Phi_n}=e^{-n}$，故处处 $e^{-n}\le e^{\Phi_n}\le f+e^{-n}$，对 λ 积分并用 T1(iv) 得 (a)（Debt-λpos、Debt-λfin-up、Debt-μfin-up）。又 $Z_n=\int\mathbf 1_{\{f\ge e^{-n}\}}\min(f,e^n)\,d\lambda+e^{-n}\lambda(f<e^{-n})$；第一项被积函数关于 n 单调不减且逐点 $\uparrow f$（f>0 处；f=0 处恒为 0），单调收敛给出 $\to\int f\,d\lambda=\mu(\Omega)$（T1(iv)）；第二项 $\le e^{-n}\lambda(\Omega)\to0$。得 (b)。**不使用 $\int f\,d\lambda=1$** |

## D 边（每条只在一个 R 内；guard 列为所消费的 guard ID）
| id | 分支 | R | guard | 前提 → 结论 |
|---|---|---|---|---|
| D-A0 | A | R_meas | G-nac | 按定义 KL=+∞>0；又 λ≪λ，故 μ≠λ。等号不成立，与「⇔ μ=λ」一致 |
| D-A1 | A | R_dens | G-inf | KL=+∞>0；μ=λ 时 f=1 λ-a.e.、KL=0，故此时 μ≠λ |
| D-A2 | A | R_dens | G-fin | 令 $Y_0=\mathbf 1_{\{f>0\}}/f$（{f=0} 上取 0；由 T1(iii)，μ-a.e. 有 $0<Y_0<\infty$）。由 T1(ii)，$\int Y_0 d\mu=\int\mathbf 1_{\{f>0\}}d\lambda=\lambda(f>0)\le\lambda(\Omega)\le1$（Debt-λ≤1），且 $\lambda(f>0)>0$（因由 T1(iv) 与 Debt-μP，$1=\mu(\Omega)=\int_{\{f>0\}}f\,d\lambda$）；$\log Y_0=-\log f$ μ-a.e.，∈L¹(μ)（L1[A]）。由 L3：$\mathrm{KL}=\int(-\log Y_0)d\mu\ge-\log\lambda(f>0)\ge0$ |
| D-A3 | A | R_dens | G-fin | 承 D-A2：KL=0 ⇒ 两个不等号都取等 ⇒ λ(f>0)=1，且 $Y_0=1$ μ-a.e.（L3 等号条件，m=1）⇒ f=1 μ-a.e. ⇒ 由 T1(iii)，在 {f>0} 上 f=1 λ-a.e.；又 λ(f=0)=λ(Ω)−λ(f>0)≤0（Debt-λ≤1）⇒ f=1 λ-a.e. ⇒ μ=λ。反之 μ=λ ⇒ f=1 λ-a.e. ⇒ KL=0（Debt-μP 用于 $\lambda(\Omega)=\mu(\Omega)=1$） |
| D-Blo0 | B-lo | R_meas | G-nac ∨ G-inf | KL=+∞（G-nac：按定义；G-inf：按守卫）；由 L2-lo，$J_\mu(\Phi)\in\mathbb R$，故 $J_\mu(\Phi)<+\infty=\mathrm{KL}$ |
| D-Blo1 | B-lo | R_dens | G-fin | Φ∈𝓑：令 $Y_\Phi=\mathbf 1_{\{f>0\}}e^\Phi/f$（{f=0} 上取 0；μ-a.e. 为正且有限）。由 T1(ii)，$\int Y_\Phi d\mu=\int_{\{f>0\}}e^\Phi d\lambda=:Z_+$，$0<e^{-\|\Phi\|_\infty}\lambda(f>0)\le Z_+\le Z:=\int e^\Phi d\lambda$（$\lambda(f>0)>0$ 同 D-A2 理由：Debt-μP、T1(iv)）；$\log Y_\Phi=\Phi-\log f$ μ-a.e.，∈L¹(μ)（L1[lo]、L2-lo）。由 L3（Debt-μP）：$\mathrm{KL}-\int\Phi d\mu\ge-\log Z_+\ge-\log Z$，即 $J_\mu(\Phi)\le\mathrm{KL}$ |
| D-Bup1 | B-up | R_meas | G-nac | 设 (μ,λ)∈$\mathcal D_{up}$（v1.3 显式量词）。取 E∈𝓕，λ(E)=0<μ(E)（$\mu(E)\le\mu(\Omega)<\infty$，Debt-μfin-up）。$\Phi_n=n\mathbf 1_E\in\mathcal B$（n∈ℕ₁），$\int e^{\Phi_n}d\lambda=\lambda(E^c)+e^n\lambda(E)=\lambda(\Omega)\in(0,\infty)$（Debt-λpos、Debt-λfin-up），故 $J_\mu(\Phi_n)=n\mu(E)-\log\lambda(\Omega)\to+\infty=\mathrm{KL}$（L2-up 保证每项实值）。不消费 Debt-λ≤1、Debt-μP |
| D-Bup2 | B-up | R_dens | G-fin ∨ G-inf | **（R3-P2 重写；不消费 B-lo、Debt-μP、$\int f\,d\lambda=1$）** 设 (μ,λ)∈$\mathcal D_{up}$，$m:=\mu(\Omega)\in(0,\infty)$（Debt-μfin-up、RD-μpos）。定义常数 $c_m:=\frac{\log m}{m-1}$（$m\ne1$），$c_1:=0$；于是 $c_m(m-1)=\log m$ 对一切 $m\in(0,\infty)$ 成立。取 $\Phi_n,Z_n$ 如 $L_Z^{\lim}$，令 $\Psi_n:=\Phi_n+c_m\in\mathcal B$（Debt-bdd）。(1) 由 T1(iii)，μ-a.e. 有 f>0、$\Phi_n\to\log f$ 且 $|\Phi_n|\le|\log f|$。(2) G-fin：log f∈L¹(μ)（L1[up]），控制收敛给出 $\int\Phi_nd\mu\to\mathrm{KL}$；G-inf：$\int\Phi_n^+d\mu\uparrow\int(\log f)^+d\mu=+\infty$（单调收敛），$\int\Phi_n^-d\mu\le\int(\log f)^-d\mu\le e^{-1}\lambda(\Omega)$（L1[up]），故 $\int\Phi_nd\mu\to+\infty$。(3) 由 $L_Z^{\lim}$(b) 与 m>0（RD-μpos），$-\log Z_n\to-\log m$。(4) 由 L-shift，$J_\mu(\Psi_n)=\int\Phi_nd\mu-\log Z_n+c_m(m-1)=\int\Phi_nd\mu-\log Z_n+\log m$。合并 (2)(3)：G-fin 时 $J_\mu(\Psi_n)\to\mathrm{KL}$，G-inf 时 $J_\mu(\Psi_n)\to+\infty=\mathrm{KL}$。单一极限 $n\to\infty$；μ、λ、m、$c_m$ 先固定 |
| D-Batt⇐ | B-att | R_dens | G-eb | 取有界版本 Φ*=log f（λ-a.e. 改动不影响 J）∈𝓑，$J_\mu(\Phi^*)=\mathrm{KL}-\log\int f\,d\lambda=\mathrm{KL}-\log\mu(\Omega)=\mathrm{KL}$（T1(iv)、Debt-μP）。又对 c∈ℝ，$J_\mu(\Phi^*+c)=J_\mu(\Phi^*)$（μ(Ω)=1）。结合 D-Blo1（B-lo → B-att，同域 $\mathcal D_{lo}$），sup 被取到 |
| D-Batt⇒ | B-att | R_dens | G-fin（由结论推出，见正文） | 若 $J_\mu(\Phi)=\mathrm{KL}$，Φ∈𝓑：由 L2-lo，KL∈ℝ；按 KL 定义 G-nac 下 KL=+∞，故不在 G-nac，且不在 G-inf，即处于 G-fin（**v1.2 不再引用 A 分支的 D-A0**）。D-Blo1 中两个不等号都取等：$Z_+=Z$ ⇒ $\int_{\{f=0\}}e^\Phi d\lambda=0$ ⇒ λ(f=0)=0；L3 等号 ⇒ $Y_\Phi=Z$ μ-a.e. ⇒ log f=Φ−log Z μ-a.e.，由 T1(iii) 与 λ(f=0)=0 也 λ-a.e. 成立 ⇒ log f 本质有界（G-eb），且 Φ=log f+log Z λ-a.e. |

## 依赖偏序（传递闭包，I4；guard 以 ID 入图；v1.2 重写）
- A：T1 ≺ L1[A] ≺ D-A2 ≺ D-A3；Debt-μP ≺ L3 ≺ D-A2；Debt-μP ≺ D-A2（λ(f>0)>0）、D-A3；Debt-λ≤1 ≺ L1[A]、D-A2、D-A3；G-fin ≺ D-A2、D-A3；G-nac ≺ D-A0；G-inf、T1 ≺ D-A1。
- B-lo：Debt-bdd、Debt-μP、Debt-λpos、Debt-λfin-stmt ≺ L2-lo ≺ D-Blo0、D-Blo1；T1 ≺ L1[lo] ≺ D-Blo1；Debt-λfin-stmt ≺ L1[lo]；Debt-μP ≺ L3 ≺ D-Blo1；Debt-μP、T1 ≺ D-Blo1；G-fin ≺ D-Blo1；G-nac、G-inf ≺ D-Blo0。
- B-up：Debt-bdd、Debt-μfin-up、Debt-λpos、Debt-λfin-up ≺ L2-up ≺ D-Bup1、L-shift；G-nac ≺ D-Bup1；Debt-μfin-up ≺ D-Bup1；T1、Debt-λpos、Debt-λfin-up、Debt-μfin-up ≺ $L_Z^{\lim}$ ≺ D-Bup2；T1 ≺ L1[up] ≺ D-Bup2；Debt-λfin-up ≺ L1[up]；L-shift ≺ D-Bup2；RD-μpos、Debt-μfin-up、Debt-bdd ≺ D-Bup2；G-fin、G-inf ≺ D-Bup2。**B-up 无跨分支消费**（v1.1 的 D-Blo0/D-Blo1 ≺ D-Bup2 已删除）；Debt-μP、Debt-λfin-stmt、L3、L2-lo 均不在 B-up 偏序中。
- B-att：Debt-μP、T1、G-eb ≺ D-Batt⇐；**D-Blo1 ≺ D-Batt⇐**（B-lo → B-att，同域 $\mathcal D_{lo}$；D-Blo1 所消费的 Debt-μP、Debt-λpos、Debt-λfin-stmt、Debt-bdd 均在 B-att scope 内，传递消费者 B-att）；L2-lo、L3、T1 ≺ D-Batt⇒；**D-Blo1（含等号分析）≺ D-Batt⇒**（B-lo → B-att，同上传递登记）。v1.2 删除 D-A0 ≺ D-Batt（A 的域 $\mathcal D_A$ 与 $\mathcal D_{lo}$ 不同，改用 KL 定义）。

## Debt 表（v0.2 I2 字段 + v0.3 I6 failure 记录；v1.2）
| id | 内容 | scope | consumer | 量词 | debt_kind | 保持其余前提不变的反例（witness_id） |
|---|---|---|---|---|---|---|
| Debt-μP | μ(Ω)=1 | A、B-lo、B-att | L2-lo、L3、D-A2、D-A3、D-Blo1、D-Batt⇐（及经 L3/D-Blo1 传递的 D-Batt⇒） | ∀μ（$\mathcal D_A$、$\mathcal D_{lo}$ 的 μ 分量） | theorem-Debt | W-μ：Ω={ω}，λ=δ_ω（概率），μ=½δ_ω（有限正，λ 不变，Φ∈𝓑）：f=½，KL=−½log2<0；$J_\mu(t)=-t/2$（常数 t）无上界。**注：B-up 无失败见证，故不在该 scope/consumer**（W-μ∈$\mathcal D_{up}$，由 D-Bup2，$m=\tfrac12$、$c_m=2\log2$ 时 $J_\mu(\Psi_n)=-\tfrac12\log2=\mathrm{KL}$；v1.2 的 B-up 链不消费 μ(Ω)=1） |
| Debt-μfin-up | μ(Ω)<∞ | B-up | L2-up、L-shift、$L_Z^{\lim}$、D-Bup1、D-Bup2 | ∀μ（$\mathcal D_{up}$ 的 μ 分量） | statement-Debt | W-μ∞：Ω=ℕ₁，λ(k)=2^{−k}（概率），μ=计数测度（μ≪λ，μ(Ω)=+∞，其余 Debt 与 RD-μpos 保持），Φ(k)=(−1)^k∈𝓑：$\int\Phi^\pm d\mu=+\infty$，$\int\Phi\,d\mu$ 为 $\infty-\infty$，$J_\mu(\Phi)$ 无定义，B-up 陈述的类型不适用 |
| Debt-λ≤1 | λ(Ω)≤1（P8；A 只需 λ(f>0)≤1） | A | L1[A]、D-A2、D-A3 | ∀λ（$\mathcal D_A$） | theorem-Debt | W-λ1：Ω={ω}，μ=δ_ω（概率），λ=2δ_ω（有限、正）：f=½，KL=−log2<0 |
| Debt-λfin-up | λ(Ω)<∞ | B-up | L1[up]、L2-up、$L_Z^{\lim}$、D-Bup1、D-Bup2 | ∀λ（$\mathcal D_{up}$） | theorem-Debt（依 object v1.2「B-up 扩展值许可」） | W-λ∞：Ω=ℕ₁，λ=计数测度，μ(k)=2^{−k}（概率，∈ μ 的 $\mathcal D_{up}$ 分量）：任意 Φ∈𝓑 有 $\int e^\Phi d\lambda=+\infty$，扩展值 $J=-\infty$，故不存在 $J_\mu(\Phi_n)\to\mathrm{KL}=-2\log2$（E1） |
| Debt-λfin-stmt | λ(Ω)<∞ | B-lo、B-att | L1[lo]、L2-lo（传递：D-Blo0、D-Blo1、D-Batt⇐、D-Batt⇒）。**不含 LZup / $L_Z^{\lim}$**（R3-C03：二者属 B-up，由 Debt-λfin-up 供给） | ∀λ（$\mathcal D_{lo}$） | statement-Debt | 同一 W-λ∞：$Z=+\infty$，$J\equiv-\infty\notin\mathbb R$，陈述类型不适用（E1/E3）；不得与 Debt-λfin-up 共用同一 Debt ID |
| Debt-λpos | λ(Ω)>0（P8） | B-lo、B-up、B-att | L2-lo、L2-up、$L_Z^{\lim}$、D-Bup1 | ∀λ（$\mathcal D_{lo}$、$\mathcal D_{up}$） | statement-Debt | W-λ0：λ=0（有限），μ=δ_ω：G-nac，KL=+∞；$Z=\int e^\Phi d\lambda=0$，$J_\mu(\Phi)=+\infty\notin\mathbb R$，与 object 中 J 的类型声明不符；等式 sup=KL=+∞ 在 $[0,+\infty]$ 中仍成立（不断言自然扩展失败） |
| Debt-bdd | 检验函数类取有界可测 𝓑 | B-lo、B-up、B-att | L2-lo、L2-up、L-shift、D-Blo1、D-Bup1、D-Bup2（$\Psi_n\in\mathcal B$）、D-Batt⇐、D-Batt⇒ | ∀Φ∈𝓑 | statement-Debt | W-bdd：Ω=ℤ∖{0}，λ(k)=c₁e^{−2\|k\|}，μ(k)=c₂k^{−2}（均为概率，μ≪λ），Φ(k)=\|k\|（k>0）、−\|k\|（k<0）：$\int e^\Phi d\lambda<\infty$，但 $\int\Phi^\pm d\mu=+\infty$，$J_\mu(\Phi)$ 无定义。不据此断言自然扩展的结论失败 |

### route-Debt（v1.2）
| id | 内容 | scope | consumer | 量词 | debt_kind | 反例 |
|---|---|---|---|---|---|---|
| RD-μpos | μ(Ω)>0 | B-up（仅本路线） | D-Bup2（$c_m$ 的定义与 $-\log Z_n\to-\log m$） | ∀μ（$\mathcal D_{up}$）；见证测试域 = 扩展域 $\mathcal D_{up}^{0}=\{0\le\mu(\Omega)<\infty,\ 0<\lambda(\Omega)<\infty\}$（v1.3，R4-C02；只去掉 μ(Ω)>0，其余前提不变） | route-Debt | W-μ0：Ω={ω}，λ=δ_ω，μ=0（μ≪λ，f=0，KL=0，G-fin；其余 Debt 保持）：$c_0$ 无定义，$Z_n=e^{-n}\to0$，$J_\mu(\Phi_n)=n\to+\infty\ne\mathrm{KL}$，本路线失效；但常数 Φ≡0 给出 $J_\mu=0=\mathrm{KL}$，**B-up 陈述成立**，故只是 route-Debt |
| ~~RD-μnorm~~ | ~~$\int f\,d\lambda=1$~~ | — | — | — | **v1.2 撤销** | v1.1 的 $L_Z^{up}$/D-Bup2 截断路线依赖它；v1.2 的 $L_Z^{\lim}$/D-Bup2 不再依赖（$Z_n\to\mu(\Omega)$ 由平移 $c_m$ 吸收），无 consumer，按 I2(b) 删除 |

注（R2→R3）：Debt-λ 按分支与 debt_kind 原子化（E1）：A 用 Debt-λ≤1（theorem）；B-up 用 Debt-λfin-up（theorem）；B-lo/B-att 用 Debt-λfin-stmt（statement）；Debt-λpos 全 B 为 statement（E3）。Debt-μ 按分支原子化（R3-C01/C02）：A/B-lo/B-att 用 Debt-μP（theorem）；B-up 用 Debt-μfin-up（statement）+ RD-μpos（route）。B-up 关于 μ(Ω)=1 **无失败见证**，按 v0.4 I6 只在本注记录，不进 failure 表。

### failure 记录（I6；按分支拆分，同一见证共享 witness_id；v1.2）
| Debt | witness_id | branch | failure_target | failure_type | failure_subtype | 说明 |
|---|---|---|---|---|---|---|
| Debt-μP | W-μ | A | theorem_conclusion | branch_conclusion_mismatch | inequality_sign_wrong | KL=−½log2<0 |
| Debt-μP | W-μ | B-lo | theorem_conclusion | branch_conclusion_mismatch | variational_value_wrong | t=−2：J=1>KL |
| Debt-μP | W-μ | B-att | theorem_conclusion | branch_conclusion_mismatch | variational_value_wrong | 右侧成立，但 sup=+∞ 不被取到 |
| Debt-μfin-up | W-μ∞ | B-up | statement_applicability | premise_inapplicable | integral_indeterminate | $\int\Phi\,d\mu=\infty-\infty$（Φ=(−1)^k） |
| Debt-λ≤1 | W-λ1 | A | theorem_conclusion | branch_conclusion_mismatch | inequality_sign_wrong | KL=−log2<0 |
| Debt-λfin-up | W-λ∞ | B-up | theorem_conclusion | branch_conclusion_mismatch | normalizer_invalid | 扩展值下 sup=−∞≠KL=−2log2（object 扩展值许可，仅限本见证）；根因 Z=+∞（R4 裁决，v0.5.1 I6；v1.2 误记 variational_value_wrong） |
| Debt-λfin-stmt | W-λ∞ | B-lo | statement_applicability | premise_inapplicable | normalizer_invalid | $Z=+\infty$，J=−∞∉ℝ（R3-C07） |
| Debt-λfin-stmt | W-λ∞ | B-att | statement_applicability | premise_inapplicable | normalizer_invalid | $Z=+\infty$，J≡−∞∉ℝ，平凡「取到」不算 equality_condition_wrong（E3、R3-C07） |
| Debt-λpos | W-λ0 | B-lo | statement_applicability | premise_inapplicable | normalizer_invalid | $Z=0$，log Z=−∞，J=+∞∉ℝ（R3-C07） |
| Debt-λpos | W-λ0 | B-up | statement_applicability | premise_inapplicable | normalizer_invalid | 同上 |
| Debt-λpos | W-λ0 | B-att | statement_applicability | premise_inapplicable | normalizer_invalid | 同上；Debt-λpos 保持 statement-Debt（E3） |
| Debt-bdd | W-bdd | B-lo | statement_applicability | premise_inapplicable | integral_indeterminate | $\int\Phi d\mu$ 为 ∞−∞ |
| Debt-bdd | W-bdd | B-up | statement_applicability | premise_inapplicable | integral_indeterminate | 同上 |
| Debt-bdd | W-bdd | B-att | statement_applicability | premise_inapplicable | integral_indeterminate | 同上 |
| RD-μpos | W-μ0 | B-up（route） | proof_route | route_failure | normalization_route_failure | 平移截断构造需 $\lim Z_n=\mu(\Omega)>0$；μ=0 时失效，陈述本身仍成立；见证在扩展测试域 $\mathcal D_{up}^{0}$（v1.3） |

（v1.2 删除：`Debt-μP | — | B-up | —` 空行（R3-C06）；`RD-μnorm` 行（随 RD-μnorm 撤销）。）

α 的 route-Debt：RD-μpos。KL 有限/无穷分情形仍是 guard，不是 Debt。

## 证据端点（I5 / v0.4【逐边 evidence】；v1.2 一边一行）
「本边等级」只看该边自身推理；「传递等级」按 I5【v0.2·传递覆盖】：凡（直接或经 L、guard 谓词）消费 T1(i) 的边，传递等级为 partial·待核。
| edge_id | guard | 本边等级 | 传递等级 | 来源 | 覆盖量词 | 分支 | 边界 |
|---|---|---|---|---|---|---|---|
| T1 | —（被 G-fin/G-inf/G-eb 边消费） | (i) partial·待核；(ii)–(iv) complete·非外部·自包含 | partial·待核 | (i) Folland *Real Analysis* 2nd ed. Thm 3.8；(ii)–(iv) 本网 T 边行内 | ∀ 有限正测度 μ≪λ，$0<\mu(\Omega),\lambda(\Omega)<\infty$ | A、B-lo、B-up、B-att | {f=0} 零集；0·∞=0；唯一性 λ-a.e. |
| L1 | — | complete·非外部·自包含 | partial·待核（T1(ii)） | 本网 L1 行 | T1 量词域内；λ(Ω)<∞ 按分支 | A、B-lo、B-up、B-att | t=0 处 0·∞=0；KL=+∞ 允许 |
| L2-lo | — | complete·非外部·自包含 | complete | 本网 L2-lo 行 | ∀(μ,λ)∈$\mathcal D_{lo}$，∀Φ∈𝓑 | B-lo、B-att | ‖Φ‖∞=0 时 Z=λ(Ω) |
| L2-up | — | complete·非外部·自包含 | complete | 本网 L2-up 行 | ∀(μ,λ)∈$\mathcal D_{up}$，∀Φ∈𝓑 | B-up | 同上 |
| L3 | — | complete·非外部·自包含 | complete | 本网 L3 行（支撑线论证） | ∀ 概率 μ，∀Y 满足所列可积性 | A、B-lo、B-att | 等号 ⇔ Y=m μ-a.e. |
| L-shift | — | complete·非外部·自包含 | complete | 本网 L-shift 行 | ∀(μ,λ)∈$\mathcal D_{up}$，∀Φ∈𝓑，∀c∈ℝ | B-up | μ(Ω)=1 时 J 平移不变 |
| $L_Z^{\lim}$ | — | complete·非外部·自包含 | partial·待核（T1(iv)） | 本网 $L_Z^{\lim}$ 行（单调收敛） | ∀(μ,λ)∈$\mathcal D_{up}$，μ≪λ，∀n∈ℕ₁ | B-up | {f=0} 上 $e^{\Phi_n}=e^{-n}$；单一极限 n→∞ |
| D-A0 | G-nac | complete·非外部·自包含 | complete | 本网 D-A0 行（KL 定义） | ∀(μ,λ)∈$\mathcal D_A$，μ≪̸λ | A | — |
| D-A1 | G-inf | complete·非外部·自包含 | partial·待核（G-inf 谓词预设 T1(i)） | 本网 D-A1 行 | ∀(μ,λ)∈$\mathcal D_A$，G-inf | A | KL=+∞ |
| D-A2 | G-fin | complete·非外部·自包含 | partial·待核（T1） | 本网 D-A2 行 | ∀(μ,λ)∈$\mathcal D_A$，G-fin | A | λ(f>0)∈(0,1] |
| D-A3 | G-fin | complete·非外部·自包含 | partial·待核（T1） | 本网 D-A3 行 | ∀(μ,λ)∈$\mathcal D_A$，G-fin | A | λ(Ω)<1 时等号不成立 |
| D-Blo0 | G-nac ∨ G-inf | complete·非外部·自包含 | G-nac 部分 complete；G-inf 部分 partial·待核（谓词预设 T1(i)） | 本网 D-Blo0 行 | ∀(μ,λ)∈$\mathcal D_{lo}$，∀Φ∈𝓑 | B-lo | KL=+∞ |
| D-Blo1 | G-fin | complete·非外部·自包含 | partial·待核（T1） | 本网 D-Blo1 行 | ∀(μ,λ)∈$\mathcal D_{lo}$，G-fin，∀Φ∈𝓑 | B-lo | {f=0} 上 $Y_\Phi=0$；$Z_+\le Z$ |
| D-Bup1 | G-nac | complete·非外部·自包含 | complete | 本网 D-Bup1 行 | ∀(μ,λ)∈$\mathcal D_{up}$，G-nac | B-up | n∈ℕ₁，n→∞ |
| D-Bup2 | G-fin ∨ G-inf | complete·非外部·自包含 | partial·待核（T1） | 本网 D-Bup2 行（控制/单调收敛 + $L_Z^{\lim}$ + L-shift） | ∀(μ,λ)∈$\mathcal D_{up}$，G-fin 或 G-inf | B-up | m=1 时 $c_1=0$；G-inf 时极限为 +∞；单一极限 n→∞ |
| D-Batt⇐ | G-eb | complete·非外部·自包含 | partial·待核（T1(iv)） | 本网 D-Batt⇐ 行 | ∀(μ,λ)∈$\mathcal D_{lo}$，G-eb | B-att | Φ* 取有界版本 |
| D-Batt⇒ | G-fin（结论内推出） | complete·非外部·自包含 | partial·待核（T1） | 本网 D-Batt⇒ 行 | ∀(μ,λ)∈$\mathcal D_{lo}$，∀Φ∈𝓑 且 J=KL | B-att | λ(f=0)=0；Φ=log f+log Z λ-a.e. |
| Debt/RD 失败记录 | — | complete·非外部·自包含 | complete | 本网 Debt 表、route-Debt 表中的具体反例 | 各 witness | 见 failure 表 | 见 failure 表 |

## 可达性反例（B-att 必要性的见证 X，非 Debt）
- Ω=ℕ₁，λ(k)=2^{−k}，μ(k)=2·3^{−k}：f(k)=2(2/3)^k，log f→−∞ 无界，KL<∞；由 D-Batt⇒，sup 不被 𝓑 中任何元素取到。

## 语义失败键映射（v0.3 I6：键 = failure_target–branch–conclusion（Debt）；v1.2 重写，β 文件内有逐行相同的完整副本）
| 语义键 | α 消费者 | β 消费者 |
|---|---|---|
| theorem_conclusion–A–KL≥0 且等号⇔μ=λ（Debt-μP） | L3、D-A2、D-A3 | D-βA1、D-βA2 |
| theorem_conclusion–A–同上（Debt-λ≤1） | L1[A]、D-A2、D-A3 | L1[A]、D-βA1、D-βA2 |
| theorem_conclusion–B-lo–J≤KL（Debt-μP） | L2-lo、L3、D-Blo1 | L2-lo、D-βA1@λΦ、D-βBlo1 |
| theorem_conclusion–B-att–可达性（Debt-μP） | D-Batt⇐、D-Batt⇒（经 L3、D-Blo1） | D-βBatt⇐、D-βBatt⇒（经 D-βA2@λΦ、D-βBlo1） |
| theorem_conclusion–B-up–J_n→KL（Debt-λfin-up） | L1[up]、L2-up、$L_Z^{\lim}$、D-Bup1、D-Bup2 | L1[up]、L2-up、$L_Z^{\lim}$、D-Bup1、D-Bup2 |
| statement_applicability–B-up–J∈ℝ（Debt-μfin-up） | L2-up、L-shift、$L_Z^{\lim}$、D-Bup1、D-Bup2 | L2-up、L-shift、$L_Z^{\lim}$、D-Bup1、D-Bup2 |
| statement_applicability–B-lo–J∈ℝ（Debt-λfin-stmt） | L1[lo]、L2-lo（→D-Blo0、D-Blo1） | L1[lo]、L2-lo、Lβ-Z（→D-βBlo0、D-βBlo1） |
| statement_applicability–B-att–J∈ℝ（Debt-λfin-stmt） | L1[lo]、L2-lo（→D-Batt⇐、D-Batt⇒） | L1[lo]、L2-lo、Lβ-Z（→D-βBatt⇐、D-βBatt⇒） |
| statement_applicability–B-lo–J∈ℝ（Debt-λpos） | L2-lo | L2-lo、Lβ-Z |
| statement_applicability–B-up–J∈ℝ（Debt-λpos） | L2-up、$L_Z^{\lim}$、D-Bup1 | L2-up、$L_Z^{\lim}$、D-Bup1 |
| statement_applicability–B-att–J∈ℝ（Debt-λpos） | L2-lo | L2-lo、Lβ-Z |
| statement_applicability–B-lo–J 良定（Debt-bdd） | L2-lo、D-Blo1 | L2-lo、Lβ-Z、D-βBlo1 |
| statement_applicability–B-up–J 良定（Debt-bdd） | L2-up、L-shift、D-Bup1、D-Bup2 | L2-up、L-shift、D-Bup1、D-Bup2 |
| statement_applicability–B-att–J 良定（Debt-bdd） | L2-lo、D-Batt⇐、D-Batt⇒ | L2-lo、Lβ-Z、D-βBatt⇐、D-βBatt⇒ |
| proof_route–B-up–平移截断构造（RD-μpos） | D-Bup2 | D-Bup2 |
（LZup / $L_Z^{\lim}$ 不出现在任何 Debt-λfin-stmt 键中，R3-C03。）

## diagram
```
R_meas ──T1 (RN, 硬; 有限正测度域)──▶ R_dens
  │ D-A0, D-Blo0, D-Bup1, L2-lo, L2-up, L-shift
  │                              │ L1, L3, L_Z^lim
  │                              ├─ D-A2 ─▶ D-A3                 (A)
  │                              ├─ D-Blo1 ───────┐             (B-lo)
  │                              ├─ D-Bup2  (L_Z^lim + L-shift;  B-up 独立链, 无 B-lo 输入)
  │                              └─ D-Batt⇐ / D-Batt⇒ ◀─┘        (B-lo→B-att, 同域 D_lo)
guards: G-nac | G-fin ⊃ G-eb | G-inf
```

## 非核心
| id | 类型 | 内容 |
|---|---|---|
| App1 | App | PAC-Bayes：取 Φ=λ·(泛化差)，再用 Markov（P7 定理 2.3） |
| App2 | App | Pinsker（T13）、高斯 KL 闭式 |
| V1 | view | report 中的「T：R_density→R_dual」读法：本网把变分表示当作结论 B，证明中没有把对偶表示当载体使用 |
