import { defineConfig } from "vite";
import { fileURLToPath } from "node:url";

const here = (path: string) => fileURLToPath(new URL(path, import.meta.url));
export default defineConfig(({ mode }) => mode === "contract" ? {
  build: {
    outDir: here("./dist-ink"), emptyOutDir: false,
    lib: { entry: here("./src/ink/model.ts"), formats: ["es"], fileName: () => "ink-contract.js" },
  },
} : {
  root: here("./ink-lab"),
  build: { outDir: here("./dist-ink"), emptyOutDir: true },
});
