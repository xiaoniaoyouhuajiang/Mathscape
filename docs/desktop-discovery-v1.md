# Desktop 内容发现 v1

状态：Markdown 选区到单次真实 LLM 调用、提案验证和内存展示已接通；运行/提案持久化、
审核、接受与来源登记待实现。
日期：2026-09-09。

## 用户流程

1. 在阅读视图打开受管 Markdown，选择连续块范围或全文。
2. 可用“构建发现输入”预览实际原文；点击“使用 LLM 发现内容”调用默认 profile。
3. 运行期间按钮禁用，其他阅读视图可看到流式接收中的活动任务；Desktop 全局只允许一个发现调用。
4. 成功时展示单元、原文明示关系、问题、完整 JSON 结果以及模型、token 和耗时摘要。
5. 非法提案展示拒绝类型与确定性诊断；API/鉴权/限流/超时等显示为调用失败。

界面始终说明有效结果“尚未接受或入图”。当前关闭或切换阅读视图后不会保存结果；活动调用
仍会完成，但没有持久化结果可供重新打开。自动候选策略仍只是选择请求的一部分，Producer
不读取该策略，当前 Desktop 也不执行接受。

Workspace 设置中心提供人工审核、自动 candidate 和每次询问三种默认方式，并提供 30–1800 秒
的 Discovery 总超时与 5–60 秒的连接超时。任务启动时解析出的实际 admission 和超时会写入
返回的运行摘要；活动状态显示最长等待时间。超时不会触发隐式重试。

Discovery 使用供应商协议对应的流式传输，并在内存中收集文本分片。流式分片只是传输进度，
不会作为部分 Proposal 暴露、保存或入图；只有流结束、完整 JSON 解码并通过 canonical DTO 与
确定性来源验证后，Desktop 才展示 Proposal。连接测试继续使用短小的非流式请求。供应商若
不支持所选协议的 streaming，会明确失败，不会隐式再发一次非流式请求。

## 后端链路

Tauri `run_discovery` 接收：

```text
request
  selection             DiscoverySelectionRequest
  profile_id            可空；null 使用 SQLite 中的默认 profile
```

服务端重新读取 Managed Artifact、验证不可变快照、解析选择器并构建
`PreparedDiscoveryRequest`；浏览器不能提交自制 DiscoveryInput。随后读取 profile，使用系统
钥匙串中的 credential 和 Workspace 的有效超时策略创建 `GenaiBackend`，交给
`DiscoveryProducer` 做一次流式调用和验证。

返回 `DesktopDiscoveryOutcome`：

- `valid`：展示用的已验证提案与运行摘要；单元额外带最多 480 字符的原文摘要。
- `invalid_proposal`：拒绝类别、消息、确定性诊断，以及可解码时的原始 Proposal。
- `failed`：稳定的 LLM 错误类别、消息与运行摘要。

完整模型正文和 reasoning 不进入运行摘要。后端成功测试还检查来源图节点数不变，防止本阶段
暗中写图。活动任务由内存 permit 管理，正常、错误或取消 future 时均释放；重复提交在真正
调用 LLM 前拒绝。当前只支持默认 profile 的 UI 路径，Rust 请求已预留显式 profile ID。

## 测试

Rust 服务测试使用受管 Markdown、持久化 profile、内存 credential store 和本地 HTTP 模拟
OpenAI-compatible endpoint，覆盖真实 `GenaiBackend → DiscoveryProducer`，并验证重复提交与
无图写入：

```sh
cargo test -p mathscape-desktop --test service
```

浏览器测试复用现有 Chromium，覆盖选区提交、运行中禁用、提案卡片、未入图提示和 usage：

```sh
npm --prefix frontend run test:reader
```

下一步应设计 discovery run/proposal 的 SQLite 持久化和结果恢复；之后再让审核或自动策略
产生 AcceptedCandidate，并通过独立来源登记事务进入图。
