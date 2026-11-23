# 用户侧地图更新提案 v1

本文把“用户编辑已入图对象”拆成四个开发步骤。目标是让用户能够安全地修改、补充和退役来源层对象，同时保留 revision、审计和来源证据。本文不改变 LLM 内容发现协议，也不把图投影当作权威数据源。

## 1. 当前基础与边界

`mathscape-domain` 已有 `MapPatch<P>`、`MapOperation`、`stable_id`、`revision_id`、`base_commit` 和原子变更的领域抽象。当前 `mathscape-store-sqlite` 的 `commit_source_registration` 只服务于来源登记，接受 create 操作；它会拒绝 `ReviseEntity` 和 `ReviseRelation`，也没有退役、重挂接和通用冲突回执。

因此本提案增加一个独立的“用户地图变更”链路：

```text
用户草稿 → UserChangeRequest → ImpactPlan → 用户确认
                                      ↓
                              MapPatch 原子提交
                                      ↓
                             CommitReceipt → 图投影刷新
```

前端只提交语义明确的编辑请求，不能直接提交任意 `EntityEnvelope`、revision 或 SQLite 字段。后端根据当前权威快照构造并校验 `MapPatch`。

## 2. 步骤一：通用 MapPatch 提交能力

### 2.1 Domain 变更

保留现有的创建和修订操作，并增加显式的退役操作：

```rust
enum MapOperation<P> {
    CreateEntity { entity: EntityEnvelope<P> },
    ReviseEntity {
        stable_id: StableId,
        expected_revision: RevisionId,
        replacement: EntityEnvelope<P>,
    },
    CreateRelation { relation: RelationRecord<P> },
    ReviseRelation {
        stable_id: StableId,
        expected_revision: RevisionId,
        replacement: RelationRecord<P>,
    },
    RetireEntity {
        stable_id: StableId,
        expected_revision: RevisionId,
        lifecycle: LifecycleStatus, // Deprecated | Superseded | Rejected
        reason: String,
    },
    RetireRelation {
        stable_id: StableId,
        expected_revision: RevisionId,
        lifecycle: LifecycleStatus,
        reason: String,
    },
}
```

`Retire*` 是带新 revision 的生命周期变更，不能物理删除对象。`Deleted` 不作为普通用户操作的生命周期状态。`Retire*` 的 `lifecycle` 只能使用允许的退役状态，不能借此把对象重新设为 `Active`。

`Retire*` 不要求调用方提交完整 replacement。后端在当前快照中读取对象，复制原 payload，构造带新 lifecycle 的 revision，并把 `reason` 写入该次 commit 的操作审计记录。退役操作必须校验 `expected_revision`；退役 revision 的 fingerprint 必须包含生命周期和操作语义，不能与原 revision 相同。

`MapPatch` 继续携带：

- `base_commit`：用户编辑所依据的地图提交；
- `author`：用户或系统 actor；
- `generation_run`：如果变更来自 LLM 或重建任务则填写；
- `message`：用户可读的变更原因；
- `request_id` 或等价的幂等键：由前端生成，后端持久化并负责幂等。

同一 patch 内同一个 `stable_id` 只能出现一次。修订必须要求新 revision 与旧 revision 不同；退役也必须写入新的 revision。

### 2.2 SQLite 通用提交器

新增独立的 `apply_map_patch`，不要复用来源登记专用的 `commit_source_registration`：

```rust
fn apply_map_patch(
    &mut self,
    request_id: &StableId,
    plan_fingerprint: &ContentHash,
    patch: &MapPatch<SourcePayload>,
    committed_at: Timestamp,
) -> Result<CommitReceipt, StoreError>;
```

事务内依次完成：

1. 校验 `base_commit` 是否仍为当前提交；
2. 校验每个目标对象的 `expected_revision`；
3. 校验关系端点、父节点、证据范围和 Artifact 存在性；
4. 写入新 revision，并更新对象的 `current_revision`；
5. 写入 `map_commits`、`map_commit_objects`、操作原因审计记录和幂等请求记录；
6. 提交事务并返回回执。

来源登记仍使用原有入口，用户编辑使用新入口。两者共享底层 revision 和事务不变量，但不共享“只允许创建”的业务规则。

### 2.3 冲突和失败

`base_commit` 或 `expected_revision` 不匹配时，事务整体失败并返回冲突信息，不自动覆盖当前数据。前端应重新读取快照、保留用户草稿并重新生成 ImpactPlan。

失败不能产生半个 commit；幂等重试必须返回原 CommitReceipt。图投影只在收到成功回执后刷新。

建议增加 `map_change_requests`（或等价的变更请求表），至少保存 `request_id`、`plan_fingerprint`、状态、`commit_id` 和回执；另以 `map_commit_operations` 保存每个操作的目标、动作和 `reason`。规划阶段可以保存计划摘要，不能把计划本身当作权威地图数据。

## 3. 步骤二：ImpactPlan

ImpactPlan 是只读分析结果，不修改地图。它由后端根据当前快照和 `UserChangeRequest` 计算，供用户确认。

### 3.1 用户请求 DTO

```rust
struct UserChangeRequest {
    request_id: StableId,          // 前端生成的 UUID v7 或等价唯一值
    base_commit: CommitId,
    actor: ActorRef,
    reason: Option<String>,
    operations: Vec<UserChangeOperation>,
}

enum UserChangeOperation {
    CreateSourceUnit(CreateSourceUnitDraft),
    EditSourceUnit {
        stable_id: StableId,
        expected_revision: RevisionId,
        changes: SourceUnitChanges,
    },
    EditSourceRelation {
        stable_id: StableId,
        expected_revision: RevisionId,
        changes: SourceRelationChanges,
    },
    RetireObject {
        stable_id: StableId,
        expected_revision: RevisionId,
        reason: String,
    },
}
```

`SourceUnitChanges` 只允许修改该类型的可编辑字段，例如显示标题、标签、完整性和 parent。来源 Artifact、Markdown byte range、证据 hash 等字段不能由普通编辑请求直接修改；它们必须通过重新发现或专门的 Artifact 修订流程产生。

`parent` 使用三态表示：字段缺失表示保持不变，`Some(None)` 表示改为根节点，`Some(Some(id))` 表示改挂到指定父节点。重挂接只有这一条入口，不再提供独立的 `ReparentSourceUnit` 操作。

### 3.2 ImpactPlan DTO

```rust
struct ImpactPlan {
    version: String,              // mathscape-impact-plan-1
    request_id: StableId,
    base_commit: CommitId,
    plan_fingerprint: ContentHash,
    requires_confirmation: bool,
    status: ImpactPlanStatus,
    direct_changes: Vec<ImpactItem>,
    affected_objects: Vec<ImpactItem>,
    diagnostics: Vec<ImpactDiagnostic>,
    patch_preview: PatchPreview,
}

enum ImpactPlanStatus {
    Ready,
    Blocked,
    Conflicted,
}

struct ImpactItem {
    object_id: StableId,
    kind: EntityKind,
    action: ImpactAction,
    reason: String,
    current_revision: Option<RevisionId>,
    proposed_revision: Option<RevisionId>,
    required_user_decision: bool,
}

enum ImpactAction {
    Create,
    Revise,
    Retire,
    Preserve,
    Detach,
    Orphan,
    RecheckEvidence,
    Block,
}

struct ImpactDiagnostic {
    code: String,
    severity: DiagnosticSeverity,
    object_id: Option<StableId>,
    message: String,
}

struct PatchPreview {
    create_count: u32,
    revise_count: u32,
    retire_count: u32,
    relation_count: u32,
}
```

`plan_fingerprint` 绑定当前快照、请求和影响分析结果。提交时必须再次检查 `base_commit` 和 fingerprint，防止用户预览后数据已被其他任务修改。

### 3.3 影响策略

默认不级联退役。关系图不是树，后继对象可能被其他来源或关系复用。

| 目标 | 默认影响 | 用户选择 |
| --- | --- | --- |
| SourceUnitRelation | 退役该关系 | 保留端点 |
| SourceSemanticUnit | 退役该单元，相关关系退役 | 是否退役其子单元 |
| SourceSpan | 若仍被引用则阻断 | 修改引用单元或保留证据 |
| parent 单元 | 子单元变为 `Orphan` | 重新指定 parent 或一起退役 |
| SourceArtifact | 保留 Artifact，标记不可用/归档 | 是否重建来源分支 |
| ConversionRun | 退役运行记录 | 保留其产物 |
| Alignment/语义对象 | 来源对象保留 | 单独处理对齐关系 |

ImpactPlan 必须明确列出“保留”“退役”“孤立”“需要用户决定”的对象，禁止把级联行为隐藏在删除按钮之后。

## 4. 步骤三：编辑 DTO 与后端转换

新增 `mathscape-domain` 中与 UI 无关的编辑 DTO，建议置于 `map_edit.rs`。这些 DTO 描述用户意图，不直接暴露数据库结构。

```rust
struct SourceUnitChanges {
    display_title: Option<String>,
    source_labels: Option<Vec<String>>,
    completeness: Option<UnitCompleteness>,
    parent: Option<Option<StableId>>,
    facts_pending: Option<bool>,
}

struct SourceRelationChanges {
    relation: Option<DiscoveryRelationKind>,
    evidence_spans: Option<Vec<StableId>>,
}

struct CommitReceipt {
    version: String,              // mathscape-map-commit-receipt-1
    request_id: StableId,
    plan_fingerprint: ContentHash,
    commit_id: CommitId,
    base_commit: CommitId,
    committed_at: Timestamp,
    created: Vec<ObjectRevisionReceipt>,
    revised: Vec<ObjectRevisionReceipt>,
    retired: Vec<ObjectRevisionReceipt>,
    preserved: Vec<StableId>,
    projection_invalidated: Vec<ViewLayer>,
}

struct ObjectRevisionReceipt {
    object_id: StableId,
    kind: EntityKind,
    previous_revision: Option<RevisionId>,
    current_revision: RevisionId,
    lifecycle: LifecycleStatus,
}
```

后端转换规则如下：

```text
UserChangeRequest
    → 读取当前对象与 revision
    → 生成完整 replacement payload
    → 生成 MapPatch
    → 重新执行领域和存储校验
    → apply_map_patch
```

编辑操作必须使用当前对象构造完整 replacement，而不是只更新一列。这样可以避免证据、trust、generation_run 等字段在编辑时被静默丢失。

用户确认前的草稿只存在内存中。若未来需要恢复未提交草稿，可以单独保存 DraftChange，不把它写入正式地图对象。

关系上的 `evidence_spans` 允许增删，表示“这条关系由哪些已经存在的 SourceSpan 支撑”。这不允许修改 SourceSpan 自身的 Markdown byte range、block id 或内容 hash；如果原文定位需要改变，必须创建新的 SourceSpan 或重新运行来源发现。

`UserChangeRequest → MapPatch` 的纯转换与影响计算归 `mathscape-orchestrator` 的 `map_edit` 模块负责，不能放在 Desktop 中。规划和提交必须调用同一套转换函数。提交时从最新快照重新构造 patch；SQLite 写事务还要再次检查 `base_commit`、目标 revision 和 plan fingerprint。任何变化都导致冲突，不使用规划阶段缓存的对象覆盖新数据。

## 5. 步骤四：Desktop 交互与 Tauri API

### 5.1 Tauri 命令

新增三类命令：

```text
plan_map_change(request) -> ImpactPlan
commit_map_change(request, plan_fingerprint) -> CommitReceipt
get_map_object_edit_context(object_id) -> EditContext
```

`plan_map_change` 和 `commit_map_change` 都在后端读取权威快照。前端发送的对象内容只作为请求意图，不作为可信快照。

`EditContext` 提供 Inspector 编辑所需的当前 revision、允许字段、当前值、可选 parent 和可用关系类型。

其最小结构为：

```rust
struct EditContext {
    object_id: StableId,
    current_revision: RevisionId,
    lifecycle: LifecycleStatus,
    kind: EntityKind,
    editable_fields: Vec<EditableField>,
    available_parents: Vec<ParentOption>,
    related_relations: Vec<RelationSummary>,
    evidence_spans: Vec<SpanSummary>,
}

struct EditableField {
    name: String,
    value: serde_json::Value,
    required: bool,
    constraints: Vec<String>,
}
```

`EditableField` 是 UI 表单的契约，不等同于数据库列；不可编辑的证据定位、trust 和 generation 元数据不出现在 `editable_fields` 中。

### 5.2 Inspector 交互

用户点击节点或关系后，在 Inspector 中显示：

- 当前对象的可编辑字段；
- 当前 revision 和生命周期；
- 来源证据预览；
- “编辑”“退役”“取消”按钮。

用户点击“编辑”后进入本地草稿状态。草稿修改不会立刻改变图或 SQLite。点击“预览变更”后调用 `plan_map_change`，展示：

- 直接修改对象；
- 将被退役的关系；
- 将被保留的证据；
- 变成孤立状态的子单元；
- 阻断项和需要用户选择的项目。

只有 ImpactPlan 为 `Ready` 且用户确认后才调用 `commit_map_change`。

### 5.3 删除和退役界面

界面中使用“退役”而不是“删除”。按钮旁必须显示影响数量，例如：

```text
退役此内容单元
将退役 3 条关系，保留 2 个原文证据，1 个子单元需要重新指定父节点。
```

如果存在阻断项，按钮不可提交，并提供跳转到相关节点的入口。子单元、证据和共享 Artifact 不自动退役。

### 5.4 提交后的状态

提交过程中 Inspector 进入 `committing` 状态，禁止重复提交。成功后：

1. 显示 CommitReceipt 摘要；
2. 请求新的 LandscapeSnapshot；
3. 刷新当前 GraphProjection；
4. 选中新 revision；
5. 将旧 revision 放入历史详情。

发生 revision 冲突时，保留本地草稿，提示“地图已更新”，提供重新加载当前对象和重新生成 ImpactPlan 的操作。失败不能假装成已提交。

## 6. 开发分解与并行方式

### 6.1 共享契约

首先确定并生成 `UserChangeRequest`、`ImpactPlan`、`EditContext` 和 `CommitReceipt` 的 Rust DTO 与 JSON Schema。前后端都以该 Schema 为边界，避免 UI 和服务端各自发明字段。

### 6.2 后端工作包

后端可以拆为三个工作包：

1. Domain 与 DTO：增加 `Retire*`、编辑 DTO、Schema 和 revision/退役不变量。
2. Orchestrator 与 Store：分别实现 `map_edit` 转换器、ImpactPlan、`apply_map_patch`、revision 更新、退役审计、请求幂等表和事务冲突检查。
3. Desktop/Tauri：暴露 `get_map_object_edit_context`、`plan_map_change`、`commit_map_change`，并在回执后刷新快照。

工作包 1 完成共享契约后，Domain/DTO 与 Store 的底层迁移可以并行；`map_edit` 和 ImpactPlan 需要读取这些 DTO，但可以先用快照 fixture 开发。真实集成必须验证 orchestrator 生成的 patch 能被 Store 完整提交。Tauri 命令依赖 DTO、ImpactPlan 和 Store 的稳定接口。

### 6.3 前端工作包

前端可以与后端并行开发，先使用固定 JSON fixture：

1. Inspector 编辑表单和本地草稿状态；
2. ImpactPlan 影响列表、阻断项和确认按钮；
3. CommitReceipt 成功、冲突、失败状态；
4. Tauri IPC 适配与真实命令接入。

前端在真实后端接入前不应自行推断级联结果。fixture 中的 ImpactPlan 必须由契约示例产生，并覆盖保留、退役、孤立和阻断四种状态。

### 6.4 集成顺序

共享 Schema → 前后端 fixture 并行 → Domain/DTO 与 Store 并行开发 → ImpactPlan 与提交器集成 → Tauri 命令测试 → Desktop 回归。ImpactPlan 和 CommitReceipt 的真实值只能由后端生成。

## 7. 验收顺序

### 第一步：MapPatch 提交器

- 可以创建和修订 SourceSemanticUnit；
- 可以修订 SourceUnitRelation；
- 可以退役实体和关系；
- 旧 revision 可读；
- base commit 冲突会整体拒绝；
- 相同 request_id 重试返回同一个 CommitReceipt。

### 第二步：ImpactPlan

- 删除单元时能列出相关关系、证据和子单元；
- 被引用的 SourceSpan 不会被静默删除；
- 父节点退役会产生 orphan 或阻断项；
- plan fingerprint 能检测预览后数据变化。

### 第三步：编辑 DTO

- DTO 不暴露数据库表结构；
- 编辑不会丢失 evidence、trust 和 generation_run；
- 修改 parent、标题和关系端点都能生成完整 MapPatch；
- 非法字段修改会在后端拒绝。

### 第四步：Desktop 交互

- Inspector 可以编辑节点和关系；
- 用户在提交前能看到 ImpactPlan；
- 退役有明确的影响确认；
- 成功后图和 Inspector 展示新 revision；
- 冲突、阻断和失败都有明确状态。

Markdown patch 和来源分支重建属于这四步完成后的下一阶段。它应创建新的 Markdown Artifact，并通过独立的派生关系连接旧 Artifact；不能把旧来源节点直接迁移到新 Markdown 上。
