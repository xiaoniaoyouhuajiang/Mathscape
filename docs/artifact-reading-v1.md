# Artifact 阅读 v1

点击来源图的 Artifact 节点，在详情中选择“阅读文件”。中央区域从地图切换到阅读器，
两侧信息保留；点击上方“地图”释放阅读资源并返回。阅读无需 Marker 或 Lean 运行。

## 统一内存读取

```text
Artifact ID → DesktopService → ArtifactPool → managed pool URI → bounded bytes
                           ↘ SQLite catalog → media type / size / conversion path context
```

- `get_artifact_content_info(artifactId)` 返回媒体类型、字节数和转换上下文。
- `read_artifact_bytes(artifactId, offset, length)` 返回二进制 IPC 响应，不用 Base64。
  offset 为 0-based 字节偏移，每次 length 为 1 字节～4 MiB；到 EOF 可以短读，超过 EOF
  拒绝。前端按声明长度读取时若收到短读，报告文件已改变。
- 读取只接受已登记的 Managed Artifact ID，不接受主机路径或任意 URI；通过池内
  路径校验拒绝符号链接。External 资源不自动下载或打开。
- 定位文件使用按 Artifact ID 筛选的 SQLite 查询，避免每次分段读取扫描整个池。
  元数据入口目前复用 Catalog snapshot，每次打开文件读取一次；不是每个字节分段
  都读取完整快照。之后若图规模增大，可单独优化元数据查询。
- 文件名、Markdown 内容和池内内容寻址布局均不改写。搬迁整个 Workspace 后依然
  通过 pool URI 解析。阅读不做逐次全文件哈希验证，完整性检查仍由 Pool 校验负责。

## 转换产物路径映射

产物提升时，把 `转换根相对路径 → Artifact ID` 清单与 ConversionRun 一起保存。
首版使用 `ConversionParameters.additional_options["mathscape.product-paths.v1"]`
存储版本化 JSON 数组，每项为 `{ "path": "book/image.png", "artifact_id": "…" }`。
这是应用保留键，不是传给 Marker 的 CLI 参数，不需要修改 SQLite 表结构。
清单和 ConversionRun 在同一登记事务中写入；此前各文件的提升仍遵循现有流程，
不声称所有文件复制与 Conversion 登记是一个整体原子事务。

路径用 `/` 分隔，只表达该次转换的虚拟目录结构，不保存 staging 绝对路径。
同一内容被多个转换复用时，可拥有不同路径上下文，不往 SourceArtifact 身份中塞入
某次转换的路径。服务验证清单目标属于该 Conversion 的输出端点。

Markdown 阅读时以其清单路径为基准解析相对图片引用，再通过映射读取图片 Artifact。
拒绝网络 URL、绝对路径、协议 URL、反斜杠和越出转换根的路径；不直接发网络请求。
多个转换上下文提供显式选择；单个上下文中映射有歧义时显示缺失提示，不猜测。

旧 Conversion 没有此清单时 PDF、Markdown 文字仍可读，但关联图片可能无法恢复。
本次不修改旧记录、不从哈希文件名猜路径；重新解析可生成带映射的新 Conversion。

## 阅读能力与边界

- PDF：使用本地打包的 PDF.js worker、CMaps、标准字体和扫描图像 codecs。
  通过 range transport 按需读取；仅渲染当前页，支持前后翻页、PDF 物理页码输入、
  缩放和内置书签目录跳转。扫描版也可渲染；不执行 OCR 或从纸面目录推断书签。
- 目录目标为页级跳转，暂不恢复页内坐标；页码不是书本印刷页码。
  当前没有 PDF 文本选择、全文搜索或批注层。
- Markdown：Markdown-it + KaTeX，支持 `$…$`、`$$…$$`、`\(…\)`、`\[…\]`，
  代码块不参与公式渲染；支持原文/渲染切换和清单内的光栅图片。
  原始 HTML 不执行、不渲染成 HTML（包括 Marker 的 HTML 表格）；链接导航暂未开放。
  KaTeX 不支持的 TeX 命令会显示错误样式，不承诺完整 LaTeX 兼容。
- JSON 格式化显示，其他 `text/*` 以 UTF-8 纯文本显示；PNG/JPEG/WebP/GIF 可独立预览。
  文本和单张图片限 16 MiB，超限明确提示，不假装完整加载。
- 切换文件/返回地图销毁旧 PDF loading/render task、释放 Blob URL；过期异步结果不能
  覆盖当前文件。前端内存中不维持整个 Artifact Pool 的文件缓存。

## 验证

```sh
cargo test -p mathscape-artifact -p mathscape-store-sqlite -p mathscape-orchestrator -p mathscape-desktop
npm --prefix frontend run build
cd frontend
npx playwright install chromium
npm run test:reader
```

浏览器测试使用自行生成的两页 PDF（含书签、足够触发 range 的填充）和 Markdown，
实际运行 PDF.js / KaTeX，仅替换 Tauri IPC；Rust 服务测试验证真实文件池、SQLite、
清理 staging 后的读取和映射恢复。两者不等于实际 Tauri WebView 端到端测试，仍需
在桌面上用真实教材手动复核。可通过 `PLAYWRIGHT_CHROMIUM_EXECUTABLE` 选择已有浏览器。
测试输出放在仓库 `.test-runs/reader`（gitignore），成功后可删除。

库参考：[PDF.js](https://mozilla.github.io/pdf.js/)、
[Markdown-it](https://github.com/markdown-it/markdown-it)、
[KaTeX 安全选项](https://katex.org/docs/options)。
