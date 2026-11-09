# 对象声明 · T04 Moore–Penrose 伪逆 / 最小范数解 / 谱滤波 · v2.1（round3：(C) 的有限维前提性质注记 C30；v2 = round2 符号 C25、路线摘要 C28，备份 object-v2.md；v1 备份 object-v1.md；v0 备份 object-v0.md）
- 材料源：/workspace/mathscape/parts/03_tools_a.md §T04；report.md §T04、§6.1 M1（谱滤波宏）
- 全局：标量域 ℝ，**有限维**（$A\in\mathbb R^{m\times n}$，$m,n\ge1$），范数为欧氏范数和 Frobenius 范数。
- 依据：layer-a/round1b-astra-decisions.md（D1、D4、D7、D9、patch_checklist_T04「object」项）。

## 工具
伪逆把「解不了 / 解不唯一」的线性方程规范成「最小二乘中范数最小的那个」。谱滤波把伪逆、岭回归、梯度流、梯度下降统一成「对奇异值作用一个函数 φ」。

## 记号与类型（I7）
- $A$ 的秩为 $r\in\{0,1,\dots,\min(m,n)\}$，紧 SVD 为 $A=\sum_{i=1}^r\sigma_iu_iv_i^T$，其中 $\sigma_1\ge\dots\ge\sigma_r>0$，$\{u_i\}\subset\mathbb R^m$、$\{v_i\}\subset\mathbb R^n$ 各自标准正交。**r=0 时 A=0，空和取 0，$\sigma_1$ 不定义。**
- **伪逆** $A^\dagger\in\mathbb R^{n\times m}$：满足 Penrose 四条件的唯一矩阵 G：$AGA=A$，$GAG=G$，$(AG)^T=AG$，$(GA)^T=GA$。
- $\mathrm{LS}(b)=\arg\min_{x\in\mathbb R^n}\|Ax-b\|$（$b\in\mathbb R^m$）；$x^\dagger:=A^\dagger b$。
- 谱滤波：对 $\phi:(0,\infty)\to\mathbb R$，令 $x_\phi:=\sum_{i\le r}\phi(\sigma_i)\,\sigma_i^{-1}(u_i^Tb)\,v_i$（r=0 时 $x_\phi=0$）。
- 参数域：$\alpha\in(0,\infty)$；时间 $t\in[0,\infty)$；迭代 $k\in\mathbb N_0$；步长 $\eta\in(0,\infty)$（各分支另加限制）。
- 向量化：$\operatorname{vec}$ 为**列主序**（按列堆叠）；$X\in\mathbb R^{n\times p}$ 时 $\operatorname{vec}X\in\mathbb R^{np}$。

## 形式结论（分支）
- **(A) 最小范数最小二乘**：∀A、∀b，$\mathrm{LS}(b)\neq\varnothing$，$x^\dagger$ 是 $\mathrm{LS}(b)$ 中**唯一的**范数最小元，且 $x^\dagger\in\operatorname{ran}A^T=(\ker A)^\perp$。若 $b\in\operatorname{ran}A$，则 $\mathrm{LS}(b)=\{x:Ax=b\}$。
- **(B) 表示式**：
  - (B-svd) ∀A：$A^\dagger=\sum_{i\le r}\sigma_i^{-1}v_iu_i^T$（r=0 时 $A^\dagger=0$）；
  - (B-row) 守卫 rank A=m：$A^\dagger=A^T(AA^T)^{-1}$（Bartlett 定义 2 的 $\hat\theta$）；
  - (B-col) 守卫 rank A=n：$A^\dagger=(A^TA)^{-1}A^T$。
  - **overlap（I1·v0.2）**：B-row 与 B-col 的守卫可同时成立（m=n=r，A 可逆），此时两式都等于 $A^{-1}$；三条 B 公式是**重叠公式分支**，不是互斥分割，逐支各自核对。
- **(C) 矩阵方程**：$A\in\mathbb R^{m\times n}$，$B\in\mathbb R^{p\times q}$，$C\in\mathbb R^{m\times q}$，未知 $X\in\mathbb R^{n\times p}$。若 $AXB=C$ 有解（Debt-cons），则 $X^*=A^\dagger CB^\dagger$ 是它唯一的 Frobenius 范数最小解。
- **(D) 谱滤波统一**（全部 ∀A、∀b，零初值约定 $x(0)=0$、$x^{(0)}=0$）：
  - (D-tik) ∀α>0：$x_\alpha:=(A^TA+\alpha^2I)^{-1}A^Tb=x_{\phi_\alpha}$，$\phi_\alpha(\sigma)=\sigma^2/(\sigma^2+\alpha^2)$；α→0⁺ 时 $x_\alpha\to x^\dagger$。
  - (D-gf) 梯度流 $\dot x=-A^T(Ax-b)$，$x(0)=0$，$t\in[0,\infty)$：唯一的 $C^1$ 解满足 $x(t)=x_{\phi_t}$，$\phi_t(\sigma)=1-e^{-t\sigma^2}$；t→∞ 时 $x(t)\to x^\dagger$。（r=0 时 $x(t)\equiv0=x^\dagger$，由空和自动覆盖。）
  - (D-gd) **正秩** r≥1：梯度下降（迭代记作 $x^{(k)}$，初值 $x^{(0)}$） $x^{(k+1)}=x^{(k)}-\eta A^T(Ax^{(k)}-b)$，$x^{(0)}=0$，$0<\eta<2/\sigma_1^2$，$k\in\mathbb N_0$：$x^{(k)}=x_{\phi_k}$，$\phi_k(\sigma)=1-(1-\eta\sigma^2)^k$；k→∞ 时 $x^{(k)}\to x^\dagger$。
  - (D-gd0) **零秩** r=0（A=0）：对任意 η>0，$x^{(k)}\equiv0=x^\dagger$（此时 $A^\dagger=0$）。（D4：单列 r=0 分支，不用步长守卫静默排除。）

（引用定理：SVD 存在定理（网 α 的 T1 消费）。Penrose 四条件解的**唯一性**在两网中都以自包含引理 L0 证明；**存在性**在网 α 由 D1 构造、在网 β 由满秩分解 E0 构造，均不再依赖 Penrose 1955 的引用。）

## 刻意排除
- 无穷维算子与非闭值域（此时 $A^\dagger$ 无界）；复数域；加权伪逆；Drazin 逆
  - 注（R3 C30）：对 (C)，有限维是**陈述前提**而非定理结论的必要条件——在 Hilbert–Schmidt 推广中最小范数解仍存在唯一，但公式 $A^\dagger CB^\dagger$ 中的 $A^\dagger,B^\dagger$ 可无界、只稠密定义，本对象不声明其闭包或延拓。
- 收敛速率、噪声 / 统计风险（Bartlett 引理 7）、TSVD 的最优截断
- 应用实例（NTK、岭回归统计意义、TSVD 与 Tikhonov 的 α–t 对应）只作 App 或 view
- **Fourier 零延拓模板：T04 上 N/A**（无 Fourier 变换、无测度域扩张或限制；D9 决议，模板继续暂缓）。

## 路线摘要（I3·v0.2 等价关系，D2）
- 候选路线：α（SVD / 谱路线，work-net.md）与 β（投影 / 勾股路线，work-net-beta.md）。对象级只记录「存在合格路线」，合格性待 Layer B。
- 等价关系定义在路线对 $(O,\rho)$ 上：α 与 β 在 A、C 分支的规范化保持引理签名不同（α 含 T1，β 为空），当前只记录为**签名不同的候选路线**，不合并；只有在两者均判为合格后，才据 I3 判断是否属于不同等价类（R2 C28）。

## 锚点
- MSE 2713697（copper.hat：SVD / Kronecker 路线；user1551：正交投影加勾股路线）
- MSE 1084677（Syed Alam Abbas：TSVD 与 Tikhonov 都是谱滤波，α→0 回到伪逆）
- Bartlett et al.（P2）定义 2：$\hat\theta=(X^TX)^\dagger X^Ty=X^T(XX^T)^{-1}y$
