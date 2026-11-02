# 扫描 PDF 的逻辑 canonical Markdown

本文冻结「一个 PDF 对外一份 canonical markdown」的第一刀。实现是注册表 / 视图，不是新的一等图实体，也不改 Artifact 的内容寻址。

物理 Markdown 字节保持不可变。每次 Marker 解析仍产出新的内容寻址快照。canonical 只是稳定逻辑身份，指向当前 head 快照和覆盖台账。

## 身份

一条 lineage 由三元组决定：

```text
source_pdf_artifact_id + parser_profile + parser_policy_version
```

稳定 id：

```text
mathscape:canonical-markdown/v1/{parser_profile}/{parser_policy_version}/{source_pdf_artifact_id}
```

- 同一三元组永远得到同一个 `canonical_id`。换 profile 或 policy（例如 Marker 行为版本）另开 lineage，不能静默覆盖。
- `parser_profile` 来自转换参数 `marker_profile`（现有 Marker promotion 会写入，例如 `mathscape.marker.math-ocr.v1`）。历史运行没有该字段时用 `mathscape.marker.unspecified`。
- `parser_policy_version` 为 `marker-{tool_version}`；没有工具版本时为 `unversioned`。
- profile 与 policy 不能含空白或 `/`。PDF 的 stable id 原样接在最后，允许自身含 `/` 与 `:`。

这不是 `EntityKind`，也不会变成一张可修订的领域实体。图上的节点只是投影。

## 注册表与台账

SQLite（迁移 `0021_canonical_markdown`）两张表：

| 字段 | 含义 |
| --- | --- |
| `canonical_id` | 上面的逻辑 id |
| `source_pdf_artifact_id` | 源 PDF Artifact |
| `parser_profile` / `parser_policy_version` | 与身份三元组一致，`(pdf, profile, policy)` 唯一 |
| `head_artifact_id` / `head_revision` | 当前对外快照。只是指针，不改快照字节 |
| `total_pages` | 已知的 PDF 页数；未知则为空 |
| 覆盖行 `conversion_id` + `markdown_artifact_id` + `page_from`/`page_to` | 某次成功或 `partial` 转换贡献的闭区间 |

`sync_canonical_markdown_from_catalog` 按当前 `ConversionRun` **整表重建**注册表。它不调用 Marker。手写的台账会被这次同步替换。

只读投影 `project_canonical_markdown` 不写库。MCP 用它回答覆盖，因此没有事先 sync 也能报告 `partial`。地图折叠只看已落库的注册表，避免把尚未登记的历史碎片图一次性改写。

Head 选择：在该 lineage 的成功 / `partial`、输出为 `text/markdown` 的 Marker 转换里，取完成时间最新的快照；同一转换里 ordinal 更小者优先（promotion 里主 Markdown 在前）。失败、超时、取消不记入覆盖。

## 页码

对外覆盖区间是 **1-based、闭区间**，`page_index_base = one`，与 `material.read` 的 `pdf_pages` 一致。

归一化：

- `ConversionRun.page_range` 的 `PageIndexBase::Zero`（Marker `--page_range`）加 1。
- `MarkdownPdfBlockMap.marker_page_ids` 与 `{page}----------------` 也是 0-based PDF 页索引。`pages_from_marker_page_ids` 把它们收成 1-based 区间。
- 本切片的自动投影使用转换记录上的 `page_range`。页图 JSON 在 Artifact 字节里，读取它不属于这条只读 SQL 投影。调用方已经解析出 `marker_page_ids` 时，用上面的纯函数，不要把两套页码直接相加。

没有 `page_range` 的转换可以成为 head，但不贡献 `covered` 页。

## 覆盖语义

```json
{
  "coverage": {
    "total_pages": 300,
    "covered": [{ "from": 12, "to": 40 }],
    "missing": [{ "from": 1, "to": 11 }, { "from": 41, "to": 300 }],
    "requested": { "from": 12, "to": 40 },
    "head_covered": [{ "from": 12, "to": 40 }],
    "partial": true,
    "complete": false,
    "page_index_base": "one"
  }
}
```

- `covered`：各不可变快照页区间的并集。
- `head_covered`：当前 head 那一份快照自己的页，不是并集。
- `missing`：在已知 `total_pages` 内、未被并集覆盖的页。页数未知时 `missing` 为空，且不能当成全文。
- `complete`：页数已知，并集盖住全书，**并且 head 自己盖住全书**。片段即使拼起来盖住全书，也仍然不是 complete，因为本切片不拼接字节。
- `partial`：请求区间没有被 `covered` 完全盖住，**或者** `complete` 为 false。因此只读到第 12–40 页时，即使这次请求的页都有文本，`partial` 仍为 true。`status=ok` 不能理解成全文可搜。

## MCP

不新增工具，也不在读取时运行 Marker。已有四个只读工具在命中 canonical 时多带 `canonical_id`、`coverage`、`partial`：

- `material.read`
- `material.grep`
- `workspace.search`
- `pdf.outline`

扫描版 PDF（请求页都没有文本层）：

- 还没有任何覆盖该请求的快照：维持 `capability_unavailable`（可附带全书覆盖，若 lineage 已存在于其他页）。
- 已有快照与请求相交：返回该快照的 Markdown 字节（优先 head），`content_type` 为 `markdown`，`served_artifact_id` 指向物理快照。不按页切开字节；区间以 `coverage` 为准。超出 32KiB 仍是 `truncated`。

有文本层的 PDF 仍返回文本层，只附加覆盖字段。

用 canonical id 调用 `material.read`（`md_byte_range`）或 `material.grep` 时，解析到 head 快照再读。这不是 artifact 重定向：旧的物理 artifact id 照常可读，并带上同一个 `canonical_id`。

## 地图

仅当该 PDF 已有注册表行时，材料图把这条 lineage 的 head 与有页码的 Markdown 快照收成 **一个** 派生节点。节点 id 是 `canonical_id`（仍用 `SourceArtifact` 这个投影种类，避免新的领域类型）。转换边和 `located_in` 改指向该节点。图片、元数据 JSON 仍留在图上。

`graph.get` 既有的 `canonical_id` 字段继续表示「解析后的图节点 id」。对这个视图节点，它就是逻辑 canonical id；`content_ref.artifact_id` 指向 head，以便原样交给 `material.read`。SourceSpan 的字节定位仍指向原来的物理快照，本切片不重挂。

未 sync 的历史转换保持原图：每个物理 Markdown 仍是独立节点。

## 非目标

本切片明确不做：

- 自动 `ensure_parsed`，或在 `material.read` 里调用 Marker
- 按 PDF 的锁、预算、解析风暴防护
- 把多段快照拼成一份新的全书 Markdown，或原地改 Artifact 字节
- 旧 SourceSpan / discovery 的 supersede、redirect、重挂
- 搜索索引改写（搜索仍可能列出物理快照，但会带上 `canonical_id`）
- 沙箱 UI、解析中的地图场景信号

后续：ensure 收进 `material.read` 的内部路径（不另做 Agent 可调用工具），并在地图上给出「解析中」信号。那一步才写新的快照和台账。
