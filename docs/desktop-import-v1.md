# Desktop PDF import v1

首版 Desktop 提供一个完整但收敛的来源层用例：选择本地 PDF，将其导入自包含
Artifact Pool，并立即刷新来源关系图。

```text
native PDF picker
       |
       v
choose_and_import_pdf (Tauri command)
       |
       v
DesktopService.import_pdf
       |
       +-- PdfEngine::classify(Fast)       验证 PDF、读取页数和抽样类型
       +-- ArtifactPool::import_file       内容寻址复制、去重、SQLite 事务登记
       +-- read_catalog_snapshot           恢复 Artifact / Conversion
       +-- ArtifactGraphProjector          构建 GraphProjection
       +-- IncidenceGraph::project         展开为二元渲染图
       |
       v
Cytoscape.js
```

## 服务边界

- 前端不能直接读取任意文件、SQLite 或 Artifact Pool。
- 原生文件选择器在 Rust/Tauri 侧运行，只把被选中的本地 PDF 交给导入用例。
- `DesktopService` 不保存长生命周期 SQLite connection；每个用例显式打开
  Workspace，从而使服务可独立测试，也避免 UI 状态成为权威状态。
- 导入先经过 PDF 内容检查；扩展名不正确或无法解析的输入不会进入 Catalog。
- Artifact Pool 完成导入后不再依赖用户选择的原始路径。
- 一次导入响应同时携带最新 `DesktopSnapshot`，避免前端在写入后显示旧图。

## Workspace 位置

生产运行默认使用 Tauri 的应用数据目录。若设置
`MATHSCAPE_WORKSPACE_ROOT`，则使用该显式目录。开发命令
`npm --prefix frontend run tauri:dev:local` 将其设置为仓库内被忽略的
`.mathscape-dev/workspace`，方便检查对象池布局和数据库。

## 当前限制

- 只接受未加密的本地 PDF；密码输入尚未进入 UI 契约。
- 当前导入只创建 `SourceArtifact`，不会自动运行 Marker，因此尚无
  `ConversionRun`。后续解析入口应复用当前 Workspace 和投影刷新路径。
- PDF 页面内嵌预览尚未实现；当前右栏只展示图节点元数据。
