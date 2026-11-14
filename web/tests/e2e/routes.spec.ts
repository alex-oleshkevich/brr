import AxeBuilder from '@axe-core/playwright';
import { expect, ready, seed, test } from './fixtures';

const THEMES = ['light', 'dark'] as const;
const VIEWPORTS = [
  { width: 1024, height: 768 },
  { width: 1280, height: 800 },
];
const AXE_TAGS = ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa', 'best-practice'];
const BUILTIN_TEMPLATE = 'chromium-headless';

const ROUTES = [
  '/',
  '/instances',
  '/instances/:instance',
  '/events',
  '/connect',
  '/builds',
  '/extensions',
  '/profiles',
  '/profiles/:profile',
  '/proxies',
  '/credentials',
  '/profiles?tab=templates',
  '/templates/:template',
  '/captcha',
  '/limits',
  '/files',
  '/storage',
  '/settings',
];

function resolve(route: string): string {
  const { profileId, instanceId } = seed();

  return route.replace(':instance', instanceId).replace(':profile', profileId).replace(':template', BUILTIN_TEMPLATE);
}

for (const colorScheme of THEMES) {
  for (const viewport of VIEWPORTS) {
    test.describe(`${colorScheme} ${viewport.width}px`, () => {
      test.use({ colorScheme, viewport });

      for (const route of ROUTES) {
        test(route, async ({ page, pageErrors }) => {
          await page.goto(resolve(route));
          await ready(page);
          await expect(page.locator('html')).toHaveAttribute('data-theme', colorScheme);

          const { scrollWidth, clientWidth } = await page.evaluate(() => ({ scrollWidth: document.documentElement.scrollWidth, clientWidth: document.documentElement.clientWidth }));
          expect(scrollWidth, 'horizontal scroll').toBeLessThanOrEqual(clientWidth);

          const { violations } = await new AxeBuilder({ page }).withTags(AXE_TAGS).analyze();
          expect(violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ')).join(' | ')}`)).toEqual([]);
          expect(pageErrors).toEqual([]);
        });
      }
    });
  }
}
