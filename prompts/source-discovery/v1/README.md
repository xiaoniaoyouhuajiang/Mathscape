# Source Discovery prompt bundle v1

本包完成来源内容发现的输出 Schema、系统 Prompt、请求构建和单次 Producer；尚不执行
自动修订、保存、接受或入图。它不构造 DialectScope，也不认证数学结论。

## 资源与调用

- `manifest.json`：任务、输入/输出契约版本、资源路径及默认输出预算。
- `system.md`：单元划分、关系方向、证据、选区授权、截断、覆盖和不可信原文规则。
- `output.schema.json`：从 Rust DTO 派生的、纳入版本管理的输出 Schema 快照。

`mathscape_orchestrator::build_discovery_request(document, selection, limits)` 先通过现有
DiscoveryInput 构建器重新验证选区，再返回 `PreparedDiscoveryRequest`：

```text
AnchoredMarkdown + ResolvedDiscoverySelection
               ↓ 重新解析、验证选区与输入指纹
DiscoveryInput + system Prompt + output Schema
               ↓
PreparedDiscoveryRequest { input, request, manifest, prompt_fingerprint }
               ↓ DiscoveryProducer（单次调用）
LlmBackend → 响应限额/解码 → 确定性验证
               ↓ 后续 application 工作
接受决策 → 受控来源登记
```

system 消息由固定 Prompt 和 Schema 组成；user 消息是序列化的完整 DiscoveryInput，
原文不插值到 system 指令中。请求无工具，默认不采集原始响应，不设 temperature。
JSON Schema 同时放入 `LlmRequest.response_format`；实际供应商是否接受该 Schema 仍需
后续联调。Producer 不实现静默降级、fallback 或重试策略，也不宣称阻止了所有提示注入。

`prompt_fingerprint` 是 manifest、Prompt 和生成 Schema 的内容哈希。
输入指纹与 Prompt 指纹都不是完整缓存键，复用时还需考虑模型、生成参数及兼容策略。
人工审核/自动接受不改变提取内容，因此不进入模型消息；接受决策留给系统执行。

## Schema 与 DTO 的一致性

Schema 使用 JSON Schema Draft 2020-12，由 `DiscoveryProposal` 及其嵌套 DTO 的
`JsonSchema` 派生，不另行手写字段。生产输出使用序列化契约：所有字段必须出现，
`Option` 字段显式为值或 null，每个对象拒绝额外字段。
Domain 的反序列化仍兼容省略可选字段；这不代表模型输出也允许省略。

当前带 kind/status 标签的互斥枚举从 oneOf 转为等价 anyOf，保留每个分支的固定标签。
这只针对当前 Discovery DTO，不是一个适用于任意 Schema 的转换器。
单测检查发布快照与生成结果完全一致，并覆盖合法/非法 JSON 和真实选区验证。

Schema 负责形状，不替代确定性验证器。临时 ID 唯一性、非空证据、范围授权、父链、
端点类型、原文标签、指纹绑定和 coverage 完整性继续由验证器判断；通过 Schema 不等于
通过这些规则，更不等于数学正确。详见 [Discovery DTO 契约](../../../docs/discovery-proposal-v1.md)。

修改 DTO 后，在仓库根目录运行下列命令查看生成结果，经审查后同步本目录快照：

```sh
cargo run -p mathscape-orchestrator --example discovery_schema --quiet
cargo test -p mathscape-orchestrator --test discovery_prompt
```

输入、字段语义或提取策略发生不兼容变化时须升级对应契约/Prompt 包版本，不能只修改
快照绕过测试。发布快照和运行时 Schema 均来源于同一生成函数。

## 预算与未实现边界

默认输出上限为 8192 tokens，仅是初始请求预算，不保证任意选区都能完整发现。
输入仍受 DiscoveryInputLimits 约束；输入字节上限不等于模型上下文或输出预算。
长章节后续需要分批调度，截断响应不能冒充成功，partial 必须有合法 unprocessed coverage。

manifest 的 `max_response_bytes = 2097152` 已由 Producer 在 JSON 解码前执行，并与 Producer
本地上限取较小值。它发生在 backend 返回完整响应后，不等同于 transport 的网络 body 限额。
停止原因检查、DTO 解码、规范表示与确定性验证也已接通。运行记录、一次有界修订、候选保存、
接受和来源登记仍待实现。当前回归全部离线，不需要 API key 或教材资产。详见
[Discovery Producer v1](../../../docs/discovery-producer-v1.md)。
