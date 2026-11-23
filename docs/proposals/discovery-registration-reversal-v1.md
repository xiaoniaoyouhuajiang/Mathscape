# Discovery Registration Reversal 提案 v1

状态：已实现（2026-10-07）。覆盖确定性规划、追加迁移与原子提交、重新登记绑定、Desktop 命令和审核 UI；字段在 JSON 中使用 snake_case。实现入口见文末，物理删除与通用 undo/redo 仍不在范围内。

## 1. 目标与语义

为“已经接受并登记一次内容发现，之后不再采纳该结果”的场景提供影响预览、原子撤回和重新发现后的登记闭环。复用现有 MapPatch、revision、ImpactPlan 和审核工作区；不增加第二套地图写入引擎，不改变 A/B/P 的数学职责。

三种操作必须分清：

| 操作 | 含义 | 数据变化 |
| --- | --- | --- |
| 放弃未登记结果 | 不再接受这次发现结果 | 更新 disposition，不修改地图 |
| 退役对象 | 对象退出活动地图 | 创建 Deprecated revision，保留定位、payload 和历史 |
| 撤回登记 | 用户不再采纳某一次登记 | 记录撤回决定，并退役其范围内可安全停用的对象 |

退役是清理活动地图的入口，不释放磁盘空间。本提案不删除 Artifact、ConversionRun、旧 revision、原始登记回执或 discovery checkpoint，也不实现日志保留策略、历史归档和垃圾回收。保留历史是本次操作的约束，不是所有运行数据必须永久保存的产品承诺。

撤回不等于回滚整个工作区，也不撤销后续人工修改。共享对象可以继续活动，而该登记仍标记为 `withdrawn`。v1 只撤回一个完整 decision，不提供逐项撤回或 `partially_withdrawn`；原 decision 本身可以只登记原提案的一部分。

未登记但已经保存接受决定的结果也应允许放弃：事务内确认没有登记回执，再标记 run 已放弃并禁止后续登记。不能把 `accepted` 等同于 `registered`；已有任意登记的 run 必须按具体 decision 撤回。已放弃结果必须通过独立“恢复审核”入口恢复，不能借“接受”按钮绕过。

### 恢复审核／撤销放弃（已实现，2026-10-08）

- [x] 为已放弃、尚无任何实际登记的 run 增加独立“恢复审核”入口，复用已有检查点，不重新调用 LLM。
- [x] 追加恢复决定的审计记录；保留原放弃事实，使用请求幂等和当前 disposition 的并发检查，不直接删除放弃记录。
- [x] 恢复仅重新开放审核，不自动接受、不登记、不修改地图或 Artifact；随后仍须完成原文重绑定、结果重验证与正常接受/登记。验证不再兼容或材料不可用时，应明确报告，不能恢复可信权限。
- [x] UI 在已放弃记录上明确提供“恢复审核”，成功后刷新状态与单元可选性；覆盖误操作恢复、重启、重复恢复和并发状态变化的测试。

这是未登记结果的用户状态恢复，不是已撤回登记的反向恢复，也不引入通用 undo/redo。迁移 12 将原放弃记录复制到追加式 `discovery_run_disposition_events`，`discovery_run_dispositions` 仅保存当前决定指针，状态为 discarded/restored。列表、接受和登记均按当前 discarded 状态判断，不能再用“存在历史放弃记录”阻断。

`restore_discovery_review` 与放弃命令共用 `DiscoveryDispositionRequest`：run_id、request_id、expected_result_fingerprint、expected_disposition_request_id、reason。恢复前只读重放原文和结果验证，写事务内再次核对完整 checkpoint、最新决定及未登记条件；恢复事件与当前指针原子提交。成功请求重试返回原回执，即使后来再次放弃也不覆盖新决定。恢复后返回原有 reviewable/accepted 状态；必须由用户另行接受或提交登记。

## 2. 当前工程基础与需要补齐的能力

- `source_registration_commits` 与 `SourceRegistrationReceipt` 已提供 decision、commit 和 created/reused 对象；`map_commit_objects` 可查询该登记实际绑定的 revision。
- 用户地图编辑已具备 preview/commit、乐观并发和请求幂等，但只编辑/退役 SourceSemanticUnit、SourceUnitRelation；级联规则目前写死。
- MapOperation 实际使用 CreateEntity、ReviseEntity、CreateRelation、ReviseRelation。退役可以继续物化为 Revise，不要求为本提案新增 RetireEntity/RetireRelation 变体。
- 来源登记目前只创建或原样复用。相同 stable ID 的当前 revision 不同即报冲突，不能重新激活已退役对象。
- 当前运行摘要、审核状态和地图样式没有完整的撤回状态；退役对象只是弱化显示，不等于已经从活动视图排除。

本提案补齐：两种退役政策、SourceSpan 生命周期修订、按登记的共享/依赖分析、撤回审计、重新登记预览与提交、前端状态和活动视图过滤。本文的新增规则优先于历史 [用户地图更新提案](user-map-update-v1.md) 中未落实或相冲突的设计。

## 3. 对象退役与登记范围内退役

### 3.1 两种政策

```rust
enum RetirementPolicy {
    IncidentRelations,   // 用户指定对象；必要的关联关系列入退役计划
    RegistrationScoped, // 后端从一个实际登记回执确定范围
}
```

政策由用例入口选择，不作为用户可以任意切换的全局设置。普通 `UserChangeRequest` 继续使用 RetireObject；撤回入口提交 decision，不接受前端指定的任意级联对象列表。

两种政策共用引用收集、最终图校验、revision 生成和原子写入设施；用例负责范围、保护条件和审计。不要把登记的保护条件硬编码进普通单节点编辑，也不要让 Desktop 直接拼数据库变更。

普通对象退役规则：

- SourceSemanticUnit、SourceUnitRelation、SourceSpan 都可以退役。
- 活动关系有任意必要端点被退役，关系必须列入影响预览并一同退役；端点包括 source、target 和 evidence，不要求两个单元端点都退役。
- 不自动退役子单元、引用者或使用该证据的单元。仍有活动依赖时，计划阻断；用户通过已有编辑入口改挂或显式退役依赖者，再重新预览。
- 已登记或已共享的对象仍可通过显式普通编辑处理，但 ImpactPlan 必须展示受到影响的登记和关系；不能伪装成仅影响该节点的操作。
- v1 退役统一使用 Deprecated。Superseded 需要真实替代关系，Rejected 表达否定判断，均不作为清理操作的默认状态。

SourceSpan 退役只改 lifecycle，不修改 MarkdownLocator、页码、block ID、文本 hash 或 Artifact。普通来源定位内容仍不可编辑。必须同时扩展 MapEditPayload 的类型分派、planner、store 类型白名单和 EditContext，而不是只开放一个按钮。

### 3.2 来源引用收集

从权威对象的 current revision 收集引用，不能从过滤后的 Cytoscape 图遍历，也不能把历史 revision 的端点当成活动依赖。v1 至少覆盖：

- SourceSemanticUnit.parent_unit、source_spans；
- source_title.source_span、source_labels[*].source_span；
- local_references[*].source_span、candidate_targets；候选目标也保守保护；
- trust.evidence；
- 所有持久化关系的角色端点，包括 evidence。

这里“活动”指 current lifecycle 不属于 Deprecated、Superseded、Rejected；包括当前尚未正式认证的 Candidate。对于其他领域类型的来源引用，须有明确的引用提取规则；无法完整分析时保留目标或阻断，不能假定不存在引用。

parent、位于 Markdown 等合成投影边不是持久化关系对象，不能加入 MapPatch；相关实体变化后重新投影即可。历史查看保留原 revision 与其当时的端点，不使用活动视图的过滤结果冒充历史。

## 4. 登记撤回的确定性规划

### 4.1 范围与共享判定

撤回范围是实际 `decision_id + registration_commit_id` 的对象集合和 revision，不是一个 run 的全部 checkpoint 单元，不是 PDF 的整条派生分支。一个 run 可以有多次不同选区的登记，必须提供登记列表供用户选择。

v1 回执通过 map_commit_objects 取得实际 revision；后续 v2 回执仍必须能提供同样的信息。回执缺失、commit 与 decision 不一致、对象或原 revision 缺失，都禁止猜测并返回明确诊断。

下列对象不默认退役：

- 原登记只是 reused 的对象；重新登记中仅修订了既有活动对象的 revised 对象也保留，不自动恢复其旧内容。
- current revision 已不同于登记绑定 revision 的对象，不论差异来自人工编辑还是其他流程。
- 其他尚未撤回的登记仍采纳该对象的情况，即使图上没有额外连线。
- 范围外活动对象依赖的对象，以及为保持这些对象有效而必须保留的依赖。

“其他登记采纳”从 source_registration_commits、map_commit_objects 和撤回决定计算，检查所有创建、复用、重新激活和修订的成员；已放弃但未登记的提案不构成地图采纳。generation_run 只能用于溯源，不能代替成员判断。

保护当前未撤回登记时，不要求其成员 revision 恰好等于当前 revision；后续修订不能让共享保护消失。已撤回登记不再产生新的保护，但当前活动对象的实际引用仍需检查。

### 4.2 批次最终状态与固定点

令 G 为当前权威快照，S 为目标登记的实际对象集合，R 为计划退役集合：

1. R 初始只包含目标登记 created（或 v2 reactivated）的活动对象，且 current revision 仍等于登记 revision，并未被其他有效登记采纳。
2. 已退役对象列为 `already_inactive`；复用、后续修订、共享采纳对象列为 preserve。
3. 对 G 中每个仍将活动的对象 A（A 不在 R），若它引用 R 中对象 B，则从 R 移除 B 并保留依赖原因。
4. 重复步骤 3，直到 R 不再变化。被保留的单元可能要求继续保留 parent、SourceSpan 和引用目标，不能只过滤一次。
5. 校验执行后的整体状态。只有最终 R 中的对象写入退役 revision；所有 R 外对象不因本次撤回被修改。

同一批内一起退役的对象互相引用不会阻断。范围外关系若仍引用候选单元，不退役这条范围外关系，而保留候选单元；范围内关系只要在最终 R 内就可退役，即使它的一部分端点被保留。

一般的共享/依赖保留不是提交错误。只有身份不一致、领域引用或定位元数据不合法、无法形成合法最终图等情况才令计划 Blocked。不能读取原文或打开 PDF 不等于领域引用非法：生命周期操作不改原文，可以保留预览不可用提示并继续规划。当前图已经存在的不一致应报告，不通过撤回顺便修复。

即使 R 为空，也可以撤回登记：写撤回审计，地图不变。不能为了满足 MapPatch 的非空要求制造假 revision，也不能把“无对象可退役”错误地当成“不能撤回”。

### 4.3 例子

登记 A 创建 U1、U2、S 和关系 E；U1/U2 使用 S。没有其他采纳或引用时，四者一起退役。

若另一有效登记 B 复用 U1，则 U1 保留；其依赖 S 也保留。U2 与 E 仍可退役，A 标记已撤回，B 不变。

若范围外单元 V 是 U1 的子单元或引用者，同样保留 U1 及其必要依赖。撤回不能把 V 变成孤儿，也不能自动退役 V。

## 5. 预览、请求与回执契约

### 5.1 登记查询

```rust
enum RegistrationState { Registered, Withdrawn }

struct RegistrationSummary {
    decision_id: StableId,
    registration_commit_id: CommitId,
    run_id: StableId,
    state: RegistrationState,
    registered_at: Timestamp,
    selected_unit_refs: Vec<String>,
    object_count: u32,
    reversal_receipt: Option<DiscoveryReversalReceipt>,
}
```

提供 `list_discovery_registrations(run_id)`，不要让前端从 run 状态或 graph 猜测 decision。撤回规划只依赖登记回执和当前领域快照，不要求重新调用 LLM 或重放 A/B；旧 checkpoint 与最新验证器不兼容，不能因此禁止已有登记的安全撤回。

### 5.2 规划与提交

```rust
struct ReverseDiscoveryRequest {
    request_id: StableId, // 前端生成，非模型生成
    decision_id: StableId,
    registration_commit_id: CommitId, // 固定的原始登记身份
    base_commit: Option<CommitId>,    // 当前地图 head，不是原始登记 commit
    reason: String,
}

struct DiscoveryReversalPlan {
    version: String, // mathscape-discovery-reversal-plan-1
    request: ReverseDiscoveryRequest,
    impact: ImpactPlan,
    summary: ReversalCounts,
}

struct ReversalCounts {
    total: u32,
    retire: u32,
    preserve: u32,
    already_inactive: u32,
}

struct CommitDiscoveryReversalRequest {
    request: ReverseDiscoveryRequest,
    expected_plan_fingerprint: ContentHash,
}
```

Ready 计划的计数按 stable ID 去重，三个处置计数之和等于 total；每项可以有多个保护原因，不按原因重复计数。Blocked/Conflicted 的计数只是已解析部分，不能据此提交。可增加按单元/关系/证据分组的显示计数，但不能混用对象数和引用次数。

复用 ImpactPlan 的 Ready/Blocked/Conflicted。退役项 action=retire；保留项 action=preserve；无效项 action=block。already_inactive 项使用 preserve 并提供相应 reason code；它不表示共享。

ImpactItem 在现有字段上补充以下可选展示信息，旧 UI/旧记录缺失时可默认空值：

```rust
display_title: Option<String>,
reason_codes: Vec<String>,
related_objects: Vec<ImpactObjectRef>,
protected_by_decisions: Vec<StableId>,

struct ImpactObjectRef {
    object_id: StableId,
    revision_id: RevisionId,
    kind: EntityKind,
    display_title: Option<String>,
    reference_kind: String, // parent、source_span、reference_target、endpoint 等
}
```

固定 reason code 至少包括 created_by_registration、reactivated_by_registration、reused_object、preexisting_revised_object、subsequently_revised、adopted_by_other_registration、referenced_by_live_object、required_by_preserved_object、already_inactive。现有 reason 字段用于可读说明，不能让前端解析说明文本来决定权限。

plan_fingerprint 绑定请求、可信 actor、原始登记及成员 revision、当前 head、受影响对象/依赖、有效登记采纳集合和最终处置。集合确定性排序，角色 ordinal 和有意义的原文顺序保留；时间戳和 UI 布局不参与。

### 5.3 回执

```rust
struct DiscoveryReversalReceipt {
    version: String, // mathscape-discovery-reversal-receipt-1
    request_id: StableId,
    decision_id: StableId,
    registration_commit_id: CommitId,
    plan_fingerprint: ContentHash,
    state: RegistrationState, // Withdrawn
    decided_at: Timestamp,
    actor: ActorRef,
    reason: String,
    map_commit: Option<CommitReceipt>, // 无地图变化时为 None
    retired_objects: Vec<StableId>,
    preserved_objects: Vec<StableId>,
    already_inactive_objects: Vec<StableId>,
}
```

回执记录提交时的事实，之后重新激活某个对象也不重写旧回执。对象的当前状态由独立查询提供。撤回请求不接受 arbitrary envelope、前端计算的 revision 或 SQL 字段。

## 6. 原子持久化、幂等与并发

新增正常业务审计表 `discovery_registration_reversals`，不是历史修复表。至少保存 request_id（唯一）、decision_id（唯一）、registration_commit_id、base_commit、plan_fingerprint、request_json（含规范化请求与可信 actor）、map_commit_id（可空）、receipt_json、decided_at。外键绑定实际登记/commit；本操作不修改原接受决定和登记回执。

来源对象新 revision、当前指针、关系端点/变更审计、撤回行和成功回执在同一写事务提交。不要先调用现有 commit_map_change 成功后再用第二个事务记录撤回。

提交顺序：

1. `BEGIN IMMEDIATE`；优先检查 request_id 的成功回执。完全相同的请求和指纹返回原回执；同键不同内容返回 idempotency_conflict。
2. 验证 decision 与 registration_commit_id，确认尚未撤回；另一 request 已撤回同 decision 时返回 already_withdrawn 和原回执供刷新，不重复提交。
3. 在事务内重新读取领域状态、成员和有效采纳集合，检查 base_commit 并运行与 preview 相同的纯 planner。
4. 比较最终 plan_fingerprint。即使 head 没变，其他仅更新审计状态的操作改变了采纳集合，也必须使旧计划失效。
5. 有地图变化时写入完整 patch；无地图变化时跳过 MapPatch。最后写撤回行、回执并提交。

所有修订检查 expected_revision；退役复制原 payload 和端点，保留生成来源，在本次 revision/commit 中记录操作 actor、原因和时间。不凭是否 active 覆盖为其他生命周期，不提升数学可信状态。来源登记提交器也必须在写事务内检查 run 的 disposition；只有在保存接受决定时检查放弃状态不足以防止并发登记。

失败整体回滚。plan 不写权威对象；提交重试仍使用原 request_id，重新预览产生新的 request_id 和指纹。数据迁移追加新版本，覆盖旧库升级；不改写历史迁移、不引入查询时的 schema 修复。

撤回规模使用具名的后端上限，不能直接套用普通 UserChangeRequest 的 1000 项用户操作限制。超限返回明确诊断，不静默截断，不通过多个独立提交拆开原子撤回。

## 7. 重新发现与重新登记闭环

### 7.1 当前冲突的根因

同一 Markdown artifact 和原文范围会生成相同 stable ID。退役把 current revision 从 r1 改为 r2，而当前登记器要求已有 ID 的 current revision 与新提案完全一致。因此新一轮 LLM 可以成功，登记却会冲突；重复接受旧 decision 只返回旧回执，也不会恢复对象。

不能通过物理删除对象、换随机 stable ID 或复制相同 Markdown 来绕过。旧请求的幂等语义保持不变；v1 的重新采纳来自新的 discovery run 和新的 acceptance decision，不提供对旧撤回请求的反向 undo。

### 7.2 登记动作

来源登记在正式写入前增加基于当前快照的确定性绑定计划：

| 当前对象 | 新结果 | 动作 |
| --- | --- | --- |
| 不存在 | 合法对象 | Create |
| 活动，payload 和端点一致 | 相同对象 | Reuse 当前 revision，即使其 revision ID 与初始 materialization 不同 |
| 已退役，payload 和端点一致 | 相同对象 | Reactivate：确认后创建新的活动 revision |
| 活动或已退役，payload/端点不同 | 同 stable ID 的新提案 | 显示差异；经明确批准后 Revise，或 Reactivate 并替换内容 |
| 原文身份、EntityKind 或定位冲突 | 同 stable ID | Block，要求修订发现边界或单独处理 |

“一致”逐字段比较 payload 与角色端点，包含 trust，不凭标题或 embedding 近似匹配；不能忽略信任差异来悄悄升级对象。SourceSpan 要求 artifact、locator、block ID、文本 hash 等不可变定位完全一致；不一致时禁止覆盖。

不同内容的替换必须携带逐对象 `object_id + expected_revision` 的明确批准，展示原值与提案值。v1 不自动替换被其他有效登记采纳的不同内容对象；这类对象阻断并指向已有人工编辑流程。取消保留原地图与新发现结果，不能声称接受的新内容已经落图。

复用活动对象不需要重复写 revision；重新激活必须生成新的 revision，而不是把 current_revision 指回旧 r1。revision fingerprint 包含 payload、角色端点、lifecycle 和本次请求身份，防止活动—退役—再活动落到同一个历史主键。只有批准的来源事实进入替换；不得继承与新内容不一致的旧证明/对齐认证。

同一批次的 parent、证据、引用和关系全部按最终状态校验。只恢复单元却留下退役 SourceSpan，或只恢复关系却留下退役端点，都禁止提交。所有动作、登记成员、登记回执在同一事务完成。

### 7.3 入口与兼容

保留 accept_discovery_result 的重验证与接受计划构造。普通 Create/Reuse 路径继续使用当前一次人工接受；需要重新激活或替换时返回 `registration_review_required`，不能返回假 `registered`。

```text
accept_discovery_result
  → Registered(SourceRegistrationReceipt)
  或 RegistrationReviewRequired(SourceRegistrationCommitPlan)

plan_source_registration_commit(request)
  → SourceRegistrationCommitPlan

commit_source_registration_commit(request, expected_plan_fingerprint)
  → SourceRegistrationReceipt
```

SourceRegistrationCommitPlan 的目标形状如下：

```rust
struct SourceRegistrationCommitRequest {
    request_id: StableId,
    decision_id: StableId,
    acceptance_plan_fingerprint: ContentHash,
    base_commit: Option<CommitId>,
    replacement_approvals: Vec<RegistrationReplacementApproval>,
}

struct RegistrationReplacementApproval {
    object_id: StableId,
    expected_revision: RevisionId,
    // 仅批准此 decision 已接受的候选内容，不接收 arbitrary replacement。
}

enum RegistrationObjectAction { Create, Reuse, Reactivate, Revise, Block }

struct SourceRegistrationCommitPlan {
    version: String, // mathscape-source-registration-commit-plan-1
    request: SourceRegistrationCommitRequest,
    impact: ImpactPlan,
    requires_confirmation: bool,
    objects: Vec<RegistrationObjectChange>,
}

struct RegistrationObjectChange {
    object_id: StableId,
    action: RegistrationObjectAction,
    display_title: Option<String>,
    current_revision: Option<RevisionId>,
    proposed_revision: Option<RevisionId>,
    changed_fields: Vec<String>,
    before: Option<serde_json::Value>, // 只读 DTO 中的已知领域 payload/端点
    proposed: serde_json::Value,      // 用于展示；提交请求不携带这些值
}
```

commit 请求携带完整 SourceRegistrationCommitRequest 和 expected_plan_fingerprint。重复/未知 approval、revision 不匹配均拒绝；没有批准的不同内容替换保持 Block。相同内容的 Reactivate 可以是 Ready，但 requires_confirmation=true。任何 Reactivate/Revise 都需要最终预览确认；普通 Create/Reuse 才允许沿用现有一次接受路径。

原 acceptance_plan_fingerprint 绑定模型结果和接受范围，impact.plan_fingerprint 绑定当前地图中的执行计划，二者不能混用。approval 变化后重新预览。plan 和 commit 使用同一绑定算法，写事务内重算，覆盖有效采纳集合和引用闭合；提交请求的成功回执也必须幂等保存。全批仅 Reuse 时记录新登记及其实际成员，不制造 revision，也不强行构造空 MapPatch。

SourceRegistrationReceipt 升级为 v2，保留 created_objects/reused_objects，并增加 reactivated_objects/revised_objects；成员表保存最终实际 revision 和对应 outcome。非活动对象恢复后记入 reactivated，即使经批准同时替换了内容；仅活动对象的内容更新记入 revised。旧 v1 记录继续可读，新增集合默认空；迁移需要兼容现有 outcome CHECK 约束及登记提交幂等字段。

只有 created 或从非活动恢复的 reactivated 对象进入未来撤回的默认候选；对既有活动对象做的 revised 不在撤回时自动回滚内容。Withdrawn decision 的旧接受请求继续返回历史登记回执，但查询同时给出 withdrawn，UI 不把幂等回执当成当前对象活动或重新采纳的证明。

## 8. Desktop 与图投影交互

### 8.1 入口和状态

- Inspector 的“退役”使用普通对象政策；显式证据模式和单元证据详情增加 SourceSpan 退役入口，不把证据节点强制放入默认地图。
- 已登记 run 的记录和登记回执页提供“撤回本次登记”；多 decision 时展示登记时间、选区和数量，选择具体登记。
- 未登记结果使用“放弃结果”，不要共用撤回按钮。
- 已撤回记录提供“查看撤回详情”“查看原始结果”“重新发现”。v1 不提供“一键恢复全部”，不重新使用旧接受请求恢复对象。

WorkflowStatus 继续描述 LLM 完成度；RegistrationState 描述每个 decision 的采纳状态；对象 lifecycle 描述当前地图。三者不能互相替代。run 有多个登记时显示“2 次登记，1 次已撤回”之类聚合摘要，不强行用一个 registered 布尔值覆盖所有登记。

登记查询、运行摘要和审核 envelope 提供 registrations/reversal 信息；原审核状态保留向后兼容字段，但前端不能只用 `accepted=true` 锁住新登记冲突审核，也不能撤回后仍把所有单元显示为 registered。

### 8.2 复用审核工作区的撤回模式

入口：登记选择 → 请求只读 ImpactPlan → 展示预览 → 用户确认 → 原子提交 → 刷新。第一版不逐项勾选撤回对象，提交后端计算的安全集合；共享对象需要进一步处理时进入独立编辑流程，再返回重算计划。

左侧分组展示：将退役、共享/依赖而保留、后续已修改而保留、已经退役，以及真正阻止提交的错误。多原因项只显示一次，可展开全部原因。复用 ImpactPlan 的 reason/依赖展示，不以 hash 作为主要名称。

右侧复用 Markdown/KaTeX 渲染、精确高亮和 PDF 回链；悬停预览、点击锁定保持一致。依赖者可打开对象 Inspector 或所属登记。旧 checkpoint 无法重放时，从登记实体和 SourceSpan 构造只读预览，不要求恢复 A/B 投影才能撤回；缺少材料或回链不阻止可证明安全的生命周期操作，须显示预览不可用原因。

预览通过 `get_source_registration_context(decision_id)` 只读取得已接受计划的 Markdown artifact 与 conversion 身份，不重放 A/B。只有 artifact 匹配时才能沿用该登记的 conversion；Markdown 阅读与 PDF 回链必须传递同一上下文。其他来源的依赖或未绑定转换的旧记录可使用唯一匹配的转换；多个来源必须明确选择，已绑定但失效的上下文不能静默替换。直接导入且无转换的 Markdown 仍使用空上下文。

审核小图的节点状态区分“将退役”“将保留”“已修改”“已退役”，兼顾颜色、描边和图例；大结果默认分组列表，沿用已有阈值。图只呈现计划，不改变领域状态。

页底展示：“退役 26 个内容单元、18 条关系、32 个原文证据；保留 4 个共享对象。材料文件与历史记录保留。”R 为空时改为：“撤回此登记，不改变活动地图；相关对象已退役或因共享、后续修改而保留。”按钮分别为“确认撤回”“取消”。原因必填，可提供“重新整理这次发现结果”的默认值供修改。

### 8.3 成功、冲突与重新登记

- 成功后一起刷新地图、Inspector、发现记录、登记回执和审核状态；若选中对象已退役，显示状态并提供历史入口，不保留可编辑表单。
- 活动地图默认排除退役对象及无可见端点的关系；通过“显示退役对象”查看其当前退役 revision。该选项不是历史快照，登记当时的对象要从具体回执绑定 revision 查看。
- 保留的共享对象继续可见；不得简单按 withdrawn run 隐藏整条分支。
- 提交期间禁用重复提交；失败保留原因和当前计划。返回 Conflicted 时提示“地图已变化，重新计算影响”，重新取得计划后再次确认，不沿用旧指纹静默提交。
- 请求成功但响应丢失时以同 request_id 重试，并从 SQLite 恢复回执；重启后重新读取登记状态，不信任页面本地状态。
- “重新发现”定位原 Markdown 选区并启动新 run，用户明确触发才调用 LLM。新结果涉及 Reactivate/Revise 时显示登记变更预览：新建、复用、重新激活、需替换/阻断；差异中的数学文本正常渲染，取消不丢失发现结果。

## 9. 模块职责、API 与开发顺序

API 最小集合：list_discovery_registrations、plan_discovery_registration_reversal、commit_discovery_registration_reversal、get_discovery_registration_reversal；普通退役沿用 plan_map_change/commit_map_change。重新登记使用第 7 节入口，不能复用旧登记重试假装重新激活。

| 模块 | 补充职责 |
| --- | --- |
| Domain | 请求、计划/回执、登记状态及 outcomes；SourceSpan 编辑 payload 的正确类型分派 |
| Orchestrator | 统一引用收集、两种退役政策、固定点保护、最终校验、重新登记绑定；不访问 SQLite |
| Store | 一致快照读取、登记成员/有效采纳查询、原子 revision 与撤回审计、幂等、追加迁移 |
| Query/Graph | 当前 lifecycle 的活动视图与显示退役视图；按回执 revision 读取历史对象与端点 |
| Desktop | 薄命令、可信 actor、用例编排；不在 app 中另写图级联算法 |
| Frontend | 登记入口、工作区计划预览、渲染回链、冲突/状态刷新；不自行判断可退役集合 |

开发分四步，每步保持可独立验证：

1. **契约与共享规划设施**：确定 DTO，扩展 SourceSpan 类型，提取 current 引用与登记采纳快照，实现两种政策和固定点算法；前端可依照契约并行构造 mock 预览。
2. **撤回事务与查询**：追加迁移、同事务提交、R 为空的审计提交、幂等、登记列表/回执读取；修订普通退役相关关系的规则并提供 SourceSpan edit context。
3. **重新登记闭环**：Create/Reuse/Reactivate/Revise 绑定、差异批准、receipt v2、原子提交和旧回执兼容。不能在完成此步之前宣称撤回后同一材料可重新入图。
4. **Desktop/UI 与验收**：登记选择、工作区撤回/重新登记模式、活动过滤、历史查看、事件刷新和端到端验证。

不为了此提案先做通用 undo/redo、物理删除、跨多个登记合并清理或新的 Agent 工作流。现有内容发现结果、Markdown patch 和 PDF 回链能力继续复用。

## 10. 必须覆盖的验收场景

| 场景 | 预期 |
| --- | --- |
| 未接受或 accepted 但尚未登记的结果放弃 | 不修改地图；禁止之后悄悄完成登记 |
| 登记内部 parent、证据和引用闭合 | 单元、关系、SourceSpan 可一起退役，内部引用不自我阻塞 |
| 其他有效登记仅复用对象，没有额外连线 | 对象及其必要依赖保留 |
| 范围外活动引用或子单元 | 不退役范围外对象；保护目标并传播到其依赖 |
| 范围内关系只有一个端点退役 | 关系一起退役，不要求两端都退役 |
| 后续人工修改 | 不覆盖/回滚其内容；登记仍可撤回 |
| title/label/local reference/trust 使用 SourceSpan | 引用全部参与检查；活动引用不允许指向新退役证据 |
| 全部共享、复用或已退役 | 只写撤回记录，map_commit=None，默认地图不变 |
| 同一 run 多次或部分范围登记 | 只撤回所选 decision 的实际成员，不按整个 run 猜范围 |
| 预览后新增外部引用或仅改变有效采纳集合 | 提交冲突，无半次变更，原因输入保留 |
| 重复提交/网络响应丢失/同键不同请求 | 返回原回执或明确幂等冲突，无重复 revision |
| 新 run 同范围、相同内容 | 新 decision 审核后重新激活，stable ID 保持，产生新 revision |
| 新 run 不同内容/后续人工修改/共享活动对象 | 展示差异，未批准不覆盖；共享替换阻断 |
| 重新登记关系、单元和证据 | 最终状态闭合，不能只恢复其中一层 |
| 老 v1 回执和已撤回 decision 的重试 | 历史仍可读，旧回执不充当当前活动状态 |
| 旧 checkpoint 无法重放或材料预览不可用 | 正式登记可按 receipt/实体规划；缺 receipt 时拒绝猜测 |
| 重启与视图切换 | 撤回状态可恢复，共享节点不消失，历史 revision/回链保留 |

单测优先覆盖纯规划与固定点；store 测试覆盖迁移、并发、回滚、幂等和成员记录；Desktop/浏览器测试覆盖入口、预览/确认、重新登记、原文高亮、冲突保留草稿和成功后的多处状态同步。

## 实现与回归入口

- 契约与共享引用检查：[Domain](../../crates/mathscape-domain/src/registration.rs)、[引用完整性](../../crates/mathscape-orchestrator/src/source_integrity.rs)。
- 规划与原子写入：[纯规划](../../crates/mathscape-orchestrator/src/registration.rs)、[Store](../../crates/mathscape-store-sqlite/src/registration.rs)、[迁移 11](../../migrations/0011_registration_reversal.sql)。
- 桌面与审核：[命令](../../apps/mathscape-desktop/src/registration.rs)、[登记审核工作区](../../frontend/src/discovery/registration-workspace.ts)。
- 回归：[固定点与引用](../../crates/mathscape-orchestrator/tests/registration_reversal.rs)、[真实事务与旧库升级](../../crates/mathscape-orchestrator/tests/registration_transactions.rs)、[Desktop 流程](../../apps/mathscape-desktop/tests/service.rs)、[浏览器交互](../../frontend/tests/registration.spec.ts)。

发现记录按实际登记显示撤回入口；重新发现按钮打开原 Markdown 选区，用户仍需明确启动新 run。活动地图默认隐藏退役对象，“显示退役对象”仅显示当前 tombstone；原始 revision 从登记快照查看。预览或提交失败不会删除发现检查点或材料文件。
