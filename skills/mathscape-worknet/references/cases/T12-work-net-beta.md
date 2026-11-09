# T12 工作网 β（逐点不等式 + 倾斜测度路线）v1.3（round4 补丁 R4-Q1–Q4，依据 layer-a/round4-astra.md §2、§7 与准则 v0.5.1；v1.2 备份 work-net-beta-v1.2.md）· 前版 v1.2（round3 补丁 R3-Q1–Q9，依据 layer-a/round3-astra.md §2、§6、§7；v1.1 备份 work-net-beta-v1.1.md；v1 备份 work-net-beta-v1.md；v0 备份 work-net-beta-v0.md）
- object_ref: /workspace/rdtx-criteria/objects/T12-kl-dv/object.md (v1.3)
- **v1.3 补丁（R4 patch_checklist_r4[] work-net-beta 组，字面落实）**：
  - **R4-Q1**（R4-C01，R4 §2 裁决「修改」）：W-λ∞@B-up 改为 `theorem_conclusion / branch_conclusion_mismatch / normalizer_invalid`（v0.5.1 I6【扩展值与 normalizer_invalid】）。
  - **R4-Q2**（R4-C03）：secondary feature 消费者索引按 R4 §2 点名项补齐（C1→D-βBlo1；C2→D-βBatt⇐、D-βBatt⇒）。
  - **R4-Q3**（checklist「必要时」项）：D-Bup1 行显式重复 $\mathcal D_{up}$ 量词（D-Bup2 行原已写）。
  - **R4-Q4**（R4-C02）：RD-μpos 的 scope 显式登记扩展测试域 $\mathcal D_{up}^{0}=\{0\le\mu(\Omega)<\infty,\ 0<\lambda(\Omega)<\infty\}$，保留为核心 route-Debt（v0.5.1 I2【route-Debt 见证域】；与 object v1.3 一致，待 R5 确认）。
- criteria_ref: /workspace/rdtx-criteria/criteria/v0.5.md（v0.4 及更早版本保留）
- 变体网：work-net.md（α，Jensen 路线）
- 分支：A / B-lo / B-up / B-att（B-att 与 B-lo 在同一 (μ,λ) 上 overlap，逐一核对）
- **分支量词域（object v1.2）**：$\mathcal D_A=\{\mu(\Omega)=1,\ \lambda(\Omega)\le1\}$；$\mathcal D_{lo}=\{\mu(\Omega)=1,\ 0<\lambda(\Omega)<\infty\}$（B-lo、B-att）；$\mathcal D_{up}=\{0<\mu(\Omega)<\infty,\ 0<\lambda(\Omega)<\infty\}$（B-up）。μ、λ 为有限正测度；「概率对」只是充分条件。
- 补丁索引：v1 的 Q1–Q8、v1.1 的 R2-Q1–Q6 保留（被下列条目覆盖处以本版为准）。R3 新增：
  - **R3-Q1**（R3-C01）：全网改用分支量词域；T1 量词改为有限正测度域，(iv) 改为 $\int f\,d\lambda=\mu(\Omega)$；R_meas/R_dens 载体同步。
  - **R3-Q2**（R3-C02，同步 α R3-P2）：B-up 改用与 α 相同的独立链（$L_Z^{\lim}$、L-shift、D-Bup2：截断 + 常数平移），**删除 D-βBlo0/D-βBlo1 ≺ D-Bup2**；RD-μnorm 撤销；新增 Debt-μfin-up、RD-μpos。
  - **R3-Q3**（R3-C04）：guard_registry 新增实例 guard `G-nac@λΦ`、`G-fin@λΦ`、`G-inf@λΦ`、`G-eb@λΦ`，各含 predicate、实例化来源、覆盖分支、互斥性、覆盖性、overlap、与源 guard 的关系。
  - **R3-Q4**（R3-C06）：删除 `Debt-μP | — | B-up | —` 空行，改为 Debt 注释。
  - **R3-Q5**（R3-C05/C08）：evidence 表一边一行；T1@λΦ、D-βA1@λΦ、D-βA2@λΦ、D-βBlo1、D-βBatt⇐、D-βBatt⇒ 各有独立行（含实例级量词、分支、边界、guard）。
  - **R3-Q6**（R3-C07）：归一化非法相关 subtype 改为 `normalizer_invalid`；RD 记录 `failure_target` 更正为 `proof_route`。
  - **R3-Q7**（R3-C08）：语义失败键映射完整复制进本文件（与 α 逐行一致），不再回指 α。
  - **R3-Q8**（R3-C03 同步）：Lβ-Z、L2-lo 只属 B-lo/B-att，`Debt-λfin-up` 的 consumer 中删去 Lβ-Z；L2 拆为 L2-lo/L2-up；D-βBatt 拆为 D-βBatt⇐、D-βBatt⇒；D-βBatt⇐ 不再引用 G-eb@λΦ（该边不经倾斜测度）；D-βBatt⇒ 不再引用 A 分支的 D-A0。
  - **R3-Q9**（R3 §5 建议）：secondary feature 消费者索引。
- 本文件自包含。

## guard_registry（v0.3 I2(c)/I4；v0.4【实例 guard】；v0.5【实例 guard 单独登记】）
### 源 guard（谓词定义域为所在分支的量词域）
| guard_id | 谓词 | 覆盖分支 | 互斥性 | 覆盖性 | overlap |
|---|---|---|---|---|---|
| G-nac | $\mu\not\ll\lambda$ | A、B-lo、B-up、B-att | 与 G-fin、G-inf 互斥 | 在每个分支域上 G-nac ∪ G-fin ∪ G-inf = 全部 (μ,λ) | 无 |
| G-fin | $\mu\ll\lambda$ 且 KL(μ‖λ)<∞ | A、B-lo、B-up、B-att | 与 G-nac、G-inf 互斥 | 同上 | 包含 G-eb |
| G-inf | $\mu\ll\lambda$ 且 KL(μ‖λ)=+∞ | A、B-lo、B-up | 与 G-nac、G-fin 互斥 | 同上 | 无 |
| G-eb | $\mu\ll\lambda$ 且 log f 本质有界 | B-att（D-βBatt⇐ 的前提；D-βBatt⇒ 的结论） | 非划分守卫 | 不要求覆盖 | G-eb ⊂ G-fin |

### 实例 guard（测度对 $(\mu,\lambda_\Phi)$；R3-Q3）
实例化来源：X-tilt，$\lambda_\Phi(E)=Z^{-1}\int_Ee^\Phi d\lambda$，Φ∈𝓑，$Z=\int e^\Phi d\lambda$。谓词定义域：$\{(\mu,\lambda,\Phi):(\mu,\lambda)\in\mathcal D_{lo},\ \Phi\in\mathcal B\}$（B-lo、B-att 共用）。记 $g:=d\mu/d\lambda_\Phi$（T1@λΦ）。
| guard_id | 谓词 | 实例化来源 | 覆盖分支 | 互斥性 | 覆盖性 | overlap | 与源 guard 的关系 |
|---|---|---|---|---|---|---|---|
| G-nac@λΦ | $\mu\not\ll\lambda_\Phi$ | X-tilt + 源 G-nac | B-lo、B-att（登记以完成实例覆盖；无核心消费边） | 与 G-fin@λΦ、G-inf@λΦ 互斥 | G-nac@λΦ ∪ G-fin@λΦ ∪ G-inf@λΦ = 上述定义域 | 无 | ⇔ G-nac（Lβ-tilt：$\lambda_\Phi$ 与 λ 互相绝对连续） |
| G-fin@λΦ | $\mu\ll\lambda_\Phi$ 且 KL(μ‖λ_Φ)<∞ | X-tilt + 源 G-fin | B-lo（T1@λΦ、D-βA1@λΦ、D-βBlo1）、B-att（D-βA2@λΦ、D-βBatt⇒） | 与 G-nac@λΦ、G-inf@λΦ 互斥 | 同上 | 包含 G-eb@λΦ | ⇔ G-fin：μ≪λ 时 $g=Ze^{-\Phi}f$（λ_Φ-a.e.），$\mathrm{KL}(\mu\|\lambda_\Phi)=\mathrm{KL}(\mu\|\lambda)+\log Z-\int\Phi\,d\mu$，后两项有限（Lβ-Z、L2-lo），故两者同有限 |
| G-inf@λΦ | $\mu\ll\lambda_\Phi$ 且 KL(μ‖λ_Φ)=+∞ | X-tilt + 源 G-inf | B-lo、B-att（登记以完成实例覆盖；无核心消费边） | 与 G-nac@λΦ、G-fin@λΦ 互斥 | 同上 | 无 | ⇔ G-inf（同上恒等式在 $(-\infty,+\infty]$ 中成立） |
| G-eb@λΦ | $\mu\ll\lambda_\Phi$ 且 log g 本质有界 | X-tilt + 源 G-eb | B-att（D-βBatt⇒ 的中间结论：g=1） | 非划分守卫 | 不要求覆盖 | G-eb@λΦ ⊂ G-fin@λΦ | ⇔ G-eb：$\log g=\log f+\log Z-\Phi$，Φ 有界、Z∈(0,∞) |
守卫不是 Debt，不进入 Debt 闭包。以 G-fin/G-inf/G-eb 及其实例为 guard 的边，在 I5 传递覆盖中继承 T1(i)（或 T1@λΦ(i)）的等级。

## R 节点（carrier 三元组 + 多标签，v0.3 I3）
| id | 分支 | primary_class | secondary_features | base carrier | semantic payload | operator set |
|---|---|---|---|---|---|---|
| R_meas | 全部 | C0 | — | $(\Omega,\mathcal F)$ 上的有限正测度对 (μ,λ)，限于所在分支量词域 | μ、λ、可测集、绝对连续关系、检验函数 Φ∈𝓑、常数 c∈ℝ | 集合测度、$\int\cdot\,d\mu$、$\int\cdot\,d\lambda$、ℝ 中的 log/exp、Φ↦Φ+c |
| R_dens | A、B-lo、B-up（G-fin/G-inf）、B-att | C3 | C1、C2、C4 | $L^1_+(\lambda)$ 中满足 $\int f\,d\lambda=\mu(\Omega)\in(0,\infty)$ 的固定版本函数，带基测度 λ | f、{f>0}、{f=0}、log f（{f=0} 上取 −∞）、$f\log f$（{f=0} 上取 0）、截断 $\Phi_n$ | 点态运算、$\int\cdot\,d\lambda$、T1(ii) 换测度 |
| R_tilt | B-lo、B-att | C3 | C5 | 与 λ 互相绝对连续的概率测度 $\lambda_\Phi$（Φ∈𝓑，$(\mu,\lambda)\in\mathcal D_{lo}$） | $\lambda_\Phi$、归一化常数 $Z\in(0,\infty)$、显式密度 $d\lambda_\Phi/d\lambda=e^\Phi/Z$、密度 $g=d\mu/d\lambda_\Phi$ | $\int\cdot\,d\lambda_\Phi$、KL(μ‖λ_Φ)、对 (μ,λ_Φ) 实例化的 A |
| R_app | opt | 非核心，不判 | — | — | PAC-Bayes 等 | — |

primary_class 依据：R_tilt 由 X-tilt 从 (λ,Φ) 生成，并被 D-βBlo1、D-βBatt⇒ 作为独立测度载体消费（C3）；归一化把 $e^\Phi\lambda$ 封装为概率测度（C5，round1b D2）。在其上重新应用 T1 时登记实例签名 T1@λΦ，不新增硬 T 类型。
**secondary feature 消费者索引（R3-Q9；v1.3 R4-Q2 补齐）**：R_dens：C1（重建 μ=fλ）→ D-βA1（$\int f\,d\lambda=\mu(\Omega)$）、D-βBatt⇐、$L_Z^{\lim}$、**D-βBlo1**（v1.3）；C2（{f>0}/{f=0}）→ D-βA1/D-βA2（{f=0} 上 $f\log f=0$）、D-Bup2（μ-a.e. f>0）、**D-βBatt⇐、D-βBatt⇒**（v1.3）；C4（限制 {μ≪λ}）→ 以 G-fin/G-inf/G-eb 为 guard 的边（D-A1、D-βA1、D-βA2、D-βBlo1、D-Bup2、D-βBatt⇐、D-βBatt⇒）。R_tilt：C5（归一化封装，$\lambda_\Phi(\Omega)=1$）→ D-βA1@λΦ、D-βA2@λΦ（满足 A 的 λ 实例前提），经它们到 D-βBlo1、D-βBatt⇒。

## T 边（typed certificate）
### T1（Radon–Nikodym），与 α 逐字段相同
| 字段 | T1（Radon–Nikodym） |
|---|---|
| 源载体 | R_meas（μ≪λ 的有限正测度对） |
| 目标载体 | R_dens（(λ, f)） |
| 保持引理（精确陈述） | ∀(Ω,𝓕)，∀ 有限正测度 μ、λ，$0<\mu(\Omega)<\infty$、$0<\lambda(\Omega)<\infty$，μ≪λ：(i)【外部】存在 𝓕-可测 $f:\Omega\to[0,\infty)$，∀E∈𝓕：$\mu(E)=\int_Ef\,d\lambda$；f 在 λ-a.e. 意义下唯一。(ii)【由 (i) 自包含：简单函数逼近 + 单调收敛】∀ 可测 $h:\Omega\to[0,\infty]$：$\int h\,d\mu=\int hf\,d\lambda$（约定 0·∞=0）；从而 h∈L¹(μ) ⇔ hf∈L¹(λ)，且两积分相等。(iii) $\mu(\{f=0\})=0$，即 f>0 μ-a.e.；对 E⊂{f>0}：μ(E)=0 ⇔ λ(E)=0。(iv) $\int f\,d\lambda=\mu(\Omega)$（**不消费 Debt**） |
| 量词 | ∀(Ω,𝓕)，∀ 有限正测度 μ、λ（$0<\mu(\Omega)<\infty$、$0<\lambda(\Omega)<\infty$），μ≪λ（各分支调用点均在此域内，理由同 α） |
| 适用分支 | A、B-lo、B-up（G-fin、G-inf）、B-att |
| 重建映射 | $\mu(E)=\int_Ef\,d\lambda$ |
| 证据端点 | (i) Folland《Real Analysis》2nd ed. Thm 3.8（σ-有限情形；有限正测度为特例），**partial·待核**；(ii)–(iv) 本行内证明，非外部·自包含 |
| 判定 | 硬 T，满足最小不可消除性（R1 §2；R3 §4 维持） |

### T1@λΦ（T1 的第二个实例，不是新的硬 T 类型；R3-Q5 实例级字段）
| 字段 | 内容 |
|---|---|
| 源载体 | R_tilt × R_meas：测度对 $(\mu,\lambda_\Phi)$，μ≪λ_Φ（由 μ≪λ 与 λ≪λ_Φ 得，Lβ-tilt） |
| 目标载体 | R_dens 的实例：$(\lambda_\Phi,g)$，$g=d\mu/d\lambda_\Phi$ |
| guard | G-fin@λΦ（本网唯一消费者 D-βBlo1、D-βBatt⇒ 都处于 G-fin@λΦ；T1@λΦ 本身只需 ¬G-nac@λΦ） |
| 保持引理 | T1 的 (i)–(iv)，以 λ:=λ_Φ 实例化：(iv) 给 $\int g\,d\lambda_\Phi=\mu(\Omega)=1$（Debt-μP 由消费者 D-βBlo1 在 B-lo/B-att scope 内供给） |
| 量词 | ∀Φ∈𝓑，∀ 概率测度 μ，∀ 有限正测度 λ（$0<\lambda(\Omega)<\infty$），μ≪λ，且 (μ,λ,Φ) 满足 G-fin、G-fin@λΦ。此时 $\lambda_\Phi$ 是概率测度（Lβ-tilt），故 $(\mu,\lambda_\Phi)$ 落在 T1 的有限正测度量词域内 |
| 适用分支 | B-lo（D-βBlo1）、B-att（D-βBatt⇒，传递消费者） |
| 重建映射 | $\mu(E)=\int_Eg\,d\lambda_\Phi$；显式版本 $g=Ze^{-\Phi}f$（D-βBlo1） |
| 边界 | g 的零集 = f 的零集；λ_Φ-a.e. 与 λ-a.e. 等价（互相绝对连续） |
| 证据端点 | (i) 同 T1：Folland Thm 3.8，**partial·待核**（实例的 λ_Φ 为概率测度，在 Thm 3.8 的 σ-有限假设内）；(ii)–(iv) 自包含 |
| 判定 | 实例签名；按 I3 最小不可消除性不另列硬 T |

## X 宏
| id | 分支 | 类型 | 展开 |
|---|---|---|---|
| X-tilt | B-lo、B-att | X-定义（见证构造），生成 R_tilt；被 D-βBlo1、D-βBatt⇒ 消费，不能降为 view（R1 §2） | Φ∈𝓑：$Z:=\int e^\Phi d\lambda$，$\lambda_\Phi(E):=Z^{-1}\int_Ee^\Phi d\lambda$ |

## 派生引理（L，非 Debt）
| id | 分支 | 载体 | 内容 |
|---|---|---|---|
| L1 | A、B-lo、B-up、B-att | R_dens | **KL 良定**：T1 量词域内，$\int(\log f)^-d\mu=\int f(\log f)^-d\lambda\le e^{-1}\lambda(\Omega)$（T1(ii)）。故 $\mathrm{KL}\in[-e^{-1}\lambda(\Omega),+\infty]$ 良定（μ-a.e.），KL<∞ ⇔ log f∈L¹(μ)。λ(Ω)<∞ 按分支来自：A：Debt-λ≤1；B-lo/B-att：Debt-λfin-stmt；B-up：Debt-λfin-up。记 L1[A]、L1[lo]、L1[up]。实例 L1@λΦ（$\lambda_\Phi(\Omega)=1$，由 Lβ-tilt）供 D-βA1@λΦ |
| L2-lo | B-lo、B-att | R_meas | **J 实值（$\mathcal D_{lo}$）**：Φ∈𝓑（Debt-bdd）⇒ $|\int\Phi d\mu|\le\|\Phi\|_\infty$（Debt-μP），$\int e^\Phi d\lambda\in[e^{-\|\Phi\|_\infty}\lambda(\Omega),e^{\|\Phi\|_\infty}\lambda(\Omega)]\subset(0,\infty)$（Debt-λpos、Debt-λfin-stmt）⇒ $J_\mu(\Phi)\in\mathbb R$ |
| L2-up | B-up | R_meas | **J 实值（$\mathcal D_{up}$）**：同上，$|\int\Phi d\mu|\le\|\Phi\|_\infty\mu(\Omega)<\infty$（Debt-μfin-up），Z 的界用 Debt-λpos、Debt-λfin-up |
| Lβ-Z | B-lo、B-att | R_meas | **Z 有限且正（Q2）**：Φ∈𝓑（Debt-bdd）⇒ $Z\in[e^{-\|\Phi\|_\infty}\lambda(\Omega),e^{\|\Phi\|_\infty}\lambda(\Omega)]\subset(0,\infty)$（Debt-λpos、Debt-λfin-stmt）。Debt-bdd 推出的事实，不另立 route-Debt（1b D3） |
| Lβ-tilt | B-lo、B-att | R_tilt | **倾斜测度的性质（自包含）**：由 Lβ-Z，λ_Φ 是概率测度（可数可加性来自单调收敛）；e^Φ>0 处处 ⇒ λ_Φ 与 λ 互相绝对连续；∀ 可测 h≥0：$\int h\,d\lambda_\Phi=Z^{-1}\int he^\Phi d\lambda$ |
| Lβ1 | A（及 A 的实例） | ℝ | ∀t∈[0,∞)：$t\log t\ge t-1$（0·log0=0），等号 ⇔ t=1。证明：g(t)=t log t−t+1，g(0)=1>0；t>0 时 g'(t)=log t，g 在 (0,1] 严格减、[1,∞) 严格增，g(1)=0。推论：$(t\log t)^-\le(t-1)^-\le1$ |
| L-shift | B-up | R_meas | **常数平移（与 α 相同）**：∀(μ,λ)∈$\mathcal D_{up}$、∀Φ∈𝓑、∀c∈ℝ：$J_\mu(\Phi+c)=J_\mu(\Phi)+c(\mu(\Omega)-1)$（Debt-μfin-up、L2-up） |
| $L_Z^{\lim}$ | B-up | R_dens | **截断归一化常数的界与极限（与 α 相同，取代 v1.1 的 $L_Z^{up}$）**：(μ,λ)∈$\mathcal D_{up}$、μ≪λ，$\Phi_n:=\max(-n,\min(n,\log f))$，$Z_n:=\int e^{\Phi_n}d\lambda$：(a) $0<e^{-n}\lambda(\Omega)\le Z_n\le\mu(\Omega)+e^{-n}\lambda(\Omega)$；(b) $Z_n\to\mu(\Omega)$。证明：处处 $e^{-n}\le e^{\Phi_n}\le f+e^{-n}$ 给 (a)；$Z_n=\int\mathbf 1_{\{f\ge e^{-n}\}}\min(f,e^n)d\lambda+e^{-n}\lambda(f<e^{-n})$，第一项单调收敛到 $\int f\,d\lambda=\mu(\Omega)$（T1(iv)），第二项 →0，给 (b)。消费 T1、Debt-λpos、Debt-λfin-up、Debt-μfin-up；**不使用 $\int f\,d\lambda=1$** |

## D 边（guard 列为所消费的 guard ID）
| id | 分支 | R | guard | 前提 → 结论 |
|---|---|---|---|---|
| D-A0 | A | R_meas | G-nac | 按定义 KL=+∞>0；又 λ≪λ，故 μ≠λ |
| D-A1 | A | R_dens | G-inf | KL=+∞>0；μ=λ 时 KL=0，故此时 μ≠λ |
| D-βA1 | A | R_dens | G-fin | 由 T1(ii)（对 $(\log f)^\pm$ 分别用），$\mathrm{KL}=\int f\log f\,d\lambda$（λ-a.e.，{f=0} 上取 0）。由 Lβ1，$f\log f\ge f-1$，$(f\log f)^-\le1$，$\int(f\log f)^-d\lambda\le\lambda(\Omega)<\infty$（Debt-λ≤1），故 $\mathrm{KL}\ge\int(f-1)d\lambda=\mu(\Omega)-\lambda(\Omega)\ge0$（T1(iv)；Debt-μP；Debt-λ≤1） |
| D-βA2 | A | R_dens | G-fin | 承 D-βA1：KL=0 ⇒ $\int(f\log f-f+1)d\lambda=\mathrm{KL}-1+\lambda(\Omega)\le0$（Debt-μP、Debt-λ≤1），被积函数非负 ⇒ λ-a.e. $f\log f=f-1$ 且 λ(Ω)=1 ⇒ 由 Lβ1，f=1 λ-a.e. ⇒ μ=λ。反之 μ=λ ⇒ KL=0 |
| D-βA1@λΦ | A 的实例（消费者 B-lo） | R_tilt | G-fin@λΦ | D-βA1 以 λ:=λ_Φ、f:=g 实例化：$\mathrm{KL}(\mu\|\lambda_\Phi)\ge\mu(\Omega)-\lambda_\Phi(\Omega)=0$。所需：Debt-μP（由 B-lo/B-att scope 供给，传递消费者 D-βBlo1、D-βBatt⇒）；$\lambda_\Phi(\Omega)\le1$ 由 Lβ-tilt（$=1$）推出，**不消费 λ 的 Debt-λ≤1**；T1@λΦ、L1@λΦ、Lβ1 |
| D-βA2@λΦ | A 的实例（消费者 B-att） | R_tilt | G-fin@λΦ | D-βA2 以 λ:=λ_Φ 实例化：$\mathrm{KL}(\mu\|\lambda_\Phi)=0$ ⇒ g=1 λ_Φ-a.e.（即 μ=λ_Φ）。所需同上 |
| D-βBlo0 | B-lo | R_meas | G-nac ∨ G-inf | KL=+∞（G-nac 按定义；G-inf 按守卫）；由 L2-lo，$J_\mu(\Phi)\in\mathbb R$，故 $J_\mu(\Phi)<\mathrm{KL}$ |
| D-βBlo1 | B-lo | R_tilt | G-fin、G-fin@λΦ | Φ∈𝓑，X-tilt 给出 λ_Φ（Lβ-Z、Lβ-tilt）。μ≪λ≪λ_Φ。令 $g:=Ze^{-\Phi}f$；由 Lβ-tilt，$\int_Eg\,d\lambda_\Phi=\int_Ef\,d\lambda=\mu(E)$ ∀E，故由 T1@λΦ(i) 的唯一性，g 是 dμ/dλ_Φ 的版本（λ_Φ-a.e.）。于是（μ-a.e.）$\mathrm{KL}(\mu\|\lambda_\Phi)=\int(\log f+\log Z-\Phi)d\mu=\mathrm{KL}(\mu\|\lambda)+\log Z-\int\Phi d\mu$（三项 μ-可积：L1[lo]、Lβ-Z、L2-lo）。由 D-βA1@λΦ：左边 ≥0 ⇒ $J_\mu(\Phi)\le\mathrm{KL}(\mu\|\lambda)$ |
| D-Bup1 | B-up | R_meas | G-nac | 设 (μ,λ)∈$\mathcal D_{up}$（v1.3 显式量词）。取 E∈𝓕，λ(E)=0<μ(E)（$\le\mu(\Omega)<\infty$，Debt-μfin-up）。$\Phi_n=n\mathbf 1_E$，$\int e^{\Phi_n}d\lambda=\lambda(\Omega)\in(0,\infty)$（Debt-λpos、Debt-λfin-up），$J_\mu(\Phi_n)=n\mu(E)-\log\lambda(\Omega)\to+\infty=\mathrm{KL}$（L2-up）。不消费 Debt-λ≤1、Debt-μP |
| D-Bup2 | B-up | R_dens | G-fin ∨ G-inf | **（与 α 逐字相同；R3-Q2）** $m:=\mu(\Omega)\in(0,\infty)$（Debt-μfin-up、RD-μpos），$c_m:=\frac{\log m}{m-1}$（m≠1），$c_1:=0$，$c_m(m-1)=\log m$。$\Psi_n:=\Phi_n+c_m\in\mathcal B$（Debt-bdd）。(1) T1(iii)：μ-a.e. f>0、$\Phi_n\to\log f$、$|\Phi_n|\le|\log f|$。(2) G-fin：控制收敛（L1[up]）$\int\Phi_nd\mu\to\mathrm{KL}$；G-inf：正部单调收敛到 +∞，负部 $\le e^{-1}\lambda(\Omega)$，故 →+∞。(3) $L_Z^{\lim}$(b)、m>0：$-\log Z_n\to-\log m$。(4) L-shift：$J_\mu(\Psi_n)=\int\Phi_nd\mu-\log Z_n+\log m\to\mathrm{KL}$（G-inf 时 +∞）。**不消费 B-lo、Debt-μP、$\int f\,d\lambda=1$** |
| D-βBatt⇐ | B-att | R_dens | G-eb | 取有界版本 Φ*=log f∈𝓑，$J_\mu(\Phi^*)=\mathrm{KL}-\log\int f\,d\lambda=\mathrm{KL}-\log\mu(\Omega)=\mathrm{KL}$（T1(iv)、Debt-μP）；结合 D-βBlo1（B-lo → B-att，同域 $\mathcal D_{lo}$）sup 被取到。**v1.2 不再引用 G-eb@λΦ**（本边不经倾斜测度） |
| D-βBatt⇒ | B-att | R_tilt | G-fin（由结论推出）、G-fin@λΦ；输出 G-eb@λΦ ⇒ G-eb | 若 $J_\mu(\Phi)=\mathrm{KL}$，Φ∈𝓑：由 L2-lo，KL∈ℝ；按 KL 定义排除 G-nac（**不引用 A 分支的 D-A0**），且非 G-inf，故 G-fin，由实例关系得 G-fin@λΦ。由 D-βBlo1 的恒等式，KL(μ‖λ_Φ)=0 ⇒ 由 D-βA2@λΦ，g=1 λ_Φ-a.e.（满足 G-eb@λΦ，故 G-eb）⇒ μ=λ_Φ ⇒ 由 T1(i) 唯一性，f=e^Φ/Z λ-a.e. ⇒ log f=Φ−log Z 本质有界，Φ=log f+log Z λ-a.e. |

## 依赖偏序（传递闭包，I4；v1.2 重写）
- A：T1 ≺ L1[A] ≺ D-βA1 ≺ D-βA2；Lβ1 ≺ D-βA1、D-βA2；Debt-μP ≺ D-βA1、D-βA2；Debt-λ≤1 ≺ L1[A]、D-βA1、D-βA2；G-fin ≺ D-βA1、D-βA2；G-nac ≺ D-A0；G-inf、T1 ≺ D-A1。
- B-lo：Debt-bdd、Debt-μP、Debt-λpos、Debt-λfin-stmt ≺ L2-lo ≺ D-βBlo0、D-βBlo1；Debt-bdd、Debt-λpos、Debt-λfin-stmt ≺ Lβ-Z ≺ X-tilt ≺ Lβ-tilt ≺ T1@λΦ ≺ D-βA1@λΦ ≺ D-βBlo1；L1@λΦ、Lβ1 ≺ D-βA1@λΦ；T1 ≺ L1[lo] ≺ D-βBlo1；Debt-λfin-stmt ≺ L1[lo]；**Debt-μP ≺ D-βA1@λΦ（跨分支 A → B-lo：A 实例所需 Debt-μP 在 B-lo scope 内满足，传递消费者 D-βBlo1）**；G-fin@λΦ ≺ T1@λΦ、D-βA1@λΦ、D-βBlo1；G-fin ≺ D-βBlo1；G-nac、G-inf ≺ D-βBlo0。
- B-up：Debt-bdd、Debt-μfin-up、Debt-λpos、Debt-λfin-up ≺ L2-up ≺ D-Bup1、L-shift；G-nac、Debt-μfin-up ≺ D-Bup1；T1、Debt-λpos、Debt-λfin-up、Debt-μfin-up ≺ $L_Z^{\lim}$ ≺ D-Bup2；T1 ≺ L1[up] ≺ D-Bup2；Debt-λfin-up ≺ L1[up]；L-shift、RD-μpos、Debt-bdd、G-fin、G-inf ≺ D-Bup2。**B-up 无跨分支消费**（v1.1 的 D-βBlo0/D-βBlo1 ≺ D-Bup2 已删除）；Debt-μP、Debt-λfin-stmt、Lβ-Z、L2-lo 不在 B-up 偏序中。
- B-att：T1、Debt-μP、G-eb ≺ D-βBatt⇐；D-βBlo1 ≺ D-βBatt⇐（B-lo → B-att，同域；D-βBlo1 的 Debt 均在 B-att scope 内）；L2-lo、T1、T1@λΦ、G-fin@λΦ、G-eb@λΦ ≺ D-βBatt⇒；**D-βBlo1 ≺ D-βBatt⇒**（B-lo → B-att）；**D-βA2@λΦ ≺ D-βBatt⇒**（A → B-att；Debt-μP 在 B-att scope 内，λ_Φ(Ω)=1 由 Lβ-tilt）。v1.2 删除 D-A0 ≺ D-βBatt 与 G-eb@λΦ ≺ D-βBatt⇐。

## Debt 表（v1.2）
| id | 内容 | scope | consumer（β） | 量词 | debt_kind | 保持其余前提不变的反例（witness_id，定义逐行写出） |
|---|---|---|---|---|---|---|
| Debt-μP | μ(Ω)=1 | A、B-lo、B-att | L2-lo、D-βA1、D-βA2、D-βA1@λΦ、D-βA2@λΦ、D-βBatt⇐（经实例传给 D-βBlo1、D-βBatt⇒） | ∀μ（$\mathcal D_A$、$\mathcal D_{lo}$） | theorem-Debt | W-μ：Ω={ω}，λ=δ_ω，μ=½δ_ω：f=½，KL=−½log2<0；常数 t 给 $J_\mu(t)=-t/2$ 无上界。**注：B-up 无失败见证，故不在该 scope/consumer**（W-μ∈$\mathcal D_{up}$ 且 D-Bup2 给出 $J_\mu(\Psi_n)=\mathrm{KL}$） |
| Debt-μfin-up | μ(Ω)<∞ | B-up | L2-up、L-shift、$L_Z^{\lim}$、D-Bup1、D-Bup2 | ∀μ（$\mathcal D_{up}$） | statement-Debt | W-μ∞：Ω=ℕ₁，λ(k)=2^{−k}，μ=计数测度，Φ(k)=(−1)^k：$\int\Phi\,d\mu$ 为 ∞−∞ |
| Debt-λ≤1 | λ(Ω)≤1 | A | L1[A]、D-βA1、D-βA2 | ∀λ（$\mathcal D_A$） | theorem-Debt | W-λ1：μ=δ_ω，λ=2δ_ω：KL=−log2<0。D-βA1@λΦ/D-βA2@λΦ 用的是 λ_Φ(Ω)=1（Lβ-tilt），不来自本 Debt |
| Debt-λfin-up | λ(Ω)<∞ | B-up | L1[up]、L2-up、$L_Z^{\lim}$、D-Bup1、D-Bup2（**v1.2 删去 Lβ-Z**） | ∀λ（$\mathcal D_{up}$） | theorem-Debt（依 object v1.2 扩展值许可） | W-λ∞：Ω=ℕ₁，λ=计数测度，μ(k)=2^{−k}：Z≡+∞，扩展值 J≡−∞，不能逼近 KL=−2log2（E1） |
| Debt-λfin-stmt | λ(Ω)<∞ | B-lo、B-att | L1[lo]、L2-lo、Lβ-Z（传递：X-tilt、Lβ-tilt、T1@λΦ、D-βBlo0、D-βBlo1、D-βBatt⇐、D-βBatt⇒）。**不含 $L_Z^{\lim}$** | ∀λ（$\mathcal D_{lo}$） | statement-Debt | 同一 W-λ∞：Z=+∞，J∉ℝ，且 λ_Φ 无法归一化（E1/E3） |
| Debt-λpos | λ(Ω)>0 | B-lo、B-up、B-att | L2-lo、L2-up、Lβ-Z、$L_Z^{\lim}$、D-Bup1 | ∀λ（$\mathcal D_{lo}$、$\mathcal D_{up}$） | statement-Debt | W-λ0：λ=0，μ=δ_ω：Z=0，J=+∞∉ℝ（E3） |
| Debt-bdd | Φ∈𝓑 | B-lo、B-up、B-att | L2-lo、L2-up、Lβ-Z（→X-tilt）、L-shift、D-βBlo1、D-Bup1、D-Bup2、D-βBatt⇐、D-βBatt⇒ | ∀Φ∈𝓑 | statement-Debt | W-bdd：Ω=ℤ∖{0}，λ(k)=c₁e^{−2\|k\|}，μ(k)=c₂k^{−2}，Φ(k)=sign(k)\|k\|：$\int\Phi^\pm d\mu=+\infty$ |

### route-Debt（v1.2）
| id | 内容 | scope | consumer | 量词 | debt_kind | 反例 |
|---|---|---|---|---|---|---|
| RD-μpos | μ(Ω)>0 | B-up（仅本路线） | D-Bup2 | ∀μ（$\mathcal D_{up}$）；见证测试域 = 扩展域 $\mathcal D_{up}^{0}=\{0\le\mu(\Omega)<\infty,\ 0<\lambda(\Omega)<\infty\}$（v1.3，R4-C02；只去掉 μ(Ω)>0，其余前提不变） | route-Debt | W-μ0：Ω={ω}，λ=δ_ω，μ=0（f=0，KL=0）：$Z_n=e^{-n}\to0$，$J_\mu(\Phi_n)=n\to+\infty$，$c_0$ 无定义，路线失效；常数 Φ≡0 给 J=0=KL，陈述成立 |
| ~~RD-μnorm~~ | ~~$\int f\,d\lambda=1$~~ | — | — | — | **v1.2 撤销** | 新 D-Bup2 不依赖，无 consumer（I2(b)） |
Z∈(0,∞) 仍由 Debt-bdd 推出（Lβ-Z），不另立 route-Debt(Z-finite)（1b D3）。

### failure 记录（逐行自包含；v1.2）
| Debt | witness_id | branch | failure_target | failure_type | failure_subtype | 说明 |
|---|---|---|---|---|---|---|
| Debt-μP | W-μ | A | theorem_conclusion | branch_conclusion_mismatch | inequality_sign_wrong | KL=−½log2<0 |
| Debt-μP | W-μ | B-lo | theorem_conclusion | branch_conclusion_mismatch | variational_value_wrong | t=−2：J=1>KL（经 D-βA1@λΦ / D-βBlo1） |
| Debt-μP | W-μ | B-att | theorem_conclusion | branch_conclusion_mismatch | variational_value_wrong | sup=+∞ 不被取到（经 D-βBatt⇐/⇒） |
| Debt-μfin-up | W-μ∞ | B-up | statement_applicability | premise_inapplicable | integral_indeterminate | $\int\Phi d\mu=\infty-\infty$ |
| Debt-λ≤1 | W-λ1 | A | theorem_conclusion | branch_conclusion_mismatch | inequality_sign_wrong | KL=−log2<0 |
| Debt-λfin-up | W-λ∞ | B-up | theorem_conclusion | branch_conclusion_mismatch | normalizer_invalid | 扩展值下 sup=−∞≠KL（object 扩展值许可，仅限本见证）；根因 Z=+∞（R4 裁决，v0.5.1 I6；v1.2 误记 variational_value_wrong） |
| Debt-λfin-stmt | W-λ∞ | B-lo | statement_applicability | premise_inapplicable | normalizer_invalid | Z=+∞，J∉ℝ，λ_Φ 不可归一化 |
| Debt-λfin-stmt | W-λ∞ | B-att | statement_applicability | premise_inapplicable | normalizer_invalid | 同上 |
| Debt-λpos | W-λ0 | B-lo | statement_applicability | premise_inapplicable | normalizer_invalid | Z=0，J=+∞∉ℝ |
| Debt-λpos | W-λ0 | B-up | statement_applicability | premise_inapplicable | normalizer_invalid | 同上 |
| Debt-λpos | W-λ0 | B-att | statement_applicability | premise_inapplicable | normalizer_invalid | 同上；保持 statement-Debt（E3） |
| Debt-bdd | W-bdd | B-lo | statement_applicability | premise_inapplicable | integral_indeterminate | ∞−∞ |
| Debt-bdd | W-bdd | B-up | statement_applicability | premise_inapplicable | integral_indeterminate | 同上 |
| Debt-bdd | W-bdd | B-att | statement_applicability | premise_inapplicable | integral_indeterminate | 同上 |
| RD-μpos | W-μ0 | B-up（route） | proof_route | route_failure | normalization_route_failure | 平移截断构造需 $\lim Z_n=\mu(\Omega)>0$；见证在扩展测试域 $\mathcal D_{up}^{0}$（v1.3） |
（v1.2 删除：`Debt-μP | — | B-up | —` 空行；`RD-μnorm` 行。）

## 证据端点（I5 / v0.4【逐边 evidence】/ v0.5；一边一行）
| edge_id | guard | 本边等级 | 传递等级 | 来源 | 覆盖量词 | 分支 | 边界 |
|---|---|---|---|---|---|---|---|
| T1 | —（被 G-fin/G-inf/G-eb 边消费） | (i) partial·待核；(ii)–(iv) complete·非外部·自包含 | partial·待核 | (i) Folland *Real Analysis* 2nd ed. Thm 3.8；(ii)–(iv) 本网 T1 行 | ∀ 有限正测度 μ≪λ，$0<\mu(\Omega),\lambda(\Omega)<\infty$ | A、B-lo、B-up、B-att | {f=0} 零集；0·∞=0；λ-a.e. 唯一 |
| T1@λΦ | G-fin@λΦ | (i) partial·待核；(ii)–(iv) complete·自包含 | partial·待核 | (i) Folland Thm 3.8（实例 λ_Φ 为概率测度）；(ii)–(iv) 本网 T1@λΦ 行 | ∀Φ∈𝓑，∀ 概率 μ，∀ $0<\lambda(\Omega)<\infty$，μ≪λ，G-fin ∧ G-fin@λΦ | B-lo、B-att | g 零集 = f 零集；λ_Φ-a.e. ⇔ λ-a.e. |
| X-tilt | — | N/A（X-定义；I5 对定义性 X 不分级） | — | 本网 X 宏行 | ∀Φ∈𝓑，$(\mu,\lambda)\in\mathcal D_{lo}$ | B-lo、B-att | 需 Lβ-Z 保证 Z∈(0,∞) |
| L1 | — | complete·非外部·自包含 | partial·待核（T1(ii)） | 本网 L1 行 | T1 量词域内；λ(Ω)<∞ 按分支 | A、B-lo、B-up、B-att | KL=+∞ 允许 |
| L1@λΦ | G-fin@λΦ | complete·非外部·自包含 | partial·待核（T1@λΦ(ii)） | 本网 L1 行（实例） | 同 T1@λΦ | B-lo、B-att | $\lambda_\Phi(\Omega)=1$ |
| L2-lo | — | complete·非外部·自包含 | complete | 本网 L2-lo 行 | ∀(μ,λ)∈$\mathcal D_{lo}$，∀Φ∈𝓑 | B-lo、B-att | — |
| L2-up | — | complete·非外部·自包含 | complete | 本网 L2-up 行 | ∀(μ,λ)∈$\mathcal D_{up}$，∀Φ∈𝓑 | B-up | — |
| Lβ-Z | — | complete·非外部·自包含 | complete | 本网 Lβ-Z 行 | ∀(μ,λ)∈$\mathcal D_{lo}$，∀Φ∈𝓑 | B-lo、B-att | Z∈(0,∞) |
| Lβ-tilt | — | complete·非外部·自包含 | complete | 本网 Lβ-tilt 行 | ∀Φ∈𝓑，$0<\lambda(\Omega)<\infty$ | B-lo、B-att | λ_Φ ~ λ |
| Lβ1 | — | complete·非外部·自包含 | complete | 本网 Lβ1 行（求导） | ∀t∈[0,∞) | A 及其实例 | t=0：0·log0=0 |
| L-shift | — | complete·非外部·自包含 | complete | 本网 L-shift 行 | ∀(μ,λ)∈$\mathcal D_{up}$，∀Φ∈𝓑，∀c∈ℝ | B-up | μ(Ω)=1 时平移不变 |
| $L_Z^{\lim}$ | — | complete·非外部·自包含 | partial·待核（T1(iv)） | 本网 $L_Z^{\lim}$ 行（单调收敛） | ∀(μ,λ)∈$\mathcal D_{up}$，μ≪λ，∀n∈ℕ₁ | B-up | {f=0} 上 $e^{\Phi_n}=e^{-n}$；n→∞ |
| D-A0 | G-nac | complete·非外部·自包含 | complete | 本网 D-A0 行 | ∀(μ,λ)∈$\mathcal D_A$，G-nac | A | — |
| D-A1 | G-inf | complete·非外部·自包含 | partial·待核（G-inf 谓词预设 T1(i)） | 本网 D-A1 行 | ∀(μ,λ)∈$\mathcal D_A$，G-inf | A | KL=+∞ |
| D-βA1 | G-fin | complete·非外部·自包含 | partial·待核（T1） | 本网 D-βA1 行 | ∀(μ,λ)∈$\mathcal D_A$，G-fin | A | {f=0} 上 f log f=0 |
| D-βA2 | G-fin | complete·非外部·自包含 | partial·待核（T1） | 本网 D-βA2 行 | ∀(μ,λ)∈$\mathcal D_A$，G-fin | A | λ(Ω)<1 时等号不成立 |
| D-βA1@λΦ | G-fin@λΦ | complete·非外部·自包含 | partial·待核（T1@λΦ） | 本网 D-βA1@λΦ 行 | 同 T1@λΦ | A 实例 → B-lo | λ_Φ(Ω)=1 |
| D-βA2@λΦ | G-fin@λΦ | complete·非外部·自包含 | partial·待核（T1@λΦ） | 本网 D-βA2@λΦ 行 | 同 T1@λΦ，且 KL(μ‖λ_Φ)=0 | A 实例 → B-att | g=1 λ_Φ-a.e. |
| D-βBlo0 | G-nac ∨ G-inf | complete·非外部·自包含 | G-nac 部分 complete；G-inf 部分 partial·待核 | 本网 D-βBlo0 行 | ∀(μ,λ)∈$\mathcal D_{lo}$，∀Φ∈𝓑 | B-lo | KL=+∞ |
| D-βBlo1 | G-fin、G-fin@λΦ | complete·非外部·自包含 | partial·待核（T1、T1@λΦ） | 本网 D-βBlo1 行 | ∀(μ,λ)∈$\mathcal D_{lo}$，G-fin，∀Φ∈𝓑 | B-lo | g 的 λ_Φ-a.e. 版本；三项 μ-可积 |
| D-Bup1 | G-nac | complete·非外部·自包含 | complete | 本网 D-Bup1 行 | ∀(μ,λ)∈$\mathcal D_{up}$，G-nac | B-up | n∈ℕ₁ |
| D-Bup2 | G-fin ∨ G-inf | complete·非外部·自包含 | partial·待核（T1） | 本网 D-Bup2 行 | ∀(μ,λ)∈$\mathcal D_{up}$，G-fin 或 G-inf | B-up | m=1 时 $c_1=0$；G-inf 极限 +∞ |
| D-βBatt⇐ | G-eb | complete·非外部·自包含 | partial·待核（T1(iv)） | 本网 D-βBatt⇐ 行 | ∀(μ,λ)∈$\mathcal D_{lo}$，G-eb | B-att | Φ* 取有界版本 |
| D-βBatt⇒ | G-fin、G-fin@λΦ（输出 G-eb@λΦ） | complete·非外部·自包含 | partial·待核（T1、T1@λΦ） | 本网 D-βBatt⇒ 行 | ∀(μ,λ)∈$\mathcal D_{lo}$，∀Φ∈𝓑 且 J=KL | B-att | f=e^Φ/Z λ-a.e. |
| Debt/RD 失败记录 | — | complete·非外部·自包含 | complete | 本网 Debt 表、route-Debt 表中的具体反例 | 各 witness | 见 failure 表 | 见 failure 表 |

## 可达性反例（B-att 必要性的见证 X，非 Debt）
- Ω=ℕ₁，λ(k)=2^{−k}，μ(k)=2·3^{−k}：f(k)=2(2/3)^k，log f 无界，KL<∞；由 D-βBatt⇒，sup 不被取到。

## route-extension note
- RX-β-unbdd（不参加核心 I2/I6）：若扩展到无界 Φ，$\int e^\Phi d\lambda=+\infty$ 时 X-tilt 无法构造 λ_Φ（例：Ω=ℕ₁，λ(k)=2^{-k}，Φ(k)=k），该路线失效；此反例必然同时去掉 Debt-bdd。

## 语义失败键映射（完整副本，与 α 逐行一致；R3-Q7）
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

签名对照（作者，不预设等价结论）：α 的规范化保持引理签名 {T1}；β 为 {T1, T1@λΦ}（同一引理的两个实例）。β 另有 R_tilt 载体与 A → B-lo、A → B-att 的跨分支实例消费；v1.2 后 α 的跨分支消费只有 B-lo → B-att（D-Blo1 → D-Batt⇐/⇒）。两网 B-up 链逐字相同。

## diagram
```
R_meas ──T1 (RN, 硬; 有限正测度域)──▶ R_dens ── D-βA1 ─▶ D-βA2                  (A)
  │                                    ├── L_Z^lim + L-shift ─▶ D-Bup2         (B-up 独立链)
  │ X-tilt (Lβ-Z, Lβ-tilt)             └── D-βBatt⇐ (G-eb)                     (B-att)
  ▼
R_tilt ──T1@λΦ──▶ (λ_Φ, g) ── D-βA1@λΦ ─▶ D-βBlo1 ─┬─▶ D-βBatt⇐ / D-βBatt⇒  (B-lo→B-att)
                        └──── D-βA2@λΦ ───────────┴─▶ D-βBatt⇒
guards: G-nac | G-fin ⊃ G-eb | G-inf ；实例 G-nac@λΦ | G-fin@λΦ ⊃ G-eb@λΦ | G-inf@λΦ
```
