# Discovery Producer v1

状态：单次 LLM 调用、短句柄绑定、响应检查、DTO 解码、SelectorResolver 和确定性验证已实现；持久化、
接受、来源登记、自动修订与 Desktop 触发待实现。
日期：2026-09-08。

## 边界

`mathscape_orchestrator::DiscoveryProducer` 消费 `PreparedDiscoveryRequest`，通过一个已有的
`LlmBackend` 执行恰好一次调用。Backend 已经封装 profile 和凭据解析，Producer 不接触 API
key。处理链为：

```text
PreparedDiscoveryRequest
        ↓ 请求契约与输入指纹一致性
DiscoveryModelInput（短句柄，不含正式块哈希/字节坐标）
        ↓
LlmBackend::execute_stream（一次，无重试）
        ↓ 停止原因、工具调用、文本字节上限
单个 JSON 对象 → DiscoveryProposal
        ↓ 当前生产契约的规范表示
SelectorResolver（绑定正式块 ID、唯一 quote、安全排版规范化）
        ↓
validate_discovery_proposal
        ↓
ValidatedDiscoveryProposal | 明确的拒绝/失败
```

Producer 不写 SQLite，不应用 `DiscoveryAdmission`，不生成 stable ID、SourceSpan、
DialectScope 或 MapPatch。`Valid` 只表示提案通过当前结构和来源确定性规则，不表示用户已经
接受、来源已经登记或数学内容正确。

## 输出分类

`DiscoveryProductionOutcome` 明确区分三类结果：

- `Valid`：包含规范化后的 `ValidatedDiscoveryProposal` 和运行摘要。
- `InvalidProposal`：模型调用成功，但响应无法作为有效提案使用；可能保留已经解码的原始
  `DiscoveryProposal`，用于审核或诊断。
- `Failed`：请求自身或 LLM backend 失败，例如配置、鉴权、限流、超时或传输错误。

提案拒绝覆盖 token 上限截断、内容过滤、不可用的停止原因、意外工具调用、空响应、超出
字节上限、非单个 JSON、非规范 JSON、选择器解析失败和确定性验证失败。选择器解析失败
保留模型原始短句柄提案，并返回 unknown block ref、quote not found 或 ambiguous quote
结构化诊断。`Completed`、`StopSequence` 以及
供应商未给出停止原因时可以继续验证；是否缺少停止原因值得警告由后续运行策略决定。

Domain 为历史输入兼容，允许省略部分 `Option` 字段；当前 Producer 契约要求模型显式输出
这些字段并使用 null。实现先解码，再与 DTO 的标准序列化值比较，从而拒绝可解码但不符合
当前生产 Schema 表示的响应。未知字段、枚举和值类型由 Serde/Domain 拒绝；位置授权、关系、
父链、证据、完整性和 coverage 由确定性验证器检查。

响应字节上限取 Producer 本地上限与 Prompt manifest 上限的较小值，默认均为 2 MiB，且在
JSON 解码前检查。该限制避免解码不受限的正文，但当前 `LlmBackend` 返回完整响应后 Producer
才接手，因此它不是网络流读取上限；若未来需要抵御供应商发送超大响应，还须在 transport
层加入流/HTTP body 限额。

## 运行摘要与隐私

`DiscoveryRunSummary` 保留任务版本、输入指纹、Prompt 指纹、profile/provider/请求模型，
以及成功收到响应后的 resolved target、停止原因、usage、response ID 和耗时。它刻意不包含
reasoning、完整正文或 raw response，也不是持久化 run schema 或 TrustRecord。

后续实现 `llm_runs` 时可以从摘要构建存储记录，但应另行加入运行 ID、状态迁移、尝试序号和
错误分类。需要保存提案正文或诊断时，应进入受管理的发现结果/Artifact，而不是塞进普通日志。

## 验证

回归使用 `ScriptedLlmBackend`，不需要 API key 或教材文件：

```sh
cargo test -p mathscape-orchestrator --test discovery_producer
```

覆盖有效提案、短句柄到正式块 ID 的恢复、未知句柄、backend 失败、非法 JSON、截断、工具
调用、响应超限、非规范可选字段和输入指纹不匹配。`PreparedDiscoveryRequest` 只能由构建器
创建，模型投影与绑定表随请求一起生成并只有只读访问，避免请求和已记录指纹在调用前分离。
[SelectorResolver](selector-resolver-v1.md)另有独立的投影、歧义和安全规范化回归。
[Desktop 内容发现](desktop-discovery-v1.md)已接通一次人工触发与
结果展示；下一步先持久化运行和提案并收集真实模型失败分布，再决定是否增加最多一次、
携带结构化诊断的 repair。
