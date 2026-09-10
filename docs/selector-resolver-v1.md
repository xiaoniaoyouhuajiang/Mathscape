# SelectorResolver v1

状态：已接入 Source Discovery Producer。它负责请求局部名称绑定和保守来源定位，不进行
数学语义对齐，也不访问选区之外的内容。
当前消费的定位意图协议为 `mathscape-selector-intent-2`。

## 数据路径

```text
内部 DiscoveryInput（正式块哈希、字节范围、输入指纹）
              ↓ 创建请求局部绑定表
模型 DiscoveryModelInput（b1/b2/...、裁切状态、可见原文）
              ↓ LLM 输出 DiscoveryProposal
SelectorResolver（句柄绑定、SelectorIntent v2 消歧、安全排版规范化）
              ↓ 正式块哈希选择器
validate_discovery_proposal
```

模型投影不包含 artifact/conversion ID、块哈希或原文字节坐标。`input_fingerprint` 仍来自
完整内部输入，模型只负责原样回显。绑定表随 PreparedDiscoveryRequest 留在内存中，Producer
必须使用同一张表解析响应，不能根据另一版本文档重新构造。

## 解析策略

1. `bN` 必须精确存在于当前绑定表；不对句柄或哈希做模糊匹配。
2. quote 首先在指定可见块中查找全部支持重叠的精确匹配。
3. 精确零匹配时，允许移除 quote 外围 `$`/`$$`、折叠 Unicode 空白，并将 `\\quad`、
   `\\qquad`、`\\,`、`\\:`、`\\;`、`\\!` 和 `\\ ` 视作排版空白。
4. occurrence 是候选序列中从 1 开始的位置；给出时直接选择该候选并校验 prefix/suffix。
   未给出 occurrence 时，prefix/suffix 过滤后的候选必须唯一。上下文只消歧，不进入范围。
5. 解析结果被还原为原文精确子串，并写入确定的 occurrence；失败返回 `quote_not_found`、
   `ambiguous_quote` 或 `invalid_disambiguator`。
6. Resolver 不处理 source/target unit 归属、parent、关系类型、coverage 或数学正确性，
   这些仍由 validator 或后续修订流程负责。

DiscoveryProposal v3 按字段角色收紧选择器：unit、关系证据、issue 与 coverage 仍可使用完整
块；每个来源标签和 reference 只能使用 quote_range。Resolver 会同样解析这些精确锚点，
Validator 再检查解析文本与 surface_form 完全相等、位于所属 unit 内，并拒绝 reference 自指。

解析结果包含 rewrite log，区分 `block_handle`、`safe_quote_normalization` 与
`occurrence_binding`。当前日志尚未
持久化，但接口保留了原始值、正式值和字段路径，供后续运行记录使用。解析失败时 Producer
保留模型原始提案，并以 `selector_resolution` 返回结构化诊断，不把它误报为普通来源验证失败。

## 明确不做

- 不用编辑距离猜测公式或引用位置。
- 不把 `<` 与 `≤`、`A` 与 `A^c`、上下标或撇号视为等价。
- 不在多个匹配中任取一个。
- 不因为成功解析位置而认定数学单元、关系或引用目标正确。

当前不提供编辑距离或候选交互。需要近似字符串候选时，应另行版本化匹配策略；不能放宽
数学字符等价规则来模拟这些能力。
