import { defineConfig } from "@playwright/test";
process.env.NO_PROXY = [process.env.NO_PROXY, "127.0.0.1", "localhost"].filter(Boolean).join(",");
process.env.no_proxy = process.env.NO_PROXY;

export default defineConfig({
  testDir: "./tests", testMatch: "ink-lab.spec.ts", workers: 1,
  outputDir: "../.test-runs/ink-browser/results",
  use: {
    baseURL: "http://127.0.0.1:1451", viewport: { width: 1100, height: 820 },
    launchOptions: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE } : {},
  },
  webServer: { command: "node scripts/ink-lab-test-server.mjs", url: "http://127.0.0.1:1451/pad", reuseExistingServer: true, timeout: 180000 },
});
