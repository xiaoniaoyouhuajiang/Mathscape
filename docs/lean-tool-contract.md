# Mathscape Lean 工具契约 v0

> 状态：冻结，可直接用于实现下一切片。  
> 日期：2026-11-08（Asia/Shanghai）；2026-11-10 增补 §3.4 常驻模式  
> 依赖：[Phase0 契约](phase0-contract.md) 的 `mcp_envelope v0` 与只读工具约定。  
> 范围：受管 Lean/mathlib profile、`mathlib.search`、`lean.check` 和验证回执。

本文是 Lean 工具对实现、MCP 适配器和 workflow 的单一语义来源。实现可以增加内部细节，但不得改变本文锁定的输入含义、可信边界和 profile 绑定方式。

## 1. 目标与边界

本切片要先打通一条可复现的只读链路：

```text
Mathscape 内容/手稿证据
  → mathlib.search
  → Agent 生成完整 Lean 源文件
  → lean.check
  → 不可变验证回执
```

本切片不实现：

- 自动更新或静默切换 Lean/toolchain/mathlib；
- 云端 Loogle、LeanExplore 或其他外部搜索作为一致性来源；
- 任意用户路径、任意 Lake 参数或 shell 命令执行；
- OCR、自然语言命题到 Lean 命题的自动忠实性证明；
- 自动创建或接受 `FormalArtifact`、语义节点或来源关系；
- 把 Lean 通过结果直接提升为 `accepted`、`canonical` 或用户已理解。

`lean.check` 证明的是提交给 Lean 的精确源文件。自然语言、Markdown 或手稿与该源文件是否忠实对应，仍是独立的对齐问题。

## 2. Profile 身份

所有搜索、导出、检查和回执都必须绑定一个受管 profile。profile 由 Mathscape 管理，当前开发 profile 为 Lean 4.33.1 + mathlib v4.33.1。

公共返回中的 profile 引用采用以下形状：

```json
{
  "id": "lean4-4.33.1-mathlib-4.33.1",
  "fingerprint": "sha256:…",
  "toolchain": "leanprover/lean4:v4.33.1",
  "mathlib_revision": "v4.33.1"
}
```

规则：

1. `fingerprint` 是实际使用的环境指纹，不是 Agent 自报的字符串。
2. profile 不存在、未就绪、缺少 `Mathlib.olean` 或 profile 元数据校验失败时，工具返回 `capability_unavailable`。
3. 调用方可以带 `profile_fingerprint` 作为期望值；与实际值不一致时返回 `invalid_arg`，不得静默改用其他 profile。
4. 工具不得修改用户 Lean 项目。检查用的临时源文件只能写入 Mathscape 管理的 profile cache/scratch 目录。
5. profile 的升级必须创建新的 profile id；旧 profile 和旧回执继续可读。v0 不提供自动升级。

profile 的现有字段、指纹算法和就绪状态以 `mathscape-tool-lean` 的 `ManagedLeanProfile` 为准；本文不重新定义其内部目录结构。

### 2.1 Profile 根目录发现与转发

profile 根目录按以下顺序解析（`mathscape_tool_lean::discover_profile_root`，只读、不创建目录）：

1. 环境变量 `MATHSCAPE_LEAN_PROFILE_ROOT`（显式设置时即使目录不存在也以它为准，便于报错可见）；
2. 工作区 `.mathscape/lean/profiles`；
3. 工作区上级目录中的 `.mathscape/lean/profiles`；
4. 当前目录 `.mathscape/lean/profiles`；
5. 平台数据目录（`LeanProfileStore::from_env_or_default` 的默认位置）。

Desktop 发起沙箱 round 时，用源 checkpoint 解析出的根目录设置 `MATHSCAPE_LEAN_PROFILE_ROOT`，经 Codex app-server 转发给 `mathscape mcp` 子进程（`MCP_FORWARDED_ENV_VARS`），因此副本工作区里也能找到同一套 profile；该目录同时以只读方式授予 harness。来源层「Lean Profile · 环境 / 外部库」卡片只读 `profile.json` 的记录状态，不重新构建或升级 profile；`ready` 且指纹 `verified` 才显示为可用。Lean 编译通过不代表证明忠实或完整。

## 3. `mathlib.search`

### 3.1 语义

`mathlib.search` 是绑定到本地 profile 的结构化声明检索。v0 使用 profile 内的 Loogle 和其 managed index；搜索结果是候选，不是证明，也不是可信声明。

云端搜索或自然语言模型可以在更高层把用户问题转成 Loogle 查询，但最终候选必须回到本地 profile 解析和 `lean.check` 验证。

### 3.2 输入

```json
{
  "profile_id": "lean4-4.33.1-mathlib-4.33.1",
  "profile_fingerprint": "sha256:…",
  "query": "add_comm",
  "module": "Mathlib",
  "max_results": 50
}
```

字段规则：

| 字段 | 必需 | 约束 |
| --- | --- | --- |
| `profile_id` | 是 | 已登记的 profile id |
| `profile_fingerprint` | 否 | 存在时必须精确匹配 |
| `query` | 是 | 非空、无控制字符，最多 4096 个 Unicode 字符 |
| `module` | 否 | 默认 `Mathlib`；必须是 Lean 模块名 |
| `max_results` | 否 | 默认 50，范围 1..100 |

MCP 输入不接受 `loogle_executable`、`index_file`、shell 参数或任意路径。index 由 profile 解析；调用方不能把搜索重定向到 profile 之外。

### 3.3 输出

成功结果沿用 Phase0 的正常 MCP result，并在结构化内容中返回：

```json
{
  "status": "ok",
  "profile": { "id": "…", "fingerprint": "sha256:…" },
  "backend": "loogle",
  "backend_fingerprint": "sha256:…",
  "index_fingerprint": "sha256:…",
  "backend_mode": "resident",
  "total": 77,
  "truncated": true,
  "candidates": [
    {
      "schema_version": 1,
      "profile_fingerprint": "sha256:…",
      "name": "Nat.add_comm",
      "kind": "unknown",
      "module": "Init.Data.Nat.Basic",
      "type": "∀ (n m : ℕ), n + m = m + n",
      "metadata_complete": false
    }
  ]
}
```

`candidates` 使用现有 `DeclarationCandidate` schema。候选的 `metadata_complete=false` 不得被解释为缺少定理，只表示本次检索没有完成完整声明元数据导出。

特殊情况：

- 没有命中：`status=ok`、`candidates=[]`，并附 `no_matches` 诊断；
- index 缺失或损坏：`status=capability_unavailable`；
- 结果超过 `max_results`：`status=ok`、`truncated=true`；
- 参数非法：MCP 调用错误 `invalid_arg`；
- Loogle 运行失败或输出无法解析：调用错误 `internal`，不得伪造空结果。
- 常驻 Loogle 启动失败（例如索引缺失或与当前 Loogle 不匹配）：`status=capability_unavailable`，诊断码 `loogle_resident_unavailable`；不会重建索引；
- 常驻 Loogle 单条查询超时：调用错误 `internal`，`details.reason=timeout`；进程已被整组杀掉，下一次调用会重新启动。

`backend_mode` 是 2026-11-10 新增的说明字段：`resident` 表示由 Desktop 持有的常驻 Loogle 回答，`oneshot` 表示本次单独启动了一次 Loogle。一次性回答前若尝试过常驻路径，会附 `backend_fallback` 说明原因（开关关闭、已降级、连不上 Desktop 等）。两种后端的 `total`、`truncated`、`candidates` 语义完全相同；该字段不改变输入 schema，因此 `tool_schema_version` 不变。Loogle 无命中时返回的 `count: 0` 加 `suggestions` 按「没有命中」处理（此前会被误判为 JSON 非法）。

### 3.4 常驻模式（2026-11-10）

一次性调用每次都要导入 Mathlib 并加载索引。用户 Mac 上实测（24 GiB 内存，profile `lean4-4.33.1-mathlib-4.33.1`，索引 376MB）：一次性调用每次约 11 秒、峰值约 5.3GB；`loogle --json --interactive` 冷启动 36 秒、热缓存约 11 秒，之后多数查询 0.1–0.2 秒，按名字精确查约 1 毫秒，复杂模式查询约 2.3 秒，常驻约 5.5GB。

**进程放在哪里。** `mathlib.search` 在 `mathscape mcp` 进程里执行，而这个进程每一轮由 Codex app-server 重新拉起、随轮次结束被整组杀掉。所以常驻 Loogle 由 Desktop 进程持有：Desktop 启动时在临时目录下建一个 0700 子目录并监听其中的 Unix socket（`mathscape-<pid>/loogle.sock`），把路径作为 `MATHSCAPE_LOOGLE_SOCKET` 放进每轮的 harness 环境，经 `env_vars` 转发给 MCP 子进程。MCP 每次调用发一行 JSON 请求、读一行 JSON 响应；没有这个变量（例如单独运行 CLI）、连不上、或 Desktop 回答「已关闭/已降级」时，走原来的一次性路径。只有当前用户能访问该 socket；Desktop 仍会重新校验请求里的 profile 路径，并要求索引位于受管 profile 缓存目录内。

**实例规则。**

| 项目 | 规则 |
| --- | --- |
| 实例数 | 整个 App 一个。身份 =（profile 指纹、module、索引文件、Loogle 可执行文件 SHA-256）；身份变化即关闭旧进程、启动新进程 |
| 启动 | 懒启动，第一次查询时才启动，不预热；等到 stdout 出现 `Loogle is ready.` 才算就绪 |
| 环境 | 直接 exec Loogle，不经过 `lake env`（省掉约 0.6GB 的 lake 进程）；`LEAN_PATH` 由一次 `lake env printenv LEAN_PATH` 取得，按 profile 指纹缓存。该命令只读，不构建、不更新用户的 Lean 依赖 |
| 索引 | 固定 `--index-mode read`；缺失或过期时报 `loogle_resident_unavailable`，从不重建 |
| 条数 | 启动时固定 `--max-results 200`，每次按请求的 `max_results` 截断；`total` 仍是 Loogle 的 `count` |
| 查询 | 一行一条；含换行或回车的查询按 `invalid_arg` 拒绝（MCP 输入层本来就拒绝控制字符） |
| 并发 | 串行：Desktop 内一把锁就是队列 |
| 超时 | 单条查询 30 秒，启动 90 秒；超时整组杀进程，本条返回超时错误，下次请求重启 |
| 失败 | 退避重启（0.5 秒起翻倍，封顶 10 秒）；连续 3 次失败（启动失败、超时、崩溃、输出错乱）后，本次 App 运行期间降级为一次性调用。被拒绝的查询不算失败 |
| 回收 | 空闲 15 分钟关闭进程，下次查询重新启动 |
| 清理 | 子进程自成进程组，关闭、超时和 App 退出时整组结束；Desktop 退出时同时删除 socket |
| 指纹 | Loogle 可执行文件与索引的指纹按（路径、大小、修改时间、inode）缓存，常驻路径不再每次重新哈希索引 |

**开关。** 工作区设置「Loogle 常驻模式」，默认打开；关闭后立即停掉常驻进程，后续调用回到一次性路径。关闭再打开会清掉降级状态。环境变量 `MATHSCAPE_LOOGLE_RESIDENT=0`（或 `false`/`off`/`no`）强制关闭，此时设置界面里的开关不可用。

**日志。** 生命周期事件（`spawn`、`ready`、`startup_failed`、`request_failed`、`restart`、`exit`、`idle_reclaim`、`key_changed`、`fallback`、`disabled`、`enabled`、`shutdown`）追加到 App 日志目录下的 `resident-processes.jsonl`。每轮的 `harness.jsonl` 记录本轮拿到的 socket 路径（`loogle_resident_socket`）。

生命周期部分在独立 crate `mathscape-resident` 中实现（启动、握手、状态机、退避、降级、空闲回收、整组清理、事件），请求编帧由各实例自己实现；之后的 Codex app-server 常驻会复用它。

## 4. `lean.check`

### 4.1 语义

`lean.check` 检查调用方提交的一个完整 Lean 源文件。源文件自身必须包含所需的 `import`、声明和证明；工具不隐式拼接 import，也不从用户项目读取同名文件。

检查过程使用指定 profile 的 Lean/Lake 环境，写入隔离 scratch 文件，执行后删除临时文件。runner 的超时、取消和输出上限沿用 `mathscape-tool-lean` 现有策略。

### 4.2 输入

```json
{
  "profile_id": "lean4-4.33.1-mathlib-4.33.1",
  "profile_fingerprint": "sha256:…",
  "source_text": "import Mathlib\n\nexample : … := by …\n",
  "evidence_refs": [
    {
      "node_id": "mathscape:source-unit/…",
      "artifact_id": "mathscape:artifact/…",
      "locator": { "type": "md_byte_range", "start": 120, "end": 240 }
    }
  ],
  "timeout_ms": 300000,
  "max_output_bytes": 1048576
}
```

字段规则：

- `source_text` 必需，非空，按 UTF-8 计算最多 1 MiB；它是要检查的完整文件，不接受路径或 URI；
- `evidence_refs` 可选，只用于 provenance 绑定，检查器不替用户读取或解释这些证据；
- `timeout_ms` 默认 300000，范围 1000..900000；
- `max_output_bytes` 默认 1048576，范围 32768..16777216；
- 输入对象拒绝未知字段；不得携带 shell、环境变量、Lake 选项或可执行文件路径。

### 4.3 检查结果

顶层 `status` 仍使用 Phase0 MCP 信封。检查是否通过放在 `check.status` 中：

`mathscape-tool-lean` 库层的 `LeanCheckResult` 直接暴露这些检查字段；MCP 适配器再把库层结果嵌入 `check`，并补上 Phase0 的顶层 `status`。

```json
{
  "status": "ok",
  "check": {
    "status": "checked",
    "profile": { "id": "…", "fingerprint": "sha256:…" },
    "source_sha256": "sha256:…",
    "diagnostics": [],
    "exit_code": 0,
    "elapsed_ms": 842
  },
  "receipt": { "receipt_id": "mathscape:lean-receipt/…" }
}
```

`check.status` 的 v0 值：

| 值 | 含义 |
| --- | --- |
| `checked` | Lean 进程成功结束，源文件通过解析、elaboration 和 kernel check |
| `failed` | Lean 正常运行，但源文件存在错误诊断 |
| `timed_out` | 超过检查期限并终止进程 |
| `cancelled` | 收到用户或系统取消 |

其中 `failed` 是一次有效检查的结果，不是 MCP 调用错误。profile 不可用返回顶层 `capability_unavailable`；参数非法返回 `invalid_arg`；runner 无法启动、输出格式损坏或内部持久化失败返回 `internal`。

诊断项至少包含：

```json
{
  "severity": "error",
  "message": "…",
  "code": "lean.unknownIdentifier",
  "line": 3,
  "column": 12,
  "end_line": 3,
  "end_column": 20
}
```

`code` 可选，来自 Lean 4.2x 起的 `error(lean.xxx):` 形式；旧形式 `error:` 不带 `code`。Lean 的诊断写在 stdout，解析同时读取 stdout 和 stderr；进程失败但没有可解析的诊断时，把两路输出合并成一条无位置的 error 诊断，不会返回空诊断。

行列号以提交的 `source_text` 为准。诊断超过输出上限时保留已捕获内容，并标记 `diagnostics_truncated=true`；不得把截断后的结果报告为 `checked`。

## 5. 验证回执

### 5.1 定位

回执是一次 Lean 检查的不可变事实记录，不是用户审批记录，也不是完整的形式化对象。它只回答：某个确定的源文件在某个确定的 profile 中得到了什么检查结果。

### 5.2 最小形状

```json
{
  "schema_version": "lean_check_receipt v0",
  "receipt_id": "mathscape:lean-receipt/…",
  "request_digest": "sha256:…",
  "source_artifact_id": "mathscape:artifact/…",
  "source_sha256": "sha256:…",
  "profile": {
    "id": "lean4-4.33.1-mathlib-4.33.1",
    "fingerprint": "sha256:…",
    "toolchain": "leanprover/lean4:v4.33.1",
    "mathlib_revision": "v4.33.1"
  },
  "check_status": "checked",
  "diagnostics": [],
  "runner": {
    "backend": "lean",
    "runner_schema": "lean_runner v0"
  },
  "evidence_refs": [],
  "created_at": "2026-11-08T12:00:00+08:00"
}
```

规则：

1. `source_artifact_id` 使用 Mathscape 的 `mathscape:artifact/sha256/<digest>` stable id，并必须指向由 Mathscape 管理的不可变源文件 artifact；当前工具切片先把源字节保存在 profile-local receipt store，后续接入 ArtifactPool 时沿用同一 id。
2. `request_digest` 由契约版本、完整源文件 hash、profile fingerprint 和有效 runner 选项计算；它用于比较重复请求，`receipt_id` 仍是一次具体回执的身份。
3. `checked` 只表示 Lean 检查通过。回执不得自动写入 `TrustRecord` 的 accepted/canonical 状态。
4. 旧 profile、旧 source artifact 和旧回执不得被升级流程覆盖；新环境产生新回执。
5. 回执写入失败时，工具返回 `internal`，不得只返回一个看似成功的 Lean 结果。

## 6. MCP 注册与事件

在现有 `tools/list` 固定顺序之后追加：

1. `mathlib.search`
2. `lean.check`

两者都是只读工具：`readOnlyHint=true`、`destructiveHint=false`、`openWorldHint=false`。内部 scratch、index 读取和回执 artifact 写入属于受控工具实现细节，不向 Agent 暴露文件系统写入口。

每次调用继续按 Phase0 规则产生 `tool_call` / `tool_result` 事件。`tool_result.status` 使用顶层 MCP `status` 或 `error`；具体的 `check.status` 作为结果 payload 保存。

## 7. 版本与验收门槛

本文版本为 `lean_tool_contract v0`。以下变化必须升级版本并同时更新 Rust DTO、MCP 描述和回执 schema：

- 改变 profile fingerprint 的含义；
- 改变 `source_text` 是否为完整文件；
- 增加或删除 `check.status` 的语义；
- 改变 `DeclarationCandidate` 的字段含义；
- 把搜索结果或回执从本地 profile 改为云端来源。

实现完成前必须通过：

- `Nat.add_comm` 和 `Real.continuous_sin` 的真实 profile 检查；
- 缺 import、未知声明、错误证明、超时和取消；
- `mathlib.search` 的命中、无命中、截断和缺失 index；
- profile fingerprint 不匹配拒绝；
- 同一 source/profile 的 `request_digest` 稳定；
- 重启后仍能读取回执，且用户 Lean 项目字节不发生变化；
- MCP tools/list 顺序、错误信封和工具调用事件回归。

完整 workflow fixture 的验收链路是：选中 Markdown 或手稿证据 → 搜索候选 → 生成完整 Lean 源文件 → 检查 → 保存回执 → 在 Agent/UI 中展示 profile、诊断和证据引用。

