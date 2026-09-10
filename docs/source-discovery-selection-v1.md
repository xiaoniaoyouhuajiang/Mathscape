# 来源发现：最小选区契约 v1

本版本实现 Markdown 选区、物理锚点解析和 `DiscoveryInput` 构建；Desktop 已在其上接通
单次 LLM Producer，但仍不保存发现成果，也不创建来源或语义节点。
上位设计位于工作区 `docs/rdtx/source-content-discovery-draft.md`。这份文档记录已落地的范围，
不表示草案中的审核、持久化、来源登记或批量调度已经实现。

## Desktop 操作

打开 Markdown 的“阅读文件”，点击正文块，再使用“设置起点”“设置终点”；也可直接
“选择全文”。高亮表示选定的连续块范围，描边表示当前点击的块。Enter/空格可选中获得焦点的块。

“构建发现输入”请求 Rust 重新加载原文、核对快照并解析锚点，随后显示实际将处理的 Markdown
和 UTF-8 字节区间。终点在起点之前时不可校验。清除选区、切换文件或转换来源会使旧结果失效。
原文/渲染切换保留选区；原文模式不是字符级编辑器，暂不能在原文中拖选字符来改端点。

“使用 LLM 发现内容”经同样的服务端重建过程调用默认 LLM profile，并展示提案或结构化失败；
详见 [Desktop 内容发现 v1](desktop-discovery-v1.md)。

“后续入图方式”目前仅进入请求：默认为审核后入图，可选自动加入候选图。
它既不实际触发入图，也不授予人工审核或形式化认证状态。

## 所属模块

- `mathscape-domain::discovery`：快照、选择器、选区请求、入图策略及结果 DTO，独立于
  SourceSemanticUnit，不要求虚构 Edition 或 DialectScope。反序列化本身不代表校验通过。
- `mathscape-orchestrator::anchor_markdown`：从原始字符串计算确定性物理块和快照。
- `mathscape-orchestrator::resolve_selection`：在可信锚定文档上解析选择器并生成有限输入。
- `mathscape-orchestrator::build_discovery_input`：把已解析范围构造成保留物理块、间隙和边界
  信息的无损 producer 输入，并执行独立的输入规模限制。
- Desktop：从受管 Artifact 读取真实字节、核验媒体类型/哈希/转换归属，再调用上述函数。
- Frontend：按后端块边界渲染并提交块 ID；不从 DOM 或 KaTeX 显示内容倒推原文位置。

## Tauri 服务

### get_markdown_document

参数：`artifactId`、`conversionId`（没有已登记转换时允许 null）。返回：

```text
AnchoredMarkdown
  snapshot
    artifact_id
    conversion_id
    content_hash
    anchor_scheme = mathscape-markdown-blocks-1
    fingerprint
  source                 原始 UTF-8 内容，不做换行或 Unicode 归一化
  blocks[]
    id
    range {start, end}    原始 UTF-8 字节半开区间
    kind                 物理显示提示，不是数学单元类型
```

存在转换上下文时必须选择其中一个；它必须把该 Markdown 列为输出。旧 Conversion 没有图片
路径清单也保留为可选上下文，图片映射缺失不再导致转换身份本身消失。

读取限制为 16 MiB、最多 20000 块。每次底层读取仍不超过 4 MiB；空白文档可以查看，
但不能提交空白选区。不是 Markdown、文件被篡改、UTF-8 损坏、超限时明确拒绝。

### resolve_markdown_selection

参数 `request`：

```text
version = mathscape-discovery-selection-1
snapshot                  完整回显后端快照
selector
  whole_document
  或 block_range {first, last}
  或 quote_range {first, start_quote, last, end_quote}
admission
  review_before_publish
  或 automatic_candidate
```

前端首版使用全文和块范围。`quote_range` 已在后端支持，用于后续 producer 的块内引用；
起止片段必须分别在指定块内精确且唯一出现，范围包含两片段。零匹配、重复匹配（包括重叠
出现）、反向端点或空白选区均拒绝，不做模糊定位。

响应包含原请求、解析后的 range、涉及的 block_ids、raw_text、input_fingerprint。
选区解析不自动扩大用户范围，也不声称数学边界已经闭合；上下文只在 DiscoveryInput.context 表达。
该解析接口是内部可复用能力；Desktop 当前从用户操作直接调用下述输入构建接口。

### prepare_discovery_input

参数仍为上述 `request`。服务端不会接受客户端拼装的已解析结果，而是重新读取受管 Artifact、
校验快照、解析选择器，再调用构建器。响应为：

```text
DiscoveryInput
  version = mathscape-discovery-input-2
  snapshot
  selection_fingerprint
  fingerprint
  requested
    source_range
    segments[]
      block {block_id, structural_kind, physical_range, source_range, text}
      或 gap {source_range, text}
  context[]              当前固定为空
  boundary {at_document_start, at_document_end}
```

`segments` 按原文顺序覆盖整个选区；拼接其中的 `text` 必须逐字节恢复所选 UTF-8 原文。
`gap` 保存块之间未独立建块的空白或源码，避免 producer 获得一份被悄悄改写的材料。
块内 quote 选择保留块的物理区间，以 `source_range` 说明实际可见部分。裁切状态通过
segment.clipped_start()/clipped_end() 计算；字节数和块数通过 region.utf8_bytes()/block_count()
计算，不再序列化。boundary 只表示 Markdown 文件首尾，不承诺数学单元完整或前后材料可访问。

2026-09-08 精简删除了重复统计、裁切和前后存在性字段，输入契约由 v1 升为 v2，重新构建
输入会生成新指纹。Markdown 分块版本与选区请求 v1 保持兼容；旧内存响应应重新获取，
不复用旧输入指纹。提案输出类型见 [Discovery Proposal v3](discovery-proposal-v3.md)。

默认 producer 限制为 128 KiB 和 512 个相交物理块。超限会整体拒绝，绝不截断；未来批量
处理应由显式分片/任务规划完成。`DiscoveryInput` 不携带 `admission`，因为审核或自动发布是
提案的处置策略，不应影响模型看到的数学材料或输入指纹。当前发现结果不写 SQLite，也不创建
持久 request/run。

## 身份与校验

块 ID 由 Artifact ID、内容哈希、分块版本和原始字节区间决定。相同文字在两个位置仍是两个块；
同一 Artifact 在两个转换上下文中的块 ID 可以一致，但输入快照指纹不同。

快照指纹同时绑定 Artifact、内容、转换上下文和锚点版本。后端不信任客户端提交的哈希与块 ID：
每次校验都重新从已登记受管文件构建文档，并比较完整快照。异文件块、旧版本和错误转换被拒绝。
选区指纹绑定快照及规范化区间；不同选择器解析到同一区间时选区身份相同。
发现输入指纹进一步绑定契约版本、快照、选区指纹、requested/context、无损 segments 和
文件边界标记，因此任一实际输入变化都会改变身份，producer 可据此校验与去重。
入图策略不改变数学输入指纹，但完整请求中保留策略，未来任务提交仍需独立处理策略变更。

## 分块与显示边界

使用固定版本 pulldown-cmark 的顶层容器和原始 offset，列表、表格、引用块、代码块暂作为
整体。对行首（允许少量缩进）的 `$$` 和 `\[` 展示公式范围与解析器范围合并，防止公式内空行
被切开；未闭合展示公式保守保留到文档末尾，标记 `math_unclosed`，不补写闭合符号。
代码块中的公式分隔符不参与该合并。

未被解析器产生显示事件的非空源码（如链接定义）也有可选块。纯空白间隔不单独建块，连续
选区仍包含块之间的原始空白；“全文”还保留头尾空白。超大容器不会为了块长限制静默截断。

前端以后台区间切原始 UTF-8 字节，逐块交给已有 Markdown-it/KaTeX 渲染。引用式链接/图片
的解析环境来自整份文档；无显示内容的源代码块回显原文。字体、缩放和渲染不会改变锚点。
分块不是数学分段；同一物理块内可能存在多个数学单元，后续可通过唯一引文进一步定位。

没有新增 MD—PDF 页块映射：Markdown 字节定位精确，不意味着 PDF bbox 精确。
也未实现长文件分页阅读、多文件队列、自动补上下文或跨转换去重。

## 测试

```sh
cargo test --workspace
cargo clippy -p mathscape-domain -p mathscape-orchestrator -p mathscape-desktop --all-targets -- -D warnings
npm --prefix frontend run build
npm --prefix frontend run test:reader
```

Rust 覆盖中文/emoji、CRLF、重复内容、列表/表格/代码/公式、原文引文歧义、跨文件与过期
快照、转换身份以及真实 Pool/Conversion 提升后读取。浏览器测试替换 IPC，但实际使用阅读器
渲染和选区逻辑，覆盖公式点击、原文切换、审核策略、异步过期结果、转换切换与错误提示。

本地教材回归可设置 `MATHSCAPE_MARKDOWN_FIXTURE`，运行 orchestrator 的 ignored 测试
`anchors_local_markdown_without_omitting_source_content`。它不要求其他开发者拥有私人教材。
此检查验证原始文本覆盖和定位，不验证 LLM 提取质量。

参考：[pulldown-cmark offset iterator](https://docs.rs/pulldown-cmark/0.13.4/pulldown_cmark/struct.Parser.html#method.into_offset_iter)。
