import type { Page } from '@playwright/test';
import { expect, ready, seed, test } from './fixtures';

const NEW_PROFILE = 'e2e-created';
const TEMPLATE_TITLE = 'Chrome — headless';
const AGENT_LABEL = 'e2e-agent';
const SLOW_SAVE_MS = 1500;

const address = (page: Page) => page.getByRole('textbox', { name: 'Address', exact: true });

test.describe.configure({ mode: 'serial' });

test('create a profile from a template', async ({ page, pageErrors }) => {
  await page.goto('/profiles');
  await ready(page);

  await page.getByRole('button', { name: 'New profile' }).first().click();
  await expect(page.getByRole('radio', { name: new RegExp(TEMPLATE_TITLE) })).toBeChecked();
  await page.getByRole('button', { name: 'Next' }).click();
  await page.getByLabel('Name').fill(NEW_PROFILE);
  await page.getByRole('button', { name: 'Create profile' }).click();

  await expect(page).toHaveURL(/\/profiles\/[0-9A-Z]{26}$/);
  await expect(page.getByRole('heading', { level: 1, name: NEW_PROFILE })).toBeVisible();
  expect(pageErrors).toEqual([]);
});

test('launch a browser from the Browsers page, then open it from the toast', async ({ page, pageErrors }) => {
  await page.goto('/instances');
  await ready(page);

  await page.getByRole('button', { name: 'Launch' }).first().click();
  const dialog = page.getByRole('dialog');
  await dialog.getByLabel('Profile', { exact: true }).selectOption({ label: NEW_PROFILE });
  await expect(dialog.getByRole('heading', { name: `Launch ${NEW_PROFILE}` })).toBeVisible();
  await expect(dialog.getByLabel('Start page (optional)')).toBeVisible();
  // Built-in profiles are not offered, and the advanced options sit behind a disclosure.
  await expect(dialog.getByLabel('Profile', { exact: true }).locator('option', { hasText: 'oneshot' })).toHaveCount(0);
  await expect(dialog.getByLabel('Priority')).toBeHidden();
  await dialog.getByRole('button', { name: 'Launch', exact: true }).click();

  // The dialog closes at once; a toast follows the launch.
  await expect(dialog).toBeHidden();
  await expect(page.getByText(`Started ${NEW_PROFILE}`)).toBeVisible();
  await page.getByRole('button', { name: 'Open', exact: true }).click();
  await expect(page).toHaveURL(/\/instances\/[0-9A-Z]{26}$/);
  await expect(page.getByRole('heading', { level: 1 })).toContainText(NEW_PROFILE);
  expect(pageErrors).toEqual([]);
});

test('live view: take control of a tab an agent holds, use the browser bar, then hand it back', async ({ page, request, baseURL, pageErrors }) => {
  const { instanceId, tabId } = seed();
  const agent = await request.post('/v1/clients', { data: { kind: 'mcp', label: AGENT_LABEL } });
  const { id: agentId } = await agent.json();
  const lease = await request.post('/v1/leases', {
    headers: { 'Brr-Client': agentId },
    data: { instance_id: instanceId, tab_id: tabId, exclusive: false },
  });
  expect(lease.ok(), await lease.text()).toBe(true);

  await page.goto(`/instances/${instanceId}?tab=${tabId}`);
  await ready(page);
  await expect(page.getByText(`In use by ${AGENT_LABEL}`)).toBeVisible();
  // The browser bar waits until you are in control.
  await expect(address(page)).toBeDisabled();

  await page.getByRole('button', { name: 'Take control' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Take control' }).click();
  await expect(page.getByText("You're in control", { exact: true })).toBeVisible();

  const target = `${baseURL}/v1/system/info`;
  await address(page).fill(target);
  await address(page).press('Enter');
  await expect
    .poll(async () => {
      const { tabs } = await (await request.get(`/v1/instances/${instanceId}/tabs`)).json();
      return tabs.find((t: { id: string }) => t.id === tabId)?.url;
    })
    .toBe(target);

  await page.getByRole('button', { name: `Hand back to ${AGENT_LABEL}` }).click();
  await expect(page.getByText(`In use by ${AGENT_LABEL}`)).toBeVisible();
  await expect(page.getByText(`Handed back to ${AGENT_LABEL}`)).toBeVisible();
  expect(pageErrors).toEqual([]);

  await request.delete(`/v1/clients/${agentId}`);
});

test('reloading the page keeps the tab you took over, and the activity still says "you"', async ({ page, request, pageErrors }) => {
  const { instanceId, tabId } = seed();
  const { id: agentId } = await (await request.post('/v1/clients', { data: { kind: 'mcp', label: AGENT_LABEL } })).json();
  const lease = await request.post('/v1/leases', {
    headers: { 'Brr-Client': agentId },
    data: { instance_id: instanceId, tab_id: tabId, exclusive: false },
  });
  expect(lease.ok(), await lease.text()).toBe(true);

  await page.goto(`/instances/${instanceId}?tab=${tabId}`);
  await ready(page);
  await page.getByRole('button', { name: 'Take control' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Take control' }).click();
  await expect(page.getByText("You're in control", { exact: true })).toBeVisible();
  const uiClient = await page.evaluate(() => localStorage.getItem('brr.ui-client'));
  expect(uiClient).toMatch(/^[0-9A-Z]{26}$/);

  await page.reload();
  await ready(page);
  await expect(page.getByText("You're in control", { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: `Hand back to ${AGENT_LABEL}` })).toBeVisible();
  expect(await page.evaluate(() => localStorage.getItem('brr.ui-client'))).toBe(uiClient);

  await page.goto(`/events?instance=${instanceId}`);
  await ready(page);
  const activity = page.getByRole('table', { name: 'Activity' });
  await expect(activity.getByText('You took over a tab').first()).toBeVisible();

  // The web client outlives its page now, so it is ended here to free the tab for the next test.
  await request.delete(`/v1/clients/${uiClient}`);
  await request.delete(`/v1/clients/${agentId}`);
});

test('tabs: open a new tab from the Tabs card', async ({ page, request, baseURL, pageErrors }) => {
  const { instanceId } = seed();
  await page.goto(`/instances/${instanceId}`);
  await ready(page);

  const before = (await (await request.get(`/v1/instances/${instanceId}/tabs`)).json()).tabs.length;
  await page.getByRole('button', { name: 'New tab' }).click();
  const dialog = page.getByRole('dialog', { name: 'New tab' });
  await dialog.getByLabel('Address (optional)').fill(`${baseURL}/v1/system/info`);
  await dialog.getByRole('button', { name: 'Open tab' }).click();

  await expect(page).toHaveURL(/\?tab=[0-9A-Z]{26}$/);
  await expect(page.getByText("You're in control", { exact: true })).toBeVisible();
  const after = (await (await request.get(`/v1/instances/${instanceId}/tabs`)).json()).tabs.length;
  expect(after).toBe(before + 1);
  expect(pageErrors).toEqual([]);
});

test('tabs: clicking anywhere on a row, or "Show in live view", shows that tab', async ({ page, pageErrors }) => {
  const { instanceId, tabId } = seed();
  await page.goto(`/instances/${instanceId}`);
  await ready(page);

  const rows = page.getByRole('table', { name: 'Tabs' }).locator('tbody tr');
  expect(await rows.count()).toBeGreaterThan(1);
  const seedRow = page.locator(`#tab-row-${tabId}`);
  // The title link stretches over the row, so a click on any cell lands on it; force skips Playwright's "something else is on top" check.
  await seedRow.locator('td').nth(1).click({ force: true });
  await expect(page).toHaveURL(new RegExp(`\\?tab=${tabId}$`));

  const other = page.locator(`tr[id^="tab-row-"]:not(#tab-row-${tabId})`).first();
  await other.getByRole('button', { name: /^Actions for / }).click();
  await page.getByRole('menuitem', { name: 'Show in live view' }).click();
  await expect(page).not.toHaveURL(new RegExp(`\\?tab=${tabId}$`));
  expect(pageErrors).toEqual([]);
});

test('stop: a persistent browser says its data is kept; Stop spins only after confirming', async ({ page, pageErrors }) => {
  const { instanceId } = seed();
  await page.goto(`/instances/${instanceId}`);
  await ready(page);

  const stop = page.getByRole('button', { name: 'Stop', exact: true });
  await stop.click();
  const dialog = page.getByRole('dialog', { name: /^Stop / });
  await expect(dialog).toContainText('Cookies and logins are kept');
  await expect(stop).not.toHaveAttribute('aria-busy', 'true');
  await dialog.getByRole('button', { name: 'Cancel' }).click();
  await expect(dialog).toBeHidden();
  expect(pageErrors).toEqual([]);
});

test('settings: edit a value and save it', async ({ page, request, pageErrors }) => {
  await page.goto('/settings');
  await ready(page);

  const field = page.getByLabel('Close an idle tab after (min)');
  const next = String(Number(await field.inputValue()) + 1);
  await field.fill(next);
  await page.getByRole('region', { name: 'Unsaved changes' }).getByRole('button', { name: 'Save' }).click();

  await expect(page.getByText('Settings saved')).toBeVisible();
  await expect(page.getByRole('region', { name: 'Unsaved changes' })).toBeHidden();
  const { settings } = await (await request.get('/v1/settings')).json();
  expect(String(settings.idle.tab_min)).toBe(next);
  expect(pageErrors).toEqual([]);
});

test('a busy Save button keeps keyboard focus and ignores a second activation', async ({ page, pageErrors }) => {
  await page.goto('/settings');
  await ready(page);

  let saves = 0;
  await page.route('**/v1/settings', async (route) => {
    if (route.request().method() !== 'PATCH') return route.fallback();
    saves++;
    await new Promise((r) => setTimeout(r, SLOW_SAVE_MS));
    return route.fallback();
  });

  const field = page.getByLabel('Close an idle tab after (min)');
  await field.fill(String(Number(await field.inputValue()) + 1));
  const save = page.getByRole('region', { name: 'Unsaved changes' }).getByRole('button', { name: 'Save' });
  await save.focus();
  await page.keyboard.press('Enter');

  await expect(save).toHaveAttribute('aria-busy', 'true');
  await expect(save).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.getByText('Settings saved')).toBeVisible();
  expect(saves).toBe(1);
  expect(pageErrors).toEqual([]);
});

test('unsaved-changes guard: a clean editor lets the page unload, a dirty one asks first', async ({ page, pageErrors }) => {
  const { profileId } = seed();
  await page.goto(`/profiles/${profileId}`);
  await ready(page);

  const unloadBlocked = () => page.evaluate(() => !window.dispatchEvent(new Event('beforeunload', { cancelable: true })));
  expect(await unloadBlocked()).toBe(false);

  await page.getByLabel('Name').first().fill(`${NEW_PROFILE}-renamed`);
  await page.getByRole('link', { name: 'Dashboard' }).click();
  await expect(page.getByRole('dialog', { name: 'Discard unsaved changes?' })).toBeVisible();
  await page.getByRole('dialog').getByRole('button', { name: 'Cancel' }).click();
  await expect(page).toHaveURL(new RegExp(`/profiles/${profileId}$`));
  expect(pageErrors).toEqual([]);
});

test('captcha settings warn before leaving with unsaved edits', async ({ page, pageErrors }) => {
  await page.goto('/captcha');
  await ready(page);

  await page.getByLabel('Daily solve limit').fill('7');
  await page.getByRole('link', { name: 'Dashboard' }).click();
  await expect(page.getByRole('dialog', { name: 'Discard unsaved changes?' })).toBeVisible();
  await page.getByRole('dialog').getByRole('button', { name: 'Discard' }).click();
  await expect(page).toHaveURL(/\/$/);
  expect(pageErrors).toEqual([]);
});

test('files: the page explains where agent files come from', async ({ page, pageErrors }) => {
  await page.goto('/files');
  await ready(page);

  await expect(page.getByRole('heading', { level: 1, name: 'Files' })).toBeVisible();
  await expect(page.getByRole('combobox', { name: 'Profile' })).toBeVisible();
  await expect(page.getByText('Agents save screenshots, PDFs and recordings here')).toBeVisible();
  expect(pageErrors).toEqual([]);
});

test('storage: Clean up is secondary and a local backend has no connection test', async ({ page, pageErrors }) => {
  await page.goto('/storage');
  await ready(page);

  await expect(page.getByRole('button', { name: 'Clean up…' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Clean now' })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Test connection' })).toHaveCount(0);

  await page.getByRole('tab', { name: /Backups/ }).click();
  await expect(page.getByRole('heading', { name: 'No backups yet' })).toBeVisible();
  expect(pageErrors).toEqual([]);
});

test('browser versions: capabilities sit on one line and a blocked Remove says why', async ({ page, pageErrors }) => {
  await page.goto('/builds');
  await ready(page);

  await expect(page.getByRole('heading', { level: 1, name: 'Browser versions' })).toBeVisible();
  await page.getByRole('button', { name: /^Actions for / }).first().click();
  await expect(page.getByRole('menuitem', { name: /Remove…/ })).toContainText(/detected|Used by/);
  expect(pageErrors).toEqual([]);
});
