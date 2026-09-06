# Frontend

这里将存放 Mathscape 的 Web 前端。构建后的静态资源由桌面应用嵌入发布产物。

首版图画布采用 Cytoscape.js。Rust 后端输出 renderer-neutral `GraphProjection`，
并将超边确定性展开为 incidence graph；前端只负责将 incidence nodes/edges
转换为 Cytoscape.js elements，不在浏览器中重新解释数学关系语义。

当前 Desktop 已接入 Tauri 服务，支持选择 PDF、导入 Managed Artifact Pool、
读取 SQLite Catalog，并将最新来源层 incidence graph 交给 Cytoscape.js 渲染。
配置 Marker 后，选择 PDF 节点还可以在右栏提交连续页码范围，并在解析完成后
直接看到派生产物及 ConversionRun。

```sh
npm install
npm run typecheck
npm run build
npm run tauri:dev:local
```
