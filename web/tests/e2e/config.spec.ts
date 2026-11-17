import type { Page } from '@playwright/test';
import { expect, ready, test } from './fixtures';

const PROXY_HOST = '127.0.0.1';
const PROXY_PORT = '3999';
const PROXY_USER = 'e2e-user';
const IN_USE_PROFILE = 'e2e-proxy-user';
const SITE_HOST = 'e2e-login.example.com';

const dialog = (page: Page) => page.locator('dialog[open]').filter({ has: page.getByRole('navigation', { name: 'Settings sections' }) });
const saveBar = (page: Page) => dialog(page).getByRole('region', { name: 'Unsaved changes' });

test.describe.configure({ mode: 'serial' });

test('proxies: pasting a proxy URL into Host fills every field', async ({ page, pageErrors }) => {
  await page.goto('/?settings=proxies');
  await ready(page);

  await dialog(page).getByRole('button', { name: 'Add proxy' }).first().click();
  const add = page.getByRole('dialog', { name: 'Add proxy' });
  await add.getByLabel('Host').fill(`socks5://${PROXY_USER}:secret@${PROXY_HOST}:${PROXY_PORT}`);

  await expect(add.getByLabel('Host')).toHaveValue(PROXY_HOST);
  await expect(add.getByLabel('Port')).toHaveValue(PROXY_PORT);
  await expect(add.getByLabel('Scheme')).toHaveValue('socks5');
  await expect(add.getByLabel('Username (optional)')).toHaveValue(PROXY_USER);

  await add.getByRole('button', { name: 'Add proxy' }).click();
  await expect(page.getByText('Proxy added')).toBeVisible();
  await expect(dialog(page).getByRole('row', { name: new RegExp(`${PROXY_HOST}:${PROXY_PORT}`) })).toBeVisible();
  expect(pageErrors).toEqual([]);
});

test('proxies: a visible Check now button reports the cause, and a used proxy is removed from its profiles, then deleted', async ({ page, request, pageErrors }) => {
  const { proxies } = await (await request.get('/v1/proxies')).json();
  const proxy = proxies.find((p: { port: number }) => String(p.port) === PROXY_PORT);
  const created = await request.post('/v1/profiles', { data: { name: IN_USE_PROFILE, template: 'chromium-headless' } });
  const profile = await created.json();
  const patched = await request.patch(`/v1/profiles/${profile.id}`, {
    data: { settings: { ...profile.settings, proxy: { proxy_id: proxy.id } } },
  });
  expect(patched.ok(), await patched.text()).toBe(true);

  await page.goto('/?settings=proxies');
  await ready(page);
  const row = dialog(page).getByRole('row', { name: new RegExp(`${PROXY_HOST}:${PROXY_PORT}`) });
  await row.getByRole('button', { name: 'Check now' }).click();
  await expect(row.getByText('Unhealthy')).toBeVisible();
  await expect(row.getByText(/checked/)).toBeVisible();

  await row.getByRole('button', { name: /Actions for/ }).click();
  await page.getByRole('menuitem', { name: 'Delete…' }).click();
  const inUse = page.getByRole('dialog', { name: /is in use/ });
  await expect(inUse.getByRole('link', { name: IN_USE_PROFILE })).toBeVisible();
  await inUse.getByRole('button', { name: 'Remove from 1 profile and delete' }).click();
  await expect(page.getByText('Proxy deleted')).toBeVisible();
  await expect(row).toHaveCount(0);
  const after = await (await request.get(`/v1/profiles/${profile.id}`)).json();
  expect(after.settings.proxy).toBeNull();

  await request.delete(`/v1/profiles/${profile.id}`);
  await request.delete(`/v1/proxies/${proxy.id}`);
  expect(pageErrors).toEqual([]);
});

test('logins: a bare host is accepted with a hint', async ({ page, pageErrors }) => {
  await page.goto('/logins');
  await ready(page);
  await expect(page.getByRole('heading', { level: 1, name: 'Logins' })).toBeVisible();

  await page.getByRole('button', { name: 'Add login' }).first().click();
  const add = page.getByRole('dialog', { name: 'Add login' });
  await add.getByLabel('Site').fill(SITE_HOST);
  await expect(add.getByText(`Saved as https://${SITE_HOST}`)).toBeVisible();
  await expect(add.getByRole('checkbox', { name: 'oneshot' })).toHaveCount(0);
  await add.getByLabel('Username', { exact: true }).fill('someone');
  await add.getByLabel('Password', { exact: true }).fill('hunter2');
  await add.getByRole('button', { name: 'Add login' }).click();

  await expect(page.getByText('Login added')).toBeVisible();
  await expect(page.getByRole('cell', { name: `https://${SITE_HOST}`, exact: true })).toBeVisible();
  expect(pageErrors).toEqual([]);
});

test('captcha: with no key the section says it is off and the policy is locked', async ({ page, pageErrors }) => {
  await page.goto('/?settings=captcha');
  await ready(page);

  await expect(page.getByTestId('captcha-status')).toContainText('Off — add a 2captcha key to turn it on');
  await expect(dialog(page).getByRole('link', { name: /Get a key/ })).toBeVisible();
  await expect(dialog(page).getByLabel('Default policy')).toBeDisabled();
  expect(pageErrors).toEqual([]);
});

test('captcha: saving a key flips the status at once', async ({ page, request, pageErrors }) => {
  await page.goto('/?settings=captcha');
  await ready(page);

  await dialog(page).getByLabel('API key').fill('e2e-key-1234');
  await saveBar(page).getByRole('button', { name: 'Save' }).click();
  await expect(page.getByText('Settings saved')).toBeVisible();
  await expect(page.getByTestId('captcha-status')).toContainText('On ·');
  await expect(dialog(page).getByLabel('Default policy')).toBeEnabled();

  await dialog(page).getByRole('button', { name: 'Remove…' }).click();
  await page.getByRole('button', { name: 'Remove API key' }).click();
  await expect(page.getByText('API key removed')).toBeVisible();
  await expect(page.getByTestId('captcha-status')).toContainText('Off — add a 2captcha key');
  expect(pageErrors).toEqual([]);
  await request.put('/v1/captcha/settings', { data: { api_key: null } });
});

test('storage: the backup passphrase never shows characters and a local backend has no connection test', async ({ page, pageErrors }) => {
  await page.goto('/?settings=storage');
  await ready(page);

  await expect(dialog(page).getByRole('button', { name: 'Clean now' })).toBeVisible();
  await expect(dialog(page).getByRole('button', { name: 'Test connection' })).toHaveCount(0);
  await expect(dialog(page).getByText(/ends in/)).toHaveCount(0);
  await expect(dialog(page).locator('#backup-passphrase')).toHaveAttribute('type', 'password');

  await dialog(page).getByRole('tab', { name: /Backups/ }).click();
  await expect(dialog(page).getByRole('heading', { name: 'No backups yet', exact: true })).toBeVisible();
  expect(pageErrors).toEqual([]);
});

test('capacity: the mode is a choice, and its memory thresholds live in Idle & resources', async ({ page, pageErrors }) => {
  await page.goto('/?settings=capacity');
  await ready(page);

  await dialog(page).getByRole('radio', { name: 'Fixed limits' }).click();
  await expect(dialog(page).getByRole('radio', { name: 'Fixed limits' })).toHaveAttribute('aria-checked', 'true');
  await expect(dialog(page).getByLabel('Keep this much memory free (MB)')).toHaveCount(0);

  await dialog(page).getByRole('radio', { name: 'Based on free memory' }).click();
  await expect(dialog(page).getByRole('radio', { name: 'Based on free memory' })).toHaveAttribute('aria-checked', 'true');
  await expect(saveBar(page)).toBeVisible();
  await saveBar(page).getByRole('button', { name: 'Discard' }).click();

  await dialog(page).getByRole('button', { name: 'Idle & resources', exact: true }).click();
  await expect(dialog(page).getByLabel('Keep this much memory free (MB)')).toBeAttached();
  expect(pageErrors).toEqual([]);
});

test('settings: edit a value and save it', async ({ page, request, pageErrors }) => {
  await page.goto('/?settings=resources');
  await ready(page);

  const field = dialog(page).getByLabel('Close an idle tab after (min)');
  const next = String(Number(await field.inputValue()) + 1);
  await field.fill(next);
  await saveBar(page).getByRole('button', { name: 'Save' }).click();

  await expect(page.getByText('Settings saved')).toBeVisible();
  await expect(saveBar(page)).toBeHidden();
  const { settings } = await (await request.get('/v1/settings')).json();
  expect(String(settings.idle.tab_min)).toBe(next);
  expect(pageErrors).toEqual([]);
});
