# 内容发现 Markdown Patch 提案 v1

本文规划把 Stage A 发现的 `suspected_conversion_error` 从“只记录 issue”扩展为独立 Patch 阶段可消费的修订任务，并让 Stage B 在修订后的 Markdown 快照上生成来源节点和关系。

## 结论与目标

这项改造在当前工程基础上可行，预计属于中等规模的工作流扩展，不需要重写 A/B 内容发现架构。需要先明确职责边界：A 只负责发现和冻结数学言语单元，P 只负责文本修订，B 只负责冻结单元内的来源事实。现有能力可以直接复用：

- `DiscoveryInput` 保留原始 UTF-8 字节、物理 block 和授权范围；
- `SelectorResolver` 可以把模型给出的文本锚点解析成确定的原文范围；
- `ValidatedUnitSet` 已经把 Stage A 的单元边界冻结；
- `ArtifactPool` 保存内容寻址的不可变字节；
- `ConversionRun` 能表达“一个 Markdown 派生出另一个 Markdown”；
- `SourceRegistrationPlan` 已经从 `MarkdownSnapshot` 读取最终的 Markdown artifact 和 conversion。

目标链路如下：

```text
原始 Markdown
    ↓
Stage A：单元发现 + 转换异常 issue
    ↓ 确定性校验并冻结 UnitSet
Stage P：根据 issue 生成、校验和审核 Markdown patch
    ↓ 原子应用
新的 Markdown artifact + Markdown patch ConversionRun
    ↓ 安全门检查或重建 A
Stage B：在有效快照上提取标签、引用、关系
    ↓
来源审核 → SourceSpan / SourceSemanticUnit / SourceUnitRelation
```

Stage A 的 issue 只描述观察、证据和上下文请求，不直接携带文本替换。Stage P 读取已验证的 issue 和原始 `DiscoveryInput`，再产生 patch candidate。这样 A 的单元边界不会因为模型同时设计了替换内容而被悄悄改变，P 也不能反向修改 UnitSet。

## 工作流状态与暂停点

v1 默认采用人工审核 patch。`DiscoveryAdmission` 只决定来源结果进入 `Active` 还是 `Candidate`，不决定是否自动修改 Markdown；patch 另有独立策略 `ReviewBeforeApply`，后续才考虑显式开启 `AutomaticApply`。

状态转移如下：

```text
A validated
    ├─ 没有 suspected_conversion_error → B
    ├─ P 没有产生可验证 patch → B（保留 issue）
    └─ 有可验证 patch → AwaitingPatchReview
                                  ↓ 用户提交决定
                             ApplyingPatch
                                  ↓
                         PatchSafetyReport
                         ├─ SafeForB → B
                         └─ RequiresARebuild → A(rebuild-once)
```

`AwaitingPatchReview` 是可持久化的暂停状态。进入该状态时保存 checkpoint、释放活动任务锁，不占用 LLM 请求的 deadline；用户审核所用时间不计入 `discovery_timeout_seconds`。重新继续 A/P/B 时，为新的活动计算段建立新的请求 deadline，但保留已有的调用次数、token 预算和 patch round 计数。

一次 discovery run 最多允许一个 P round。`RequiresARebuild` 后自动重跑一次 A；重跑产生的新 conversion issue 不再启动第二个 P。只要新的 A UnitSet 有效，B 可以带着未修订 issue 继续，界面标记 `patch_round_exhausted`；如果新的 UnitSet 无法验证或不能覆盖修订范围，则停在 `NeedsAttention`，不调用 B。

## 不变的原则

### 原 Markdown 不被覆盖

Markdown artifact 是来源证据的一部分，内容哈希和字节不能被静默改变。修订结果必须是新的、内容寻址的 `SourceArtifact`。原 Markdown、修订 Markdown 以及 patch 的审核决定都保留在工作区中。

不采用 Git commit 作为主版本机制，原因是：

1. Artifact Pool 管理的是任意 PDF 派生物和用户导入的 Markdown，不要求材料位于 Git 仓库；
2. 来源节点需要通过 artifact ID 和内容哈希稳定回溯，不能依赖某个工作树、分支或 Git 对象库；
3. 同一份 Markdown 可能产生多条经过不同 patch 选择的派生分支，Artifact 的内容寻址模型可以同时保留它们；
4. 工作区搬迁、离线复制和数据库恢复时，受管字节与目录元数据可以一起自包含；
5. Git 可以作为导出或人工审阅工具，但不应成为数学来源图的权威身份系统。

因此，patch 不是普通地图 `MapPatch`。`MapPatch` 修改的是已经入图的领域对象；Markdown patch 修改的是输入 artifact 的派生字节，两者必须有不同的领域契约。

## Patch DTO 与确定性协议

### Stage A 输出

Stage A schema 升级到新版本（建议 `mathscape-unit-discovery-2`），允许模型携带可忽略的临时 issue ref，但不把它当作身份。A 不输出 replacement、不生成 PatchSet，也不接受任何会改变单元目录的 patch 动作：

```rust
struct DiscoveryIssueProposal {
    model_ref: Option<String>,         // 可选，仅用于日志，不能作为身份
    kind: DiscoveryIssueKind,
    unit: Option<String>,
    evidence: Vec<DiscoverySelector>,
    observation: String,
    context_request: Option<DiscoveryContextRequest>,
}
```

校验通过后，由编排器依据已解析的 evidence 起点、unit 归属和 issue kind，按稳定原文顺序分配 canonical `issue_ref`（例如 `a-i1`、`a-i2`）。模型提供的 `model_ref` 永远不能覆盖它。canonical refs 写入 UnitSet checkpoint 和 P 输入；A repair 只引用这些 refs，完整提案替换时也沿用已持久化的 issue registry，已解决的 issue 标记为 resolved，新 issue 获得新的序号。

事实阶段的 `FactIssue` 保持现有结构，不增加 `issue_ref`。P 只消费 A 的 issue，不消费 B 的 issue；事实引用和关系事实的临时 ref 仍由编排器在验证后分配。

旧的 `mathscape-unit-discovery-1` checkpoint 和 `mathscape-discovery-repair-1` response 必须继续可读。读取旧 checkpoint 时，系统先用兼容 DTO 解码，再生成 canonical issue registry；新 A/P 协议不回写旧 JSON。只有当 repair wire shape 发生变化时才新增 repair v2，旧 v1 继续走适配器。

### Stage P 输出

P 使用单独的 prompt、manifest 和 schema（建议 `mathscape-markdown-patch-1`）。它接收原始 `DiscoveryInput` 的可见 block 文本、已冻结的 UnitSet 摘要和 issue 证据，以及本次 patch 任务的范围和限制。P 输出候选 patch：

```rust
struct MarkdownPatchProposal {
    patch_ref: String,                 // 例如 p1，提案内唯一
    issue_ref: String,                 // 必须指向已冻结的 A issue
    target: TextOccurrence,            // v1 限定为同一 block 内的 text_occurrence
    replacement: String,
    rationale: String,
}
```

`TextOccurrence.text` 是待替换的逐字原文；`occurrence` 仍使用一基计数。`replacement` 是 Markdown 文本，不是 HTML、LaTeX AST 或脚本。模型不输出字节偏移、artifact ID、哈希或 StableId，这些由系统解析和生成。

P 不能创建、删除或重命名 unit，不能修改 parent、unit_kind、completeness，也不能把一个 issue 的 patch 用来修复另一个 issue。没有安全替换时，P 返回空 patch 并保留 issue。

### v1 约束

为控制风险，第一版只接受以下 patch：

- target 完全位于本次 DiscoveryInput 的授权选区内；
- target 能在一个物理 block 内唯一解析；重复文本必须显式提供 occurrence；
- target 必须完全落在该 canonical `issue_ref` 的已解析 evidence 范围内；issue evidence 跨 block、无法唯一解析或只依赖规范化匹配时，该 issue 不可修补；
- 目标原文与解析到的 UTF-8 字节逐字相等；P 只能使用 exact matching，不能使用 SelectorResolver 的安全规范化档、编辑距离或“相似文本”猜测；
- patch 之间不能重叠，不能对同一原文范围给出冲突替换；
- replacement 不得包含 NUL；可以修改 block 内的公式定界符、公式内部字符、普通文本和不改变分块结果的内部换行；但不得造成 Markdown heading、代码围栏、列表项或 block 分隔结构变化，最终以修订前后的分块结果为准；
- 单个 patch、patch 集合和总替换字节数都有上限；
- 只有 `suspected_conversion_error` 可以附带 patch，截断、缺失上下文和数学不确定性仍使用 issue；
- 无法确认原文时输出 issue，不制造修订文本。

禁止把 patch 用来改写标题边界、parent、unit_kind、证明关系或数学结论。那些属于 Stage 1/Stage 2 的结构或语义判断。

确定性验证结果应包含：解析后的原始范围、旧文本哈希、替换后文本哈希、受影响的 issue/unit、警告和拒绝原因。验证通过只表示“这个文本变更可以安全应用”，不表示替换内容在数学上正确。

P1 必须定义具名上限，即使具体数值以后再调：`MAX_PATCHES_PER_RESPONSE`、`MAX_REPLACEMENT_BYTES`、`MAX_PATCH_CONTEXT_BLOCKS_BEFORE`、`MAX_PATCH_CONTEXT_BLOCKS_AFTER`、`MAX_PATCH_OUTPUT_BYTES` 和 `MAX_PATCH_ROUNDS_PER_RUN`。超限结果拒绝或返回空 patch，不静默截断。

## Patch 集合、审核和应用

系统把通过验证的候选组成一个 `MarkdownPatchSet`：

```rust
struct MarkdownPatchSet {
    version: String,                   // mathscape-markdown-patch-set-1
    run_id: StableId,
    base_snapshot: MarkdownSnapshot,
    base_input_fingerprint: ContentHash,
    patches: Vec<ResolvedMarkdownPatch>,
    fingerprint: ContentHash,
}

struct ResolvedMarkdownPatch {
    patch_ref: String,
    issue_ref: String,
    range: TextByteRange,
    old_text_hash: ContentHash,
    replacement: String,
    rationale: String,
}
```

用户可以逐项接受、拒绝或编辑 replacement。编辑后的 patch 必须重新做确定性校验，并重新计算 patch fingerprint；旧审核决定不能继续复用。建议状态为 `proposed`、`accepted`、`rejected`、`stale`、`applied`、`failed`。

部分选择的规则固定如下：

- 只接受子集时，只把该子集应用为一个新的 Markdown artifact 和一条 patch `ConversionRun`；被拒绝的 issue 保留，并通过 offset map 重新绑定到新快照，继续让 B 看见；
- 用户修改 replacement 时，系统先保存 draft，再重新验证并计算新 fingerprint；旧接受决定立即失效；
- 全部拒绝或全部 patch 校验失败时，不写新 artifact，直接用原 snapshot 进入 B，并保留转换 issue；
- base artifact、base input fingerprint 或旧文本哈希发生漂移时，停止，不进入 B，也不做来源登记；
- 同一 patch set 的重复提交返回原回执；不同 patch set 只要产生相同字节，可以复用内容寻址 artifact。被拒绝的项不能在同一 run 中回头应用，必须建立新的 patch 分支。

draft 不是只存在于页面内存。v1 直接在 `markdown_patch_operations` 中保存当前 replacement、draft revision、draft actor 和更新时间；最终决定引用 draft fingerprint，不再另增 draft 表。预览不写 artifact、不写来源对象，但应保存 draft 以便关闭页面、恢复 workspace 后继续。

审核界面至少提供：

- 原 Markdown 与应用后 Markdown 的并排 diff；
- 数学公式的渲染预览，同时保留原始 Markdown 文本；
- patch、issue、unit 和 block 的相互定位；
- 单项选择、全选、修改 replacement、恢复模型建议；
- 应用前显示受影响的单元数、是否需要重新执行 A 的提示和新的 artifact 预览。

应用过程由后端完成：重新读取并核验 base artifact，按原始范围从后向前或使用统一 offset map 应用非重叠 patch，重新计算完整文件哈希，然后通过 Artifact Pool 注册新 Markdown。任意校验失败都不写入新 artifact。

## Stage A、P、B 的衔接

### Patch Safety Gate

P 应用 patch 后，系统先执行确定性的 `PatchSafetyReport`，而不是直接把 diff 拼到 B 的 prompt 中。报告至少检查：

- 修订前后的 Markdown block 数量、顺序和 `structural_kind` 一致；
- heading level、代码围栏、列表边界和 block 换行结构没有改变；公式 `$` / `$$` 或其内部字符可以在 block 内修改，只要重新分块结果仍然一致；
- 每个 patch 完全位于一个已冻结 unit 的内部，不触碰 unit selector 的起止边界；
- patch 不跨越 unit、coverage、parent 或 task 的边界；
- 以“第几个 block + 块内 UTF-8 字节范围”建立 old→new offset map，不能要求临时 `bN` handle 在修订后保持字符串不变；
- old→new offset map 能把所有 UnitSet 的物理范围唯一映射到修订文本；
- 修订后的 input、snapshot 和 artifact 哈希全部重新计算。

报告输出两种结论：

```text
SafeForB             可以确定性重绑定 UnitSet，直接重新规划 B
RequiresARebuild     不能证明单元结构未受影响，必须重新执行 A
```

`SafeForB` 不是数学正确性证明，只表示 patch 没有破坏 A 的结构契约。即使通过该门，P 的 replacement 仍然需要用户审核或自动策略授权。

### v1 推荐路径

第一版直接实现确定性的 UnitSet 重绑定，并保留安全失败回退：

1. 生成新的 `MarkdownSnapshot`，其 `artifact_id`、`content_hash` 和 `conversion_id` 指向修订 Markdown；
2. 将用户原选区通过 offset map 映射到新文件；
3. 用新 snapshot 重建 `DiscoveryInput`；
4. `SafeForB` 时，系统通过确定性范围重绑定重建新的 UnitSet 检查点，再重新规划 B；
5. `RequiresARebuild` 时，暂停 B，重新执行一次 A 单元发现和确定性验证，再用新的 UnitSet 规划 B；
6. B 只接收新 input、新 UnitSet 和新 block 表，输出的所有 source label、reference、relation evidence 都绑定修订 Markdown。

这样 P 与 A 的职责保持分离，同时不会为每个安全的局部公式修订强制增加一次模型调用。一次审核可以应用多个 patch，整个集合只触发一次安全门；只有安全门无法证明边界稳定时才额外重跑 A。

重绑定不能只做一个全局的 `old_offset + delta`。实现应保存每个 patch 的旧范围、新范围和映射状态，并逐个迁移每个 unit selector、issue evidence、coverage 和边界标记。任何一个范围出现部分覆盖、端点无法映射、结构指纹变化或多个新范围候选，都返回 `RequiresARebuild`，不能猜测。

`RequiresARebuild` 的额外 A 调用在用户已经接受 patch 后自动开始，不再要求第二次确认，但必须在任务浮窗中显示并允许取消；它计入新的活动调用 timeout、总调用数和 token 预算。重跑后的 UnitSet 如果不能覆盖已应用 patch 的有效范围，或确定性验证失败，工作流停在 `NeedsAttention`，不进入 B。重跑后的新 issue 只记录并随结果展示，不再触发第二个 P round。

### Stage 2 的契约

Stage 2 的 `FactIssue` 结构保持不变。`SourceFactInput` 增加一个可选的、由系统提供的只读 `source_warnings`（必要时升级 input version），用于让 B 看见被拒绝或未能修订的 A issue；它不是 B 生成的 `FactIssue`，也不允许 B 借此修改 unit 或原文。除此之外，B 仍然接收 blocks、冻结单元表和事实任务，且其 `PreparedFactRequest` 使用修订后的 `DiscoveryInput`：

- B 不需要理解 patch 格式；
- B 不得引用旧 Markdown 的 block handle；
- `source_warnings` 中的 A issue 使用重绑定后的 exact selectors；没有未处理 issue 时为空；
- B 产生的 `SourceSpan` 使用修订 artifact 的 `MarkdownLocator` 和文本哈希；
- `SourceRegistrationPlan.markdown_artifact_id` 自动变为修订 Markdown artifact；
- 原 Markdown 与修订 Markdown 的派生关系由 `ConversionRun` 保留。

如果用户拒绝所有 patch，工作流可以继续使用原 snapshot，但结果必须保留未处理的转换 issue。若 Stage B 已经开始，禁止在中途替换 Markdown；需要回到 P 的审核边界，并重新通过安全门，以保证一个 run 只对应一个有效 source snapshot。

### Prompt 边界

P 不应继承 A 或 B 的完整对话历史，也不应重复接收一份完整的 discovery proposal。系统为每个 P 请求构造独立的 `MarkdownPatchInput`：

- base snapshot 和 input fingerprint；
- 需要修订的 issue 及其确定性 evidence 范围；
- evidence 所在的完整 block，以及受限数量的前后 block；
- 冻结 UnitSet 的轻量目录（unit ref、kind、范围和 parent），不重复发送所有 unit 正文；
- patch 允许修改的字段、结构不变量和输出上限。

同一 issue 的上下文窗口应由字节上限控制，窗口不足时让 P 返回“无法安全修订”，不能扩大到整个文档。B 仍然接收经过重绑定后的完整授权选区和任务上下文，但不接收 patch diff 或 P 的推理文本。这样 patch 阶段增加的是一次有界的专用调用，不会把 A 的结构任务、P 的编辑任务和 B 的事实提取任务混在一个 prompt 中。

## Artifact 与来源图建模

应用 patch 后创建一个新的 `ConversionRun`：

```text
原 Markdown artifact --input--> MarkdownPatch ConversionRun --output--> 修订 Markdown artifact
```

`ConversionRun` 的固定填法如下：

- `tool_name = "mathscape-markdown-patch"`；
- `tool_version = "mathscape-markdown-patch-set-1"`；
- `command_record_status = Unavailable`，`command_template = None`；
- `page_range = None`、`redo_inline_math = None`、`output_format = "markdown"`；
- `additional_options` 只包含 `patch_fingerprint`、`patch_schema_version`、`selection_fingerprint` 三个约定键；
- `exit_status = Succeeded`，时间字段由后端记录，环境、warnings 和 cost 按现有可选字段处理。

输入端点是 base Markdown artifact，输出端点是修订 Markdown artifact。修订 artifact 的 media type、edition 和 rights 从 base Markdown 复制，文件名可以增加受控的 `.patched.md` 后缀。完整 patch set、审核决定和应用结果保存在 SQLite，便于查询和幂等；不能只把信息塞进不可检索的前端状态。

建议新增表（或等价的版本化 JSON 记录）：

- `markdown_patch_sets`：base artifact、base input fingerprint、patch fingerprint、状态、结果 artifact、conversion、时间和 actor；
- `markdown_patch_operations`：patch_ref、issue_ref、原始范围、旧文本哈希、模型 replacement、当前 draft replacement、draft revision、draft actor、状态和 ordinal；
- `markdown_patch_decisions`：decision_id、patch set、选择、authority、expected fingerprint、actor；
- `markdown_patch_applications`：应用结果、错误、patched artifact 和 offset map 摘要。

建议把 patch manifest 作为数据库可重放记录；只有需要导出或跨工作区传递时，再生成独立 JSON artifact。这样不会让每个 patch 在来源图里额外产生难以理解的技术节点。

已有原 Markdown 来源节点不自动被修改或删除。修订分支产生新的 SourceSpan 和 SourceSemanticUnit；旧分支保留，用户可以通过现有地图编辑流程显式退役旧节点。后续语义对齐阶段可以判断两条来源单元是否对应，不能在 patch 应用时凭标题自动合并。

v1 只处理尚未完成来源登记的当前 discovery run。系统在提交 patch 前检查 base Markdown 是否已经有该 run 之外的活动 `SourceSpan`、`SourceSemanticUnit` 或 `SourceUnitRelation`；已有来源对象的 artifact 不允许通过本接口二次修订。用户仍可以对没有 Marker `ConversionRun` 的普通 Markdown artifact 开始一次新的 patch run，此时 patch ConversionRun 直接以该 Markdown 为输入。已登记文档的修订属于后续独立的分支重建流程。

## Desktop/API 变更

新增或扩展以下后端能力：

```text
get_markdown_patch_review(run_id)
preview_markdown_patch(run_id, draft)
submit_markdown_patch_decision(run_id, expected_patch_fingerprint, decision)
```

`preview_markdown_patch` 对 draft 做纯确定性校验并生成 diff；它不写 artifact、ConversionRun 或来源对象，但会把 draft revision 持久化到 SQLite。`submit_markdown_patch_decision` 统一处理接受子集、全部拒绝和采用已校验 replacement，提交时再次校验 fingerprint。成功返回 `MarkdownPatchApplyReceipt`，至少包含 patch set、旧/新 artifact、旧/新 input fingerprint、patch conversion、PatchSafetyReport、重建状态和失败诊断。它不直接接受来源节点；来源接受仍沿用重新验证后的 discovery acceptance。

三个接口都绑定 `run_id` 和当前 patch fingerprint。接受后 run 进入 B 或自动重跑 A；拒绝后直接使用原 snapshot 进入 B。

任务浮窗增加 `reviewing_patches`、`applying_patches`、`rebuilding_units` 三个阶段。审核页关闭或切换阅读器不能丢失 patch 草稿；草稿属于 run 和 patch set fingerprint，不属于页面组件。

## 开发拆分

### P1：领域契约与纯函数

- 新增 `markdown_patch` domain DTO、版本常量、状态和审核决定；
- 为 Stage A issue 增加稳定 `issue_ref`，新增独立 Patch prompt、schema 和 manifest；
- 实现纯函数 patch resolver、冲突检测、字节范围校验、patch fingerprint；
- 实现应用后的 UTF-8 内容和 old→new offset map；
- 实现 `PatchSafetyReport`，覆盖 block/heading/围栏/数学分隔符和 unit 边界不变量；
- 实现 `ValidatedUnitSet` 的确定性范围重绑定，重建 canonical selectors、coverage 和 fingerprint；
- 定义 patch round、暂停/恢复状态、活动调用 timeout 与人工审核等待的边界；
- 覆盖唯一/重复 occurrence、越界、重叠、替换冲突、换行结构变更、安全门和重放测试。

### P2：Artifact Pool 与 SQLite

- 为 Artifact Pool 增加受控 `import_bytes`/`import_reader`，沿用内容哈希、no-clobber 和完整性检查；
- 新增 patch set、decision、application 的迁移和类型化读写；
- 记录 `mathscape-markdown-patch` ConversionRun；
- 实现同一个 patch set 的幂等应用和 base artifact 漂移拒绝。

### P3：Orchestrator 工作流

- Stage A 校验后持久化 issue 检查点，独立启动 P；
- P 接受 patch 后创建修订 artifact，重建 snapshot/input，并运行 Patch Safety Gate；
- `SafeForB` 时确定性重绑定 UnitSet，`RequiresARebuild` 时重新执行 A；
- 以新的 UnitSet 重规划 Stage B，禁止旧 B 结果混入；
- 将修订 artifact 传入 source registration 和 replay/revalidation；
- 增加暂停审核、取消、失败、拒绝 patch、部分选择、draft 恢复、重复提交和 patch round 终止测试。

### P4：Desktop 审核与回溯

- 原文/修订 diff 和公式渲染预览；
- 单项 patch 选择、replacement 编辑和重新校验；
- 任务状态与 patch conversion 结果展示；
- 点击来源单元时打开修订 Markdown 并高亮新的 SourceSpan；
- 在材料图/景观图中显示 Markdown patch ConversionRun 的输入输出关系。

### P5：后续优化

- 同一原始 Markdown 的 patch 分支比较和重新选择；
- 更精细的结构保持证明，扩大 `SafeForB` 的覆盖范围；
- 允许经过人工审核的跨 block 结构 patch；
- 将人工确认过的 patch 作为后续模型的错误样例，但不把它们自动提升为数学真理。

## 验收标准

一条最小可验收链路应满足：

1. Stage A 对一个包含转换异常的 fixture 输出带稳定 `issue_ref` 的 issue；
2. 独立 P 根据该 issue 输出至少一个 patch candidate；
3. 系统拒绝越界、歧义、重叠或会改变 block 结构的 patch；
4. 用户接受 patch 后，原 Markdown 字节和节点保持不变，并产生新的受管 Markdown artifact；
5. 新 artifact 通过 `ConversionRun` 指向原 artifact，patch set 可重放且重复提交幂等；
6. `SafeForB` 能确定性重绑定 UnitSet，`RequiresARebuild` 能阻止 B 并重新执行 A；
7. Stage B 的事实任务均绑定新 input fingerprint；
8. 生成的 SourceSpan、SourceSemanticUnit、SourceUnitRelation 的 Markdown locator 指向新 artifact；
9. 重新打开 workspace 后可以恢复 patch 审核结果和 Stage B 检查点；
10. 审核等待不占用活动 LLM timeout，恢复时不丢失 draft；
11. `RequiresARebuild` 最多触发一次 A 重建，重建后的新 issue 不再触发 P；
12. 部分接受时只有接受子集进入新 artifact，拒绝项随 issue 留在 B 输入中；
13. 拒绝 patch 或 patch 应用失败时，原流程仍可明确地继续或停止，不产生半个来源登记；
14. 已登记来源对象的 artifact 被明确拒绝进入 v1 patch 流程。

## 风险与取舍

最大的工程风险不是字符串替换，而是“修订后旧 selector 是否仍有意义”。因此 v1 把文本修订放进独立 P 阶段，用 Patch Safety Gate 决定是否可以直接重绑定 A 的结构。无法证明边界稳定时才重跑 A；安全的局部修订不需要额外模型调用。这样既保持职责分离，也避免让旧锚点、旧指纹和旧事实任务混用。

这项能力能改善 Marker 转换造成的局部文字错误和后续来源定位质量，但不会证明替换后的公式、定理或证明在数学上正确。数学语义解析、Lean 形式化和来源 patch 必须继续保持不同的可信状态。

## 当前实现边界（v1 首轮）

当前首轮已经提供了独立的 P producer、确定性 resolver、安全门、UnitSet 重绑定、Artifact Pool/SQLite 持久化以及 Desktop 审核/预览/提交接口。它可以对已经保存的 A 检查点显式触发 P，审核后生成派生 Markdown artifact 和 `mathscape-markdown-patch` ConversionRun；原 artifact 不会被覆盖，草稿、决定和应用回执可在重启后恢复。

首轮 Desktop 仍保留原有 A→B 自动运行路径，P 通过结果面板中的“生成 Markdown 修订建议”显式触发；应用后需要基于修订 artifact 重新发起发现。将 `AwaitingPatchReview`、`SafeForB` 的 B 恢复和 `RequiresARebuild` 的一次性 A 重建编排进同一个活动任务，是下一轮工作流状态机改造，不应伪装成当前接口已经完成。
