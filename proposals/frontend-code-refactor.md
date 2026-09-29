# RFC: 前端架构稳定化整改方案 (Frontend Stabilization RFC)

- **状态**: Approved (整改阶段)
- **目标分支/模块**: `frontend/`
- **负责人**: 前端工程团队
- **关联文档**: [architecture.md](../docs/architecture.md)

---

## 1. 背景与现状

Mathscape 定位于面向数学学习者与研究者的本地优先（Local-First）桌面知识工坊。当前前端基于 Tauri + Vite + Vanilla TypeScript 构建，负责呈现材料层（PDF/Markdown）、来源图（Incidence Graph）、交互审核及人工地图编辑。

随着系统演进，现有实现表现出扎实的领域建模与扎实的算法功底（超边展开、增量 Diff 布局、分块范围传输、两阶段变更分析等）。但随着功能扩展，前端进入了重度原型向工业级软件演进的瓶颈期：
1. **代码膨胀与职责过载**：`main.ts` 单文件超过 1600 行，承担了 DOM 结构注入、状态缓存、事件总线、图生命周期、Marker 解析调度与 Inspector 编辑联动等多重职责。
2. **状态与契约分散**：运行状态定义（`DiscoveryRunSummaryView` 等）与状态文本标签分散在 `main.ts`、`discovery-task.ts`、`review/types.ts` 等多个文件中，存在后端契约变更时“漏改某一处”的维护隐患。
3. **全局通信缺乏编译期约束**：使用松散的 `window.dispatchEvent` / `CustomEvent` 进行全局广播，事件名称与 payload 缺乏类型安全保障与生命周期清理机制。
4. **纯逻辑单元测试缺失**：目前仅有端到端测试（Playwright），对于图拓扑投影、补丁标记、范围计算等确定性纯函数的测试反馈周期过长。

经过技术决策，现阶段**不进行 Svelte / Preact / Zustand 级别的技术栈重写**，避免破坏现有稳定性；优先聚焦于**“边界清晰、状态收敛、可测试、生命周期完备”**的渐进式稳定化整改。

---

## 2. 核心架构决策 (Decisions)

### 决策 1：维持 Vanilla TypeScript 体系，不引入外部重型框架
- **结论**：暂缓引入 Svelte / Preact / Zustand / Lit。
- **依据**：现有交互已跑通，当前业务痛点在于模块组织与状态生命周期，而非渲染框架本身。引入框架将导致构建配置重构、大量组件重写以及潜在的状态同步双轨制，ROI 过低。
- **方案**：使用面向对象的 Controller 模式与显式状态上下文对象，实现职责清晰的高内聚、低耦合结构。

### 决策 2：建立集中式前端 Wire DTO 与状态标签中心
- **结论**：收拢跨模块重复定义的接口和状态文本。
- **目录规划**：
  - `frontend/src/core/ipc.ts`：底层 Tauri 命令调用封装与错误处理工具；
  - `frontend/src/discovery/types.ts`：发现运行（Discovery Run）、诊断（Diagnostics）、审查封套（Review Envelope）等 DTO；
  - `frontend/src/discovery/labels.ts`：统一定义状态徽章文本（`review_status`、`workflow_status` 等）与标签格式化函数。

### 决策 3：实现强类型、可注销的事件中心 (`core/events.ts`)
- **结论**：替代 `window.dispatchEvent`，构建类型安全的轻量事件中心。
- **接口规范**：
  ```typescript
  export interface MathscapeEventMap {
    "source-map-changed": { reason?: string };
    "discovery-runs-changed": void;
    "open-source": {
      artifactId: string;
      conversionId?: string | null;
      title: string;
      focus?: ReaderFocus;
      patchRunId?: string;
      patchMode?: PatchReviewMode;
      refreshMap?: boolean;
    };
    "open-review": { outcome: DiscoveryOutcome; meta?: ReviewRunMeta };
    "reader-opened": void;
    "open-settings": void;
    "workspace-settings-changed": WorkspaceSettings;
  }

  export interface IEventBus {
    emit<K extends keyof MathscapeEventMap>(event: K, payload: MathscapeEventMap[K]): void;
    on<K extends keyof MathscapeEventMap>(event: K, handler: (payload: MathscapeEventMap[K]) => void): () => void;
  }
  ```
  每个 `on` 方法必须返回注销函数（unsubscribe），杜绝内存泄漏。

### 决策 4：拆分 `main.ts` 为单一职责控制器
- **结论**：`main.ts` 作为启动装配入口，业务逻辑下沉到专属控制器：
  - `shell/app-shell.ts`：骨架 DOM 渲染、全局 DOM 引用查询（`requiredElement`）与折叠侧栏状态管理；
  - `graph/graph-controller.ts`：Cytoscape 实例维护、过滤预设切换、焦点深度控制、图例与右键菜单交互；
  - `artifact/artifact-controller.ts`：PDF 导入调用、Marker 环境状态展示、解析请求调度与状态同步；
  - `discovery/discovery-ledger.ts`：运行历史列表渲染、诊断模态框交互、草稿放弃与重跑入口；
  - `inspector/inspector-controller.ts`：选中节点详情展示、只读与编辑/退役模式切换、变更请求组装与两阶段提交。

### 决策 5：保留现有解析轮询作为兼容过渡
- **结论**：保留 `syncParseState()` 的 1 秒定时轮询机制，不作为阻塞项整改。
- **依据**：后端当前尚未提供完整的 `mathscape://parse-state-changed` 广播事件。现有轮询逻辑具备防御性容错（非空比较、网络抖动保留最后已知状态），待后端事件通道成熟后平滑替换。

### 决策 6：引入 Vitest 建立纯逻辑测试基线
- **结论**：安装 `vitest`，优先对无 DOM 或轻量 DOM 的高风险纯函数建立毫秒级单元测试。
- **第一阶段测试范围**：
  - `graph/cytoscape.ts`：超边到语义边/junction 节点的展开逻辑；
  - `graph/focus.ts`：子图聚焦与深度过滤算法；
  - `reader/markdown.ts`：数学公式定界符保全与补丁标记（`removePatchMarkers`、`markdownPatchSource`）；
  - `map-edit/inspector-edit.ts`：用户变更请求（`UserChangeRequest`）构造纯逻辑。

---

## 3. 架构分层目标视图

```text
frontend/src/
├── core/                       # 核心基础设施
│   ├── events.ts               # 强类型事件总线 (Typed EventTarget)
│   ├── ipc.ts                  # Tauri invoke 封装与异常语义化
│   └── dom.ts                  # DOM 辅助工具 (requiredElement, textElement 等)
├── shell/                      # 页面外壳与全局宿主
│   └── app-shell.ts            # 顶栏、侧栏折叠、全局容器装配
├── graph/                      # 图可视化子系统
│   ├── graph-controller.ts     # 图生命周期、过滤与聚焦编排
│   ├── cytoscape.ts            # Cytoscape 适配与增量 Diff 算法
│   ├── focus.ts                # 子图聚焦算法
│   ├── projection.ts           # 后端 IncidenceGraph 投影类型契约
│   └── taxonomy.ts             # 节点/边样式与分类
├── artifact/                   # 材料与转换子系统
│   └── artifact-controller.ts  # PDF 导入、Marker 解析调度
├── discovery/                  # 内容发现与记录子系统
│   ├── types.ts                # 统一的运行、诊断、封套 DTO
│   ├── labels.ts               # 统一的状态格式化标签
│   ├── discovery-ledger.ts     # 运行历史面板与诊断展示
│   └── discovery-task.ts       # 任务浮窗与实时进度流
├── inspector/                  # 检查器与地图编辑子系统
│   └── inspector-controller.ts # 节点详情、编辑/退役/新建控制器
├── map-edit/                   # 两阶段地图编辑核心
│   ├── api.ts                  # 规划与提交 IPC
│   ├── types.ts                # 变更协议契约
│   ├── inspector-edit.ts       # 表单构建
│   ├── impact-preview.ts       # 级联预览
│   └── commit-status.ts        # 提交状态反馈
├── reader/                     # 阅读器子系统 (PDF / Markdown / 选区)
├── review/                     # 审核工作台子系统
├── llm/                        # 模型设置中心
└── main.ts                     # 应用入口：装配各个控制器并建立事件联动
```

---

## 4. 实施阶段与步骤

### 阶段 F0: RFC 整理与方案对齐（已完成）
- 清理历史探查日志与冗余文本，形成统一的整改 RFC 规范。

### 阶段 F1: 核心契约集中化与类型化事件总线
1. 创建 `src/core/events.ts`，定义 `MathscapeEventMap` 与 `EventBus` 单例；
2. 创建 `src/discovery/types.ts` 与 `src/discovery/labels.ts`，收敛来自 `main.ts`、`review/`、`discovery-task.ts` 的重复类型和标签映射；
3. 将现有的 `window.dispatchEvent` 和 `window.addEventListener` 渐进迁移至类型化事件中心。

### 阶段 F2: `main.ts` 控制器拆解
1. 抽离 `src/core/dom.ts` 与 `src/shell/app-shell.ts`，解耦 HTML 模板拼接与全局 DOM 句柄查询；
2. 抽离 `src/graph/graph-controller.ts`，接管预设过滤、复选框同步、语义边切换与聚焦逻辑；
3. 抽离 `src/artifact/artifact-controller.ts`，接管 PDF 导入与 Marker 解析调度；
4. 抽离 `src/discovery/discovery-ledger.ts`，接管运行历史列表刷新与诊断渲染；
5. 抽离 `src/inspector/inspector-controller.ts`，接管节点详情与 `map-edit` 模态切换；
6. `main.ts` 改造为引导入口，实例化各控制器并绑定事件，整体结构控制在清晰、可测试的范围。

### 阶段 F3: 引入 Vitest 与纯逻辑单元测试基线
1. 配置 `vitest` 与 `vite.config.ts`；
2. 编写首批纯逻辑单测：
   - `tests/unit/graph-projection.spec.ts`：验证超边展开与 junction 规则；
   - `tests/unit/graph-focus.spec.ts`：验证聚焦深度提取算法；
   - `tests/unit/markdown-markers.spec.ts`：验证补丁高亮与标记清理；
   - `tests/unit/map-edit-builder.spec.ts`：验证 `UserChangeRequest` 构造逻辑。
3. 确保 `npm run typecheck`、`npm run test:unit` 以及原有 Playwright 测试全部通过。

### 阶段 F4 (暂缓项，待未来阶段评估)
- 探索声明式 UI（Svelte 5 / Preact）或轻量模板库（lit-html）；
- 后端提供稳态事件推送后，用 `listen` 替换 Marker 解析轮询；
- 对 `review/workspace.ts` 与 `map-edit/inspector-edit.ts` 内部做进一步的局部声明式优化。

---

## 5. 验收标准 (Definition of Done)

1. **编译与类型安全**：`npm run typecheck` 零错误，无新增隐式 `any`；
2. **测试验证**：原有 Playwright 端到端测试全部通过；新增纯逻辑单测全部通过；
3. **架构职责**：`main.ts` 不再直接操作具体业务 DOM 节点，转为声明式实例化控制器；
4. **生命周期防御**：所有新增事件监听均支持清理，异步状态锁均有 `try/finally` 保护。
