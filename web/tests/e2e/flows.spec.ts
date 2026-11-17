import { createHash } from 'node:crypto';
import { createServer } from 'node:http';
import { type AddressInfo, connect, type Socket } from 'node:net';
import type { APIRequestContext, Page } from '@playwright/test';
import { expect, ready, seed, test } from './fixtures';

const NEW_PROFILE = 'e2e-created';
const TEMPLATE_TITLE = 'Chrome — headless';
const AGENT_LABEL = 'e2e-agent';
const SLOW_SAVE_MS = 1500;
const FLOW_TIMEOUT_MS = 180_000;
const FLOW_TEMPLATE = 'e2e-flow-template';
const FLOW_PROFILE = 'e2e-flow-profile';
const FLOW_EXTENSION = 'e2e-flow-extension';
const FLOW_EXTENSION_VERSION = '1.2.3';
const EXIT_IP = '203.0.113.7';
const TRACE_BODY = `ip=${EXIT_IP}\nloc=ZZ\n`;
const CRC_POLY = 0xedb88320;

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
  await page.goto('/browsers');
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
  await expect(page).toHaveURL(/\/browsers\/[0-9A-Z]{26}$/);
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

  await page.goto(`/browsers/${instanceId}?tab=${tabId}`);
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

  await page.goto(`/browsers/${instanceId}?tab=${tabId}`);
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

  await page.goto(`/history?tab=activity&instance=${instanceId}`);
  await ready(page);
  const activity = page.getByRole('table', { name: 'Activity' });
  await expect(activity.getByText('You took over a tab').first()).toBeVisible();

  // The web client outlives its page now, so it is ended here to free the tab for the next test.
  await request.delete(`/v1/clients/${uiClient}`);
  await request.delete(`/v1/clients/${agentId}`);
});

test('tabs: open a new tab from the Tabs card', async ({ page, request, baseURL, pageErrors }) => {
  const { instanceId } = seed();
  await page.goto(`/browsers/${instanceId}`);
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
  await page.goto(`/browsers/${instanceId}`);
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
  await page.goto(`/browsers/${instanceId}`);
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

test('a busy Save button keeps keyboard focus and ignores a second activation', async ({ page, pageErrors }) => {
  await page.goto('/?settings=resources');
  await ready(page);

  let saves = 0;
  await page.route('**/v1/settings', async (route) => {
    if (route.request().method() !== 'PATCH') return route.fallback();
    saves++;
    await new Promise((r) => setTimeout(r, SLOW_SAVE_MS));
    return route.fallback();
  });

  const dialog = page.locator('dialog[open]').filter({ has: page.getByRole('navigation', { name: 'Settings sections' }) });
  const field = dialog.getByLabel('Close an idle tab after (min)');
  await field.fill(String(Number(await field.inputValue()) + 1));
  const save = dialog.getByRole('region', { name: 'Unsaved changes' }).getByRole('button', { name: 'Save' });
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
  await page.goto(`/profiles/${profileId}?tab=settings`);
  await ready(page);

  const unloadBlocked = () => page.evaluate(() => !window.dispatchEvent(new Event('beforeunload', { cancelable: true })));
  expect(await unloadBlocked()).toBe(false);

  await page.getByLabel('Name').first().fill(`${NEW_PROFILE}-renamed`);
  expect(await unloadBlocked()).toBe(true);
  await page.getByRole('navigation', { name: 'Main' }).getByRole('link', { name: 'Overview' }).click();
  await expect(page.getByRole('dialog', { name: 'Discard unsaved changes?' })).toBeVisible();
  await page.getByRole('dialog', { name: 'Discard unsaved changes?' }).getByRole('button', { name: 'Cancel' }).click();
  await expect(page).toHaveURL(new RegExp(`/profiles/${profileId}\\?tab=settings$`));
  expect(pageErrors).toEqual([]);
});

test('captcha settings warn before closing with unsaved edits', async ({ page, pageErrors }) => {
  await page.goto('/?settings=captcha');
  await ready(page);

  await page.getByLabel('Daily solve limit').fill('7');
  await page.keyboard.press('Escape');
  const guard = page.getByRole('dialog', { name: 'Discard unsaved changes?' });
  await expect(guard).toBeVisible();
  await guard.getByRole('button', { name: 'Discard' }).click();
  await expect(page.getByRole('dialog', { name: 'Captcha' })).toHaveCount(0);
  await expect(page).toHaveURL(/\/$/);
  expect(pageErrors).toEqual([]);
});

test('history files: the tab explains where agent files come from', async ({ page, pageErrors }) => {
  await page.goto('/history?tab=files');
  await ready(page);

  await expect(page.getByRole('heading', { level: 1, name: 'History' })).toBeVisible();
  await expect(page.getByRole('tab', { name: 'Files' })).toHaveAttribute('aria-selected', 'true');
  await expect(page.getByRole('combobox', { name: 'Profile' })).toBeVisible();
  await expect(page.getByText('Agents save screenshots, PDFs and recordings here')).toBeVisible();
  expect(pageErrors).toEqual([]);
});

test('browser versions: Remove explains why it is blocked and deletes nothing', async ({ page, request, pageErrors }) => {
  const count = async () => ((await (await request.get('/v1/builds')).json()).builds as unknown[]).length;
  const before = await count();
  await page.goto('/versions');
  await ready(page);

  await expect(page.getByRole('heading', { level: 1, name: 'Browser versions' })).toBeVisible();
  await page.getByRole('button', { name: /^Actions for / }).first().click();
  await page.getByRole('menuitem', { name: /Remove…/ }).click();
  const dialog = page.getByRole('dialog', { name: /can't be removed here|is in use/ });
  await expect(dialog).toContainText(/package manager|Switch them to another browser version first/);
  await dialog.getByRole('button', { name: 'OK' }).click();
  await expect(dialog).toBeHidden();
  expect(await count()).toBe(before);
  expect(pageErrors).toEqual([]);
});

function crc32(bytes: Buffer): number {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit++) crc = crc & 1 ? (crc >>> 1) ^ CRC_POLY : crc >>> 1;
  }

  return (crc ^ 0xffffffff) >>> 0;
}

// A stored (uncompressed) zip: enough for the daemon to unpack a minimal extension without a network.
function zipOf(files: Record<string, string>): Buffer {
  const local: Buffer[] = [];
  const central: Buffer[] = [];
  let offset = 0;
  for (const [name, text] of Object.entries(files)) {
    const nameBytes = Buffer.from(name);
    const data = Buffer.from(text);
    const crc = crc32(data);
    const header = Buffer.alloc(30);
    header.writeUInt32LE(0x04034b50, 0);
    header.writeUInt16LE(20, 4);
    header.writeUInt16LE(0x21, 12);
    header.writeUInt32LE(crc, 14);
    header.writeUInt32LE(data.length, 18);
    header.writeUInt32LE(data.length, 22);
    header.writeUInt16LE(nameBytes.length, 26);
    local.push(header, nameBytes, data);

    const entry = Buffer.alloc(46);
    entry.writeUInt32LE(0x02014b50, 0);
    entry.writeUInt16LE(20, 4);
    entry.writeUInt16LE(20, 6);
    entry.writeUInt16LE(0x21, 14);
    entry.writeUInt32LE(crc, 16);
    entry.writeUInt32LE(data.length, 20);
    entry.writeUInt32LE(data.length, 24);
    entry.writeUInt16LE(nameBytes.length, 28);
    entry.writeUInt32LE(offset, 42);
    central.push(entry, nameBytes);
    offset += header.length + nameBytes.length + data.length;
  }
  const directory = Buffer.concat(central);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(Object.keys(files).length, 8);
  end.writeUInt16LE(Object.keys(files).length, 10);
  end.writeUInt32LE(directory.length, 12);
  end.writeUInt32LE(offset, 16);

  return Buffer.concat([...local, directory, end]);
}

function varint(value: number): Buffer {
  const out: number[] = [];
  let rest = value;
  do {
    const byte = rest & 0x7f;
    rest >>>= 7;
    out.push(rest === 0 ? byte : byte | 0x80);
  } while (rest !== 0);

  return Buffer.from(out);
}

const protobuf = (field: number, value: Buffer) => Buffer.concat([varint((field << 3) | 2), varint(value.length), value]);

// The daemon takes a bare .zip for Camoufox; a CRX3 wrapper makes it a Chromium extension. The key is not verified.
function crx3(zip: Buffer): Buffer {
  const key = Buffer.from('e2e-fixture-public-key-der');
  const crxId = createHash('sha256').update(key).digest().subarray(0, 16);
  const header = Buffer.concat([protobuf(2, protobuf(1, key)), protobuf(10_000, protobuf(1, crxId))]);
  const prefix = Buffer.alloc(12);
  prefix.write('Cr24', 0, 'latin1');
  prefix.writeUInt32LE(3, 4);
  prefix.writeUInt32LE(header.length, 8);

  return Buffer.concat([prefix, header, zip]);
}

// An HTTP proxy that tunnels to wherever it is asked and answers every request like a Cloudflare trace page, so a proxy health check passes offline.
function startTraceProxy(): Promise<{ port: number; stop: () => Promise<void> }> {
  const sockets = new Set<Socket>();
  const server = createServer((_request, response) => {
    response.setHeader('content-type', 'text/plain');
    response.end(TRACE_BODY);
  });
  server.on('connect', (request, client: Socket, head) => {
    const [host, port] = (request.url ?? '').split(':');
    const target = connect(Number(port), host, () => {
      client.write('HTTP/1.1 200 Connection Established\r\n\r\n');
      target.write(head);
      client.pipe(target);
      target.pipe(client);
    });
    for (const socket of [client, target]) {
      sockets.add(socket);
      socket.on('close', () => sockets.delete(socket));
      socket.on('error', () => socket.destroy());
    }
  });

  return new Promise((done) =>
    server.listen(0, '127.0.0.1', () =>
      done({
        port: (server.address() as AddressInfo).port,
        stop: () =>
          new Promise((closed) => {
            for (const socket of sockets) socket.destroy();
            server.closeAllConnections();
            server.close(() => closed());
          }),
      }),
    ),
  );
}

async function supportsExtensions(request: APIRequestContext): Promise<boolean> {
  const { builds } = await (await request.get('/v1/builds')).json();

  return builds.some((b: { engine: string; capabilities: string[] }) => b.engine === 'chromium' && b.capabilities.includes('unpacked_extensions'));
}

test('first-user flow: Template with a proxy and an extension, Profile, launch, stop, then the run in History', async ({ page, request, pageErrors }) => {
  test.setTimeout(FLOW_TIMEOUT_MS);
  const withExtension = await supportsExtensions(request);
  test.info().annotations.push({ type: 'extension', description: withExtension ? 'loaded from an uploaded zip' : 'no Chromium build can load extensions here; the Template has none' });

  const upstream = await startTraceProxy();
  const before = (await (await request.get('/v1/settings')).json()).settings.proxy.check_url as string;
  const patched = await request.patch('/v1/settings', { data: { 'proxy.check_url': `http://127.0.0.1:${upstream.port}/trace` } });
  expect(patched.ok(), await patched.text()).toBe(true);

  try {
    const proxyResponse = await request.post('/v1/proxies', { data: { scheme: 'http', host: '127.0.0.1', port: upstream.port } });
    expect(proxyResponse.ok(), await proxyResponse.text()).toBe(true);
    const proxy = await proxyResponse.json();
    const checked = await request.post(`/v1/proxies/${proxy.id}/check`, { data: {} });
    expect((await checked.json()).healthy, 'the trace proxy passes its health check').toBe(true);

    if (withExtension) {
      const manifest = JSON.stringify({ manifest_version: 3, name: FLOW_EXTENSION, version: FLOW_EXTENSION_VERSION });
      const added = await request.post('/v1/extensions', { data: { source: 'local', file: crx3(zipOf({ 'manifest.json': manifest })).toString('base64') } });
      expect(added.ok(), await added.text()).toBe(true);
    }

    // Template: start from the recommended built-in, then pick the proxy and the extension.
    await page.goto('/templates');
    await ready(page);
    await page.getByRole('button', { name: 'New template' }).first().click();
    const creating = page.getByRole('dialog', { name: 'New template' });
    await creating.getByRole('radio', { name: new RegExp(TEMPLATE_TITLE) }).check();
    await creating.getByLabel('Name').fill(FLOW_TEMPLATE);
    await creating.getByRole('button', { name: 'Create template' }).click();
    await expect(page).toHaveURL(/\/templates\/[0-9A-Z]{26}$/);
    await expect(page.getByRole('heading', { level: 1, name: FLOW_TEMPLATE })).toBeVisible();

    await page.locator('#network').getByLabel('Proxy', { exact: true }).selectOption(`proxy:${proxy.id}`);
    if (withExtension) {
      await page.locator('#extensions').getByRole('checkbox', { name: FLOW_EXTENSION }).check();
    }
    await page.getByRole('region', { name: 'Unsaved changes' }).getByRole('button', { name: 'Save' }).click();
    await expect(page.getByText('Template saved')).toBeVisible();

    const saved = await (await request.get(`/v1/templates/${FLOW_TEMPLATE}`)).json();
    expect(saved.settings.proxy).toEqual({ proxy_id: proxy.id });
    expect(saved.extensions ?? []).toHaveLength(withExtension ? 1 : 0);

    // Profile: from the Template's card.
    await page.goto('/templates');
    await ready(page);
    await page.getByRole('button', { name: `Actions for ${FLOW_TEMPLATE}` }).click();
    await page.getByRole('menuitem', { name: 'New profile from this' }).click();
    const newProfile = page.getByRole('dialog', { name: 'New profile' });
    await newProfile.getByLabel('Name').fill(FLOW_PROFILE);
    await newProfile.getByRole('button', { name: 'Create profile' }).click();
    await expect(page).toHaveURL(/\/profiles\/[0-9A-Z]{26}$/);
    await expect(page.getByRole('heading', { level: 1, name: FLOW_PROFILE })).toBeVisible();
    await expect(page.getByRole('link', { name: FLOW_TEMPLATE })).toBeVisible();

    // Launch, then stop.
    await page.getByRole('button', { name: 'Launch', exact: true }).click();
    await page.getByRole('dialog', { name: `Launch ${FLOW_PROFILE}` }).getByRole('button', { name: 'Launch', exact: true }).click();
    await expect(page.getByText(`Started ${FLOW_PROFILE}`)).toBeVisible();
    await page.getByRole('button', { name: 'Open', exact: true }).click();
    await expect(page).toHaveURL(/\/browsers\/[0-9A-Z]{26}$/);
    await page.getByRole('button', { name: 'Stop', exact: true }).click();
    await page.getByRole('dialog', { name: /^Stop / }).getByRole('button', { name: 'Stop browser' }).click();
    await expect(page).toHaveURL(/\/history\/[0-9A-Z]{26}$/);

    // History: the run lists with its Template, and its tabs show what it ran with.
    await page.goto('/history');
    await ready(page);
    const row = page.getByRole('table', { name: 'Runs' }).getByRole('row', { name: new RegExp(FLOW_PROFILE) }).first();
    await expect(row).toContainText(FLOW_TEMPLATE);
    await row.getByRole('link', { name: FLOW_PROFILE }).click();
    await expect(page).toHaveURL(/\/history\/[0-9A-Z]{26}$/);
    await expect(page.getByRole('heading', { level: 1, name: FLOW_PROFILE })).toBeVisible();
    await expect(page.getByRole('tab', { name: 'Summary' })).toHaveAttribute('aria-selected', 'true');

    await page.getByRole('tab', { name: 'Settings', exact: true }).click();
    await expect(page).toHaveURL(/\?tab=settings$/);
    await expect(page.getByText('Not recorded for runs before')).toHaveCount(0);
    const proxyField = page.locator('#network').getByLabel('Proxy', { exact: true });
    await expect(proxyField).toHaveValue(`proxy:${proxy.id}`);
    await expect(proxyField).toBeDisabled();
    if (withExtension) await expect(page.getByRole('heading', { name: 'Extensions used', exact: true })).toBeVisible();

    await page.getByRole('tab', { name: 'Template & extensions' }).click();
    await expect(page.getByRole('link', { name: FLOW_TEMPLATE })).toHaveAttribute('href', `/templates/${saved.id}`);
    if (withExtension) {
      await expect(page.getByRole('listitem').filter({ hasText: FLOW_EXTENSION })).toContainText(FLOW_EXTENSION_VERSION);
    } else {
      await expect(page.getByText('No extensions were loaded.')).toBeVisible();
    }

    await page.getByRole('tab', { name: 'Network' }).click();
    await expect(page.getByText(`127.0.0.1:${upstream.port}`)).toBeVisible();
    await expect(page.getByText(EXIT_IP)).toBeVisible();
    expect(pageErrors).toEqual([]);
  } finally {
    await request.delete(`/v1/profiles/${FLOW_PROFILE}`);
    await request.delete(`/v1/templates/${FLOW_TEMPLATE}`);
    const { proxies } = await (await request.get('/v1/proxies')).json();
    for (const p of proxies.filter((p: { port: number }) => p.port === upstream.port)) await request.delete(`/v1/proxies/${p.id}`);
    await request.patch('/v1/settings', { data: { 'proxy.check_url': before } });
    await upstream.stop();
  }
});
