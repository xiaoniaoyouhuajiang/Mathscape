# Desktop LLM 配置 v1

状态：已实现。日期：2026-09-08。

Desktop 顶栏的 LLM 状态按钮打开配置页。用户可以选择供应商和 wire protocol，填写模型 ID、
可选 Base URL 与 API key，并选择“保存配置”或“保存并测试”。连接测试会真实发送一个最多
128 output tokens 的短请求，因此可能产生极小的供应商费用；普通保存不会访问模型服务。

配置分为两个存储边界：

- 当前 Workspace 的 `mathscape.sqlite3` 通过 migration 2 保存版本化 `LlmProfile`、默认项和
  `CredentialRef`，不保存密钥；底层支持多个 profile，首版 UI 编辑一个默认 profile。
- API key 由系统原生钥匙串保存，条目由稳定 Workspace ID 和 profile ID 定位。Workspace 在
  同一台机器上移动后引用不变；复制到另一台机器时必须重新输入 API key。

留空 API key 表示保留原密钥；勾选清除才会删除钥匙串条目。UI 回显只有
`has_credential`，后端不会把密钥传回 WebView。保存请求的 Rust Debug 实现也固定脱敏。

Tauri 暴露三个窄命令：`get_llm_settings`、`save_llm_profile` 和 `test_llm_profile`。应用启动时
先完成 SQLite migration，避免首页快照与配置读取并发初始化新数据库。测试连接读取已经保存的
profile 和运行期密钥，构造供应商中立请求后调用 `GenaiBackend`；不会创建地图实体或运行记录。
