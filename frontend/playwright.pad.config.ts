import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir:"./tests",testMatch:"ink-pad.spec.ts",workers:1,outputDir:"../.test-runs/ink-lan-browser/results",
  // Fixture-only bypass lets Chromium install the service worker. Real-device
  // trust is a separate acceptance step; Rust TLS tests verify the CA chain.
  use:{ignoreHTTPSErrors:true,acceptDownloads:true,viewport:{width:1100,height:820},launchOptions:{args:["--ignore-certificate-errors"],...(process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE?{executablePath:process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE}:{})}},
  webServer:{command:"npm run pad:build && cargo run --manifest-path ../Cargo.toml -p mathscape-desktop --example ink_lan_browser -- ../.test-runs/ink-lan-browser",url:"https://127.0.0.1:1453/health",ignoreHTTPSErrors:true,reuseExistingServer:false,timeout:180000},
});
