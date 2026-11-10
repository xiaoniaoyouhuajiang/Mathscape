# Codex 会话持久化（harness 常驻 + 线程续接）

状态：实现中（`feat/codex-session-persistence`）。上游决策见 2026-11-10 与用户的讨论；实验记录见 PR 说明「实验结论」。

## 1. 背景

PR #47 之前，沙箱对话的每一轮都是：

1. 启动一个 `codex app-server` 和一个新的 `mathscape mcp --workspace <会话副本> --round <round_id>`；
2. `thread/start`（`ephemeral: true`）→ `turn/start` → 本轮结束后整组杀掉进程；
3. 历史只能靠 `build_dialogue_prompt` 把 transcript 压成「[用户] …／[助手] …」纯文本塞进本轮用户消息。

后果：

- 上一轮的工具调用和结果不进入下一轮上下文，Agent 每轮都要重新读 skill、重新查工具；
- developer instructions 开头是 `This is Mathscape round {round_id}`，每轮都变，前缀缓存只能命中 Codex 自己的系统提示；
- 历史文本有字符预算，超出后从最早的消息丢，历史段开头每轮都在变；
- 沙箱会话放在 `~/Library/Application Support/com.mathscape.desktop/sandbox/`，移动工作区目录时带不走。

## 2. 方案

### 2.1 执行接口（ACP 形状，harness 无关）

`AgentExecutor` 改为「连接 → 线程 → 轮次」三层，对应 ACP 的 `initialize` / `session/new`·`session/load` / `session/prompt`·`session/cancel` / `session/request_permission`：

| 操作 | 含义 | ACP 对应 |
| --- | --- | --- |
| `open_session(SessionSpec)` | 声明一个 harness 会话（一个沙箱会话 = 一个 harness 进程）；工具白名单、skill、授权在这里固定 | `initialize` |
| `new_thread(session, NewThread)` | 新开一条 harness 线程（一条「对话」） | `session/new` |
| `load_thread(session, LoadThread)` | 续接已有线程：进程内已加载 → 直接用；否则按 id 恢复；必要时先把工作区里的 rollout 副本放回 harness 目录 | `session/load` |
| `prompt(session, Prompt)` | 在线程上跑一轮，返回 `round_id`，事件流仍是 `agent_events v0` | `session/prompt` |
| `cancel(round_id)` | 中断本轮 | `session/cancel` |
| `ApprovalHandler` | 审批回调；默认实现就是现有的只读白名单策略 | `session/request_permission` |
| `close_session(session)` / `close_all()` | 关进程 | — |

旧的 `start_round` 保留为一次性便捷方法（内部 = 临时会话 + 新线程 + 一轮，结束即关），测试和以后的非沙箱入口还能用。

Codex 是目前唯一实现。ACP 实现留到真正接第二个 harness 时再做（届时只是多一个实现，接口不动）。

### 2.2 进程模型

- 每个沙箱会话一个常驻 `codex app-server`，同一进程里可以有多个线程（每条「对话」一个）。`mathscape mcp` 跟着这个 app-server 活。
- 生命周期复用 `mathscape-resident` 的 `Supervisor`：懒启动、握手超时、退避重启、空闲回收（15 分钟，与 Loogle 一致）、整组杀进程。
- 关闭时机：空闲回收、会话丢弃、切换到别的沙箱会话、App 退出。进程身份（key）包含工作区路径、工具白名单、skill sha256、`tool_schema_version`、MCP 程序路径和授权；任何一项变化都会关掉旧进程再起新的（`key_changed`），已有线程下次用时按 id 恢复。
- `--round` 不能再作为 MCP 启动参数（进程跨轮存活）。改为 `mathscape mcp --round-file <session>/current_round`：适配器在每轮 `turn/start` 前写入本轮 round id，MCP 每次工具调用时读取。同一会话一次只跑一轮（Desktop 已保证），所以没有竞争。

### 2.3 线程持久化与可迁移

- `thread/start` 改为 `ephemeral: false`，带 `serviceName: "mathscape"` 与 `threadSource: "mathscape"`，`clientInfo.name` 仍是 `mathscape`（rollout 的 `originator` 也是它）。
- **不替换 `CODEX_HOME`**：继续用用户的 `~/.codex`，登录、配置、magpie 路由都不变，也不改 `config.toml`。
- **工作区里的正本**：每轮结束后，把该线程的 rollout（`thread.path`，取不到就在 `$CODEX_HOME/sessions` 下按 thread id 找）复制到 `<会话目录>/harness/codex/<原文件名>`。
- **恢复顺序**：
  1. 进程里已加载 → 直接 `turn/start`；
  2. `thread/resume { threadId, cwd }`（总是带 cwd）；
  3. 失败且工作区有副本 → 复制到 `$CODEX_HOME/sessions/mathscape/<原文件名>`（只在目标不存在时写），再按 id 恢复；
  4. 仍失败 → 新开线程，用 Mathscape 自己的 transcript 重建（`build_dialogue_prompt`），transcript 里写一条 System 行「上下文已降级：…」。
- **会话目录挪进工作区**：沙箱根从 `<app_data>/sandbox` 改为 `<母工作区>/.mathscape/sandbox`（`MATHSCAPE_SANDBOX_ROOT` 仍优先）。会话副本复制 checkpoint 时跳过沙箱根本身，避免套娃。启动时把旧位置里属于当前母工作区的会话**复制**过来（不删原件，已存在的同名会话不覆盖），迁移记录写在新根的 `migrations.jsonl`。
- **路径漂移**：每条线程记录绑定时的工作区副本路径。恢复时若路径变了（工作区被移动），本轮开头注入「工作区已迁移到 X，旧路径 Y 作废」。

### 2.4 稳定前缀

- `developerInstructions` 只放每条线程都不变的内容：工具白名单（会话级固定）、凭据说明、scratch 路径、skill 路径与「首轮先读 SKILL.md」。去掉 `round {round_id}`。
- 每轮会变的内容（round id、线程续接提示、工作区变化摘要、skill/工具更新提示）放进本轮 `turn/start.input` 的**第一个文本项** `<mathscape_turn_context>…</mathscape_turn_context>`，用户消息是第二个文本项。它们只追加在上下文末尾，不破坏已缓存的前缀。

### 2.5 版本比对

每条线程在 `session.json` 里记录 `tool_schema_version`、skill sha256 与 skill 版本。续接时与当前值比对：不同 ⇒ 进程 key 已变（MCP 进程会重启），并在本轮开头注入「工具/skill 已更新到 X（原 Y），请重读变化部分」，之后更新记录。

### 2.6 工作区变化摘要

非首轮时，Desktop 统计「自上轮结束以来」本 investigation 的提案变化（新接受 / 新丢弃 / 新暂存或被替换的 change set，按 `updated_at` 与上一轮结束时刻比较），写成一段简短摘要注入本轮开头，并提醒 Agent：上下文里的旧工具结果可能已过时，以本轮重新调用工具的结果为准。没有变化时只保留提醒，不列清单。

## 3. 存储布局（新）

```text
<母工作区>/.mathscape/sandbox/
  checkpoints.json  discarded.jsonl  migrations.jsonl
  sessions/<sandbox_session_id>/
    session.json        # 新增 dialogue_threads[*].harness（harness 线程绑定，见下）
    transcript.jsonl  events.jsonl  harness.jsonl
    current_round       # 本轮 round id（MCP --round-file 读取）
    harness/codex/rollout-<ts>-<thread_id>.jsonl   # rollout 副本（正本是 Mathscape 自己的 transcript）
    scratch/  workspace/
  exports/
```

`dialogue_threads[*].harness`：

```json
{ "backend": "codex", "thread_id": "<uuid>", "rollout_file": "rollout-….jsonl",
  "workspace": "/abs/path/at/bind/time", "tool_schema_version": "mathscape-mcp/16",
  "skill_version": "0.6.5", "skill_sha256": "…", "last_turn_at": "RFC3339" }
```

## 4. 取舍

- **不用 `turn/start.additionalContext`**：codex 0.166 要求 `initialize.capabilities.experimentalApi = true` 才接受它（实测报 `turn/start.additionalContext requires experimentalApi capability`）。开实验 API 会把整个连接带进不稳定接口，所以改用稳定的多文本项 `input`，效果上同样是追加在末尾。
- **不用 `thread/resume.path`**：默认的 paginated 线程只认 `CODEX_HOME/sessions/` 下的 rollout，`path` 指向别处会报 `no rollout found`，同目录的旧路径会报 stale path；`path` 与 legacy 格式都标为 UNSTABLE。
- **rollout 只是缓存**：版本不兼容、文件缺失、路径不符时退回 transcript 重建，不阻塞对话。
- **不换 `CODEX_HOME`**：换了就找不到 `~/.codex` 的登录与配置。代价是 Mathscape 的线程会出现在用户的 Codex 历史里（来源标为 mathscape）。

## 5. 不在本 PR

- ACP 适配器与第二个 harness；
- 导出包携带 rollout 副本；
- 跨机器迁移时 Lean profile 根等外部路径的重新发现（沿用现有发现逻辑）。
