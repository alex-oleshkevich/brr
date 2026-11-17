import AxeBuilder from '@axe-core/playwright';
import { expect, ready, seed, test } from './fixtures';

const THEMES = ['light', 'dark'] as const;
const VIEWPORTS = [
  { width: 1024, height: 768 },
  { width: 1280, height: 800 },
];
const AXE_TAGS = ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa', 'best-practice'];
const BUILTIN_TEMPLATE = 'chromium-headless';
const MAX_FILLED_BUTTONS = 3;
const SETTINGS_SECTIONS = ['general', 'proxies', 'captcha', 'resources', 'logins', 'sandbox', 'hosts', 'capacity', 'storage', 'agents', 'updates'];

const ROUTES = [
  '/',
  '/browsers',
  '/browsers/:instance',
  '/history',
  '/history?tab=activity',
  '/history?tab=files',
  '/history/:run',
  '/profiles',
  '/profiles/:profile',
  '/templates',
  '/templates/:template',
  '/extensions',
  '/logins',
  '/versions',
  ...SETTINGS_SECTIONS.map((id) => `/?settings=${id}`),
];

function resolve(route: string): string {
  const { profileId, instanceId, runId } = seed();

  return route.replace(':instance', instanceId).replace(':profile', profileId).replace(':run', runId).replace(':template', BUILTIN_TEMPLATE);
}

for (const colorScheme of THEMES) {
  for (const viewport of VIEWPORTS) {
    test.describe(`${colorScheme} ${viewport.width}px`, () => {
      test.use({ colorScheme, viewport });

      for (const route of ROUTES) {
        test(route, async ({ page, pageErrors }) => {
          await page.goto(resolve(route));
          await ready(page);
          const inSettings = route.includes('settings=');
          if (inSettings) await expect(page.getByRole('dialog', { name: /./ })).toBeVisible();
          await expect(page.locator('html')).toHaveAttribute('data-theme', colorScheme);
          await expect(page.locator('[aria-busy="true"]:visible')).toHaveCount(0);

          const { scrollWidth, clientWidth } = await page.evaluate(() => ({ scrollWidth: document.documentElement.scrollWidth, clientWidth: document.documentElement.clientWidth }));
          expect(scrollWidth, 'horizontal scroll').toBeLessThanOrEqual(clientWidth);

          const scope = inSettings ? page.locator('dialog[open]') : page.locator('body');
          const filled = await scope.locator('.btn-primary:visible, .btn-danger:visible').count();
          expect(filled, 'filled buttons').toBeLessThanOrEqual(MAX_FILLED_BUTTONS);

          const { violations } = await new AxeBuilder({ page }).withTags(AXE_TAGS).analyze();
          expect(violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ')).join(' | ')}`)).toEqual([]);
          expect(pageErrors).toEqual([]);
        });
      }
    });
  }
}
