import { defineConfig, searchForWorkspaceRoot } from "vite";
import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join } from "node:path";

// Keep PDF fonts, CMaps and scan codecs local in both dev and embedded builds.
const assets = new Map<string, Buffer>();
const pdfRoot = fileURLToPath(new URL("./node_modules/pdfjs-dist/", import.meta.url));
for (const folder of ["cmaps", "standard_fonts", "wasm"]) {
  for (const name of readdirSync(join(pdfRoot, folder))) {
    assets.set("pdf-assets/" + folder + "/" + name, readFileSync(join(pdfRoot, folder, name)));
  }
}

export default defineConfig({
  clearScreen: false,
  plugins: [{
    name: "local-pdf-assets",
    generateBundle() {
      for (const [fileName, source] of assets) this.emitFile({ type: "asset", fileName, source });
    },
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        const path = (req.url ?? "").split("?")[0].replace(/^\//, "");
        const data = assets.get(path);
        if (!data) return next();
        res.setHeader("Content-Type", path.endsWith(".wasm") ? "application/wasm" : path.endsWith(".js") ? "text/javascript" : "application/octet-stream");
        res.end(data);
      });
    },
  }],
  server: {
    strictPort: true,
    // 教程投影和桌面／前端共用的品牌资源位于 frontend 之外。
    fs: { allow: [
      searchForWorkspaceRoot(process.cwd()),
      fileURLToPath(new URL("../fixtures/", import.meta.url)),
      fileURLToPath(new URL("../apps/mathscape-desktop/icons/", import.meta.url)),
    ] },
    watch: {
      ignored: ["**/apps/mathscape-desktop/**"],
    },
  },
});
