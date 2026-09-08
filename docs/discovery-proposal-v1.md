# Discovery Proposal v1：最小输出契约

状态：Rust DTO、选择器授权解析、确定性验证器、输出 Schema、版本化 Prompt 与离线请求
构建器已实现；LLM 调用和保存待实现。
日期：2026-09-08。上位设计为工作区 `docs/rdtx/source-content-discovery-draft.md`；
本文件将其中的输出示意收敛为当前工程契约。

## 处理边界

```text
DiscoveryInput → producer → DiscoveryProposal → 确定性验证 → 保存发现结果
                                    ↓
                         有界修订或明确失败

已保存发现结果 + 相关原文 + 继承环境 → 按需上下文化 → DialectScope
```

校验与保存是程序职责，不是第三次 LLM 分析。内容发现可以独立完成和浏览，后续上下文化
根据需要读取发现成果。此 DTO 不构造 DialectScope、LocalSymbol、ProofContext、Alignment
或 R/D/T/X；定义、notation、method、remark、narrative 等单元保留相关原文即可。
首版不增加独立 scope_effects、placement、引用实体或反向列表。

## 已落地的类型

`mathscape-domain::DiscoveryProposal` 是可反序列化的未信任传输对象，没有 EntityHeader、
TrustRecord、EntityPayload 或 MapPatch 转换。字段如下：

| 字段 | 职责 |
| --- | --- |
| version | 固定 `mathscape-discovery-proposal-1`；枚举解码拒绝其他版本 |
| input_fingerprint | 回显 DiscoveryInput.fingerprint，不由模型计算 |
| units | 临时 ID、类型、原文标签/标题、展示标题、选择器、父候选、完整性 |
| relations | 本提案单元端点、关系、证据选择器、简短依据 |
| references | 引用者、原文位置、引用原文、目标标签、可空的本提案目标 |
| issues | 问题类型、可空的受影响单元、证据、观察、可空的补上下文请求 |
| coverage | 区域选择器，以及 processed / skipped / unprocessed |

五个集合均必须出现，可以为空；不能用遗漏集合表示“没有发现”。Domain 解码允许可选字段
为 null 或省略，序列化时统一输出 null；生产输出 Schema 更严格，要求这些字段显式出现。
未知字段在各层拒绝，不接受模型添加的 stable_id、trust、字节偏移等。
引用和关系仅支持提案内 unit temp_id，首版不支持凭空引用已有图节点。

运行 ID、请求 ID、模型、prompt 版本、预算、重试次数、人工审核/自动发布策略由系统调用
记录保存，不让模型重复填入。当前 DiscoveryInput 未包含 request_id，因此 Proposal 以
输入指纹绑定材料；未来运行记录另行关联请求、输入、提案，不能把材料指纹当作唯一任务 ID。
精确复用最终还需包含 producer 契约和 prompt 等兼容性策略，当前指纹本身不代表缓存已实现。

## 选择器与类型

`DiscoverySelector` 仅允许 block_range 和 quote_range，可转换为既有 SourceSelector
以复用语法；该转换不代表选区已获授权。producer 不允许使用 whole_document。

提案解析器以实际 DiscoveryInput 的可见区间为边界：

- 块 ID 必须出现在该输入中；知道完整 Artifact 的块 ID 不赋予读取权限。
- 块范围按原文位置解释，不能按字符串 ID 排序。
- 完整块范围若伸出已选范围则拒绝，不能静默裁切；块内选区使用 quote_range。
- quote 在对应块的可见片段中必须精确、唯一，并解析到原始 UTF-8 字节区间；不能匹配
  输入未提供的文本。零匹配、重复匹配、逆序、跨越未提供区域都返回诊断。
- requested/context 的权限和覆盖角色分开；当前 v2 构建器和验证器要求 context 为空，
  后续引入上下文时须升级契约和指纹。

因此，不能只调用现有面向整份 AnchoredMarkdown 的 resolve_selection 就声称提案已验证。

DiscoveryUnitKind 使用现有来源类型的子集，排除 proof_step。证明先整体识别。
completeness 复用 UnitCompleteness；左右同时截断用 uncertain，并保留两个 issue。
文件边界、单元完整性、模型处理覆盖、数学正确性彼此独立。

关系方向固定为：proves：proof → statement；solves：proof/solution → problem；
hints：hint → problem；explains：说明者 → 被说明者；cites：引用者 → 被引者；
applies_method：使用者 → method，必须有原文明示证据。
不输出 has_part；parent 是唯一包含记录。类型映射只转换枚举，不提升整个提案为正式实体。
原文引用可保存为 reference；同一引用同时产生 cites 时，规范化阶段应复用该事实。

## 不确定性与覆盖

reference.target 非空仅表示模型提出一个本地候选目标，不等于已解析或已审核。
未找到目标时保留原文和 target_label，target 为 null。

补上下文需求附在 issue.context_request，复用该问题的 evidence、unit 和 observation，
不单独维护 context_requests 数组。允许 before、after、enclosing_unit、local_reference
四种请求，后者带目标标签；不接受路径、URL 或命令。请求仅提供调度依据，是否补材料由
系统的权限、预算与可用性决定，当前不会执行。

coverage 的 processed 表示模型声称遍历了区域，不保证所有单元都被找到；skipped 适用于
已看过但无须建单元的页眉等内容，必须说明原因；unprocessed 表示未处理，必须说明原因。
当前验证器要求 requested 内全部非空白内容被覆盖，纯空白间隙由程序核算；context
不得冒充 requested 完成覆盖。覆盖应无互相冲突的重复区间，不能用单位计数推导完成。
空 units + 未说明的 coverage 缺口不能视为成功。

## 已实现的验证接口

`mathscape_orchestrator::validate_discovery_proposal(input, proposal, limits)` 已实现：

1. 校验输入版本/指纹、输入可信来源、输出字节/对象/字符串数量限制。
2. 检查临时 ID 非空、唯一，所有引用闭合，父链无环。
3. 解析获授权选择器，检查 evidence、标签/标题与引用原文的定位。
4. 检查关系方向与端点类型、证据归属、父链、重叠、完整性与 issue 的一致性。
5. 归一化 coverage，要求非空白 requested 文本全部有处置，并输出全部结构化诊断。

默认限制为每个集合 1024 项、总计 4096 个选择器、提案序列化后 2 MiB。LLM 响应读取层
仍须在反序列化前设置独立字节限制。验证成功返回不可从外部字段构造的
`ValidatedDiscoveryProposal`，其中选择器已经规范化为 UTF-8 字节范围、块 ID 和原文；
状态为 complete 或 partial。该类型仍不是 EntityPayload，也不能直接生成 MapPatch。

诊断含稳定枚举 code、字段 path 和可读 message，可用于 UI 展示或一次有界修订。当前检查
覆盖输入完整性与指纹、提案指纹、限制、临时 ID、引用闭合、父链环、选择器授权、原文标签、
关系类型和证据、引用位置、单元重叠、完整性及 coverage。数学分类是否正确、证据在数学上
是否充分、是否穷尽所有单元仍需模型或人工判断。

JSON 反序列化只保证形状；空字符串、重复 ID、缺失证据、未知目标和错误指纹会在验证阶段
被拒绝。数学错误仍可能通过确定性验证。当前没有提交接口。

保存时记录原始提案、诊断、输入与运行来源，系统分配稳定发现身份。自动发布只允许进入
候选发现视图，不授予审核或形式化状态。后续提升为 SourceSemanticUnit 仍须满足当前 Domain
的 Edition、SourceSpan、DialectScope 条件，并保留发现候选到正式对象的映射。
Markdown 独立定位的 SourceSpan 契约仍待修订，本次没有通过虚构 PDF 页码绕过此条件。

后续修订方向见 [`内容结构生成、来源登记与语义晋升草案`](discovery-registration-promotion-draft.md)：
建议允许上下文未解析的来源单元先受控登记，将完整 DialectScope 的构建延后。
该草案区分接受决策与数学可信性；它尚未改变上述当前 Domain 的必填条件。

## Schema 与 Prompt

版本化资源见 [Source Discovery v1](../prompts/source-discovery/v1/README.md)。输出 Schema
从 DTO 的 JsonSchema 序列化契约派生，并通过测试锁定已发布快照，避免两套字段手工漂移。
形状校验和确定性来源校验分工不变；Schema 不承担范围授权或数学正确性判断。

`build_discovery_request(document, selection, limits)` 返回验证过的 DiscoveryInput、
供应商中立 LlmRequest、manifest 与 Prompt 指纹。固定指令在 system 消息，材料 JSON 在
user 消息；输出使用 JSON Schema，无工具调用。此接口不发送请求、不保存候选、不改变
接受状态；响应处理、运行记录与发布仍由后续 Producer/application 实现。

## 最小样例与验证

仓库样例为 `crates/mathscape-domain/tests/fixtures/discovery-proposal.json`，对应自编原文：

```markdown
习题 1：设 $a_n=a$，用定义证明 $a_n$ 收敛于 $a$。

解答：任取 $\epsilon>0$，取 $N=1$。当 $n>N$ 时，$|a_n-a|=0<\epsilon$。因此收敛于 $a$。
```

样例的 b-problem、b-solution 和 sha256:111… 是明确的测试占位值，不是已登记 Artifact
或真实运行指纹。orchestrator 测试将它们替换为实际生成的块 ID 与输入指纹，并运行完整
验证器；另有用例覆盖越权块、裁切块、指纹篡改、父链环、错误关系、证据归属、重叠、
coverage 缺口、partial、完整性和规模限制。

```sh
cargo test -p mathscape-domain --test discovery_proposal
cargo test -p mathscape-orchestrator --test discovery_input
cargo test -p mathscape-orchestrator --test discovery_validation
cargo test -p mathscape-orchestrator --test discovery_prompt
```
