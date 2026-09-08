# LLM 接入基础设施 v1

状态：供应商中立契约、`genai` 单次调用适配器、结构化输出辅助和确定性测试后端已实现；
Discovery producer、运行记录持久化、桌面设置页与 agent runtime 待实现。
日期：2026-09-08。

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

`response_format_for<T>` 从 `schemars::JsonSchema` 生成响应 schema；`execute_typed<T>` 只负责
一次调用与 JSON 反序列化，不替代 DiscoveryProposal 的确定性验证器。

### 密钥

profile 只保存 `CredentialRef`，绝不保存 API key。`CredentialResolver` 在调用时解析引用；
当前 `MemoryCredentialResolver` 仅供进程运行期与测试使用，并对 Debug 输出脱敏。桌面应用
应另行实现系统钥匙串 resolver，在 profile SQLite 行中仍只记录引用。

请求、错误和运行日志不得包含 Authorization header 或解析后的 secret。原始模型响应可能
含用户材料，因此 `capture_raw_response` 默认关闭；若为审计开启，应按 workspace 私有数据
处理，而不是写普通日志。

## 调用语义

`LlmBackend::execute` 表示一次尝试：适配器设置连接与总请求超时，但不自动重试。这样 429、
超时和不合法 JSON 不会在底层产生不可见的额外费用。orchestrator 后续根据任务策略决定：

1. 是否对传输错误退避重试；
2. 是否把 DTO 校验诊断交给模型做一次有界修订；
3. 总尝试数、token、时间和费用预算；
4. 用户取消后是否丢弃迟到响应；
5. 何时将 proposal 保存为候选，或进入人工审核。

测试和离线开发使用 `ScriptedLlmBackend`：它按队列返回固定结果并记录请求，producer 测试不需
API key 或网络。协议适配器测试使用进程内 HTTP 模拟服务，验证请求路径、鉴权、结构化输出、
usage 和限流分类，不依赖真实供应商。

## 持久化建议

v1 尚未增加 SQLite 表。接入应用层时建议分为两类：

- `llm_profiles`：可编辑配置，仅含 profile、endpoint、模型和 credential reference；
- `llm_runs`：不可变调用事实，关联任务、输入指纹、prompt/契约版本、profile、解析后的模型
  身份、时间、状态、尝试序号、usage 和错误类别。

默认不保存完整 prompt、API key、Authorization header 和原始响应。若为 proposal 审计保存
输入/输出正文，应进入受管理 artifact，并由 run 记录引用其内容哈希，避免 SQLite 大字段和
重复副本。运行记录不能直接代表数学结论可信；可信度来自 DTO 校验、审核和后续形式化状态。

## 下一步接入顺序

1. 为 DiscoveryProposal 及其嵌套 DTO 补 `JsonSchema`，实现版本化 Discovery prompt producer。
2. 用 `ScriptedLlmBackend` 完成 `DiscoveryInput → proposal → validate` 的离线集成测试。
3. 增加 profile/run SQLite repository 与系统钥匙串 resolver，再接 Desktop 设置及任务状态。
4. 增加 orchestrator 的取消、预算、退避和一次有界 repair policy。
5. 当 Lean/图分析需要多轮工具循环时，在同一 `LlmBackend` 契约之上增加 agent runtime；
   不让 agent 框架成为 Domain 或存储依赖。

当前不实现流式 UI、多供应商 fallback、自动模型路由、agent memory 或任意 MCP 工具执行。
这些能力需在 Discovery 单次结构化调用得到真实成本与失败数据后再设计。
