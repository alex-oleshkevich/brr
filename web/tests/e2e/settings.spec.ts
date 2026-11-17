import type { Locator, Page } from '@playwright/test';
import { expect, ready, test } from './fixtures';

const SECTIONS = [
  { id: 'general', label: 'General' },
  { id: 'proxies', label: 'Proxies' },
  { id: 'captcha', label: 'Captcha' },
  { id: 'resources', label: 'Idle & resources' },
  { id: 'logins', label: 'Logins' },
  { id: 'sandbox', label: 'Sandbox' },
  { id: 'hosts', label: 'Hosts' },
  { id: 'capacity', label: 'Capacity' },
  { id: 'storage', label: 'Storage' },
  { id: 'agents', label: 'Agents' },
  { id: 'updates', label: 'Updates' },
] as const;

type SectionId = (typeof SECTIONS)[number]['id'];

const DISCARD_TITLE = 'Discard unsaved changes?';
const IDLE_TAB_LABEL = 'Close an idle tab after (min)';
const RESTART_HINT = 'Takes effect after the daemon restarts';
const COVERAGE_TIMEOUT_MS = 180_000;

const settingsDialog = (page: Page) => page.locator('dialog[open]').filter({ has: page.getByRole('navigation', { name: 'Settings sections' }) });
const sectionNav = (page: Page) => settingsDialog(page).getByRole('navigation', { name: 'Settings sections' });
const discardDialog = (page: Page) => page.getByRole('dialog', { name: DISCARD_TITLE });
const params = (page: Page) => new URL(page.url()).searchParams;

test.describe.configure({ mode: 'serial' });

test('the sidebar Settings button opens the dialog over the current page', async ({ page, pageErrors }) => {
  await page.goto('/profiles');
  await ready(page);
  await expect(settingsDialog(page)).toHaveCount(0);

  await page.getByRole('button', { name: 'Settings', exact: true }).click();

  await expect(settingsDialog(page)).toBeVisible();
  await expect(page.getByRole('dialog', { name: 'General' })).toBeVisible();
  expect(new URL(page.url()).pathname).toBe('/profiles');
  expect(params(page).get('settings')).toBe('general');
  for (const { label } of SECTIONS) await expect(sectionNav(page).getByRole('button', { name: label, exact: true })).toBeVisible();
  expect(pageErrors).toEqual([]);
});

test('?settings= follows the section and is removed when the dialog closes', async ({ page, pageErrors }) => {
  await page.goto('/history?tab=files&settings=general');
  await ready(page);
  await expect(settingsDialog(page)).toBeVisible();

  for (const { id, label } of SECTIONS) {
    await sectionNav(page).getByRole('button', { name: label, exact: true }).click();
    await expect.poll(() => params(page).get('settings')).toBe(id);
    await expect(sectionNav(page).getByRole('button', { name: label, exact: true })).toHaveAttribute('aria-current', 'page');
    await expect(settingsDialog(page).getByRole('heading', { level: 2, name: label, exact: true })).toBeVisible();
  }

  await settingsDialog(page).getByRole('button', { name: 'Close settings' }).click();
  await expect(settingsDialog(page)).toHaveCount(0);
  expect(params(page).has('settings')).toBe(false);
  expect(new URL(page.url()).pathname).toBe('/history');
  expect(params(page).get('tab')).toBe('files');

  await page.goto('/?settings=storage');
  await expect(page.getByRole('dialog', { name: 'Storage' })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(settingsDialog(page)).toHaveCount(0);
  expect(params(page).has('settings')).toBe(false);
  expect(pageErrors).toEqual([]);
});

test('an unknown section id opens nothing', async ({ page, pageErrors }) => {
  await page.goto('/?settings=nope');
  await ready(page);

  await expect(settingsDialog(page)).toHaveCount(0);
  expect(pageErrors).toEqual([]);
});

test('unsaved edits: switching sections and closing both ask first', async ({ page, pageErrors }) => {
  await page.goto('/?settings=resources');
  await ready(page);
  const dialog = settingsDialog(page);
  const field = dialog.getByLabel(IDLE_TAB_LABEL);
  const original = await field.inputValue();
  const edited = String(Number(original) + 1);

  await field.fill(edited);
  await expect(dialog.getByRole('region', { name: 'Unsaved changes' })).toBeVisible();

  await sectionNav(page).getByRole('button', { name: 'Capacity', exact: true }).click();
  await expect(discardDialog(page)).toBeVisible();
  await expect(discardDialog(page)).toContainText('Idle & resources has unsaved changes');
  await discardDialog(page).getByRole('button', { name: 'Cancel' }).click();
  await expect(discardDialog(page)).toBeHidden();
  expect(params(page).get('settings')).toBe('resources');
  await expect(field).toHaveValue(edited);

  await page.keyboard.press('Escape');
  await expect(discardDialog(page)).toBeVisible();
  await discardDialog(page).getByRole('button', { name: 'Cancel' }).click();
  await expect(settingsDialog(page)).toBeVisible();
  expect(params(page).get('settings')).toBe('resources');

  await sectionNav(page).getByRole('button', { name: 'Capacity', exact: true }).click();
  await discardDialog(page).getByRole('button', { name: 'Discard' }).click();
  await expect.poll(() => params(page).get('settings')).toBe('capacity');

  await sectionNav(page).getByRole('button', { name: 'Idle & resources', exact: true }).click();
  await expect(settingsDialog(page).getByLabel(IDLE_TAB_LABEL)).toHaveValue(original);
  await expect(settingsDialog(page).getByRole('region', { name: 'Unsaved changes' })).toBeHidden();

  await sectionNav(page).getByRole('button', { name: 'Capacity', exact: true }).click();
  await expect(discardDialog(page)).toHaveCount(0);
  expect(params(page).get('settings')).toBe('capacity');
  expect(pageErrors).toEqual([]);
});

test('Esc closes only the stacked child dialog first', async ({ page, pageErrors }) => {
  await page.goto('/?settings=proxies');
  await ready(page);
  await expect(settingsDialog(page)).toBeVisible();

  await settingsDialog(page).getByRole('button', { name: 'Add proxy' }).first().click();
  const child = page.getByRole('dialog', { name: 'Add proxy' });
  await expect(child).toBeVisible();

  await page.keyboard.press('Escape');
  await expect(child).toBeHidden();
  await expect(settingsDialog(page)).toBeVisible();
  expect(params(page).get('settings')).toBe('proxies');

  await page.keyboard.press('Escape');
  await expect(settingsDialog(page)).toHaveCount(0);
  expect(params(page).has('settings')).toBe(false);
  expect(pageErrors).toEqual([]);
});

test('General says which fields need a daemon restart', async ({ page, pageErrors }) => {
  await page.goto('/?settings=general');
  await ready(page);

  const dialog = settingsDialog(page);
  await dialog.getByText('Advanced', { exact: true }).click();
  await expect(dialog.getByLabel('Listen address')).toBeVisible();
  await expect(dialog.getByText(RESTART_HINT, { exact: false }).first()).toBeVisible();
  expect(pageErrors).toEqual([]);
});

test('Agents gives copyable config and a working test', async ({ page, pageErrors }) => {
  await page.goto('/?settings=agents');
  await ready(page);

  const dialog = settingsDialog(page);
  await expect(dialog.getByLabel('MCP server entry', { exact: true })).toHaveText('{"command":"brr","args":["mcp"]}');
  await expect(dialog.getByLabel('HTTP MCP URL', { exact: true })).toHaveText(/\/mcp$/);
  await expect(dialog.getByLabel('REST base URL', { exact: true })).toHaveText(/\/v1$/);
  await expect(dialog.getByLabel('Client header name', { exact: true })).toHaveText('Brr-Client');
  await expect(dialog.getByLabel('CDP URL', { exact: true })).toHaveText(/^ws:\/\/.+\/cdp$/);

  await dialog.getByRole('button', { name: 'Test', exact: true }).click();
  await expect(page.getByRole('status').filter({ hasText: 'The REST API answered' })).toBeVisible();
  expect(pageErrors).toEqual([]);
});

type Home = { section: SectionId; backend?: 's3' | 'r2' };

const home = (section: SectionId, backend?: Home['backend']): Home => ({ section, backend });

const HOMES: Record<string, Home> = {
  'ui.theme': home('general'),
  'server.bind': home('general'),
  'server.socket': home('general'),
  'vault.key_source': home('general'),

  'proxy.health_interval_min': home('proxies'),
  'proxy.check_url': home('proxies'),

  'captcha.default_policy': home('captcha'),
  'captcha.daily_usd': home('captcha'),
  'captcha.daily_solves': home('captcha'),
  'captcha.human_wait_s': home('captcha'),
  'captcha.provider_url': home('captcha'),

  'idle.tab_min': home('resources'),
  'idle.instance_min': home('resources'),
  'idle.grace_s': home('resources'),
  'adaptive.min_free_ram_mb': home('resources'),
  'adaptive.max_load_per_core': home('resources'),
  'adaptive.reserve_mb.chromium_headless': home('resources'),
  'adaptive.reserve_mb.chromium_display': home('resources'),
  'adaptive.reserve_mb.camoufox': home('resources'),
  'adaptive.pressure_check_s': home('resources'),
  'instance.auto_restart': home('resources'),
  'instance.start_timeout_s': home('resources'),
  'instance.stop_timeout_s': home('resources'),
  'tab.console_buffer': home('resources'),
  'tab.network_buffer': home('resources'),
  'login_check.interval_min': home('resources'),

  'bitwarden.server_url': home('logins'),
  'bitwarden.sync_interval_min': home('logins'),

  'sandbox.mode': home('sandbox'),
  'sandbox.image': home('sandbox'),
  'sandbox.memory_mb': home('sandbox'),
  'sandbox.cpus': home('sandbox'),
  'sandbox.pids': home('sandbox'),
  'sandbox.shm_mb': home('sandbox'),

  'limits.mode': home('capacity'),
  'limits.max_instances': home('capacity'),
  'limits.max_instances_per_engine.chromium': home('capacity'),
  'limits.max_instances_per_engine.camoufox': home('capacity'),
  'limits.max_instances_per_tag': home('capacity'),
  'limits.max_tabs_per_instance': home('capacity'),
  'limits.max_tabs_total': home('capacity'),
  'admission.default': home('capacity'),
  'admission.queue_timeout_s': home('capacity'),
  'admission.priority': home('capacity'),
  'lease.exclusive_wait_s': home('capacity'),

  'storage.backend': home('storage'),
  'storage.endpoint': home('storage', 's3'),
  'storage.region': home('storage', 's3'),
  'storage.bucket': home('storage', 's3'),
  'storage.access_key_id': home('storage', 's3'),
  'storage.path_style': home('storage', 's3'),
  'storage.public_endpoint': home('storage', 's3'),
  'storage.r2_account_id': home('storage', 'r2'),
  'retention.artifacts_max_age_days': home('storage'),
  'retention.artifacts_max_total_gb': home('storage'),
  'retention.download_max_mb': home('storage'),
  'retention.free_disk_floor_gb': home('storage'),
  'retention.unused_days': home('storage'),
  'retention.events_days': home('storage'),
  'backup.keep': home('storage'),
  'artifacts.url_ttl_s': home('storage'),
  'results.inline_max_mb': home('storage'),

  'oneshot.profile': home('agents'),
  'oneshot.pool_size': home('agents'),
  'oneshot.linger_s': home('agents'),
  'client.mcp_http_idle_min': home('agents'),
  'client.rest_idle_min': home('agents'),
  'mcp.default_toolsets': home('agents'),

  'builds.check_at': home('updates'),
  'content.adblock_lists': home('updates'),
  'builds.github_api': home('updates'),
  'builds.edge_repo': home('updates'),
  'extensions.cws_update_url': home('updates'),
  'extensions.amo_api': home('updates'),
};

// Nested maps name their leaves (limits.max_instances_per_engine.chromium); arbitrary maps keep the parent key.
function flatten(value: unknown, prefix: string, out: string[]) {
  if (value !== null && typeof value === 'object' && !Array.isArray(value) && Object.keys(value).length > 0 && prefix !== 'limits.max_instances_per_tag') {
    for (const [key, next] of Object.entries(value)) flatten(next, prefix ? `${prefix}.${key}` : key, out);
  } else if (prefix) out.push(prefix);
}

function locate(scope: Locator, key: string): Locator {
  return scope.locator(`[name="${key}"]`);
}

test('every Settings field has exactly one home', async ({ page, request, pageErrors }) => {
  test.setTimeout(COVERAGE_TIMEOUT_MS);
  const { settings } = (await (await request.get('/v1/settings')).json()) as { settings: unknown };
  const keys: string[] = [];
  flatten(settings, '', keys);
  expect(keys.length, 'GET /v1/settings returned keys').toBeGreaterThan(0);
  expect(
    keys.filter((key) => !(key in HOMES)),
    'settings keys with no home in this test: add the field to a section, then map it here',
  ).toEqual([]);

  const homes = new Map<string, SectionId[]>(keys.map((key) => [key, []]));

  async function scan(id: SectionId, backend?: Home['backend']) {
    const scope = settingsDialog(page);
    for (const key of keys) {
      const home = HOMES[key];
      if (!home || (backend ? home.backend !== backend : home.backend !== undefined && id === 'storage')) continue;
      const expected = home.section === id;
      const found = locate(scope, key);
      if (expected) await expect.soft(found, `${key} in ${id}`).toHaveCount(1);
      if ((await found.count()) > 0) homes.get(key)!.push(id);
    }
  }

  for (const { id } of SECTIONS) {
    await page.goto(`/?settings=${id}`);
    await ready(page);
    await expect(settingsDialog(page)).toBeVisible();
    await scan(id);

    if (id === 'storage') {
      for (const [radio, backend] of [['S3-compatible', 's3'], ['Cloudflare R2', 'r2']] as const) {
        await settingsDialog(page).getByRole('radio', { name: radio }).click();
        await scan(id, backend);
      }
    }
  }

  const misplaced = keys.filter((key) => {
    const found = [...new Set(homes.get(key))];

    return found.length !== 1 || found[0] !== HOMES[key]!.section;
  });
  expect(
    misplaced.map((key) => `${key}: expected ${HOMES[key]!.section}, found in [${[...new Set(homes.get(key))].join(', ')}]`),
    'a Settings field must appear in exactly one section',
  ).toEqual([]);
  expect(pageErrors).toEqual([]);
});
