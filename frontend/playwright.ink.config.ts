import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests", testMatch: "ink-lab.spec.ts", workers: 1,
  outputDir: "../.test-runs/ink-browser/results",
  use: {
    baseURL: "http://127.0.0.1:1451", viewport: { width: 1100, height: 820 },
    launchOptions: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE } : {},
  },
  webServer: { command: "node scripts/ink-lab-test-server.mjs", url: "http://127.0.0.1:1451/pad", reuseExistingServer: false },
});
