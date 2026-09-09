# SelectorResolver v1

状态：已接入 Source Discovery Producer。它负责请求局部名称绑定和保守来源定位，不进行
数学语义对齐，也不访问选区之外的内容。

## 数据路径

```text
内部 DiscoveryInput（正式块哈希、字节范围、输入指纹）
              ↓ 创建请求局部绑定表
模型 DiscoveryModelInput（b1/b2/...、裁切状态、可见原文）
              ↓ LLM 输出 DiscoveryProposal
SelectorResolver（句柄绑定、quote 唯一定位、安全排版规范化）
              ↓ 正式块哈希选择器
validate_discovery_proposal
```

模型投影不包含 artifact/conversion ID、块哈希或原文字节坐标。`input_fingerprint` 仍来自
完整内部输入，模型只负责原样回显。绑定表随 PreparedDiscoveryRequest 留在内存中，Producer
必须使用同一张表解析响应，不能根据另一版本文档重新构造。

## 解析策略

1. `bN` 必须精确存在于当前绑定表；不对句柄或哈希做模糊匹配。
2. quote 首先在指定可见块中做支持重叠的精确唯一匹配。
3. 精确零匹配时，允许移除 quote 外围 `$`/`$$`、折叠 Unicode 空白，并将 `\\quad`、
   `\\qquad`、`\\,`、`\\:`、`\\;`、`\\!` 和 `\\ ` 视作排版空白。
4. 规范化结果必须唯一，并被还原成原文中的精确子串；否则返回 `quote_not_found` 或
   `ambiguous_quote`。
5. Resolver 不处理 source/target unit 归属、parent、关系类型、coverage 或数学正确性，
   这些仍由 validator 或后续修订流程负责。

解析结果包含 rewrite log，区分 `block_handle` 与 `safe_quote_normalization`。当前日志尚未
持久化，但接口保留了原始值、正式值和字段路径，供后续运行记录使用。解析失败时 Producer
保留模型原始提案，并以 `selector_resolution` 返回结构化诊断，不把它误报为普通来源验证失败。

## 明确不做

- 不用编辑距离猜测公式或引用位置。
- 不把 `<` 与 `≤`、`A` 与 `A^c`、上下标或撇号视为等价。
- 不在多个匹配中任取一个。
- 不因为成功解析位置而认定数学单元、关系或引用目标正确。

需要 occurrence、prefix/suffix 或候选交互时，应扩展定位意图协议；不能放宽唯一性检查来
模拟这些能力。
