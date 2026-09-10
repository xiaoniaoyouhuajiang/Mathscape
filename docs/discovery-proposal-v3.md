# Discovery Proposal v3

状态：已实现。当前输出契约为 `mathscape-discovery-proposal-3`，嵌入的定位意图仍为
`mathscape-selector-intent-2`。v3 没有改变 quote 的 occurrence/prefix/suffix 语义；它收紧了
来源标签和引用所需的证据精度。

## 数据路径

```text
DiscoveryInput → 模型投影（b1/b2/...）→ DiscoveryProposal v3
                                            ↓
                  SelectorResolver：句柄绑定与安全 quote 规范化
                                            ↓
                  Validator：范围、精确证据、端点和图约束
                                            ↓
                         ValidatedDiscoveryProposal candidate
```

提案仍是不可信 DTO，不携带 map stable ID、接受状态或写入权限，也不构造 DialectScope、
Alignment 或 R/D/T/X。当前 Rust 解码器只接受 v3；历史 Prompt/Schema 资源按版本保留。

## 精确来源标签

每个 unit 的 `source_labels` 是必需数组；无标签时为 `[]`。每一项为：

```json
{
  "surface_form": "(20)",
  "selector": {
    "kind": "quote_range",
    "first": "b7",
    "start": {"quote": "(20)", "occurrence": 1, "prefix": null, "suffix": null},
    "last": "b7",
    "end": {"quote": "(20)", "occurrence": 1, "prefix": null, "suffix": null}
  }
}
```

Resolver 将短句柄和 quote 意图还原为正式来源坐标；Validator 要求解析出的文本与
`surface_form` 完全相等且位于 unit selectors 内。原文分别印出 `(20)`、`(21)`、`(22)`
时应产生三项，不能生成原文没有的 `(20)–(22)`。引用中提到但没有在此声明的标签不是该
unit 的 source label。展示概括继续使用 `display_title`。

验证成功的 `ValidatedDiscoveryUnit` 同时提供普通 unit selectors 与
`ValidatedDiscoverySourceLabel`，后者保存每个标签的规范提案和已解析字节范围，供后续来源
登记建立精确标签索引。

## 精确引用位置

`DiscoveryReferenceProposal.selector` 的 Rust 类型与 JSON Schema 只允许 quote_range，不能
使用 block_range。Validator 要求选择器文本与 `surface_form` 完全相等并位于 source unit
内。因此“整段包含该引用”不再等同于“定位了这一次引用”；重复文本必须用
occurrence/prefix/suffix 表达具体意图。

`target` 只允许指向本提案中另一个 unit。`target == source` 返回 `invalid_reference`；一个
大 unit 内部引用自己的公式、但没有独立目标 unit 时应保留 `target_label` 并令 `target=null`。
输入外的正常引用同样可以保持 null，不能为闭合图而虚构节点。

## 按角色划分的选择器约束

| 位置 | 允许的选择器 |
| --- | --- |
| unit selectors | block_range 或 quote_range |
| relation evidence | block_range 或 quote_range |
| issue evidence | block_range 或 quote_range |
| coverage | block_range 或 quote_range |
| source_labels[].selector | 仅 quote_range，且精确等于 surface_form |
| reference.selector | 仅 quote_range，且精确等于 surface_form |

全部选择器仍只能使用当前 DiscoveryInput 可见且获授权的块。来源标签选择器也计入提案的总
选择器限额。Schema 保证角色的结构形状，Resolver 负责模型意图到正式坐标的翻译，Validator
独立复算并检查归属；三者都不判断数学结论是否正确。

版本化 Prompt、manifest 和生成的 Schema 见
[Source Discovery v3](../prompts/source-discovery/v3/README.md)。
