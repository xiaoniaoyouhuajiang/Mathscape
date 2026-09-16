# Mathscape

Mathscape（“数学景观”）是面向数学学习者和研究者的本地优先工具，目标是将重要数学材料转成可追溯的超图，在多个视图中组织笔记、理解证明并探索跨领域关联。LLM 提出候选，程序验证与用户审核控制登记。

当前已打通 PDF 导入、Marker 转换、阅读选区、LLM 来源发现、审核登记和来源图浏览。数学语义、对齐与 Lean 编排仍未形成端到端实现；Agent 调度是待实验方向。

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

新开发者按顺序阅读四份主文档，再沿文中代码入口深入：

1. [架构与当前能力](docs/architecture.md)
2. [数据模型与不变量](docs/data-model.md)
3. [存储与集成](docs/storage-and-integrations.md)
4. [开发与验证](docs/development.md)

历史工程草案已合并，原文可从 Git 历史查阅。版本化 Prompt、Schema 和回归材料继续保留。

## 当前命令

```sh
cargo check --workspace
cargo test --workspace
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

要在 Desktop 中启用 PDF 解析，需要把固定 Marker 安装入口显式传给应用：

```sh
MATHSCAPE_MARKER_EXECUTABLE=/absolute/path/to/marker/bin/marker_single \
  npm --prefix frontend run tauri:dev:local
```

应用会使用该环境自己的 Python 探测 `marker-pdf` 版本；当前固定 profiles 支持
Marker 2.x。选择材料图中的 PDF Artifact 后，可在右栏触发连续页码范围解析。
