import { defineConfig } from '@playwright/test';

const TEST_TIMEOUT_MS = 60_000;
const EXPECT_TIMEOUT_MS = 10_000;

export default defineConfig({
  testDir: '.',
  globalSetup: './global-setup.ts',
  timeout: TEST_TIMEOUT_MS,
  expect: { timeout: EXPECT_TIMEOUT_MS },
  // One daemon, and the flows change its state.
  workers: 1,
  reporter: [['list']],
  use: {
    launchOptions: { executablePath: process.env.BRR_TEST_CHROME },
    trace: 'retain-on-failure',
  },
  outputDir: process.env.BRR_E2E_OUT ?? 'test-results',
});
