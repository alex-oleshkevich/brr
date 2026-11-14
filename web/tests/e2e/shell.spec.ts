import type { Page } from '@playwright/test';
import { expect, ready, seed, test } from './fixtures';

const AGENT_LABEL = 'e2e-shell-agent';
const address = (page: Page) => page.getByRole('textbox', { name: 'Address', exact: true });
const NEEDS_YOU_TITLE = '(1) Needs you · ';

test('navigation uses the UI words and groups', async ({ page, pageErrors }) => {
  await page.goto('/');
  await ready(page);

  const nav = page.getByRole('navigation', { name: 'Main' });
  for (const label of ['Dashboard', 'Browsers', 'Activity', 'Files', 'Connect an agent', 'Profiles', 'Proxies', 'Logins', 'Extensions', 'Browser versions', 'Captcha', 'Capacity', 'Storage', 'Settings']) {
    await expect(nav.getByRole('link', { name: label, exact: true })).toBeVisible();
  }
  for (const old of ['Instances', 'Events', 'Builds', 'Credentials', 'Limits', 'Templates']) {
    await expect(nav.getByRole('link', { name: old, exact: true })).toHaveCount(0);
  }
  for (const group of ['Run', 'Set up', 'System']) await expect(nav.getByText(group, { exact: true })).toBeVisible();
  expect(pageErrors).toEqual([]);
});

test('the capacity pill states the limit and links to Capacity', async ({ page, pageErrors }) => {
  await page.goto('/');
  await ready(page);

  const pill = page.getByRole('link', { name: /^Capacity: / });
  await expect(pill).toContainText(/\d+\/\d+ browsers/);
  await expect(pill).toHaveAttribute('href', '/limits');
  await expect(page.getByRole('button', { name: /^Theme: / })).toBeVisible();
  expect(pageErrors).toEqual([]);
});

test('dashboard answers what is running and what needs you', async ({ page, pageErrors }) => {
  await page.goto('/');
  await ready(page);

  await expect(page.getByRole('heading', { name: 'Running now' })).toBeVisible();
  await expect(page.getByRole('table', { name: 'Running browsers' }).getByRole('columnheader', { name: 'In use by' })).toBeVisible();
  await expect(page.getByRole('table', { name: 'Running browsers' }).getByRole('link', { name: /^Open / }).first()).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Needs attention' })).toBeVisible();
  for (const meter of ['Browsers running', 'Tabs open', 'Memory used', 'CPU busy']) {
    await expect(page.getByRole('progressbar', { name: meter })).toBeVisible();
  }

  await page.getByRole('button', { name: 'Launch browser' }).click();
  await expect(page.getByRole('dialog')).toBeVisible();
  expect(pageErrors).toEqual([]);
});

test('connect page gives copyable config and a working test', async ({ page, pageErrors }) => {
  await page.goto('/connect');
  await ready(page);

  await expect(page.getByRole('heading', { level: 1, name: 'Connect an agent' })).toBeVisible();
  await expect(page.getByLabel('MCP server entry', { exact: true })).toHaveText('{"command":"brr","args":["mcp"]}');
  await expect(page.getByLabel('HTTP MCP URL', { exact: true })).toHaveText(/\/mcp$/);
  await expect(page.getByLabel('REST base URL', { exact: true })).toHaveText(/\/v1$/);
  await expect(page.getByLabel('Client header name', { exact: true })).toHaveText('Brr-Client');
  await expect(page.getByLabel('CDP URL', { exact: true })).toHaveText(/^ws:\/\/.+\/cdp$/);

  await page.getByRole('button', { name: 'Test', exact: true }).click();
  await expect(page.getByRole('status').filter({ hasText: 'brr answered' })).toBeVisible();
  expect(pageErrors).toEqual([]);
});

test('a tab that needs a human raises the bar, the title and an amber bell, and clears when done', async ({ page, request, baseURL, pageErrors }) => {
  const { instanceId, tabId } = seed();
  const { id: agentId } = await (await request.post('/v1/clients', { data: { kind: 'mcp', label: AGENT_LABEL } })).json();
  const headers = { 'Brr-Client': agentId };
  const lease = await request.post('/v1/leases', { headers, data: { instance_id: instanceId, tab_id: tabId, exclusive: false } });
  expect(lease.ok(), await lease.text()).toBe(true);

  await page.goto('/profiles');
  await ready(page);

  // await_human holds its request open until a person is done, so it is not awaited here.
  const waiting = fetch(`${baseURL}/v1/tabs/${tabId}/await_human`, {
    method: 'POST',
    headers: { ...headers, 'content-type': 'application/json' },
    body: JSON.stringify({ reason: 'login', timeout_ms: 60_000 }),
  });

  const bar = page.getByRole('status').filter({ hasText: 'needs you to log in on' });
  await expect(bar).toContainText(`${AGENT_LABEL} needs you to log in on`);
  await expect(bar).toContainText('left');
  await expect(bar.getByRole('link', { name: 'Step in' })).toHaveAttribute('href', new RegExp(`/instances/${instanceId}\\?tab=${tabId}`));
  await expect.poll(() => page.title()).toContain(NEEDS_YOU_TITLE);
  await page.getByRole('link', { name: 'Browsers', exact: true }).click();
  await expect.poll(() => page.title()).toBe(`${NEEDS_YOU_TITLE}Browsers · brr`);

  await page.getByRole('button', { name: /^Notifications, \d+ unread/ }).click();
  await expect(page.getByRole('dialog', { name: 'Notifications' }).getByText(`${AGENT_LABEL} needs you to log in on`)).toBeVisible();
  await page.keyboard.press('Escape');

  // Only the web UI may finish a human step.
  const { id: uiId } = await (await request.post('/v1/clients', { data: { kind: 'ui', label: 'e2e-ui' } })).json();
  const done = await request.post(`/v1/tabs/${tabId}/human-done`, { headers: { 'Brr-Client': uiId }, data: {} });
  expect(done.ok(), await done.text()).toBe(true);
  await waiting;

  await expect(bar).toBeHidden();
  await expect.poll(() => page.title()).toBe('Browsers · brr');
  await expect(page.getByRole('button', { name: 'Notifications', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Notifications', exact: true }).click();
  await expect(page.getByRole('dialog', { name: 'Notifications' }).getByText('Resolved')).toBeVisible();
  expect(pageErrors).toEqual([]);

  await request.delete(`/v1/clients/${agentId}`);
});

test('while a tab waits for you, the browser bar works and the status says it is your turn', async ({ page, request, baseURL, pageErrors }) => {
  const { instanceId, tabId } = seed();
  const { id: agentId } = await (await request.post('/v1/clients', { data: { kind: 'mcp', label: AGENT_LABEL } })).json();
  const headers = { 'Brr-Client': agentId };
  const lease = await request.post('/v1/leases', { headers, data: { instance_id: instanceId, tab_id: tabId, exclusive: false } });
  expect(lease.ok(), await lease.text()).toBe(true);

  await page.goto(`/instances/${instanceId}?tab=${tabId}`);
  await ready(page);
  await expect(address(page)).toBeDisabled();

  const waiting = fetch(`${baseURL}/v1/tabs/${tabId}/await_human`, {
    method: 'POST',
    headers: { ...headers, 'content-type': 'application/json' },
    body: JSON.stringify({ reason: 'login', timeout_ms: 60_000 }),
  });

  await expect(address(page)).toBeEnabled();
  await expect(page.getByText(`Your turn · ${AGENT_LABEL} is waiting`)).toBeVisible();
  await expect(page.getByText(`In use by ${AGENT_LABEL}`)).toHaveCount(0);

  const target = `${baseURL}/v1/system/info`;
  await address(page).fill(target);
  await address(page).press('Enter');
  await expect
    .poll(async () => {
      const { tabs } = await (await request.get(`/v1/instances/${instanceId}/tabs`)).json();
      return tabs.find((t: { id: string }) => t.id === tabId)?.url;
    })
    .toBe(target);

  await page.getByRole('button', { name: `Done — hand back to ${AGENT_LABEL}` }).click();
  await waiting;
  await expect(page.getByText(`In use by ${AGENT_LABEL}`)).toBeVisible();
  expect(pageErrors).toEqual([]);

  await request.delete(`/v1/clients/${agentId}`);
});

test('activity groups kinds, filters by clicking a subject, and says "by you"', async ({ page, pageErrors }) => {
  const { instanceId } = seed();
  await page.goto('/events');
  await ready(page);

  await expect(page.getByRole('heading', { level: 1, name: 'Activity' })).toBeVisible();
  const type = page.getByLabel('Type', { exact: true });
  await expect(type.locator('option')).toHaveCount(8);
  await expect(page.getByLabel('Profile or browser')).toBeVisible();
  // The raw kind lives only in the expanded JSON.
  await expect(page.getByRole('table', { name: 'Activity' }).getByText('instance.state_changed')).toHaveCount(0);

  await page.getByRole('table', { name: 'Activity' }).getByRole('button', { name: /^Browser · / }).first().click();
  await expect(page).toHaveURL(/subject=instance%3A/);

  await page.getByRole('button', { name: 'Show event details' }).first().click();
  await expect(page.getByLabel('Event data').first()).toContainText('"kind"');

  await page.goto(`/events?instance=${instanceId}`);
  await ready(page);
  await expect(page).toHaveURL(new RegExp(`subject=instance%3A${instanceId}`));
  expect(pageErrors).toEqual([]);
});
