# Markdown–PDF 精确回链提案 v1

## 1. 目标

让 Markdown 中的内容发现 anchor 在具备足够元数据时能够定位到原始 PDF 页，并为用户提供“Markdown 渲染、Markdown 原文、PDF 原页”三种互相同步的阅读方式。

本提案覆盖路线图第 5 项。它依赖 P1 的稳定 run/review 入口和 P2 的回链工作区，不改变内容发现的 A/B Prompt。

## 2. 当前能力与限制

当前已有：

- `SourceSpan` 支持 `MarkdownLocator`；
- `SourceSpan` 也支持可选的 `PageCoordinate` 和 bounding boxes；
- `ConversionRun` 保存输入/输出 Artifact 以及转换页码范围；
- Artifact Pool 能按 ID 读取 PDF、Markdown 和 Marker 产物；
- Markdown reader 支持 block ID、UTF-8 byte range 和 KaTeX。

当前内容发现登记通常只构造 Markdown locator，没有把每个 Markdown block 映射到 PDF 页。因此不能假装已经具备精确 PDF 回链；没有 page map 时只能打开该 ConversionRun 的页码范围。

## 3. 来源映射契约

新增受管的转换来源索引，建议作为 `ConversionRun` 的附加元数据或独立 Artifact：

```rust
struct MarkdownPdfBlockMap {
    version: String,
    conversion_id: StableId,
    markdown_artifact_id: StableId,
    pdf_artifact_id: StableId,
    entries: Vec<MarkdownPdfBlockEntry>,
}

struct MarkdownPdfBlockEntry {
    block_id: String,
    markdown_range: TextByteRange,
    pdf_page_indices: Vec<u32>,
    text_fingerprint: ContentHash,
    confidence: MappingConfidence,
}
```

`MappingConfidence` 使用 `exact`、`candidate`、`unmapped` 三态；一个
Markdown block 可能跨越多个 PDF 页面，因此不能使用单一页码字段。

规则：

- `markdown_artifact_id`、`pdf_artifact_id` 和 `conversion_id` 必须属于同一转换；
- block range 必须覆盖真实 Markdown block；
- page index 使用 PDF 0-based index，展示时明确转换为 1-based；
- `confidence` 低于阈值时只能提示候选页，不得宣称精确定位；
- 映射文件本身进入 Artifact Pool，并由转换 manifest 声明。

## 4. Marker/转换模块工作

Marker 产物扫描阶段应尽可能读取：

- 页码到 Markdown block 的边界；
- Marker page id；
- 输出片段的文本 fingerprint；
- 公式或图片块的页码归属。

当前固定 Marker 请求已启用 `--paginate_output`。Marker 2.x 会在 Markdown
中写入带 `page_id` 的页分隔符；Mathscape 在 promotion 阶段用确定性解析器
建立 sidecar map：

1. 读取 Markdown 页分隔符并建立页段；
2. 用相同的 `anchor_markdown` 规则建立 Markdown block 索引；
3. 计算 block 与页段的交集；
4. 生成并放入 Artifact Pool 的 `MarkdownPdfBlockMap`，不修改 Markdown 字节。

没有分页标记的旧转换不会猜测页码，继续返回 `MarkdownOnly`。

这一步属于 PDF/Marker ingestion，不应交给 LLM。

## 5. Desktop API

```rust
get_source_backlink(anchor: ReviewAnchor)
    -> SourceBacklink
```

```rust
enum SourceBacklink {
    ExactPdfPage {
        pdf_artifact_id: StableId,
        pdf_page_index: u32,
        marker_page_id: Option<u32>,
        confidence: f32,
    },
    CandidatePdfPages {
        pdf_artifact_id: StableId,
        pages: Vec<u32>,
        reason: String,
    },
    MarkdownOnly {
        reason: String,
    },
}
```

接口必须先验证 Markdown 内容哈希和 conversion 身份。不能使用当前阅读器中的文本猜测 PDF 页。

## 6. 前端交互

审核工作区的来源预览提供三个标签：

- `渲染 Markdown`：默认视图，公式使用 KaTeX；
- `Markdown 原文`：显示字节范围和 block ID；
- `PDF 原页`：若为 `ExactPdfPage`，打开并定位页码；若为候选页，显示候选列表；若只有 Markdown，明确说明原因。

点击 unit、relation evidence 或 patch issue 时，三个标签共享当前 anchor。PDF 页面不能假设存在字符级高亮；第一版至少保证页码定位和当前内容摘要对照。

## 7. 迁移与兼容

- 旧 ConversionRun 没有映射文件时继续支持 Markdown 回链；
- 不重新计算旧 SourceSpan 的 page locator；
- 新解析产生的转换优先生成 map；
- map 版本变化时保留旧 Artifact，并在读取时选择兼容版本；
- PDF 或 Markdown hash 变化时映射失效。

## 8. 验收标准

1. 有映射的 unit anchor 能打开正确 PDF 页；
2. 无映射时不会错误跳转到任意页，而是显示 Markdown-only 原因；
3. PDF、Markdown 和来源结果之间可以往返切换，不丢失 `run_id` 和审核状态；
4. 页码体系（PDF index、Marker page id、印刷页码）在 UI 中明确区分；
5. 旧材料和没有 page map 的 fixture 全部保持兼容。

## 9. 非目标

- 不在 PDF 中伪造字符级高亮；
- 不用 LLM 推断页码；
- 不要求一次性为历史所有材料补齐映射；
- 不改变 SourceSemanticUnit 的数学语义。
