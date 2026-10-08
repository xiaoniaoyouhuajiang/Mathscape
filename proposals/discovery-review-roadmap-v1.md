# 内容发现审核与回链路线图 v1

本文将当前内容发现相关的六项工作拆成四个可独立验收的提案。它们建立在现有的 `DiscoveryRunCheckpoint`、`SourceReviewProjection`、Artifact Pool、来源登记和用户地图编辑能力之上，不改变 A/B 内容发现的数学职责。

## 1. 工作拆分

| 提案 | 覆盖目标 | 主要边界 |
| --- | --- | --- |
| [Discovery Run Ledger & Recovery](discovery-run-recovery-v1.md) | 1. 结果恢复与持久化入口 | 后端运行记录查询、重放验证、前端任务入口 |
| [Discovery Review Workspace](discovery-review-workspace-v1.md) | 2. 页面重构、3. 渲染回链、4. 审核局部图 | 前端审核工作区、Markdown/KaTeX 预览、局部图 |
| [Markdown–PDF Provenance Backlink](markdown-pdf-backlink-v1.md) | 5. Markdown 到 PDF 的精确回链 | Marker 页码/块映射、来源定位 DTO、PDF 阅读器联动 |
| [Discovery Registration Reversal](discovery-registration-reversal-v1.md) | 6. 按一次实际登记撤回已入图结果 | ImpactPlan、两种退役政策、共享保护、SourceSpan 与重新登记闭环 |

## 2. 推荐顺序

```text
P1 Run Recovery
       ↓
P2 Review Workspace ───────┐
       ↓                   │
P3 Markdown → PDF Backlink │
       ↓                   │
P4 Registration Reversal ←─┘
```

更具体地说：

1. 先完成 P1，使完成但尚未接受的结果不再依赖前端内存；
2. 再完成 P2，将结果、渲染 Markdown 和审核局部图组织成一个工作区；
3. P3 可以在 P2 的回链接口稳定后并行开发，但需要 Marker/转换元数据支持；
4. P4 最后开发，因为它依赖 P1 的 run 身份、来源登记回执和现有地图编辑/ImpactPlan。

## 3. 总体不变量

- Discovery checkpoint 是审计数据，不是可直接信任的领域实体；重新展示前必须确定性重验证。
- 原始 PDF、原始 Markdown 和修订 Markdown 都是不可变 Artifact，不被撤回操作删除。
- 回链以 `artifact_id + conversion_id + byte_range + block_ids` 为身份，不依赖前端字符串搜索。
- SourceReviewProjection 是可重建的只读 DTO，不成为新的权威存储。
- 撤回已入图结果只能生成新的生命周期 revision，不能物理删除旧 revision。
- 被多个 run 复用的对象不能因撤回某一个 run 而直接退役。

## 4. 共同验收场景

至少应覆盖以下端到端场景：

1. 完成一次内容发现但不接受，重启应用后仍能从运行记录打开结果；
2. 点击 unit、引用、关系和诊断，右侧显示带公式渲染和精确高亮的 Markdown 上下文；
3. 在审核工作区中查看局部结构图，悬停/点击节点与原文高亮双向同步；
4. 如果存在页码映射，从 Markdown anchor 打开对应 PDF 页，否则明确提示只能定位到转换页范围；
5. 对 run 下选定的实际登记生成撤回 ImpactPlan，复用对象、受影响关系和可安全退役对象分别显示；
6. 取消或失败不产生半个 revision、半个撤回 commit，也不丢失原始发现记录。
