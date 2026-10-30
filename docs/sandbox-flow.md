# Mathscape 沙箱 Flow 契约（探索床 / 可拆卸副作用）

> 读者：Mob（实现）· 起草：2026-10-30（Asia/Shanghai）· 状态：**契约草案，用户决策已锁定（2026-10-30），待用户审阅本文后合并**
> 依据：[Phase0 契约](phase0-contract.md)（**Phase0**）、[Phase2 写路径契约](phase2-write-contract.md)（**Phase2**）、用户 2026-10-30 对话锁定的「沙箱化 flow」方向。
> 标签：**[锁定]** = 用户已拍板；**[本文提议]** = 为闭合实现补的细节，可在不违背 [锁定] 的前提下微调；**[待核]** = 未定。
> 本 PR **只有文档**，不含 Rust / TS 实现。
>
> **命名**：本文「沙箱 Flow / Sandbox session」是 **Mathscape 产品概念**（可回滚的探索会话）。**不要**与 Codex CLI 的 `SandboxMode`（`read-only | workspace-write | …`，见 Phase0 附）混淆；下文凡指后者写 **Codex sandbox**。

---

## 1. 一句话 / 为什么现在做

**一句话**：[锁定] 在正式「定向调查 flow」（`user.ask` → `propose_changes` → Desktop 接受）钉死之前，先做一层 **可拆卸的沙箱对话体验**：用户用自己的**测试工作区 checkpoint** 开一场会话，本机 Codex + 现有 Mathscape MCP tools 自由对话；会话产生的聊天记录与 Agent 构造的语义副作用可以 **物理删除或整包导出**，从而反复从同一 checkpoint 进入，支撑后续迭代与 prompt / 壳体验探索。

动机（用户原话归纳）[锁定]：
1. 近几模块要建立在**个人体感**上，而不是只看 diff。
2. 先**淡化** workflow 名词（调查状态机、propose/accept 仪式），把壳当成「带 Mathscape 工具的对话面」。
3. 在真实 tools 能力下探索引用方式、面板交互、更合适的 prompt，再反哺 Phase2 定向 flow。
4. **可拆卸**是硬要求：副作用不能污染正式库，也不能在丢弃后残留含糊状态。

与 Phase2 的关系 [锁定]：
- Phase2 契约（已合入 main）仍是定向写路径的权威口径；**本文不修改、不削弱** Phase2 的锁定决策。
- 沙箱是 **先行探索床**；定向 flow（Phase2 §9 实现序）在沙箱摸清壳与 prompt 之后再接，顺序上 **S0/S1 可先于** `user.ask` / `propose_changes` 产品化。
- 沙箱 **允许** 暂不实现完整 propose→accept；试写若存在，必须落在隔离层内（§3），不得写用户选定的正式 / 源 checkpoint。

---

## 2. 范围与 Non-goals

### 2.1 In scope（本契约要定的）

| # | 项 | 说明 |
| --- | --- | --- |
| 1 | Checkpoint | 用户个人测试工作区上的可复现起点（§3.1） |
| 2 | Sandbox session | 一次探索会话的生命周期：打开 → 对话 → 丢弃 / 导出（§3.2–3.4） |
| 3 | 隔离与可拆卸 | 副作用存放位置、丢弃语义、导出包最小格式（§3.3–3.4） |
| 4 | 对话壳（淡化 workflow） | Desktop 最小 UI 与 Agent 挂载（§4） |
| 5 | 与 4b / Phase2 的依赖切分 | 实现切片 S0→S1→S2 与后置项（§5） |

### 2.2 Non-goals [锁定]

- 完整定向 flow UI（`user.ask` 待答仪式、propose 接受面板、retire 预览）——属 Phase2，不在沙箱必达。
- Agent 回答里的任意内嵌 JS / 不受限可执行 widget（富交互声明式块可后置）。
- 虚拟形象系统、多调查自动合并、文本流式 token delta 花活（Phase0 §7）。
- Sidecar 便携打包（Phase0 / Phase2 已推迟）。
- 在**源 checkpoint / 正式工作区**上做破坏性试写。
- 把沙箱会话伪装成 Phase2 `investigation` 的完整状态机（可共用底层 round/事件，但产品语义先是「对话会话」）。

---

## 3. 模型：Checkpoint · Session · 隔离 · 拆卸

### 3.1 Checkpoint [锁定语义 / 本文提议形状]

**Checkpoint** = 用户指定的、可反复回到的工作区起点。

- **来源** [锁定]：用户自己的**测试工作区**（内含可引用的真实对象），不是仓库夹具替代品；夹具仅可作 CI 回归补充。
- **不变式** [锁定]：开启沙箱会话**不得修改** checkpoint 目录（含其中的 `mathscape.sqlite3` 与 artifacts）。所有可变状态进 session 隔离层。
- **标识** [本文提议]：`checkpoint_id`（稳定字符串）+ `checkpoint_fingerprint`（对工作区根关键文件的内容指纹，至少覆盖 `mathscape.sqlite3` 的大小与 mtime 或内容哈希）。打开会话时记录二者；导出包原样带回。
- **打开方式** [本文提议]：Desktop「从 checkpoint 开沙箱」→ 选定已有工作区路径（或已登记的测试库别名）→ 创建 session。

### 3.2 Sandbox session [锁定]

一场会话绑定：一个 `sandbox_session_id`、一个 checkpoint、一个（副本）工作区根、零或多条 round（Codex / AgentExecutor）。

状态（最小）[本文提议]：

| 状态 | 含义 |
| --- | --- |
| `active` | 可对话；MCP `--workspace` 指向隔离工作区根 |
| `exported` | 已导出至少一次；session 仍可继续或随后丢弃 |
| `discarded` | 隔离层已物理删除；终态 |

不允许第三态「半删残留」。`discarded` 后同一 `sandbox_session_id` 不可复活；要继续探索 → 从同一 checkpoint **新开** session。

### 3.3 隔离层 [锁定目标 / 本文提议机制]

**目标** [锁定]：会话内 Agent / 用户通过 Mathscape 能力产生的**一切副作用**（聊天记录、round trace、若有试写对象 / 暂存）都必须落在可整块拆掉的隔离层；源 checkpoint 只读。

**v0 机制** [本文提议]：**会话级工作区副本**。

1. 打开 session 时，把 checkpoint 工作区根 **完整复制**（或等价快照）到 Desktop 管理的沙箱目录，例如：  
   `<app_data>/sandbox_sessions/<sandbox_session_id>/workspace/`  
   （具体根路径实现自定，契约只要求「独立目录 + 可整目录删除」。）
2. `mathscape mcp --workspace` 与 Desktop 打开的图，均指向该副本根。
3. Codex 侧继续沿用 Phase0：对 harness 文件系统的 **Codex sandbox** 仍为 `read-only`（防任意写宿主机）；Mathscape 工具读写的是**副本库**，不是源库。二者正交。
4. v0 **不要求** copy-on-write / 叠加 sqlite；若实现发现全量复制过大，可在不破坏「丢弃=物理删隔离层、源 checkpoint 只读」的前提下换成 COW，属实现优化，须在实现 PR 写明。

**试写策略（分阶段）** [锁定方向 + 本文提议分期]：

| 阶段 | Agent 写能力 | 说明 |
| --- | --- | --- |
| **S0/S1** | **默认只读 tools**（现有 7 个 MCP） | 探索引用、对话壳、事件面板；副本上亦无正式 propose/accept |
| **S1 可选** | 若需「构造语义节点」体感 | 仅允许写入**隔离副本**；仍**不**走 Phase2 正式接受入源图。具体 API 可先用 Desktop 已有 map_edit 的受限调试入口或后续 `propose` 的沙箱适配——实现 PR 选定一种并写进清单 |
| **定向 flow 接回后** | Phase2 `propose_changes` 等 | 沙箱可演进为「投影在副本上预览接受」，但源 checkpoint 仍只经显式「晋升 / 不在本文」路径；本文不定义晋升 |

### 3.4 拆卸：丢弃 vs 导出 [锁定]

两种互不含糊的出口：

#### 丢弃（默认安全出口）[锁定]

- 动作：物理删除该 session 的隔离目录（含副本工作区、transcript、trace、本地缓存）。
- 结果：源 checkpoint **字节级不变**；无「软删除残留」。
- UI：结束会话或「丢弃本次」；需一次确认（不可撤销）。

#### 导出（保留为样本）[锁定语义 / 本文提议包格式]

- 动作：打出一份自描述目录或归档，**不**代替丢弃；导出后 session 仍可继续，直到用户丢弃。
- **最小包内容** [本文提议]：

```
sandbox_export_<session_id>/
  manifest.json          # 见下
  transcript.jsonl       # 用户消息 + Agent 可见回复（按时间）
  events.jsonl           # 可选：AgentExecutor §3 事件原样（若已接通 4b）
  side_effects/          # 可选：本会话在副本上新建/改动的对象快照或 change 列表
```

`manifest.json` 最小字段 [本文提议]：

| 字段 | 含义 |
| --- | --- |
| `schema` | 固定 `"mathscape.sandbox_export.v0"` |
| `sandbox_session_id` | 会话 id |
| `checkpoint_id` | 起点 id |
| `checkpoint_fingerprint` | 打开时指纹 |
| `created_at` / `exported_at` | RFC3339 带时区 |
| `backend` | 如 `codex` |
| `tool_allowlist` | 本会话启用的 MCP 工具名列表 |
| `notes` | 可选，用户一句话 |

不要求导出可再导入为活会话（v0）；导出是**样本与对照**，不是迁移格式。若日后要「从导出恢复」，另开契约。

---

## 4. 对话壳（淡化 workflow）[锁定方向]

### 4.1 产品语义

沙箱壳 = **带 Mathscape 工具的对话面**，不是调查状态机面板。

- 不强调 `investigation` / `awaiting_user` / `change_set` 等 Phase2 名词（实现内部可复用 round_id）。
- 用户感知：选 checkpoint → 开聊 → 看工具调用与回复 → 丢弃或导出。

### 4.2 Desktop 最小 UI（S1）[本文提议]

| 元素 | v0 要求 |
| --- | --- |
| 入口 | 「沙箱」或「探索会话」：选 checkpoint、开 / 列 / 丢弃 / 导出 |
| Flow 面板 | 可折叠；建议贴边 dock + 可拖锚点的圆形入口（点=展开/收起）；快捷键展开 |
| 历史 | User / Agent / System 角色条（颜色+短名即可）；**不做**虚拟形象系统 |
| 事件 | 消费 `events(round_id)`：turn / tool_call·result / error / cancel（依赖 4b 或与 S1 同批的最小事件条） |
| 待答 / 提案条 | **不**作为 S1 必达；有 `user.ask` 后再加 |

### 4.3 Agent 挂载 [锁定]

- 本机 **Codex**（已有 `CodexExecutor`）+ `mathscape mcp --workspace <sandbox_copy> --round <round_id>`。
- 工具面 v0 = Phase1 已有 **7 个只读工具**；allowlist 可配置，默认全开只读集。
- `approvalPolicy: never`、Codex sandbox `read-only`、`ephemeral: true` 沿用 Phase0 附，除非实现 PR 证明必须调整（调整不得导致写入源 checkpoint）。

### 4.4 引用与视图信号（S2，薄）[锁定可后置 / 本文记方向]

- `@` / 点选 → 对象 chip（带 `object_id`）；候选来自 search / graph / 最近上下文。
- 工具命中在图上短高亮（读路径回声）；**不做**完整 staged 幽灵层（那是 Phase2 接受路径的事）。
- 富交互声明式块、沙箱 iframe widget：**后置**，本契约不展开。

---

## 5. 实现切片与依赖

### 5.1 建议顺序 [锁定方向]

| 切片 | 内容 | 依赖 | 用户体感 |
| --- | --- | --- | --- |
| **S0** | Checkpoint 登记/选择；副本隔离；丢弃；导出（manifest+transcript 最小集） | 无 | 同一 checkpoint 反复进；丢弃后源库不变 |
| **S1** | 对话壳 + 本机 Codex 挂上 7 tools；最小事件条 | S0；**4b** 事件消费（可与 S1 同批薄做） | 真实 tools 对话 |
| **S2** | `@` 引用 chip + 读路径图上高亮 | S1 | 引用顺手、图有回声 |
| **∥** | Phase1 硬化、Phase2-0 领域模型 | 与沙箱并行 | 多为非壳 |
| **后** | 定向 flow 接回（Phase2 §9） | 沙箱体感 + Phase2 模型 | 仪式化 ask/propose/accept |

### 5.2 与 Phase0 清单 4b

4b（真机 Codex + Desktop 消费事件）是 S1 事件条的前提。允许：
- **同 PR** 做「4b 最小事件条 + S1 壳」，或
- 先合极薄 4b，再合 S1。

不得在无事件流时假装「对话已接通」。

### 5.3 文档义务（每个实现 PR）[本文提议]

PR 描述固定三节（用户体感调试约定，保持简单）：
1. `用户能看见的变化`
2. `如何从 checkpoint 点一遍（≤8 步）`
3. `刻意没做`

不强制 Mock 回放基建、不强制 GIF；导出样本可选附在 issue。

---

## 6. 待核

| # | 问题 | 建议默认 | 为何可后定 |
| --- | --- | --- | --- |
| T1 | 副本策略：全量复制 vs COW | v0 全量复制；过大再 COW | 不改变只读 checkpoint 不变式 |
| T2 | 沙箱内是否首版就开放「副本上试写」 | S1 默认只读；试写另开子切片 | 先稳壳与 tools 体感 |
| T3 | 导出是否需要 `side_effects/` 必选 | v0 可选；有试写后再必选 | 只读阶段 transcript 已够 |
| T4 | checkpoint 是否支持「只读绑定远程/只读卷」 | 首版仅本地目录 | 降低 Desktop 复杂度 |

无项阻塞 S0 文档合并；实现 PR 选定 T1/T2 时在 PR 正文写明。

---

## 7. 本文提议汇总

| 项 | 落点 |
| --- | --- |
| 会话级工作区副本作隔离层 | §3.3 |
| 沙箱目录布局与 `discarded` 终态 | §3.2–3.3 |
| 导出包 `mathscape.sandbox_export.v0` | §3.4 |
| S1 UI：dock + 角色条，无虚拟形象 | §4.2 |
| S0→S1→S2 切片与 4b 同批许可 | §5 |
| 与 Codex `SandboxMode` 的命名区分 | 文首 |

---

## 8. 修订记录

| 日期 | 说明 |
| --- | --- |
| 2026-10-30 | 初稿。用户锁定沙箱探索床、可拆卸副作用、淡化 workflow；同意先写短契约再实现。 |
