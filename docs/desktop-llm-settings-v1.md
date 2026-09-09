# Desktop 设置中心 v1

状态：已实现。日期：2026-09-09。

Desktop 顶栏的 LLM 状态按钮打开统一设置中心。内容发现策略区保存默认审核方式、Discovery
请求总超时和高级连接超时；LLM Profile 区允许选择供应商和 wire protocol，填写模型 ID、
可选 Base URL 与 API key，并选择“保存配置”或“保存并测试”。连接测试会真实发送一个最多
128 output tokens 的短请求，因此可能产生极小的供应商费用；普通保存不会访问模型服务。

配置分为三个存储边界：

- 当前 Workspace 的 `mathscape.sqlite3` 通过 migration 2 保存版本化 `LlmProfile`、默认项和
  `CredentialRef`，不保存密钥；底层支持多个 profile，首版 UI 编辑一个默认 profile。
- migration 3 的 singleton `workspace_settings` 保存版本化运行策略文档。缺少记录时采用安全默认：
  人工审核、Discovery 总超时 600 秒、连接超时 15 秒；允许范围分别为 30–1800 秒和 5–60 秒。
- API key 由系统原生钥匙串保存，条目由稳定 Workspace ID 和 profile ID 定位。Workspace 在
  同一台机器上移动后引用不变；复制到另一台机器时必须重新输入 API key。

审核方式有“人工审核”“验证后自动接纳为 candidate”和“每次运行时选择”。后两者都不能绕过
确定性验证，也不直接产生 canonical 实体。“每次运行时选择”会在 Markdown 阅读页禁用提交，
直到用户为该次任务明确选择。设置变更会通知当前打开的阅读视图。当前版本把实际 admission
记录在任务和返回摘要中，但尚未实现 proposal 持久化/登记，因此“自动 candidate”暂不执行入图。

Discovery 创建 backend 时使用当次解析出的不可变运行策略。活动任务显示请求总超时；返回的
运行摘要记录实际 admission、连接超时和总超时。超时不会自动重试，错误会同时显示两个有效
阈值。连接测试固定使用 45 秒总超时，但复用 Workspace 的连接超时。

留空 API key 表示保留原密钥；勾选清除才会删除钥匙串条目。UI 回显只有
`has_credential`，后端不会把密钥传回 WebView。保存请求的 Rust Debug 实现也固定脱敏。

Tauri 暴露 LLM profile 的三个窄命令 `get_llm_settings`、`save_llm_profile`、
`test_llm_profile`，以及运行策略的 `get_workspace_settings`、`save_workspace_settings`。应用启动时
先完成 SQLite migration，避免首页快照与配置读取并发初始化新数据库。测试连接读取已经保存的
profile 和运行期密钥，构造供应商中立请求后调用 `GenaiBackend`；不会创建地图实体或运行记录。
