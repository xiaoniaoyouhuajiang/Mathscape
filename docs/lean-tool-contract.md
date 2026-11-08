# Mathscape Lean 工具契约 v0

> 状态：冻结，可直接用于实现下一切片。  
> 日期：2026-11-08（Asia/Shanghai）  
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
  "line": 3,
  "column": 12,
  "end_line": 3,
  "end_column": 20
}
```

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

