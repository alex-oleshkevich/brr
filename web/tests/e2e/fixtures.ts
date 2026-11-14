import { expect, test as base } from '@playwright/test';
import type { Seed } from './global-setup';

export const seed = (): Seed => JSON.parse(process.env.BRR_E2E_SEED!) as Seed;

export const test = base.extend<{ pageErrors: Error[] }>({
  // Set by global setup, which runs after the config is read.
  baseURL: async ({}, use) => use(process.env.BRR_E2E_URL),
  pageErrors: async ({ page }, use) => {
    const errors: Error[] = [];
    page.on('pageerror', (e) => errors.push(e));
    await use(errors);
    expect(errors, 'uncaught page errors').toEqual([]);
  },
});

export { expect };

// The page is ready once its skeletons are gone and the theme has been applied.
export async function ready(page: import('@playwright/test').Page): Promise<void> {
  await expect(page.locator('main h1, main h2').first()).toBeVisible();
  await expect(page.locator('[aria-busy="true"]:visible')).toHaveCount(0);
}
