# 对象声明 · T18 Hahn–Banach / Riesz 表示（万能逼近的对偶证明）· v1（round1b P5）

## 工具
用对偶空间刻画稠密性：子空间不稠密 ⇒ 存在非零的有界线性泛函零化它（Hahn–Banach）⇒ 该泛函可以表示成测度（Riesz）⇒ 再用激活函数的「判别性」（Fourier 唯一性）推出这个测度为 0，矛盾。

## 全局标量域（P5）
- **本对象所有分支、所有 R 节点的标量域都是 ℝ**：赋范空间、泛函、测度都是实的。Fourier 输出 $\hat\mu$ 取值可以在 ℂ 中，但它只是**实测度上的复值辅助量**，不是复 Banach 空间版本。原先的 Debt-R 改为这条全局前提。

## 记号
- $I_n=[0,1]^n$（紧度量空间），$C(I_n)$ 为实值连续函数空间，取 sup 范数。
- $M(I_n)$ 为 $I_n$ 上有限带号正则 Borel 测度全体。
- σ：ℝ→ℝ，$S_\sigma=\operatorname{span}\{x\mapsto\sigma(y^Tx+\theta):y\in\mathbb R^n,\theta\in\mathbb R\}$。
- **sigmoidal**：$\sigma(t)\to1$（$t\to+\infty$），$\sigma(t)\to0$（$t\to-\infty$）。
- **判别型（discriminatory）**：若 $\mu\in M(I_n)$ 对所有 $y,\theta$ 满足 $\int\sigma(y^Tx+\theta)\,d\mu=0$，则 $\mu=0$。

## 形式结论（本对象必须证完的承诺）
- **(A) 对偶稠密判据**：E 为实赋范空间，S⊂E 为线性子空间。则 $\overline S=E$ ⇔ 每个满足 $L|_S=0$ 的 $L\in E^*$ 都有 $L=0$。（方向 ⇐ 用 Hahn–Banach；方向 ⇒ 由连续性直接得到。）
- **(B) Cybenko 定理 1**：若 σ **连续**且判别型，则 $S_\sigma$ 在 $(C(I_n),\|\cdot\|_\infty)$ 中稠密。
- **(C) Cybenko 引理 1**：若 σ **有界、可测**且 sigmoidal，则 σ 判别型。
- **(D) 推论（Cybenko 定理 2）**：若 σ 连续且 sigmoidal，则 $S_\sigma$ 在 $C(I_n)$ 中稠密。证明是 (B)+(C)；连续加 sigmoidal 推出有界。

（Hahn–Banach 定理本身、Riesz–Markov 表示 $C(K)^*\cong M(K)$、有限带号测度的 Fourier 唯一性，都作为**引用定理**使用，本对象不重证。它们在网中作为 T 边的保持引理出现。）

## 刻意排除
- 无界激活（ReLU 等），以及 Leshno 等人「非多项式 ⇔ 万能」的论证
- 整个 $\mathbb R^n$ 上的一致逼近（MO 316760：一般不可能），以及 $L^p$ 版本
- 逼近速率、宽度/深度界、构造性证明
- 复标量版本、一般局部凸空间
- 应用实例（具体网络结构）只作 App，可选

## 锚点
- 论文：Cybenko 1989，定理 1 的证明（Hahn–Banach → Riesz → 判别型），引理 1（$\sigma_\lambda$ → 半空间指示函数，有界收敛，再用 Fourier 论证），定理 2。
- 习题：
  - MSE 3821313（引理 1 的一步，travvytree 采纳答案）
  - MO 316760（紧性不可去，pcp 的高票答案，非采纳）
  - MSE 4749136（延伸）
- 背景：/workspace/mathscape/parts/05_tools_c.md §T18；/workspace/mathscape/report.md §3.9。

## Changelog
- v0（2026-10-30 03:15 UTC+8）：初版。
