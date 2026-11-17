import { expect, ready, seed, test } from './fixtures';

const EXTRA = { key: 'ref', value: 'e2e' };

type Landing = { path: string; params?: Record<string, string> };

const target = (path: string, params: Record<string, string> = {}): Landing => ({ path, params });
const settings = (section: string): Landing => target('/', { settings: section });

const SIMPLE: [string, Landing][] = [
  ['/instances', target('/browsers')],
  ['/events', target('/history', { tab: 'activity' })],
  ['/files', target('/history', { tab: 'files' })],
  ['/builds', target('/versions')],
  ['/credentials', target('/logins')],
  ['/connect', settings('agents')],
  ['/settings', settings('general')],
  ['/limits', settings('capacity')],
  ['/proxies', settings('proxies')],
  ['/captcha', settings('captcha')],
  ['/storage', settings('storage')],
];

function lands(landing: Landing, withExtra: boolean) {
  return (url: URL) => {
    if (url.pathname !== landing.path) return false;
    const expected = { ...landing.params, ...(withExtra ? { [EXTRA.key]: EXTRA.value } : {}) };

    return Object.entries(expected).every(([key, value]) => url.searchParams.get(key) === value);
  };
}

for (const [from, landing] of SIMPLE) {
  test(`${from} redirects to ${landing.path}`, async ({ page, pageErrors }) => {
    await page.goto(from);
    await expect(page).toHaveURL(lands(landing, false));
    await ready(page);
    expect(pageErrors).toEqual([]);
  });

  test(`${from} keeps an extra query parameter`, async ({ page, pageErrors }) => {
    await page.goto(`${from}?${EXTRA.key}=${EXTRA.value}`);
    await expect(page).toHaveURL(lands(landing, true));
    await ready(page);
    expect(pageErrors).toEqual([]);
  });
}

test('/instances/:id lands on the running browser and keeps ?tab=', async ({ page, pageErrors }) => {
  const { instanceId, tabId } = seed();
  await page.goto(`/instances/${instanceId}?tab=${tabId}`);

  await expect(page).toHaveURL(lands(target(`/browsers/${instanceId}`, { tab: tabId }), false));
  await ready(page);
  expect(pageErrors).toEqual([]);
});

test('/instances/:id of a finished run ends in History', async ({ page, pageErrors }) => {
  const { runId } = seed();
  await page.goto(`/instances/${runId}`);

  await expect(page).toHaveURL(lands(target(`/history/${runId}`), false));
  await ready(page);
  expect(pageErrors).toEqual([]);
});

test('/browsers/:id of a finished run lands in History, and /history/:id of a running one in Browsers', async ({ page, pageErrors }) => {
  const { runId, instanceId } = seed();

  await page.goto(`/browsers/${runId}`);
  await expect(page).toHaveURL(lands(target(`/history/${runId}`), false));
  await ready(page);

  await page.goto(`/history/${instanceId}`);
  await expect(page).toHaveURL(lands(target(`/browsers/${instanceId}`), false));
  await ready(page);
  expect(pageErrors).toEqual([]);
});

test('/events?instance= still filters the activity by that browser', async ({ page, pageErrors }) => {
  const { instanceId } = seed();
  await page.goto(`/events?instance=${instanceId}`);

  await expect(page).toHaveURL(lands(target('/history', { tab: 'activity' }), false));
  await ready(page);
  await expect(page).toHaveURL(new RegExp(`subject=instance%3A${instanceId}`));
  expect(pageErrors).toEqual([]);
});
