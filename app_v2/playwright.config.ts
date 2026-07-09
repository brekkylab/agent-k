import { defineConfig, devices } from '@playwright/test';

const PORT = 4211;
const BASE_URL = process.env.E2E_BASE_URL ?? `http://127.0.0.1:${PORT}`;

export default defineConfig({
  testDir: './e2e',
  timeout: 30_000,
  expect: {
    timeout: 6_000,
  },
  use: {
    baseURL: BASE_URL,
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
  },
  webServer: process.env.E2E_BASE_URL
    ? undefined
    : {
        command: `pnpm exec vite --host 127.0.0.1 --port ${PORT}`,
        url: BASE_URL,
        reuseExistingServer: true,
        timeout: 60_000,
      },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
    {
      name: 'mobile-chromium',
      use: { ...devices['Pixel 7'] },
    },
  ],
});
