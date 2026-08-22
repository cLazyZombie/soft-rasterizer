import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "tests/e2e",
  fullyParallel: false,
  workers: 1,
  timeout: 30_000,
  expect: {
    timeout: 5_000,
  },
  reporter: [["line"], ["./tests/e2e/reporter.js"]],
  use: {
    baseURL: "http://127.0.0.1:4173",
    viewport: { width: 1000, height: 720 },
    colorScheme: "dark",
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
  },
  projects: [
    {
      name: "chromium-1x",
      use: { browserName: "chromium", deviceScaleFactor: 1 },
    },
    {
      name: "chromium-2x",
      use: { browserName: "chromium", deviceScaleFactor: 2 },
    },
  ],
  webServer: {
    command:
      "pnpm exec vite preview --config vite.config.js --mode test --host 127.0.0.1 --port 4173",
    url: "http://127.0.0.1:4173",
    reuseExistingServer: false,
    timeout: 30_000,
  },
});
