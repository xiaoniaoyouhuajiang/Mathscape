> 这是检查动作的详细版；规范版本和案例路径见 [`reference-index.md`](reference-index.md)。检查结果不是数学认证。

# I1–I7 构建检查规则

I1–I7 是每次建网时的自检，不是认证章。对每条检查都记录 pass/fail/N/A 理由；I1、I2、I4、I7 不可 N/A。I3、I5 只有条款规定的情况下可 N/A；N/A 不是通过。

## I1 结论承诺对齐

- **检查什么**：逐分支把对象卡的形式结论、前提、适用域、参数范围和结论形式（极限、速率界、渐近等价）与网的终点逐项对照；overlap 分支分别核对，不能当互斥分割。
- **不通过**：网多证、少证、漏分支，或把排除项/App/view 当核心结论。
- **怎么改/怎么记**：补缺的 R/D/T/X 和量词；拆分混合分支；把排除项移出核心；若无法闭合，保留 staged 并写 blocked/Debt，不改对象卡来迁就网。
- **金样例**：T04 object 的 B-row 与 B-col 在 rank A=m、rank A=n 时可 overlap（m=n=r、A 可逆），两式都等于 A⁻¹；work-net 必须逐支核对三条 B 公式，不能把 B-row/B-col 当互斥二选一。

## I2 Debt 闭包

- **检查什么**：Debt 集合必须充分：在保留已登记 Debt、guard、引理和明确前提的情况下，每条核心边都能成立；不能靠一个未登记的隐藏假设补洞。除此之外，每条 Debt 原子化并写 scope、consumer、量词、debt_kind、failure_target 和具体见证；guard_registry（含实例 guard）与 L 分开；检查放宽域有限/正质量、跨分支消费和 consumer/偏序一致。每条核心边列出所消费 Debt，且每条核心 Debt 有 consumer；对象卡与网的分支前提一致。
- **不通过**：把 guard/L 当 Debt；同一 ID 跨分支承担不同类型；“去掉所有 Debt 仍合理”这种无法判断的总句；跨分支边消费未登记或未满足的 Debt；放宽域只写“按有限正测度理解”。
- **怎么改/怎么记**：按分支拆 Debt ID；补 guard registry 和传递 consumer；将对象外路线写 route-extension note；补有限/正质量的正式量词和失败见证；无失败见证就删除该 scope/consumer，不伪造 failure。
- **金样例**：T12 的 B-up 独立链使用 Debt-μfin-up（statement-Debt，μ(Ω)<∞）和 RD-μpos（route-Debt，μ(Ω)>0，见证域 D_up^0），不消费 B-lo 的 Debt-μP；W-μ∞ 使 ∫Φ dμ 为 ∞−∞，W-μ0 只击穿平移截断路线而不击穿 B-up 陈述。

## I3 硬 T（不可消除的表示切换）

- **检查什么**：目标 R 是否引入源定义不能直接展开的新语义对象/算子；是否需要独立保持引理；删去 T 后能否用源内 D/X 重建全部结论。硬 T 还要写源/目标载体、精确保持引理、量词、分支、重建映射和证据端点。
- **不通过**：同表示换元、定义性计算、改名被标为硬 T；缺保持引理或证书；删 T 后结论仍可重建却保留 T。
- **怎么改/怎么记**：降为 D/X/view，或补 typed certificate；对实例重新登记 guard。若核心没有候选 T，可记 N/A，但必须写哪些候选被降级及理由。
- **金样例**：T18 work-net 将 T1（NormedSpace→DualFunctional 的 Hahn–Banach 零化子推论）、T2（DualFunctional→Measure 的 Riesz–Markov）和带显式 f=ν̂ 的 T3（FourierFunction→Measure 唯一性）作为硬 T；原定义性计算 T4 已降为 X6，因为可由定义展开。

## I4 关键依赖偏序

- **检查什么**：按分支画 DAG，取传递闭包；凡边消费上游 D/T 输出、Debt、guard 或非定义性前提，都要作为偏序节点/指针入图并标消费者分支。X、view、App 不进入核心偏序。
- **不通过**：把纯写作顺序当依赖；不同分支硬接成一链；跨分支消费没有传递 consumer；实例边只指向源 guard。
- **怎么改/怎么记**：补前提节点、消费指针、实例 guard 和跨分支传递关系；无法判定时保留偏序缺口和 Debt，不宣称路线完成。
- **金样例**：T12 work-net 的 B-up 偏序明确为 T1、Debt-λpos、Debt-λfin-up、Debt-μfin-up ≺ L_Z^lim ≺ D-Bup2，并写明 B-up 无跨分支消费，Debt-μP、Debt-λfin-stmt 不在 B-up 偏序中；D-Bup2 不得偷偷消费 B-lo 的边。

## I5 证据端点

- **检查什么**：每条核心边一行 evidence，写等级 complete|partial|待补、来源（外部定理号/题号或“非外部·自包含”）、覆盖量词、分支、边界和 locator。complete 必须覆盖该边及其非定义性前提；作者笔记不算端点。
- **不通过**：空端点；“见表”或整段证明代替逐边行；只支撑方法却标 complete；外部定位未核对却写 complete。
- **怎么改/怎么记**：补确切 locator、量词和覆盖声明；无法核对就降为 partial·待核；定义性 X 是否需要端点若适用 N/A，写明理由和 residual risk。
- **金样例**：T08 work-net 明确说 MSE 197602 的端点只支撑“近/远切分 + 尾部估计”方法，不能支撑 C2 唯一内点主项；因此 D6 可有 partial（外部），同时用本网完整模估计标 complete（非外部·自包含），D6'–D9c 仍要逐边 evidence。

## I6 失败模式一致

- **检查什么**：每条 Debt 和每个分支边界都有具体、可检查、保持其余 Debt/guard/结论前提的 witness；填写 failure_target、顶层 failure_type、failure_subtype 和 branch。跨路线比较用语义键 failure_target–branch–conclusion；route-extension note 不参加核心比较。
- **不通过**：只写“待讨论”；反例同时去掉多条前提；把 statement applicability 误写 theorem conclusion；Z=0/+∞ 用 integral_indeterminate；没有失败见证仍保留 Debt。
- **怎么改/怎么记**：拆分 witness 和分支记录；J 不实值记 statement_applicability / premise_inapplicable；归一化常数 Z 非法记 normalizer_invalid；只有对象明确允许扩展值且结论仍失败时才记 theorem_conclusion。
- **金样例**：T12 的 W-λ∞ 在 B-up 明确允许扩展值，sup J=-∞≠KL=-2log2，记录 theorem_conclusion / branch_conclusion_mismatch / normalizer_invalid；同一见证在 B-lo/B-att 只使 J∉ℝ，记录 statement_applicability / premise_inapplicable / normalizer_invalid。W-μ∞ 则是 ∞−∞，只能用 integral_indeterminate。

## I7 类型与量词

- **检查什么**：每个 R/D/T/X 声明实/复、有限/扩展值、参数域、函数代表元、a.e. 基准、量词和极限次序；log f、1/f、e^Φ/f 在零集上的可测延拓要写明；复值量不得使用序关系夹逼；退化分支不能被未定义的参数界静默排除。
- **不通过**：同一符号兼两种角色；漏写正/有限质量；复值主项使用 ≤ 夹逼；多个极限次序不明；r=0 却使用未定义的 σ_1 或步长界。
- **怎么改/怎么记**：把量词复制到对象、R/T、保持引理和全部核心边；补代表元/延拓及次序；单列退化分支；不确定时标 undecidable_under_current_criteria，不猜测。
- **金样例**：T04 object 明确 r∈{0,…,min(m,n)}；r=0 时 A=0、空和取 0、A†=0，梯度下降有独立的 D-gd0（任意 η>0），不能套用正秩分支的 0<η<2/σ₁²。T08 的 C2 则明确 g 取 t₀ 连续代表元，并用模估计处理复值 g。

## N/A 和判决

只有 I3、I5（以及 I6 的 guard/App-Debt 部分）可 N/A，必须写满足哪条 N/A 条件及 residual risk。任一适用不变量违反即 fail；全部适用不变量满足才 pass；条款本身无法判定才是 undecidable_under_current_criteria。这些结果只作为草案诊断交用户，不是 agent 的认证动作。

