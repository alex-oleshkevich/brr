import { expect, ready, test } from './fixtures';

const PROXY_HOST = '127.0.0.1';
const PROXY_PORT = '3999';
const PROXY_USER = 'e2e-user';
const IN_USE_PROFILE = 'e2e-proxy-user';
const SITE_HOST = 'e2e-login.example.com';

test.describe.configure({ mode: 'serial' });

test('proxies: pasting a proxy URL into Host fills every field', async ({ page, pageErrors }) => {
  await page.goto('/proxies');
  await ready(page);

  await page.getByRole('button', { name: 'Add proxy' }).first().click();
  const dialog = page.getByRole('dialog', { name: 'Add proxy' });
  await dialog.getByLabel('Host').fill(`socks5://${PROXY_USER}:secret@${PROXY_HOST}:${PROXY_PORT}`);

  await expect(dialog.getByLabel('Host')).toHaveValue(PROXY_HOST);
  await expect(dialog.getByLabel('Port')).toHaveValue(PROXY_PORT);
  await expect(dialog.getByLabel('Scheme')).toHaveValue('socks5');
  await expect(dialog.getByLabel('Username (optional)')).toHaveValue(PROXY_USER);

  await dialog.getByRole('button', { name: 'Add proxy' }).click();
  await expect(page.getByText('Proxy added')).toBeVisible();
  await expect(page.getByRole('row', { name: new RegExp(`${PROXY_HOST}:${PROXY_PORT}`) })).toBeVisible();
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

  await page.goto('/proxies');
  await ready(page);
  const row = page.getByRole('row', { name: new RegExp(`${PROXY_HOST}:${PROXY_PORT}`) });
  await row.getByRole('button', { name: 'Check now' }).click();
  await expect(row.getByText('Unhealthy')).toBeVisible();
  await expect(row.getByText(/checked/)).toBeVisible();

  await row.getByRole('button', { name: /Actions for/ }).click();
  await page.getByRole('menuitem', { name: 'Delete…' }).click();
  const dialog = page.getByRole('dialog', { name: /is in use/ });
  await expect(dialog.getByRole('link', { name: IN_USE_PROFILE })).toBeVisible();
  await dialog.getByRole('button', { name: 'Remove from 1 profile and delete' }).click();
  await expect(page.getByText('Proxy deleted')).toBeVisible();
  await expect(row).toHaveCount(0);
  const after = await (await request.get(`/v1/profiles/${profile.id}`)).json();
  expect(after.settings.proxy).toBeNull();

  await request.delete(`/v1/profiles/${profile.id}`);
  await request.delete(`/v1/proxies/${proxy.id}`);
  expect(pageErrors).toEqual([]);
});

test('logins: a bare host is accepted with a hint', async ({ page, pageErrors }) => {
  await page.goto('/credentials');
  await ready(page);
  await expect(page.getByRole('heading', { level: 1, name: 'Logins' })).toBeVisible();

  await page.getByRole('button', { name: 'Add login' }).first().click();
  const dialog = page.getByRole('dialog', { name: 'Add login' });
  await dialog.getByLabel('Site').fill(SITE_HOST);
  await expect(dialog.getByText(`Saved as https://${SITE_HOST}`)).toBeVisible();
  await expect(dialog.getByRole('checkbox', { name: 'oneshot' })).toHaveCount(0);
  await dialog.getByLabel('Username', { exact: true }).fill('someone');
  await dialog.getByLabel('Password', { exact: true }).fill('hunter2');
  await dialog.getByRole('button', { name: 'Add login' }).click();

  await expect(page.getByText('Login added')).toBeVisible();
  await expect(page.getByRole('cell', { name: `https://${SITE_HOST}`, exact: true })).toBeVisible();
  expect(pageErrors).toEqual([]);
});

test('captcha: with no key the page says it is off and the policy is locked', async ({ page, pageErrors }) => {
  await page.goto('/captcha');
  await ready(page);

  await expect(page.getByTestId('captcha-status')).toContainText('Off — add a 2captcha key to turn it on');
  await expect(page.getByRole('link', { name: /Get a key/ })).toBeVisible();
  await expect(page.getByLabel('Default policy')).toBeDisabled();
  expect(pageErrors).toEqual([]);
});

test('captcha: saving a key flips the status at once', async ({ page, request, pageErrors }) => {
  await page.goto('/captcha');
  await ready(page);

  await page.getByLabel('API key').fill('e2e-key-1234');
  await page.getByRole('region', { name: 'Unsaved changes' }).getByRole('button', { name: 'Save' }).click();
  await expect(page.getByText('Settings saved')).toBeVisible();
  await expect(page.getByTestId('captcha-status')).toContainText('On ·');
  await expect(page.getByLabel('Default policy')).toBeEnabled();

  await page.getByRole('button', { name: 'Remove…' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Remove API key' }).click();
  await expect(page.getByText('API key removed')).toBeVisible();
  await expect(page.getByTestId('captcha-status')).toContainText('Off — add a 2captcha key');
  expect(pageErrors).toEqual([]);
  await request.put('/v1/captcha/settings', { data: { api_key: null } });
});

test('settings: #backups scrolls to its section and the passphrase never shows characters', async ({ page, pageErrors }) => {
  await page.goto('/settings#backups');
  await ready(page);

  await expect(page.locator('#backups')).toBeInViewport();
  await expect(page.getByText(/ends in/)).toHaveCount(0);
  expect(pageErrors).toEqual([]);
});

test('capacity: only the active mode shows its fields', async ({ page, pageErrors }) => {
  await page.goto('/limits');
  await ready(page);
  await expect(page.getByRole('heading', { level: 1, name: 'Capacity' })).toBeVisible();

  await page.getByRole('radio', { name: 'Fixed limits' }).check();
  await expect(page.getByLabel('Keep this much memory free (MB)')).toHaveCount(0);

  await page.getByRole('radio', { name: 'Based on free memory' }).check();
  await expect(page.getByLabel('Keep this much memory free (MB)')).toBeVisible();
  expect(pageErrors).toEqual([]);
});
