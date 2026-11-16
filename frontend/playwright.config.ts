import { defineConfig } from "@playwright/test";
process.env.NO_PROXY = [process.env.NO_PROXY, "127.0.0.1", "localhost"].filter(Boolean).join(",");
process.env.no_proxy = process.env.NO_PROXY;

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
    command: "npm run pad:build && npx vite --host 127.0.0.1 --port 1431",
    url: "http://127.0.0.1:1431/tests/reader.html",
    reuseExistingServer: true,
    timeout: 120000,
  },
});
