# Mathscape 文件产物暂存契约 v0

> 状态：v0 冻结契约；`mathscape-mcp` 已提供受限 `artifact.stage` 实现，正式提升与读取工具仍待后续切片。
> 日期：2026-11-09（Asia/Shanghai）
> 依赖：Phase0 契约、Phase2 写路径契约和 Lean 工具契约。
> 范围：Agent 生成的 Markdown、Lean 源文件和其他受控文本文件的暂存、验证绑定和后续提升。

本文定义文件内容如何进入 Mathscape 的暂存区。暂存文件不是正式 Artifact，不进入 accepted 景观图，也不改变地图 head。正式 Artifact、FormalArtifact 和来源关系仍必须经过用户接受和现有提交管线。

## 1. 目标与兼容性决策

artifact.stage 的逻辑链路是：

    Agent 生成文件内容
      → 保存不可变暂存字节
      → 可选运行专用验证器
      → 等待用户决定
      → 被 propose_changes 引用
      → 用户接受后提升为正式 Artifact

本文明确区分三个写面：

| 写面 | 负责什么 | 是否改变正式图 |
| --- | --- | --- |
| artifact.stage | 保存文件字节、元数据和验证回执引用 | 否 |
| propose_changes | 提出对象、关系和文件提升/关联计划 | 否，只保存 change_set |
| Desktop 接受 / promote | 将暂存内容提升为正式 Artifact，并提交正式关系 | 是 |

现有 Phase2 契约把 propose_changes 定义为 Agent 唯一的正式提案写入口。artifact.stage 是已记录的受限例外：它只写隔离暂存区，不写地图、不登记正式 Artifact，也不改变任何接受状态。如果未来撤销这个例外，应把 stage 请求收回为 propose_changes 的内部子步骤，而不继续单独暴露 MCP 工具。

## 2. v0 支持的文件类型

v0 只接受 Agent 直接提交的 UTF-8 文本，不接受路径、URI、shell 命令或二进制 base64。

| media_type | 用途 | 默认验证 |
| --- | --- | --- |
| text/markdown | 自然语言证明、解释、推导、学习笔记 | UTF-8 和大小限制 |
| text/x-lean | Lean 源文件 | UTF-8 和大小限制；可绑定 lean.check 回执 |
| text/plain | 其他纯文本推导或中间结果 | UTF-8 和大小限制 |

后续可以增加 JSON、图片或压缩包，但必须另立大小、媒体类型和安全规则。v0 不支持 Lean 项目目录或多文件 bundle；多文件项目需要多个 staged artifact，并在后续契约中增加 manifest。

media_type 是文件事实，不等于数学可信状态。text/x-lean 不表示已经通过 Lean，text/markdown 也不表示内容已经被审核。

## 3. artifact.stage 输入和输出

输入字段：

    {
      "request_id": "uuid",
      "media_type": "text/markdown",
      "name": "proof-idea.md",
      "content": "这里是 Agent 生成的证明说明……",
      "evidence_refs": [
        {
          "node_id": "mathscape:source-unit/…",
          "artifact_id": "mathscape:artifact/sha256/…",
          "locator": { "md_byte_range": { "start": 120, "end": 360 } }
        }
      ],
      "validation_refs": [
        {
          "validator": "lean.check",
          "receipt_id": "mathscape:lean-receipt/…",
          "profile_id": "mathlib-2026"
        }
      ],
      "supersedes": {
        "staged_artifact_id": "mathscape:staged-artifact/…",
        "content_hash": "sha256:…"
      }
    }

字段规则：

| 字段 | 必需 | 约束 |
| --- | --- | --- |
| request_id | 是 | 调用幂等键；非空，最长 128 字符 |
| media_type | 是 | 只能是 v0 白名单值 |
| name | 是 | 逻辑文件名，1..200 个 Unicode 字符 |
| content | 是 | UTF-8 文本，最多 1 MiB |
| evidence_refs | 否 | 仅引用已存在的来源/手稿对象，不读取路径 |
| validation_refs | 否 | 只接受可由服务端重新加载并校验的回执 id；`lean.check` 在 v0 还需要 `profile_id`，且只适用于 `text/x-lean` |
| supersedes | 否 | 替换同一暂存意图的旧版本；必须带旧 hash |

name 只用于展示和提升时的默认文件名，不能包含路径分隔符、..、控制字符或绝对路径。服务端不得根据它拼接任意文件路径。

Agent 不得提交 staged_artifact_id、正式 Artifact id、revision、accepted、canonical、kernel_verified 或其他可信终态字段。所有身份和状态由服务端生成。

为兼容 Phase0 MCP 信封，v0 的顶层业务状态仍为 ok；暂存状态放在 stage.status：

    {
      "status": "ok",
      "stage": {
        "status": "staged",
        "staged_artifact_id": "mathscape:staged-artifact/…",
        "request_id": "uuid",
        "content_hash": "sha256:…",
        "media_type": "text/markdown",
        "name": "proof-idea.md",
        "byte_size": 2048,
        "visibility": "investigation",
        "validation": [],
        "evidence_refs": [],
        "created_at": "2026-11-09T…Z"
      }
    }

stage.status 的 v0 值：

| 值 | 含义 |
| --- | --- |
| staged | 字节和元数据通过校验，可以被 propose_changes 引用 |
| blocked | 文件字节已保留，但输入或验证产生阻塞诊断，不能被正式接受 |
| superseded | 被同一暂存意图的新版本替代 |
| discarded | 用户或 Desktop 丢弃，内容仍保留审计记录 |
| promoted | 已由用户接受并提升为正式 Artifact |

blocked 不是静默失败。服务端仍保存源字节、hash、诊断和 request_id。

## 4. 幂等、身份和 provenance

- 相同 Workspace、相同 request_id、相同规范化输入：返回同一结果；
- 相同 request_id、内容或媒体类型不同：调用错误 invalid_arg，details.reason = idempotency_conflict；
- 不同 request_id 但内容 hash 相同：底层字节可以去重，暂存记录仍保留各自 provenance 和可见性；
- supersedes 必须指向当前可替换的 staged/blocked 版本，旧版本转为 superseded；
- 旧版本字节和诊断不覆盖、不删除。

服务端从已绑定的 round / investigation 注入 proposed_by、round_id、generation、visibility、created_at 和 staged_artifact_id。Agent 不可伪造这些字段。

暂存对象和正式 Artifact 使用不同命名空间：

- 暂存：mathscape:staged-artifact/<opaque-id>
- 正式内容：mathscape:artifact/sha256/<digest>

即使暂存内容的 hash 已知，也不能把 hash 直接当成正式 Artifact 引用。

## 5. 验证回执绑定

暂存和验证是两个独立事实：

    artifact.stage
      → 文件字节和 staged artifact

    lean.check
      → 在 profile 上检查精确源文本
      → LeanCheckReceipt

如果输入带 validation_refs，服务端必须重新加载回执并验证：

1. 回执存在且完整性校验通过；
2. 回执的 source_sha256 等于本次 staged content hash；
3. 回执的 profile fingerprint 与当前请求声明一致；
4. validator 名称与媒体类型相容；
5. Agent 没有自行声明 checked 或 kernel_verified。

验证绑定只表示“这份字节曾经得到该验证结果”。它不表示自然语言证明与 Lean 命题已经完成忠实对齐，也不自动提升 TrustRecord。

## 6. 存储边界

暂存内容写入 Workspace 的受管 staging store，建议使用：

    <workspace>/staging/agent-artifacts/
      objects/sha256/…
      records/…

字节按 hash 不可变保存；v0 记录以 JSON 文件保存名称、媒体类型、provenance、可见性、验证摘要和生命周期。若后续增加 SQLite 索引，记录与文件对象仍必须遵循现有 ArtifactPool 的“字节先验证、记录不指向缺失内容”原则。

artifact.stage 不调用正式 ArtifactPool::import_bytes，因此不会：

- 写入 Artifact Catalog；
- 出现在 workspace.search；
- 出现在 graph.get / graph.neighborhood；
- 创建 SourceArtifact revision；
- 改变地图 head。

用户显式接受后，Desktop 的 promote 流程才可以把 staged bytes 通过 ArtifactPool 提升为正式 Artifact。暂存记录可以保留 promoted_artifact_id，用于审计和回放，不因提升而删除原暂存字节。

### 6.1 Harness scratch 与 Mathscape staging 的边界

Harness 的通用文件编辑能力和 artifact.stage 服务于两个不同的存储层：

| 层 | 允许的操作 | 生命周期 | 是否是 Mathscape Artifact |
| --- | --- | --- | --- |
| Harness scratch | 创建、修改、删除、编译和运行普通文件 | 可变；随 round 或 sandbox 清理 | 否 |
| artifact.stage | 保存 Agent 选定版本的不可变文件字节和元数据 | 持久；受 staged 生命周期管理 | 暂存对象，不是正式 Artifact |
| ArtifactPool | 用户接受后提升内容并登记目录 | 持久；受 Workspace 生命周期管理 | 是 |

边界规则：

1. Harness 只能写入当前 round 分配的隔离 scratch 根；不能直接写 Workspace 根、ArtifactPool 对象目录、SQLite 数据库或地图文件。
2. Harness 的 file-change 事件只记录编辑事实，不自动创建 staged artifact，也不自动进入 workspace.search 或图投影。
3. Agent 必须显式调用 artifact.stage，才能把某个文件版本带入 Mathscape 的 provenance、验证和提案流程。
4. artifact.stage 不提供 IDE 式编辑、增量补丁或任意路径写入；它保存一次明确的内容快照。
5. v0 的输入只接受 content。未来若需要避免大文件复制，只能使用服务端签发、绑定当前 round 的 harness_file_ref；不得开放裸路径、file URI 或任意目录读取。
6. lean.check 可以检查 Agent 提交的 source_text；后续若支持 staged_artifact_id 或 harness_file_ref，服务端仍必须重新读取受控字节并绑定 source hash，不能把路径本身当作证据。
7. round 结束、取消或 sandbox 丢弃时，scratch 文件可以清理；已经 stage 的字节不随 scratch 自动删除，仍按 staged 生命周期处理。

因此，Harness 是“可变工作台”，artifact.stage 是“显式版本边界”，ArtifactPool 是“用户接受后的正式内容存储”。任何自动把 Harness 文件变成正式 Artifact 的行为都不属于 v0。

## 7. 生命周期、可见性与并发

生命周期：

    stage
      ├── staged ──► promoted
      │      ├──► superseded
      │      └──► discarded
      └── blocked ─► staged（修正版）
                 ├──► superseded
                 └──► discarded

- staged 和 blocked 都是可读审计事实；
- discarded 只改变当前可见状态，不物理删除字节；
- promoted 只表示文件已提升，数学可信状态仍由独立 TrustRecord 管理；
- 通用删除语义不在本文定义，物理清理由后续 Workspace 垃圾回收策略负责。

v0 的 Agent stage 默认 visibility=investigation：

- 当前 investigation 的 Agent 可引用；
- 同一 Workspace 的其他 investigation 默认不可见；
- Desktop 的提案/暂存面板可以查看；
- 不进入 accepted 图和普通 workspace.search。

同一 MCP round 内的 stdio 请求按顺序处理；暂存写使用内容对象和记录文件的原子提升，不持有地图全局锁。v0 不承诺多个独立 MCP 进程同时写同一 Workspace 时的跨进程 request_id 去重，宿主必须避免为同一 round 启动并行写进程。propose_changes 引用 staged artifact 时必须重新检查状态、hash 和可见性。

## 8. 与 propose_changes 的连接

提案可以引用：

    {
      "artifact_refs": [
        {
          "staged_artifact_id": "mathscape:staged-artifact/…",
          "intended_role": "proof_narrative",
          "required_validation": {
            "validator": "lean.check",
            "state": "passed"
          }
        }
      ]
    }

服务端在生成 change_set 时必须重新验证 staged 状态、investigation 可见性、内容 hash、验证回执和媒体类型。propose_changes 只暂存文件关联和提升计划，不直接把文件写进 ArtifactPool。正式提升和正式图写入仍绑定同一次用户接受流程。

## 9. MCP 权限与后续工具

artifact.stage 不是只读工具：

    {
      "readOnlyHint": false,
      "destructiveHint": false,
      "idempotentHint": true,
      "openWorldHint": false
    }

它必须校验 Phase2 round token；当前实现把非空 `--round` 当作由宿主绑定的 opaque token，generation 的签发、失效和取消由宿主负责，MCP 工具不自行解释 token。缺少 token 时返回 `forbidden`。它不应加入当前只读 MCP 默认白名单，直到 Phase2 写路径和用户接受 UI 已经接通。

建议未来追加：

1. artifact.stage
2. artifact.stage.read（只读，可后置）

artifact.discard 和 artifact.promote 属于 Desktop/用户控制面，不开放给 Agent。

## 10. v0 验收门槛

实现前必须能测试：

1. Markdown、Lean、plain text 三种媒体类型成功暂存；
2. 空内容、超限内容、控制字符、路径穿越名称和不支持媒体类型被拒绝；
3. 相同 request_id 重试返回同一 staged id；
4. 相同 request_id 不同内容返回 idempotency_conflict；
5. 相同内容不同 request_id 可以共享字节但保留独立 provenance；
6. Lean receipt 的 source hash/profile fingerprint 不匹配时，验证引用被拒绝；
7. stage 后 Artifact Catalog、map head、workspace.search 和 graph projection 不变；
8. 重启后 staged record 和字节仍可读取；
9. supersede/discard/promote 的状态转换遵守生命周期；
10. propose_changes 不能引用其他 investigation、blocked、discarded 或 superseded 的 staged artifact；
11. 同一 investigation 并发 stage 不产生重复 request 记录或半写入对象；
12. 用户接受后 ArtifactPool 中的正式 artifact hash 与 staged content hash 一致。

## 11. 后续扩展

v0 不定义多文件 Lean project manifest、二进制 bundle、自动 Markdown 解析、自然语言证明评分、Agent 直接删除暂存文件、自动接受或 FormalArtifact / Alignment / ProofContext 的完整语义对象。

这些能力都应引用 staged_artifact_id 和验证回执，而不是重新发明一套文件身份和生命周期。
