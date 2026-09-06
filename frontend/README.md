# Frontend

这里将存放 Mathscape 的 Web 前端。构建后的静态资源由桌面应用嵌入发布产物。

首版图画布采用 Cytoscape.js。Rust 后端输出 renderer-neutral `GraphProjection`，
并将超边确定性展开为 incidence graph；前端只负责将 incidence nodes/edges
转换为 Cytoscape.js elements，不在浏览器中重新解释数学关系语义。

当前仅建立图数据契约和适配器，Desktop 页面骨架将在应用用例入口完成后接入。
