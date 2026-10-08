# Discovery Run Ledger 与结果恢复提案 v1

## 1. 目标

让一次内容发现结果具有稳定的用户入口。用户关闭 Markdown 阅读器、切换到 PDF、重启应用或暂时不接受结果后，仍能从工作区找到同一个 `run_id`，重新打开经过确定性重验证的审核结果。

本提案覆盖路线图中的第 1 项，不负责重新设计审核页面，也不负责撤回已入图对象。

## 2. 当前基础与缺口

当前已经存在：

- `discovery_runs` 表及 `DiscoveryRunCheckpoint`；
- `discovery_units`、`discovery_fact_tasks`、`discovery_pending_tasks`；
- 验证报告、尝试记录和 `snapshot_json`；
- `revalidate_discovery_checkpoint`；
- `SourceReviewProjection` 的纯函数投影；
- 等待 Markdown Patch 审核时的恢复入口 `get_pending_markdown_patch_review`。

当前缺口：

- 没有列出某个 Artifact 的历史 discovery runs 的 API；
- 没有按 `run_id` 重建完成/部分完成结果的 API；
- 前端 `MarkdownSelection` 只在当前 reader 实例中保存 `lastOutcome`；
- 已完成但未接受的结果在数据库中存在，却没有用户可发现的入口；
- 运行状态与接受状态分散在 `discovery_runs`、`discovery_acceptance_decisions` 和 `source_registration_commits`，前端需要一个统一摘要。

## 3. 运行记录语义

`discovery_runs.status` 只描述流水线状态，不代替用户接受决定。建议新增只读派生状态：

```text
processing       = running / awaiting_patch_review
reviewable       = completed / partial / needs_attention
accepted         = 存在 discovery_acceptance_decisions
registered       = 存在 source_registration_commits
discarded        = 用户明确放弃，但保留 checkpoint
```

`discarded` 不应复用 LLM 工作流的 `cancelled`。取消表示运行没有正常完成，放弃表示用户审阅后不希望继续使用结果。

v1 可以新增 `discovery_run_dispositions` 表，也可以先以查询结果派生，随后再加入 `discarded` 的显式写入。若需要用户在多个设备/窗口间看到一致状态，应采用显式表。

## 4. 后端 DTO 与 API

### 4.1 运行摘要

```rust
struct DiscoveryRunSummary {
    run_id: StableId,
    artifact_id: StableId,
    conversion_id: Option<StableId>,
    input_fingerprint: ContentHash,
    result_fingerprint: Option<ContentHash>,
    pipeline_version: String,
    workflow_status: DiscoveryCheckpointStatus,
    review_status: String, // processing | reviewable | accepted | registered | discarded
    unit_count: usize,
    fact_count: usize,
    pending_task_count: usize,
    unresolved_reference_count: usize,
    relation_count: usize,
    updated_at: Timestamp,
}
```

### 4.2 查询接口

```rust
list_discovery_runs(artifact_id: Option<StableId>)
    -> Vec<DiscoveryRunSummary>

get_discovery_review(run_id: StableId)
    -> DiscoveryReviewEnvelope

discard_discovery_run(run_id: StableId, reason: String)
    -> DiscoveryRunDispositionReceipt
```

```rust
struct DiscoveryRunDispositionReceipt {
    run_id: StableId,
    disposition: String, // discarded
    request_id: StableId,
    decided_at: Timestamp,
}
```

`DiscoveryReviewEnvelope` 包含：

- 重新验证后的 `DesktopDiscoveryView`；
- 重新投影的 `SourceReviewProjection`；
- 原始材料身份和当前 Artifact 可读性检查；
- 接受/登记摘要；
- 如果 checkpoint 无法重现，返回诊断而不是伪造部分结果。

`get_discovery_review` 必须调用现有的确定性重验证链路，不能直接把 `snapshot_json` 反序列化后当作可信结果。

当 run 仍处于 `awaiting_patch_review` 时，返回 Patch 审核状态和可恢复的 patch set；当 run 处于 `completed`/`partial` 时，才返回完整的来源审核投影。

### 4.3 幂等和并发

- 查询是只读的；
- `discard_discovery_run` 使用 `request_id`，并要求 `expected_result_fingerprint`；
- 已登记 run 不能被“放弃”，必须走 P4 的撤回流程；
- Artifact 内容哈希漂移时，结果显示为 `stale`，不能定位原文或接受。

## 5. SQLite 设计

优先复用现有 checkpoint 表。只有需要显式“放弃”时才新增：

```sql
discovery_run_dispositions(
  run_id TEXT PRIMARY KEY,
  disposition TEXT NOT NULL, -- discarded
  reason TEXT NOT NULL,
  actor TEXT NOT NULL,
  request_id TEXT NOT NULL UNIQUE,
  decided_at TEXT NOT NULL
)
```

不复制整个 `SourceReviewProjection` 到数据库；它应该由 checkpoint 重验证生成，以避免 DTO 与权威数据漂移。

## 6. Desktop 交互

增加一个可折叠的“内容发现记录”入口：

- 按材料显示最近 run；
- 显示状态、单元数、引用数、未解析引用数和登记状态；
- “打开审核结果”调用 `get_discovery_review`；
- “放弃结果”只对未登记 run 开放；
- 打开 PDF 或 Markdown 后，原审核结果仍可从记录入口返回。

阅读器中暂时可以保留当前内嵌结果，但不再把它作为唯一入口。

- [ ] TODO：对已放弃但未登记的 run 提供“恢复审核”，保留放弃历史、追加恢复审计，不重新调用 LLM 或自动入图；仍走正常原文重绑定与接受验证。具体边界见[撤回提案中的恢复审核 TODO](discovery-registration-reversal-v1.md)。当前实现仍禁止接受已放弃 run，此项尚未实现。

## 7. 验收标准

1. 运行完成后关闭 reader，重新打开同一材料可以找到并恢复结果；
2. 完全退出并重启应用后，已完成但未接受的结果仍可查看；
3. 未解析引用的状态和候选目标与原 run 一致；
4. 当前 Markdown 内容哈希变化时，系统拒绝静默展示旧锚点；
5. 正在等待 Patch 审核的 run 仍由现有恢复入口打开；
6. 已登记 run 不会被“放弃”误处理。

## 8. 非目标

- 不改变 A/B Prompt；
- 不实现 run 级撤回地图对象；
- 不在本提案中解决 PDF 页码级定位；
- 不把模型原始回复直接暴露为审核权威。
