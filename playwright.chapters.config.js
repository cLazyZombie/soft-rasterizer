import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "tests/e2e",
  testMatch: ["gallery.spec.js", "gallery.production.spec.js"],
  fullyParallel: false,
  workers: 1,
  timeout: 60_000,
  expect: {
    timeout: 10_000,
  },
  reporter: [["line"], ["./tests/e2e/reporter.js"]],
  outputDir: "test-results/chapters",
  use: {
    viewport: { width: 1000, height: 720 },
    colorScheme: "dark",
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
  },
  projects: [
    {
      name: "chapters-chromium-1x",
      testMatch: "gallery.spec.js",
      use: {
        baseURL: "http://127.0.0.1:4174",
        browserName: "chromium",
        deviceScaleFactor: 1,
      },
    },
    {
      name: "chapters-production-chromium-1x",
      testMatch: "gallery.production.spec.js",
      use: {
        baseURL: "http://127.0.0.1:4175",
        browserName: "chromium",
        deviceScaleFactor: 1,
      },
    },
  ],
  webServer: [
    {
      command:
        "pnpm exec vite preview --config vite.gallery.config.js --mode test --host 127.0.0.1 --port 4174",
      url: "http://127.0.0.1:4174",
      reuseExistingServer: false,
      timeout: 30_000,
    },
    {
      command:
        "pnpm exec vite preview --config vite.gallery.config.js --host 127.0.0.1 --port 4175",
      url: "http://127.0.0.1:4175",
      reuseExistingServer: false,
      timeout: 30_000,
    },
  ],
});
