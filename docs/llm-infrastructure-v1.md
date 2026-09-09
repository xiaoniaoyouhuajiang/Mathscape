# LLM 接入基础设施 v1

状态：供应商中立契约、`genai` 单次及流式调用适配器、结构化输出辅助、确定性测试后端、
profile 持久化、桌面设置页和 Discovery producer 已实现；运行记录持久化与 agent runtime 待实现。
日期：2026-09-09。

## 目标与边界

LLM 是 Mathscape 数学解析流水线的非可信 producer，不是 Domain Kernel，也没有数据库写入权。
首版调用链为：

```text
版本化 prompt + DiscoveryInput
              ↓
     provider-neutral LlmRequest
              ↓
       LlmBackend（一次调用）
              ↓
  文本 / JSON / tool call + 运行元数据
              ↓
DTO 解码 → 确定性验证 → 审核或受控发布
```

`mathscape-llm` 只负责模型目标、消息、工具、输出格式、调用和响应规范化。prompt 组装、
重试/修订预算、任务取消策略、DTO 语义校验、保存和发布属于 orchestrator/application。
因此以后替换 Rust LLM 库，不会改变 EMIR、DiscoveryProposal 或 SQLite schema。

## 已实现契约

### Profile 与能力声明

`LlmProfile` 是可持久化配置，包含稳定 profile ID、供应商名、明确的 wire protocol、模型名、
可选 base URL、凭据引用和能力声明。供应商身份与协议分离：自建中转站可声明任意 provider，
同时明确采用 `open_ai_chat_completions`；系统不根据模型名猜测协议。

endpoint 必须为无内嵌账号、query 或 fragment 的 HTTP(S) base URL，并以 `/` 结尾。
能力采用 supported / unsupported / unknown 三态。明确 unsupported 的结构化输出、工具或
reasoning 请求会在网络调用前失败；unknown 表示允许尝试，最终以供应商响应为准。

首个适配器基于 `genai 0.6.5`，覆盖：OpenAI Chat Completions 与 Responses、Anthropic
Messages、Gemini GenerateContent、Ollama，以及若干已知供应商和兼容中转站。`genai` 类型
不出现在 Mathscape 公共契约中。

### 请求与响应

`LlmRequest` 提供：

- 有版本的 task 身份与可选输入指纹；
- system / user / assistant / tool 消息和多 part 内容；
- JSON Schema 工具定义、tool choice 和 tool call 往返；
- text、JSON object、JSON Schema 三种输出要求；
- temperature、top-p、token 上限、stop、seed 和 reasoning effort；
- 默认关闭的原始响应采集。

请求在出网前执行确定性形状校验。`LlmResponse` 统一保留解析后的文本或 tool call、停止原因、
token usage、响应 ID、实际模型身份、耗时及可选原始响应。供应商错误规范化为配置、凭据、
鉴权、限流、超时、传输、供应商拒绝、无效响应和能力不支持等稳定类别。

Domain/producer 持有的 canonical JSON Schema 不因供应商而改变。发送 OpenAI Chat Completions
或 Responses 严格结构化请求时，适配器会复制并 lowering 为其支持的子集：当前仅移除 `$ref`
同级的 `title`、`description` 等非验证注解，保留 `$ref`、`$defs` 和所有断言约束；Anthropic、
Gemini、Ollama 原生协议继续收到原始 Schema。返回内容仍按 canonical DTO 和确定性验证器处理。

HTTP 错误响应只允许提取常见 JSON `error.code`、`error.param` 和 `error.message`，控制字符会被
归一化且字段有长度上限；未知字段、请求回显和非 JSON 响应体不会进入 UI 或日志。这样既能
显示供应商的可操作诊断，又不会无边界地传播网关内部信息。

`response_format_for<T>` 从 `schemars::JsonSchema` 生成响应 schema；`execute_typed<T>` 只负责
一次调用与 JSON 反序列化，不替代 DiscoveryProposal 的确定性验证器。

### 密钥

profile 只保存 `CredentialRef`，绝不保存 API key。`CredentialResolver` 在调用时解析引用；
当前 `MemoryCredentialResolver` 仅供进程运行期与测试使用，并对 Debug 输出脱敏。Desktop
使用 `KeyringCredentialStore`：macOS 写入 Keychain Services，Windows 写入 Credential Manager，
受支持的 Linux 桌面写入 Secret Service；profile SQLite 行中仍只记录引用。

请求、错误和运行日志不得包含 Authorization header 或解析后的 secret。原始模型响应可能
含用户材料，因此 `capture_raw_response` 默认关闭；若为审计开启，应按 workspace 私有数据
处理，而不是写普通日志。

## 调用语义

`LlmBackend::execute` 表示一次完整响应尝试；`execute_stream` 表示一次流式传输尝试，并将事件
收集回同一种 `LlmResponse`。没有原生 streaming 的测试或未来 backend 可使用 trait 的完整
响应 fallback；`GenaiBackend` 对当前支持协议使用真实 SSE/流式适配器。Discovery 固定调用
`execute_stream`，但仍只在完整结果到达后解码和验证，流式分片不越过 producer 信任边界。

两种方式都设置连接与总请求超时，且不自动重试。这样 429、
超时和不合法 JSON 不会在底层产生不可见的额外费用。orchestrator 后续根据任务策略决定：

1. 是否对传输错误退避重试；
2. 是否把 DTO 校验诊断交给模型做一次有界修订；
3. 总尝试数、token、时间和费用预算；
4. 用户取消后是否丢弃迟到响应；
5. 何时将 proposal 保存为候选，或进入人工审核。

测试和离线开发使用 `ScriptedLlmBackend`：它按队列返回固定结果并记录请求，producer 测试不需
API key 或网络。协议适配器测试使用进程内 HTTP 模拟服务，验证请求路径、鉴权、结构化输出、
SSE 分片收集、usage、网关状态保留和限流分类，不依赖真实供应商。

## 持久化建议

SQLite schema v3 已增加前两类表，运行事实仍待实现：

- `llm_profiles`（已实现）：可编辑配置，仅含 profile、endpoint、模型和 credential reference；
- `workspace_settings`（已实现）：singleton 版本化运行策略文档，不包含密钥；
- `llm_runs`：不可变调用事实，关联任务、输入指纹、prompt/契约版本、profile、解析后的模型
  身份、时间、状态、尝试序号、usage 和错误类别。

默认不保存完整 prompt、API key、Authorization header 和原始响应。若为 proposal 审计保存
输入/输出正文，应进入受管理 artifact，并由 run 记录引用其内容哈希，避免 SQLite 大字段和
重复副本。运行记录不能直接代表数学结论可信；可信度来自 DTO 校验、审核和后续形式化状态。

## 下一步接入顺序

1. DiscoveryProposal 的 `JsonSchema`、版本化 [Prompt 包](../prompts/source-discovery/v1/README.md)、
   请求构建与[单次 Producer](discovery-producer-v1.md) 已完成。
2. `ScriptedLlmBackend` 已覆盖离线集成测试，[Desktop](desktop-discovery-v1.md)也已接通真实
   backend 调用及有效提案/结构化失败展示；下一步持久化运行与提案。
3. 增加 run SQLite repository 及 LLM 任务状态；profile、钥匙串和 Desktop 设置已完成。
4. 根据真实失败数据增加 orchestrator 的取消、预算、退避和至多一次有界 repair policy。
5. 当 Lean/图分析需要多轮工具循环时，在同一 `LlmBackend` 契约之上增加 agent runtime；
   不让 agent 框架成为 Domain 或存储依赖。

当前实现流式传输及“流式接收中”状态，不把模型尚未验证的增量 JSON 直接渲染为 Proposal；
也不实现多供应商 fallback、自动模型路由、agent memory 或任意 MCP 工具执行。这些能力需在
Discovery 单次结构化调用得到真实成本与失败数据后再设计。
