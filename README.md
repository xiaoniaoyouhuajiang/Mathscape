# Mathscape

Mathscape（“数学景观”）是一个将数学材料摄取、语义建模、关系发现、图算法与 Lean 形式化验证统一起来的本地优先工具。

当前仓库处于 Domain Kernel 的初始搭建阶段。设计依据暂时位于父目录的 `docs/rdtx/`，首个实验数据位于父目录的 `data/fixtures/epsilon-n-cesaro-v1/`；后续会通过显式导入流程将它们转化为 Mathscape workspace，而不是直接产生跨仓库运行时依赖。

## 结构

```text
apps/       桌面应用与 CLI 入口
crates/     Rust Domain Kernel、存储、查询、图算法与编排模块
frontend/   将嵌入桌面产物的 Web 前端
prompts/    有版本的 LLM prompt 与结构化输出契约
migrations/ SQLite schema migration
docs/       Mathscape 自身的工程文档
testdata/   进入版本管理、供默认测试直接使用的轻量固定产物
```

大型 PDF 与实验语料位于仓库外部，通过 `MATHSCAPE_CORPUS_ROOT` 定位；默认测试不得依赖该变量。`testdata/` 只保存足以复现解析和扫描行为的小型派生产物及其来源摘要。

Artifact Pool 的长期存储、可搬迁 Workspace 和 Managed/External 边界见
[`docs/artifact-pool-boundary.md`](docs/artifact-pool-boundary.md)。

## 当前命令

```sh
cargo check --workspace
cargo test --workspace
cargo run -p mathscape-cli
```

## Desktop 开发

安装前端依赖后，可从仓库根目录运行：

```sh
npm --prefix frontend install
npm --prefix frontend run tauri:dev:local
```

`tauri:dev:local` 将自包含 Workspace 放在仓库的 `.mathscape-dev/workspace/`
中，该目录已被 Git 忽略，便于直接检查 SQLite、对象池与调试状态。普通
`tauri:dev` 则使用操作系统应用数据目录；也可以通过
`MATHSCAPE_WORKSPACE_ROOT` 显式指定任意 Workspace。
