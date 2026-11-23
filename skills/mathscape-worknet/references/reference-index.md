# Mathscape 数学地图引用索引

这份索引解决一个问题：skill 中出现的术语、案例和准则必须能找到明确来源。看到 `T04`、`T08` 或某个规范名称时，先按本表解析，不要依赖模型记忆。

## 1. 来源根目录

优先顺序如下：

1. 当前 skill 包的 `references/`；
2. 宿主显式提供的 `workspace_root` 下、索引列出的 `docs/`（含 `docs/proposals/`）路径；
3. 可选的后端阅读包根目录（例如 `mathscape-backend-reading-pack/`）；
4. 其他路径只有在用户明确提供并且可读时才使用。

`workspace_root` 是本轮运行参数，不是 Agent 自己猜出的当前目录。`docs/` 中提案目录之外的文档主要提供当前契约和实现事实；`docs/proposals/` 只提供设计背景，除非明确标记为已锁定契约，否则不能用来证明某项能力已经存在。

如果某个路径不存在、版本无法核对或不在允许的根目录内，记录 `reference_unavailable`，不要把缺失材料当成“工作区没有该内容”。

## 2. 核心概念来源

| ref_id | 内容 | 首选路径 | 说明 |
|---|---|---|---|
| `primer` | 白话入门 | `references/reader-primer.md` | 第一次接触时先读 |
| `EMIR-0.2` | 来源—语义—对齐中间表示 | `docs/rdtx/emir-source-semantics-alignment.md` | 定义来源层、语义层、对齐层和成熟度 |
| `RDTX-0.1-draft` | R/D/T/X 工作网规范 | `docs/rdtx/rdtx-map-construction-spec.md` | 设计草案，不是冻结数据库 Schema |
| `RDTX-TRY` | 小黄的完整实践报告 | `references/rdtx_try.md`（副本来自 workspace `docs/rdtx_try.md`） | 17 篇论文、18 个工具、案例和 R/D/T/X 拆解；T04/T08/T12/T18 的工具卡在报告第 4 节附近 |
| `criteria-v0.5.1` | I1–I7 当前准则 | `references/checks-i1-i7.md`（本地检查摘要）或阅读包 `inputs/criteria/v0.5.1.md`（完整条款） | 需要完整条款时读正式文件 |
| `criteria-stable` | 相对稳定声明和剩余风险 | 阅读包 `inputs/criteria/STABLE-v0.5.1.md` | α/β 未正式等价、Folland 端点待核等 |
| `map-edit-draft` | Agent 构图与写入边界 | `docs/map-edit-contract-draft.md` 或阅读包 `inputs/map-edit-contract-draft.md` | 设计草案，区分已确认和假定工具 |
| `phase2-write` | propose → staged/blocked → 用户接受 | `docs/phase2-write-contract.md` | Phase2-0 只编译来源层写入 |
| `artifact-staging` | 文件产物暂存 | `docs/artifact-staging-contract.md` | `artifact.stage` 的实际边界 |

## 3. 案例索引

案例名是工作网的例子，不是默认可引用的数学证据。使用案例前先读取对应的 `object.md`，再读取对应的 `work-net.md`。

| case_ref | 主题 | 对象卡 | 工作网 | 重点 |
|---|---|---|---|---|
| `T04` | Moore–Penrose 伪逆、最小范数解、谱滤波 | `references/cases/T04-object.md` | `references/cases/T04-work-net-alpha.md` | α 路线；另有 `T04-work-net-beta.md` 投影/勾股路线 |
| `T08` | Laplace 原理、log-sum-exp、最大项支配 | `references/cases/T08-object.md` | `references/cases/T08-work-net.md` | X 宏、局部化、尾部估计和端点风险 |
| `T12` | KL 散度与 Donsker–Varadhan 变分表示 | `references/cases/T12-object.md` | `references/cases/T12-work-net-alpha.md` | α/β 两条路线，Debt 分支和失败见证 |
| `T18` | Hahn–Banach、Riesz 表示、万能逼近 | `references/cases/T18-object.md` | `references/cases/T18-work-net.md` | 硬 T、对偶表示和 Fourier 唯一性 |

在 `RDTX-TRY` 中，`T04`、`T08`、`T12`、`T18` 是“数学工具卡”的编号；在 `examples/` 中，同样的编号是完整对象和工作网的案例编号。两者相关但不是同一个文件，引用时要写清 `RDTX-TRY#T08` 或 `case:T08`。

这些文件的来源是后端阅读包；如果需要核对原始布局，对应路径是：

```text
mathscape-backend-reading-pack/examples/<case>/...
mathscape-backend-reading-pack/inputs/criteria/objects/<case>/...
```

如果案例文件没有随当前运行环境提供，Agent 只能说“案例未加载”，不能引用其中的结论。

## 4. 校准材料

- `criteria/rounds/round1-*` 到 `round5-*`：准则争议、修订和应用记录；
- `criteria/v0-changelog.md`：规则沿革；
- `criteria/protocol.md`：Layer A/B/C 校准协议；
- `criteria/v0-plain.md`：早期白话辅助读物，和 v0.5.1 冲突时以 v0.5.1 为准。

## 5. 引用规则

每次使用外部材料时，在工作记录中写：

```text
ref_id
resolved_path
revision_or_version
what_was_read
what_claim_it_supports
```

只写 `T08`、`EMIR` 或“见报告”不算可追踪引用。案例中的“重点”是导航提示，不替代对象卡、工作网和逐边 evidence。
