# Discovery Registration Reversal 提案 v1

## 1. 目标

为“用户已经接受一次内容发现并入图，但之后认为整体结果不满意”的场景提供可审计、可预览、可撤销的批量操作。

本提案覆盖路线图第 6 项。它不是物理删除，也不是重新运行内容发现；它是基于一次来源登记的批量生命周期修订。

## 2. 为什么不能复用单节点退役

当前用户地图编辑支持单个对象退役，但一次 discovery registration 可能同时产生：

- 多个 SourceSpan；
- 多个 SourceSemanticUnit；
- 多个 SourceUnitRelation；
- 父子关系和证据引用；
- 被旧 run 复用的对象；
- 被后续用户编辑或其他 run 引用的对象。

逐个点击退役会让用户无法知道整体影响，也容易退役共享对象。因此入口必须以 `decision_id`/`commit_id` 为范围，先生成 ImpactPlan，再一次确认。

## 3. 操作语义

### 3.1 尚未入图

P1 的“放弃 run”只改变运行记录 disposition：

- 不产生 MapPatch；
- 不修改 Artifact；
- 不修改地图；
- checkpoint 和模型结果仍可读。

### 3.2 已入图

新增语义命令：

```rust
reverse_discovery_registration {
    request_id: StableId,
    decision_id: StableId,
    expected_commit_id: CommitId,
    reason: String,
}
```

流程：

```text
decision/commit
      ↓
读取 SourceRegistrationReceipt
      ↓
按创建/复用/当前 revision 分组
      ↓
ImpactPlan
      ↓ 用户确认
MapPatch(退役或 supersede revision)
      ↓
CommitReceipt
```

## 4. 影响分析规则

### 4.1 可直接退役

满足全部条件的对象才进入默认选中集合：

- 出现在本次登记的 `created_objects`；
- 当前 revision 仍是本次登记生成的 revision；
- 没有被其他活动对象作为 parent、evidence、candidate target 或 relation endpoint 引用；
- 对象不是工作区中唯一的共享 canonical 对象。

### 4.2 需要人工决定

以下对象只列为 `affected` 或 `shared`，不默认退役：

- receipt 中的 `reused_objects`；
- 已有后续 revision 的对象；
- 被其他来源单元引用的 SourceSpan；
- 同时被其他 run 使用的 SourceSemanticUnit；
- 退役会导致关系端点不闭合的关系对象。

### 4.3 关系处理

本次创建且端点都被退役的关系可以一起退役。共享关系需要单独列出。不能因为退役一个 unit 就自动物理删除关系记录。

## 5. DTO

### 5.1 请求

```rust
struct ReverseDiscoveryRequest {
    request_id: StableId,
    decision_id: StableId,
    expected_commit_id: CommitId,
    reason: String,
}
```

### 5.2 ImpactPlan 扩展

沿用现有 `ImpactPlan`，增加以下 reason/action 值：

```text
action = retire | keep_shared | blocked | supersede
reason = created_by_target_run
       | reused_by_other_run
       | referenced_by_active_object
       | already_superseded
       | endpoint_dependency
```

建议为计划增加：

```rust
struct DiscoveryReversalSummary {
    decision_id: StableId,
    source_registration_commit: CommitId,
    created_count: usize,
    reused_count: usize,
    retireable_count: usize,
    shared_count: usize,
    blocked_count: usize,
}
```

### 5.3 回执

复用现有 `CommitReceipt`，并增加：

```rust
struct DiscoveryReversalReceipt {
    version: String,
    request_id: StableId,
    decision_id: StableId,
    original_commit_id: CommitId,
    reversal_commit_id: CommitId,
    retired_objects: Vec<StableId>,
    retained_shared_objects: Vec<StableId>,
    blocked_objects: Vec<StableId>,
}
```

## 6. Domain/Store 实现

- 不新增物理删除操作；
- 使用现有 `MapPatch` 和生命周期 revision；
- `RetireObject` 的后端构造必须读取当前 envelope，复制 payload 后生成新 revision；
- `expected_revision` 必须匹配当前对象；
- commit 内所有对象原子写入；
- 幂等键为 `request_id`，重复提交返回原回执；
- 原始 `source_registration_commits` 和 discovery checkpoint 永久保留。

默认生命周期建议：

- 用户认为结果错误：`Superseded` 或 `Deprecated`；
- 明确拒绝模型结果：`Rejected`；
- 不使用 `Deleted` 作为普通撤回语义。

## 7. Desktop 交互

入口位置：

- Discovery run 记录页的“撤回本次登记”；
- 来源审核/登记回执页的“查看登记影响”；
- 不在单节点 Inspector 中伪装成普通退役。

确认页必须显示：

1. 本次 run 和原始材料；
2. 将退役的对象及类型；
3. 将保留的共享对象；
4. 被其他内容依赖的阻塞对象；
5. 可能留下的关系和证据边；
6. “撤回后仍保留历史，可重新运行内容发现”的说明。

撤回完成后，默认地图隐藏退役对象，但历史视图仍可以查看旧 revision 和原始登记回执。

## 8. 一致性与安全门

- 如果 `expected_commit_id` 已变化，重新生成 ImpactPlan；
- 如果对象在预览后产生新 revision，提交失败，不自动覆盖；
- 如果来源登记没有 receipt，拒绝按 run 撤回；
- 如果 run 只部分登记，撤回范围以实际 decision/receipt 为准，不按 checkpoint 中所有单元猜测；
- 如果对象已被其他用户明确编辑，默认阻塞而不是覆盖。

## 9. 验收标准

1. 未入图 run 可以放弃，但地图完全不变；
2. 已登记 run 能生成包含 created/reused/shared/blocked 分类的 ImpactPlan；
3. 撤回只写新 revision，不删除旧对象、Artifact 或 checkpoint；
4. 共享对象不会被错误退役；
5. 预览和提交之间发生 revision 漂移时提交失败；
6. 重复提交具有幂等性；
7. 撤回后可以基于同一 PDF/Markdown 重新运行内容发现。

## 10. 非目标

- 不负责修复模型提案内容；
- 不负责 PDF/Markdown 回链；
- 不做跨多个 discovery run 的合并清理；
- 不提供物理删除数据库记录的功能。

