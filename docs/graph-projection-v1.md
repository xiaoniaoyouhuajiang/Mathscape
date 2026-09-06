# Graph Projection 与超边投影规范 v1

## 目标与技术选择

数学地图的权威事实保存在 Domain 与 SQLite 中。Graph Projection 是按用户视图重建的只读投影，不是新的权威存储，也不允许从前端布局反向推断数学语义。

首版采用以下组件：

- Rust `mathscape-graph` 定义 renderer-neutral `GraphProjection`；
- Rust `petgraph` 为投影建立运行时索引并承载后续图算法；
- Rust 将带角色的超边确定性展开为 incidence graph；
- 前端 Cytoscape.js 渲染 incidence nodes/edges；
- Desktop 延续既定的嵌入式 Web 前端边界，后续使用 Tauri 承载静态前端。

Cytoscape.js 支持 JSON elements、自动布局、compound nodes 与扩展，适合关系节点和层级分组。Sigma.js/Graphology 更偏向 WebGL 大规模平面网络；若将来全景视图达到 Cytoscape.js 无法满足的规模，可增加独立的聚合投影，而不替换本规范。

参考：

- <https://js.cytoscape.org/>
- <https://docs.rs/petgraph/>
- <https://www.sigmajs.org/docs/>
- <https://v2.tauri.app/start/frontend/>

## 两级投影

### 语义投影

`GraphProjection` 保留数学地图的关系结构：

```text
GraphProjection
  schema_version
  lens
  groups[]
  nodes[]
  relations[]

ProjectionRelation
  relation_id
  kind
  layer
  endpoints[]

ProjectionEndpoint
  node
  role
  ordinal
  direction
```

这里的 relation 仍是第一类超边。它有自己的稳定身份、类型、状态、标签和任意数量的角色端点。

### 渲染投影

`IncidenceGraph::project` 将每条超边转换为一个关系节点和若干 incidence edges：

```text
Domain / GraphProjection

ConversionRun(input = PDF, output = [Markdown, Metadata])

                    deterministic projection
                              |
                              v

PDF --input--> [ConversionRun] --output[0]--> Markdown
                                  |
                                  +--output[1]--> Metadata
```

前端只接收：

```text
IncidenceGraph
  schema_version
  lens
  nodes[]   # entity | relation | group
  edges[]   # one endpoint incidence per edge
```

前端不得重新生成关系节点、猜测 endpoint 方向或折叠超边；这些行为必须由 Rust 投影规则控制。

## ID 规则

所有渲染 ID 都由权威身份确定性派生：

```text
entity:<entity StableId>
relation:<relation StableId>
group:<projection group id>
incidence:<relation StableId>:<role>:<ordinal|single>
```

同一查询结果重复构建时必须产生相同 ID。画布位置、选中状态和展开状态可以使用这些 ID 保存，但它们不是 Domain ID。

集合在序列化前按照 ID 排序。重复实体、重复关系、未知 endpoint、未知 group 和 group 环路必须在 Rust 侧拒绝。

## 端点方向

Domain 的 `EndpointRole` 描述语义角色，但角色名称本身不隐含方向。每个 View Projection 必须显式提供 `IncidenceDirection`：

| Direction | 二元展开 | 例子 |
| --- | --- | --- |
| `into_relation` | entity → relation | input、premise、source |
| `out_of_relation` | relation → entity | output、conclusion、target |
| `undirected` | relation — entity | equivalent、member、symmetric alignment |

同一 role 出现多次时必须带互不重复的 ordinal；单次出现可以使用 `ordinal = null`。显示标签使用 role，ordinal 只区分有序的重复槽位。

v1 总是展开关系节点。即使一条关系只有两个 endpoint，也不自动折叠为普通边，因为关系可能拥有自身证据、revision、状态和详情面板。未来允许在不损失信息的视图中增加纯显示层折叠，但不能改变投影身份。

## Layer、Lens 与边界节点

`ViewLayer` 表示节点在数学地图中的主要层：

```text
source
local_semantic
global_semantic
alignment
rdtx
formalization
```

`ProjectionLens` 表示本次查询的阅读任务，例如 Source、RDTX、Provenance 或 Landscape。一个 Lens 可以包含多个 Layer，但默认应聚焦一个主层。

相邻层中为了提供上下文而加入的实体必须标记 `boundary = true`。前端以弱化样式显示它，用户可以继续展开或跳转到对应 Lens。不得通过是否处于边界改变 Domain 身份。

## Group 与复合节点

Group 是视图分组，不是数学实体。它用于章节、局部 Theory、宏或聚合结果的视觉容器，并映射到 Cytoscape.js compound node：

```text
ProjectionGroup
  id
  label
  layer
  parent?
```

Group 可以嵌套，但必须引用已存在的 parent 且不能成环。Entity node 和 Relation node 都可以属于一个 group。Group 的重命名或布局不会修改 Domain。

## Cytoscape.js 映射

Rust `IncidenceNode` 映射为 Cytoscape node：

```text
data.id
data.label
data.elementKind
data.domainId
data.domainKind
data.layer
data.lifecycle
data.boundary
data.parent
```

Rust `IncidenceEdge` 映射为 Cytoscape edge：

```text
data.id
data.source
data.target
data.relationId
data.role
data.ordinal
data.direction
data.directed
```

`directed = false` 的边不显示箭头。relation node 使用独立形状，group 使用 compound 样式，boundary node 使用弱化样式。

## Rust 运行时索引

`ProjectionIndex` 使用 petgraph `StableDiGraph` 对渲染投影建立内存索引。其索引号只在当前内存实例有效，绝不能写入 SQLite、URL、用户收藏或前端状态；跨进程身份始终使用 `RenderElementId` 与 Domain `StableId`。

图算法应运行在 Rust 投影或权威查询结果上。前端 Cytoscape.js 的布局坐标不是数学信息，也不能进入语义判断。

## 版本与演进

首版 wire schema 为：

```text
mathscape.graph-projection.v1
```

前端必须拒绝未知 schema。增加可选显示字段可以保持 v1；改变 ID、方向、端点或分组语义时必须提升 schema 版本。

本规范暂不包含：

- SQLite 图查询实现；
- Artifact catalog 到 Source Lens 的 projector；
- fixture 到语义 Lens 的 projector；
- 布局持久化；
- 二元关系折叠；
- 大地图分块和渐进加载协议。
