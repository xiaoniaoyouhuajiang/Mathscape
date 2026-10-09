import { mkdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { startInkLab } from "./ink-lab-server.mjs";

const root = fileURLToPath(new URL("../../.test-runs/ink-browser/", import.meta.url));
await mkdir(root, { recursive: true });
const lab = await startInkLab({ host: "127.0.0.1", port: 1451, outputDir: `${root}/exports-${process.pid}`,
  staticDir: process.env.INK_LAB_TEST_STATIC_DIR });
await writeFile(`${root}/access.json`, JSON.stringify({ port: lab.port, hostUrl: lab.hostUrl, hostToken: lab.hostToken, padToken: lab.padToken }), { mode: 0o600 });
for (const signal of ["SIGINT", "SIGTERM"]) process.once(signal, () => void lab.close().then(() => process.exit(0)));
