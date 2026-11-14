> 路径和版本解析见 [`reference-index.md`](reference-index.md)；文中出现的准则名、案例名和工具名都必须先按索引解析。

# 逐类创建、调整与废弃规则

以下“创建”都表示提出草案，实际写入走 `propose_changes` → `staged/blocked` → 用户 `accept/discard`。已开放写入的包括来源层（`source_span`、`source_unit`、`source_relation`）与语义层第一期对象（`semantic_entity`、`alignment`、`debt`、`dialect_scope`）；其余 R/D/T/X 与证明状态工具仍标为【假定】。任何 `edit` / `retire` 都带 `target_id` 和 `expected_revision`；不要自带正式 id。

## 通用前置

先用 `workspace.search`（查理论与共享对象时可带 `kinds: ["theory"]` / `["semantic_entity"]` 和 `scope: "investigation"`）、`graph.get`、必要时 `graph.neighborhood` 和 `material.read` 找到来源与邻域，避免重复建同名对象。所有对象带来源证据、revision、scope 和 rationale。没有 locator 就不能声称已核对原文。`proves` 是来源中的证明关系，不是内核证书。

## 来源层

### 来源单元与来源关系

- **什么时候创建**：发现一个能独立解释、引用和回链的定义、定理、证明步骤、方法或问题时创建 `source_unit`；只有来源本身表达了“由……得到/包含/位于”等关系时创建 `source_relation`；缺少原文片段锚点时先用 `material.locate` 取精确范围并创建 `source_span`。
- **前置条件**：已有 Artifact/Span 或可靠 `content_ref`；先查同一材料和 revision。来源对象支持 `create` 与 `edit`（`source_span` 不可变，只支持 `create`；来源对象不支持 Agent 提议 `retire`）。
- **关键字段**：kind、标题/来源标签、source_spans、父单元、关系类型、端点和证据片段；摘要不能替代正文回链。
- **常见错误**：把整章当单元或单个超长 span；把解析修正当原文；把相似文字直接合成一个共享实体；无锚点凭记忆建 span。
- **工具**：已确认 `workspace.search`、`graph.get`、`graph.neighborhood`、`material.read`、`material.locate`、`propose_changes`。

## 对象卡、上下文与守卫

### 对象卡

- **什么时候创建**：开工前，针对目标定理/工具创建一张卡；不是每一个被对齐实体一张。
- **前置条件**：读过对应 object.md 和来源；确认目标的形式结论、全局前提、分支、量词域、排除项和路线标识。
- **关键字段**：object_ref、工具一句话、逐分支结论承诺、全局标量/域/正则性、参数和极限次序、分支 guard、排除项、来源和 revision。
- **常见错误**：把“已有 R”当结论；漏写 B-up 的有限正质量；把对象外路线扩展塞进核心结论；每个实体单独造卡。
- **工具**：当前没有独立的对象卡节点类型；核心精确声明直接建为 `semantic_entity (declaration)`，路线级规划保存在 `propose_changes` 的 `rationale`、`unresolved` 或会话 scratch 中。

### SemanticEntity（含 Theory）与 Alignment

- **什么时候创建**：不同来源单元确实指向同一可复用数学概念、精确说法或题型时，先查已有 `theory`（专题书架）与 `semantic_entity`，缺少时建立 `SemanticEntity`，再建立带关系类型、作用域、参数/项下标对照和证据的 `Alignment`；相似措辞本身不够。
- **理论（`theory`）书架规则**：理论保持扁平（`home_theory: null`），尺度类似课本一章/一个专题（如「线性递推数列」），只回答“这几个概念和定理大概是哪一块儿的”。建前先 `workspace.search`（`kinds: ["theory"]`, `scope: "investigation"`）；新建时填好中英文 `aliases`，不用单个概念名或带 `=`/`$` 的具体算式起名。关于某类对象的定理跟该对象挂同一主书架；日后需重组时，可通过 `edit` 修改非 theory 对象的 `home_theory` 平滑迁移。
- **前置条件**：来源单元已锚定；能区分 `expresses`、`specializes`、`overlaps_with` 等 11 种对齐关系和候选状态；不确定时保留 `retrieved_candidate` / `partially_aligned` / `disputed`，不覆盖已有实体。
- **关键字段**：`semantic_kind`（`concept | declaration | theory | problem_schema`）、`home_theory`、`preferred_labels`、`aliases`、`conventions`、`mathlib_baseline`（`status: "unchecked"`）；对齐线的 `source`、`target`、`relation`、`source_context`、`parameter_map`、`term_index_map`、`lost_information`、`evidence_spans`、`status`；禁止 `same_as`。
- **常见错误**：把两本书的 `SourceSemanticUnit` 直接合并；把 `parameter_map`（第几条命题）说成数列错开；把 Alignment 当 T；自设 `accepted` 或 `trust`；用具体算式给理论起名。
- **工具**：已确认 `propose_changes`（`kind: "semantic_entity" | "alignment"`，支持 `create` / `edit` / `retire`）、`workspace.search`、`graph.get`、`graph.neighborhood`（`edge_types: ["aligns", "in_theory"]`）。

### DialectScope（约定卡）

- **什么时候创建**：来源在某一节范围内固定编号起点（如从第 1 项起编）、符号约定或术语叫法（如把 Fibonacci 数列叫“兔子数列”）时创建；锚定在章节 `section` 单元上（`applies_to: "anchor_subtree"`），其下所有子单元自动复用。
- **前置条件**：来源范围可定位，每条约定至少有一条 `evidence_spans`；没有约定卡时对齐线写 `uncontextualized` 并注明原因，不要用空壳约定冒充已解析。
- **关键字段**：`label`、`anchor_unit`、`applies_to`、`introduced_by_unit`、`conventions`（含 `key`、`subject`、`value`、`text`、`explicitness`、`evidence_spans`）。
- **常见错误**：每一步证明复制一份 scope；用 ProofContext 的临时假设污染来源方言；推断出来的约定（`inferred`）不标明。
- **工具**：已确认 `propose_changes`（`kind: "dialect_scope"`，支持 `create` / `edit` / `retire`）、`graph.get`、`graph.neighborhood`（`edge_types: ["scope_of"]`）。

### guard_registry 与实例 guard

- **什么时候创建**：对象存在分支划分、覆盖/互斥条件或把定理重新用于新载体/新测度时创建。泛 guard 和实例 guard 都要入 registry；如 T12 的 T1@λΦ 是独立实例。
- **前置条件**：对象卡已列分支；能说明谓词、覆盖哪些分支、两两互斥性和 overlap。overlap 不是互斥分割，要逐重叠支检查。
- **关键字段**：guard_id、predicate、覆盖分支、mutually_exclusive、covering、overlap、实例化参数、被哪些核心边消费。
- **常见错误**：把 guard 记成 Debt；实例边只引用源 guard；把 overlap 当互斥；用一个 G-fin 代替 T1@λΦ。
- **工具**：rdtx.upsert_guard【假定】。

## 表示和证明结构

### RepresentationBinding

- **什么时候创建**：一个来源 DialectScope 要复用共享 Theory 中的某个 R 时创建；每种方言保留自己的符号映射和边界适配。
- **前置条件**：R 已存在或同提案创建；知道 binder 映射、边界重编号和适用 profile；不等价的 R 不静默合并。
- **关键字段**：dialect_scope、theory_ref、R_ref、局部→共享符号映射、量词/类型适配、证据、revision。
- **常见错误**：把 binding 当运输证明；只因名称相同就复用 R；漏记 n>n_0 与 n≥N 的边界适配。
- **工具**：emir.upsert_representation_binding【假定】。

### R 节点

- **什么时候创建**：一个被核心 D/T 边消费的表示、载体或算子集合出现时创建；同一表示只建一个可复用 R。
- **前置条件**：对象卡已说明它服务的分支；写清 carrier 三元组 (base carrier, semantic payload, operator set)，必要时写 primary_class/secondary_features（C3 > C4 > C2 > C5 > C1 > C0）。
- **关键字段**：R id、分支/对象、carrier_type、隐含载荷、合法对象与算子、量词域、代表元、绑定和来源。
- **常见错误**：改名就新建 R；把源定义和新算子混在一个 R；由 X 生成且被核心边消费的中间载体不登记独立 R。
- **工具**：rdtx.upsert_node(rdtx_kind=R)【假定】。

### X（宏）

- **什么时候创建**：重复的定义展开、见证集、切分、换元或方法模式会遮住主链时创建宏；X 是压缩，不是偷藏前提。
- **前置条件**：能列出完整展开、输入输出接口和所有 side Debt/guard；被核心边消费的中间载体另建 R。
- **关键字段**：宏接口、展开子图/局部定义、分支、量词、消费它的边、是否 view/App。
- **常见错误**：用 X 隐藏缺失证明；把真正跨表示的保持引理压成 X；把 view/App 放进核心偏序。
- **工具**：rdtx.upsert_node(rdtx_kind=X)【假定】、rdtx.upsert_edge【假定】。

### ProofContext / ProofState

- **什么时候创建**：进入一条证明路线时建 ProofContext；目标变成新子目标、引入/解除局部声明或分支状态改变时建新 ProofState。
- **前置条件**：已有 object、路线和来源引用；能区分长期来源方言与本题局部 binder/假设。
- **关键字段**：所属路线、父 context、局部 binder、临时假设、当前 goal、状态 revision、产生它的 D/T 边。
- **常见错误**：把整个 DialectScope 复制到每步；覆盖旧 state；把局部 witness 写成全局前提。
- **工具**：emir.upsert_proof_context / emir.upsert_proof_state【假定】。

### D 边

- **什么时候创建**：同一表示内，由定义、已登记 Debt/guard/L 和已有边可逐步推出结论时创建。
- **前置条件**：两端在同一 R/ProofState；列出所有非定义性前提、消费的 Debt/guard/L 和偏序位置。
- **关键字段**：edge_id、from/to、分支、前提、结论、消费索引、状态、序、证据。
- **常见错误**：用 D 跨越两个 carrier；漏记跨分支消费者；把原文 proves 当形式证书；把“看起来等价”当 D。
- **工具**：rdtx.upsert_edge(edge_kind=D)【假定】。

### T 边与重建证书

- **什么时候创建**：目标 R 引入源定义不能直接展开的新语义对象/算子，且需要独立保持引理时创建硬 T。若只是换坐标、同表示换元或定义性计算，降为 D/X/view。
- **前置条件**：通过 I3 的最小不可消除性判断；若删 T 后已有源内 D/X 能重建全部结论，就不能保留硬 T。
- **关键字段**：源/目标 carrier、精确保持引理、量词、适用分支、重建映射、证据端点、实例 guard、消费者。
- **常见错误**：把同义改写标成 T；T 没有保持引理或重建映射；把 T1@λΦ 当新硬 T 类型；跨表示误用 D。
- **工具**：rdtx.upsert_node(rdtx_kind=T)、rdtx.upsert_edge(edge_kind=T)【假定】。

## Debt、引理、证据和失败

### Debt 三类

- **什么时候创建**：核心边的成立确实依赖一个可去掉的假设时创建原子 Debt；不存在失败见证时不要保留该分支 Debt。
- **theorem-Debt**：去掉后对象的定理结论失败。例如 T12 Debt-μP 由 W-μ 见证 KL 变负；写 scope=A/B-lo/B-att，不扩到 B-up。
- **statement-Debt**：去掉后原 object 陈述因定义域、类型或算子适用性失效，但不据此断言自然扩展结论失败。例如 T12 Debt-μfin-up 的 W-μ∞ 使 ∫Φ dμ 成为 ∞−∞，记录 statement_applicability / premise_inapplicable / integral_indeterminate。
- **route-Debt**：只说明当前路线需要它。例如 T12 RD-μpos 仅服务 B-up 的平移截断路线；见证域写扩展域 D_up^0，并记录 proof_route / route_failure / normalization_route_failure。若是对象外域扩展且不是被移除的域条件，标 route-extension note，不进核心 I2/I6。
- **前置条件**：每条写 debt_kind、failure_target、scope/量词、consumer、保持其余 Debt/guard/结论前提不变的具体 witness；不同分支类型不同就拆 ID。跨分支消费要把被消费 Debt/guard 和传递 consumer 放进偏序，不能默认全局清偿。
- **常见错误**：把 guard/L 当 Debt；写一个“所有 Debt”大债；把 B-up 消费 B-lo 的 Debt；用不满足其他前提的反例；把 route-Debt 写成定理失败。
- **工具**：挂在对齐线或共享对象上的地图 Debt 已支持 `propose_changes`（`kind: "debt"`，`status` 最多写 `claimed_paid` 并附 `paid_by`，`paid`/`waived` 由用户确认）；路线级失败见证 `rdtx.record_failure`【假定】。

### 派生引理 L

- **什么时候创建**：事实由定义或已列 Debt 推出，并被至少一条核心边消费时创建；它不是额外假设。
- **前置条件**：写完整陈述、量词、输入 Debt/guard、输出和 consumer；若作为 Debt 清偿物，consumer 索引必须可追踪。
- **关键字段**：L id、陈述、作用域、依据、消费边、证据。
- **常见错误**：把未证明的关键假设命名为 L；L 没有端点；把 L* 聚合成一条 evidence。
- **工具**：rdtx.upsert_lemma【假定】。

### 每边证据

- **什么时候创建**：每条核心 D/T（以及被核心边消费的必要 L/保持引理）一边一行；X 是否需要端点按 v0.5.1 的 N/A 规则处理并写理由。
- **前置条件**：已有唯一 edge_id 和可回溯 locator/span/proves|states 关系。
- **关键字段**：edge_id、等级 complete|partial|待补、来源（外部或“非外部·自包含”）、覆盖量词、分支、边界、locator、待核说明。
- **常见错误**：用“见表”、L*、D-* 聚合；作者笔记当端点；只支撑方法却标 complete；漏写分支边界。
- **工具**：rdtx.attach_edge_evidence【假定】；定位先用已确认读工具。

### 失败记录（I6）

- **什么时候创建**：Debt 或分支边界有可检查的失败见证，或路线实际失败时创建；失败是一等记录，不删除节点。
- **前置条件**：见证保持其余 Debt、guard 和结论前提；每个被击穿的分支单独记录，可共用 witness id；填写 failure_target、顶层 failure_type、failure_subtype、branch。
- **关键字段**：目标（节点/边/分支结论）、scope、witness、theorem_conclusion|statement_applicability|proof_route、no_solution_or_minimum_absent|limit_nonexistent_or_wrong|rate_or_order_changed|premise_inapplicable|branch_conclusion_mismatch|route_failure，以及 subtype（如 integral_indeterminate、normalizer_invalid、normalization_route_failure）。Z=0 或 +∞/log Z 不可定义用 normalizer_invalid；只有 ∞−∞ 用 integral_indeterminate。对象许可扩展值且结论仍失败时 target 才可为 theorem_conclusion。
- **常见错误**：写“待讨论”；同一见证同时偷偷去掉多条 Debt；把 J 不实值写成 theorem failure；把无失败见证伪造成记录。
- **工具**：rdtx.record_failure【假定】。

### 第二路线与 (O,ρ) 等价

- **什么时候创建**：第一条路线完成后有独立证明思路（如 T04 α=SVD、β=投影/勾股，或 T12 α=Jensen、β=逐点不等式+倾斜测度）时，复制对象承诺而不是复制旧边，建立新 ρ。
- **前置条件**：两条路线都带同一 object_ref，各自完成 I1–I7；没有字段缺失、未登记 consumer 或 undecidable_under_current_criteria 才能比较。
- **关键字段**：路线 id、R/D/T/X、Debt scope、规范化保持引理签名、失败语义键 failure_target–branch–conclusion、跨分支传递消费者、比较结果。改名、写作顺序、X 展开/折叠不影响比较；不同路线可都合格但不等价。
- **常见错误**：把两条路线合并成一个对象等价类；比较单条边；用字面 edge id 比较；未补 I5 就宣布等价。
- **工具**：rdtx.upsert_node / rdtx.upsert_edge【假定】、get_change_set【已确认】。

### TrustRecord

- **什么时候创建**：需要记录来源、解析、切分、类型、证明、对齐或整理轴的当前状态时创建；不是“网建完自动可信”。
- **前置条件**：有对应证据和审查范围；区分 candidate/staged/accepted；模型 confidence 不算审查者。
- **关键字段**：对象 ref、轴、状态、证据、审查者/用户接受回执、时间和 revision；只更新声明的轴。
- **常见错误**：用一个总分覆盖所有轴；用 Lean 通过替代来源忠实；agent 自填 accepted_by；把 proves 当 Trust 证书。
- **工具**：emir.upsert_trust_record【假定】；正式接受由 Desktop 用户操作。

## 何时调整、拆分、合并、废弃

有新来源或理解时用新 revision；同接口且证据表明确实同一对象才合并。一个 R 承载两种 carrier 就拆 R；一个来源单元含两个承诺就拆来源单元并保留关系。候选路线失败记录 record_failure 后保留历史，正式退役由用户/Desktop 完成。任何合并都不能抹掉 DialectScope、ProofContext、Debt、guard、证据或失败记录。

