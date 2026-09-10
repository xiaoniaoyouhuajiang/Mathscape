# Source Discovery prompt bundle v3

本包在 v2 的短句柄与 SelectorIntent v2 之上收紧来源证据精度：来源单元使用带精确
quote 锚点的复数 `source_labels`，`reference.selector` 在 Schema 层只能是
`quote_range`，并禁止 reference 自指。旧 v2 包保留为历史契约。

运行链路不变：DiscoveryInput 经模型投影产生 `b1`、`b2` 等请求局部句柄；LLM 输出
DiscoveryProposal；SelectorResolver 将句柄和 quote 意图规范化为正式来源坐标；Validator
独立复算范围、证据归属与引用端点。通过验证的提案仍只是 candidate，不直接成为地图实体。

角色约束如下：

- unit selectors、relation evidence、issue evidence、coverage：允许 block_range 或 quote_range；
- 每个 `source_labels[].selector`：必须为 quote_range，选择文本必须等于 surface_form；
- 每个 `reference.selector`：必须为 quote_range，选择文本必须等于 surface_form；
- `reference.target`：只能指向另一项本地 unit；没有独立目标时使用 null。

Schema 由 Rust `DiscoveryProposal` DTO 生成。修改后在仓库根目录运行：

```sh
cargo run -p mathscape-orchestrator --example discovery_schema --quiet
cargo test -p mathscape-orchestrator --test discovery_prompt
```

Prompt、Schema 与 manifest 的内容一起参与 prompt fingerprint。默认输出上限仍为 8192
tokens、响应正文上限为 2 MiB；本包不实现自动修订、持久化或语义对齐。
