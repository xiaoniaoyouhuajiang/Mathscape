# 内容发现审核工作区提案 v1

## 1. 目标

把内容发现结果从“阅读器顶部的一长串裸文本”升级为可持续使用的审核工作区：结果、原文、公式、引用、关系和局部结构图相互联动，且不会因打开另一个材料而丢失当前 run。

本提案合并路线图中的第 2、3、4 项，因为它们共享同一个前端工作区和 `SourceReviewProjection` 数据源。它不负责 PDF 页码精确映射；PDF 联动由 P3 提案补充。

## 2. 现有 DTO 可复用性

`SourceReviewProjection` 已包含：

- `source`：Artifact 和输入指纹；
- `units`：类型、父级、事实状态、接受状态和 anchors；
- `references`：解析状态、候选目标、anchor；
- `structure_relations`：父子结构；
- `relations`：语义关系、证据 anchors、依据；
- `diagnostics` 和 `pending_tasks`。

因此第一版不需要新增数学实体。需要新增的是一个前端展示契约和少量来源读取 API。

## 3. 工作区布局

```text
┌────────────────────────────────────────────────────┐
│ Run 状态 · 材料 · 指纹 · 返回地图                   │
├──────────────────┬─────────────────────────────────┤
│ 发现结果/审核     │ 来源预览                         │
│                  │                                 │
│ 总览             │ Markdown / KaTeX                 │
│ 单元             │ 当前锚点高亮                     │
│ 引用             │ 查看 PDF                         │
│ 关系             │                                 │
│ 诊断             │                                 │
│ 局部结构图       │                                 │
└──────────────────┴─────────────────────────────────┘
```

结果页不再依赖“打开原文后覆盖整个结果页”的交互。切换材料时保存 `run_id`，回到材料后可以从 P1 的运行记录恢复。

## 4. 回链展示契约

### 4.1 Anchor 是唯一定位依据

```ts
interface ReviewAnchor {
  artifact_id: string;
  conversion_id: string | null;
  start: number;
  end: number;
  block_ids: string[];
  excerpt: string;
  purpose: "unit" | "source_title" | "source_label" | "reference" |
    "relation_evidence" | "diagnostic_evidence";
}
```

前端禁止用结果中的短文本在全文中重新搜索，因为同一表述可能出现多次。

### 4.2 渲染片段

新增可复用的 `RenderedAnchorFragment`：

1. 读取 anchor 所在的完整 Markdown block；
2. 使用现有 `markdownRenderer()` 和 KaTeX 渲染；
3. 用 renderer-only marker 在源范围处插入高亮；
4. 对普通文本显示红/黄/绿高亮，对公式显示公式级高亮；
5. 保留“查看原文”折叠区，用于检查精确字节范围。

这可以复用当前 Markdown Patch 的 renderer-only marker 机制，但不能把 patch 的 `<del>/<ins>` 语义误用于来源回链。

### 4.3 交互

- 点击 unit、reference、relation、issue：右侧定位并锁定高亮；
- 验证诊断若拥有直接 evidence anchor，则直接回链；只有 `related_units` 时，回退到相关 unit 的第一个 anchor，并明确显示“诊断未提供独立原文范围”；
- 悬停 unit 或 relation：只临时高亮，不改变当前锁定内容；
- “返回选区”：回到用户最初选择的 block range；
- “打开原文”：进入完整 Markdown 阅读器，但保存当前 `run_id`；
- “原文/渲染”切换：默认渲染，原文作为诊断模式；
- 失效 anchor：显示指纹漂移原因，不进行近似跳转。

## 5. 审核局部图

### 5.1 图模型

局部图只使用本次 run：

- 节点：`SourceSemanticUnit`；
- 结构边：`structure_relations`；
- 语义边：`relations`；
- SourceSpan 不作为默认节点；
- 证据模式下以小标记或辅助边表示 SourceSpan。

节点样式：

- 可接受：青绿色；
- 阻塞：橙色；
- 待完成事实：灰色；
- 已选中入图：绿色描边；
- 当前回链：黄色外框。

关系必须保留 `relation` 标签和方向；点击关系时同步显示其 evidence anchors。

### 5.2 实现选择

复用当前 Cytoscape 依赖，但使用独立的 `SourceReviewGraph` 容器和局部元素 ID。不能把审核图直接并入全局 landscape graph，否则会混淆“待审核对象”和“已登记对象”。

小于约 40 个单元时显示图；更大结果默认显示分层列表，允许用户切换图模式，避免一次审核产生不可读的毛线团。

## 6. 后端接口

P1 的 `get_discovery_review(run_id)` 是主要输入。可增加一个轻量接口：

```rust
get_markdown_anchor_preview(anchor: ReviewAnchor)
    -> RenderAnchorContext
```

但第一版也可以直接复用 `get_markdown_document`，由前端按 `block_ids` 读取完整 block。后端仍需验证：

- Artifact 身份属于 run；
- byte range 位于当前内容哈希；
- block ID 与范围相交；
- conversion ID 属于该 Markdown。

## 7. 验收标准

1. 结果页不再把数学公式显示为裸 LaTeX；
2. 点击任一 unit、引用、关系证据或带 anchor 的诊断，右侧显示正确 Markdown 上下文并高亮；没有独立 anchor 的诊断必须明确回退行为；
3. 高亮以 byte range/block ID 为准，同文重复短语不会跳错；
4. 关闭阅读器后重新打开 run，审核选择和当前结果仍可恢复；
5. 局部图节点与清单使用同一 `unit_ref`，点击二者结果一致；
6. SourceSpan 默认不污染主图，但证据模式仍可查看；
7. 内容哈希变化后显示失效状态，不做模糊定位。

## 8. 非目标

- 不在前端推断新的数学关系；
- 不在审核图中创建正式 map 对象；
- 不用浏览器 DOM 文本反推 Markdown byte offset；
- 不在本提案中实现按 run 撤回。
