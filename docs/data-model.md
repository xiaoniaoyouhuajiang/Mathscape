# 数据模型与规则

## 来源、语义、对齐与 R/D/T/X

来源层保存“哪份材料、哪段原文、如何得到”：Work/Edition 表达书目，Artifact 表达内容，ConversionRun 表达派生，SourceSpan 定位证据，SourceSemanticUnit 表达局部数学言语单元，SourceUnitRelation 表达有证据的来源关系。

语义层计划表达可复用的数学对象、命题、表示和方法；对齐层连接来源局部表达与共享实体，并保留假设、证据和未完成义务。不同书中相似文字仍是不同来源，不因名称、相似度或模型判断直接合并。

R 是表示，D 是固定表示内推理，T 是跨表示运输，X 是图扩展与宏。它们应从语义、对齐与证明状态派生，不能取代来源证据。上述语义能力仍处探索阶段；研究依据是外部 [EMIR](../../docs/rdtx/emir-source-semantics-alignment.md) 与 [R/D/T/X](../../docs/rdtx/rdtx-map-construction-spec.md)，不是当前数据库的完整实现说明。

EMIR 区分来源方言 DialectScope、连接方言与共享表示的 RepresentationBinding，以及证明局部 ProofContext/ProofState。单个证明步骤不应复制整套方言环境；这些是后续语义开发的设计依据，当前来源登记可保留未上下文化状态。

## 身份与修订

- StableId 标识对象，RevisionId 标识版本；描述变化不能靠静默覆盖消除历史。MapPatch 表达带修订前置条件的受控变更；各存储入口仍须执行对应不变量检查。
- Artifact 的内容哈希与字节大小不可改变；媒体类型、文件名、页数和版权描述可以修订。字节变化必须产生新 Artifact。
- ArtifactLocation 与追加式 ArtifactCheck 独立于内容身份；移动文件或重新检查不产生新的数学对象。
- 转换和来源关系使用角色化端点，证据也是明确端点；不能将超边压成失去证据与身份的普通边。
- 来源单元只保存 parent，不复制 children、对齐或证明上下文的反向列表。新增关联不应迫使原单元修订。
- 当前登记按 Artifact 与范围确定性生成来源身份，不执行跨来源数学去重。同一稳定身份可复用当前 revision，或经明确审核重新激活/替换；定位身份冲突和共享内容替换必须阻断，不能借幂等之名覆盖。

## 定位与输入授权

Markdown 定位使用原始 UTF-8 字节半开区间，保留换行、空白间隙与原文。块是物理锚点，不是数学单元；渲染后的 DOM、KaTeX 文本或模型重写不能作为定位依据。

后端从受管 Artifact 重读并核验快照、转换归属与选区；客户端传来的哈希和“已解析结果”不具权威性。请求内 bN 句柄不能跨请求或批次复用。超限必须拒绝或显式规划，不能静默截断或扩大授权范围。

SelectorResolver 只做受限、可记录的规范化，不用编辑距离猜测数学符号；多个匹配不能任取一个。位置可解析不等于单元边界或数学解释正确。跨块精度审查和原始提案必须保留，缺少上下文只能记录问题，不能补造来源。

Markdown 精确定位不等于 PDF 精确定位。SourceSpan 可以只含有效 Markdown 定位；没有可靠映射时不制造 PDF bbox。PDF 物理页索引为零起点，印刷页码、Marker 页号与界面页码不能混用；持久坐标采用显式、归一化坐标约定。

Ink 区分可编辑 `InkDocument`、书写会话、追加式操作历史和固定前缀 `InkSnapshotManifest`。新建和自动保存保留操作事实；显式保存快照才生成 Artifact。`SourceSpan.ink` 保存快照 Artifact、稳定 page_id、排序去重的 stroke_ids 和选择方式，验证笔迹在该前缀中可见且属于指定页；局部擦除后的 fragment 有独立身份。只读选择不编辑笔迹，当前文档的后续修改不改变旧定位。

用户指定 Ink 选区的来源登记是独立入口：只创建可定位的 SourceSpan，不伪造 discovery run，也不自动晋升为数学单元或证明。请求绑定快照和选区，事务保存地图对象、成员 revision 与幂等回执；相同证据复用，退役对象须经现有治理流程处理。草稿删除、快照清理与引用保护待系统级删除模型统一设计，自动保存不意味着永久留存。

## 从提案到登记

```text
模型提案 → 结构与来源校验 → 候选结果 → 用户接受 → 来源登记
```

每个箭头有独立条件：Schema/DTO 解码仅验证形状；确定性验证检查原文、授权、父链、覆盖、证据归属和端点。coverage 说明处理覆盖，不保证数学单元发现完整；任务 processed 不证明没有遗漏。

通过验证不等于被接受，被接受不等于登记成功，正式来源登记不等于数学正确。`proves` 是原文证明关系，不是内核证明证书。TrustRecord 将 provenance、parse、segmentation、typing、proof、alignment、curation 分轴记录；模型 confidence 不自动提升可信状态。即使未来 Lean 通过，也须另行检查翻译是否忠实于原文。

持久化 JSON 只保存运行事实，不能直接恢复 ValidatedUnitSet 或 ValidatedFactTask 的权限。接受时必须重新绑定原文、重跑校验并重现结果指纹；前端只提交 run、预期指纹和选择，不提交“可信节点”。

接受选择必须包含 parent，并闭合已选 source 的内容关系 target。部分结果可接受，但未完成事实仍须标记 pending。Edition 未解析、DialectScope 未构建不阻断来源登记，也不能用虚假书目或空环境冒充已经解析。

接受决定绑定结果、审核者和选择；重复提交复用决定与回执。来源对象、关系端点和登记回执在一次提交事务中写入。接受计划与正式提交是不同步骤，提交失败不能被展示成已登记。来源登记后的类型仍可 unchecked，证明为 none，对齐为 unmatched。

退役创建 Deprecated revision，不物理删除。普通退役会级联必要的关联关系，但不自动删除子单元或引用者；登记撤回仅退役实际登记范围内无其他采纳或活动依赖的创建/重新激活对象，保留共享对象、后续修改与其依赖闭包。登记采纳状态、工作流状态和对象 lifecycle 独立。旧回执绑定原 revision；旧决定的重试不重新激活对象，重新采纳使用新 run 和登记变更审核。

未登记结果的放弃/恢复也是独立的用户决定：追加审计保留所有周期，当前指针确定是否允许审核。恢复前需重绑定原文、重放校验；成功只开放原结果审核，不调用 LLM 或自动入图，已有登记不能走此恢复入口。

## 代码入口

- 身份与来源：[domain](../crates/mathscape-domain/src/lib.rs)、[source](../crates/mathscape-domain/src/source/mod.rs)、[trust](../crates/mathscape-domain/src/trust.rs)。
- 不可信阶段 DTO：[staged_discovery](../crates/mathscape-domain/src/staged_discovery.rs)；定位：[selector_resolver](../crates/mathscape-orchestrator/src/selector_resolver.rs)、[occurrence_resolver](../crates/mathscape-orchestrator/src/occurrence_resolver.rs)。
- 验证与登记：[unit_discovery](../crates/mathscape-orchestrator/src/unit_discovery.rs)、[source_facts](../crates/mathscape-orchestrator/src/source_facts.rs)、[source_registration](../crates/mathscape-orchestrator/src/source_registration.rs)、[原子提交](../crates/mathscape-store-sqlite/src/source_map.rs)。
- 登记生命周期：[契约](../crates/mathscape-domain/src/registration.rs)、[规划](../crates/mathscape-orchestrator/src/registration.rs)、[原子撤回与重新登记](../crates/mathscape-store-sqlite/src/registration.rs)。
