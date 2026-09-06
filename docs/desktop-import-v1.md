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

## Marker 解析触发

Marker 作为显式外部依赖配置，不允许前端提交任意可执行文件或命令参数：

```text
MATHSCAPE_MARKER_EXECUTABLE=/absolute/path/to/marker_single
```

应用启动后通过 Marker 自身 Python 环境读取 `marker-pdf` 版本。只有版本探测通过
时，PDF 节点的解析按钮才可用。解析请求只包含：

- 输入 Artifact 稳定 ID；
- 1-based、首尾均包含的连续 PDF 页码范围；
- `fast_native_v1` 或 `math_ocr_v1` 固定 profile。

服务将其转换为 0-based `PdfSelection`，并依次执行：解析 Managed PDF、扫描隔离
staging、校验固定产物布局、按 digest 提升产物、写入 `ConversionRun`、清理成功
run、刷新来源图。执行失败或清单不完整时不会开始产物提升，并保留 staging 诊断。

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
- Marker 解析当前是等待型 Desktop 命令，UI 会保持响应但尚无逐页进度事件和
  取消按钮；Kernel 已使用可取消 runner，后续可在应用层登记活动任务。
- PDF 页面内嵌预览尚未实现；当前右栏只展示图节点元数据。
