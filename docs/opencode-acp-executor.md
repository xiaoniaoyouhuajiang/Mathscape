# OpenCode ACP 执行器

`mathscape-agent::OpenCodeExecutor` 已实现 ACP stdio 适配；Desktop 对话面板可选择
Codex 或 OpenCode。默认程序是 `opencode acp`，使用用户已有的 OpenCode 登录/模型配置，
不索取或代理模型凭证。

Desktop 在模型选择器旁提供执行器选择器，显示各自的本机探测结果；未安装或不可用的
执行器不能选择。默认 Codex，选择保存在 Mathscape 应用数据目录的 `agent-backend`
文件，不写 Codex 配置。`MATHSCAPE_AGENT_BACKEND=opencode` 可在开发时覆盖并固定选择；
`MATHSCAPE_OPENCODE_PROGRAM` 覆盖 OpenCode 程序路径，`MATHSCAPE_CODEX_PROGRAM` 行为不变。
两种执行器共享 MCP 程序路径与生命周期日志 sink，OpenCode 状态在应用数据目录的
`opencode/` 下。模型选择按执行器保存；OpenCode 不提供推理强度选择。

切换执行器只影响下一轮；进行中的事件、文本读取、取消仍由原执行器处理。
每条对话保存各执行器的绑定，同时保留旧的 `harness` 字段作为最近使用的绑定，
旧 Codex 记录无需迁移。切换后的第一轮走「上下文已降级」重建路径，从 transcript
补充上下文，避免沿用缺少另一执行器轮次的旧线程；之后同执行器继续原生线程。
关闭会话或应用时会关闭两个执行器。

## 会话与事件

每个 `session_key` 一个懒启动的常驻进程，由 `mathscape-resident::Supervisor`
管理握手、空闲回收和整个进程组的关闭；授权、工作区等 spec 变化后下次使用重启。
ACP 传输与事件映射在 `acp` 模块，OpenCode 配置和权限策略在 `opencode`。
`session/new` 注册 `mathscape mcp --workspace … --round-file …`，每轮先重写
round 文件。`harness.env` 同时转发到 OpenCode 和 Mathscape MCP 子进程。

已加载线程返回 `Live`；`session/load` 成功返回 `Resumed`，失败交由调用方重建上下文。
load 的历史重放不进入当前轮事件。没有状态副本导入，`thread_state_path` 为 `None`。
每轮上下文注记是用户文本之前的独立块。

工具按 `toolCallId` 配对并缓存 title/kind/rawInput，终态即使省略这些字段也保留身份。
Mathscape 标题通过已知工具目录/白名单的精确查表还原，不按下划线拆分。
本地工具使用 `opencode` server 及 `shell`、`file_change`、`file_read` 稳定名称。
业务状态取自 `rawOutput.output` JSON；ACP failed 始终为 error。

prompt 结果提供 usage；空文本、无工具调用且 outputTokens=0 的 end_turn 被判失败，
用中文提示检查模型额度/认证。无进展期限默认 90 秒，可用 `no_progress_timeout`
调整；到期先取消，再失败，超过 `interrupt_grace` 仍未响应则关闭进程组。
用户取消产生 cancel(false)、cancel(true)、turn_end(cancelled)。

模型目录来自最新 `session/new` 的 model configOptions，无缓存时短暂启动一个探测进程。
选择保存在 `state_dir/model`，新进程通过注入配置选模，活跃会话在当前轮结束后通过
`session/set_config_option` 切换。已持久化的选择不会因一个会话切换失败而回滚；错误会返回
调用者。effort 暂不支持。

## 授权与隔离限制

**隔离弱于 Codex，不使用 bwrap，也没有操作系统级文件/网络限制。**
这是应用层审批策略，不能防止审批与工具实际访问之间的路径替换（TOCTOU），
不能约束 OpenCode 自身、用户装的插件，或绕过 request_permission 执行的工具。
原生工具的全 ask 行为在本次真实运行中受免费额度耗尽阻碍，不能将 fixture 通过
等同于对所有 OpenCode 版本、工具和搜索行为的实际访问保证。

- 原生文件工具开启；只允许 workspace、read_roots、skill_roots、scratch_dir 内读取。
- 原生写入仅 scratch_dir 内；工作区的其他位置也拒绝写入。
- OpenCode shell 默认关闭，本 PR 尚未实现逐命令审批；所有 shell 请求拒绝。
- webfetch/websearch 拒绝。这里不声称进程或其模型 API 网络被隔离。
- 默认 Mathscape 白名单复用 Codex 的固定只读目录；显式 tools_allow 替换该默认目录。
- `ApprovalHandler::Decline` 优先；绝不选择 allow_always。无法分类、无路径、缺少
  allow_once/reject_once 选项均拒绝或取消审批。
- 审批收集所有 filepath/filePath/path/parentDir 和 locations[].path；相对路径按 cwd
  解析，任何 `..` 直接拒绝。用标准库 canonicalize 解析真实路径；不存在的目标先解析
  最深已存在祖先，再附加后缀。根目录也解析，所以支持 macOS `/tmp` 一类别名。
  文件/目录 symlink、symlink 下新文件不能借名称前缀越出授权根；悬空 symlink 拒绝。

配置仅由 `OPENCODE_CONFIG_CONTENT` 注入，不在 workspace 写配置。全局/工作区已有的
自然语言 instructions 会累加，而非被替换；适配器指令存放在 state_dir 下。
所有 `mathscape_*` 配置为 ask，不注入逐工具 allow/deny；每次调用交由 Mathscape
审批层按启用工具精确匹配，避免依赖 OpenCode 通配规则的匹配顺序。

## 检查

`cargo test -p mathscape-agent` 使用 `mathscape-acp-fixture` 验证协议、进程生命周期、
模型、权限决定、真实临时文件结果、symlink 拒绝、历史吞掉及额度/取消分支。

手动真实检查（需要可用额度，不会因零调用而假通过）：

```sh
MATHSCAPE_OPENCODE_LIVE=1 MATHSCAPE_OPENCODE_MODEL=opencode/hy4-flash-free \
  cargo test -p mathscape-agent --test opencode_session opencode_live_native_policy -- --ignored --nocapture
```

真实检查会要求 scratch 写入实际成功，其他写入未发生、外部 sentinel 未泄露，并检查
所有负面工具动作确实尝试；额度不足、模型拒绝尝试或工具未请求审批均不是通过。

本次 `opencode/hy4-flash-free` 仅尝试一次：90 秒无进展后取消并以 failed 结束，
scratch 没有写入，真实检查失败（并非授权验证通过）。fixture 中的权限与磁盘检查
通过不代表本次真实模型完成了这些操作；仍需有可用额度时补做真实验证。
OpenCode 本地日志同一轮的 HY4 主模型记录 `AI_APICallError: Free usage limit reached`。
