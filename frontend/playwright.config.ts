import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests",
  testIgnore: ["**/unit/**", "**/ink-lab.spec.ts", "**/ink-pad.spec.ts", "**/ink-server.test.mjs"],
  outputDir: "../.test-runs/reader",
  use: {
    baseURL: "http://127.0.0.1:1431",
    launchOptions: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE
      ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE } : {},
  },
  webServer: {
    command: "npm run dev -- --port 1431",
    url: "http://127.0.0.1:1431/tests/reader.html",
    reuseExistingServer: !process.env.CI,
  },
});
