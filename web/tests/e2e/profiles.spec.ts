import { expect, ready, seed, test } from './fixtures';

const RECOMMENDED_TITLE = 'Chrome — headless (recommended for agents)';
const VISIBLE_TITLE = 'Chrome — visible window on this computer';
const PERSISTENT_HELP = 'Keeps logins, cookies and history between launches.';
const ADVANCED_NOTE = 'Changing these can make the browser easier to detect.';
const BUILTIN_TEMPLATE = 'chromium-headless';
const DELETE_PROFILE = 'e2e-delete-me';
const RESTORED_PROFILE = 'e2e-restored';
// Encrypting a backup runs a slow key derivation in debug builds.
const SLOW_BACKUP_MS = 90_000;
const PASSPHRASE = 'e2e passphrase';

test.describe.configure({ mode: 'serial' });

test('profiles list: a running profile offers Open browser, and cannot be deleted', async ({ page, pageErrors }) => {
  const { profileId } = seed();
  await page.goto('/profiles');
  await ready(page);

  const row = page.getByRole('row', { name: /e2e-seed/ });
  await expect(row.getByRole('link', { name: 'e2e-seed' })).toHaveAttribute('href', `/profiles/${profileId}`);
  await expect(row.getByRole('cell').nth(1)).toHaveText('Keeps data');
  await expect(row.getByRole('cell').nth(2)).toHaveAttribute('title', RECOMMENDED_TITLE);

  await row.getByRole('button', { name: 'Actions for e2e-seed' }).click();
  await expect(page.getByRole('menuitem', { name: 'Open browser' })).toBeVisible();
  await expect(page.getByRole('menuitem', { name: 'Launch', exact: true })).toHaveCount(0);
  await expect(page.getByRole('menuitem', { name: /Delete… \(stop it first\)/ })).toHaveAttribute('aria-disabled', 'true');
  expect(pageErrors).toEqual([]);
});

test('profiles list: built-in profiles sit behind Show system profiles', async ({ page, pageErrors }) => {
  await page.goto('/profiles');
  await ready(page);

  await expect(page.getByRole('link', { name: 'oneshot' })).toHaveCount(0);
  await page.getByRole('button', { name: 'Show system profiles' }).click();
  await expect(page.getByRole('heading', { name: 'System', exact: true })).toBeVisible();
  await expect(page.getByRole('link', { name: 'oneshot' })).toBeVisible();
  expect(pageErrors).toEqual([]);
});

test('templates: built-in and yours sit in two groups with plain titles', async ({ page, pageErrors }) => {
  await page.goto('/templates');
  await ready(page);

  await expect(page.getByRole('heading', { level: 1, name: 'Templates' })).toBeVisible();
  await expect(page.getByRole('heading', { level: 2, name: 'Built-in', exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { level: 2, name: 'Yours', exact: true })).toBeVisible();
  await expect(page.getByRole('link', { name: RECOMMENDED_TITLE })).toBeVisible();
  await expect(page.getByRole('link', { name: VISIBLE_TITLE })).toBeVisible();

  const card = page.locator('article').filter({ has: page.getByRole('link', { name: RECOMMENDED_TITLE }) });
  for (const fact of ['Engine', 'Display', 'Proxy', 'Extensions', 'Profiles made from it']) await expect(card.getByText(fact, { exact: true })).toBeVisible();
  await card.getByRole('button', { name: /^Actions for / }).click();
  await expect(page.getByRole('menuitem', { name: 'Duplicate', exact: true })).toBeVisible();
  await expect(page.getByRole('menuitem', { name: 'New profile from this' })).toBeVisible();
  await expect(page.getByRole('menuitem', { name: /^Delete/ })).toHaveCount(0);
  expect(pageErrors).toEqual([]);
});

test('new profile: the recommended template is preselected and the kind explains itself', async ({ page, pageErrors }) => {
  await page.goto('/profiles');
  await ready(page);

  await page.getByRole('button', { name: 'New profile' }).first().click();
  await expect(page.getByRole('radio', { name: RECOMMENDED_TITLE })).toBeChecked();
  await expect(page.getByRole('radio', { name: /Firefox stealth \(Camoufox\)/ }).first()).toBeVisible();

  await page.getByRole('button', { name: 'Next' }).click();
  await expect(page.getByText(PERSISTENT_HELP, { exact: false })).toBeVisible();
  expect(pageErrors).toEqual([]);
});

test('profile page: Launch lives in the header, the sections are tabs, advanced is collapsed', async ({ page, request, pageErrors }) => {
  const created = await request.post('/v1/profiles', { data: { name: DELETE_PROFILE, template: BUILTIN_TEMPLATE } });
  expect(created.ok(), await created.text()).toBe(true);
  const { id } = await created.json();

  await page.goto(`/profiles/${id}`);
  await ready(page);

  await expect(page.getByRole('button', { name: 'Launch', exact: true })).toBeVisible();
  // Save appears only in the sticky bar, and only once something changed.
  await expect(page.getByRole('button', { name: 'Save' })).toHaveCount(0);
  for (const tab of ['Overview', 'Settings', 'Logins & checks', 'Cookies & bookmarks', 'Backups']) await expect(page.getByRole('tab', { name: tab, exact: true })).toBeVisible();
  await expect(page.getByRole('tab', { name: 'Overview' })).toHaveAttribute('aria-selected', 'true');

  await page.getByRole('tab', { name: 'Settings', exact: true }).click();
  await expect(page).toHaveURL(/\?tab=settings$/);
  await expect(page.getByText(`Copied from Template ${RECOMMENDED_TITLE}`, { exact: false })).toBeVisible();
  await expect(page.locator('details#advanced')).not.toHaveAttribute('open', '');
  await expect(page.getByText(ADVANCED_NOTE)).toBeVisible();

  await page.getByLabel('Tags').fill('e2e');
  await expect(page.getByRole('region', { name: 'Unsaved changes' }).getByRole('button', { name: 'Save' })).toBeVisible();
  await page.getByRole('region', { name: 'Unsaved changes' }).getByRole('button', { name: 'Discard' }).click();

  await page.getByRole('button', { name: 'Launch', exact: true }).click();
  await expect(page.getByRole('dialog', { name: /Launch/ })).toBeVisible();
  await page.keyboard.press('Escape');
  expect(pageErrors).toEqual([]);
});

test('profile page: a running profile shows Running · Open browser', async ({ page, pageErrors }) => {
  const { profileId, instanceId } = seed();
  await page.goto(`/profiles/${profileId}`);
  await ready(page);

  const open = page.getByRole('link', { name: /Running · Open browser/ });
  await expect(open).toHaveAttribute('href', `/browsers/${instanceId}`);
  await expect(page.getByRole('button', { name: 'Launch', exact: true })).toHaveCount(0);
  expect(pageErrors).toEqual([]);
});

test('profile page: saving a running profile says the changes apply after restart', async ({ page, pageErrors }) => {
  const { profileId } = seed();
  await page.goto(`/profiles/${profileId}?tab=settings`);
  await ready(page);

  await page.getByLabel('Tags').fill('e2e-saved');
  await page.getByRole('region', { name: 'Unsaved changes' }).getByRole('button', { name: 'Save' }).click();

  await expect(page.getByText('Saved. e2e-seed is running — changes apply after restart.')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Restart now' })).toBeVisible();
  expect(pageErrors).toEqual([]);
});

test('backups tab: setting the passphrase enables backups', async ({ page, request, pageErrors }) => {
  const { profiles } = await (await request.get('/v1/profiles')).json();
  const { id } = profiles.find((p: { name: string }) => p.name === DELETE_PROFILE);

  await page.goto(`/profiles/${id}?tab=backups`);
  await ready(page);

  await expect(page.getByRole('tab', { name: 'Backups' })).toHaveAttribute('aria-selected', 'true');
  await expect(page.getByRole('button', { name: 'Back up now' })).toBeDisabled();
  await page.getByRole('button', { name: 'Set passphrase…' }).click();
  const dialog = page.getByRole('dialog', { name: 'Set a backup passphrase' });
  await dialog.getByLabel('Passphrase', { exact: true }).fill(PASSPHRASE);
  await dialog.getByLabel('Confirm passphrase').fill(PASSPHRASE);
  await dialog.getByRole('button', { name: 'Set passphrase' }).click();

  await expect(page.getByText('Backup passphrase set')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Back up now' })).toBeEnabled();
  await expect(page.getByText("Logins aren't included in backups").first()).toBeVisible();
  expect(pageErrors).toEqual([]);
});

test('backups tab: back up, then restore under a name you choose', async ({ page, request, pageErrors }) => {
  test.setTimeout(SLOW_BACKUP_MS * 2);
  const { profiles } = await (await request.get('/v1/profiles')).json();
  const { id } = profiles.find((p: { name: string }) => p.name === DELETE_PROFILE);

  await page.goto(`/profiles/${id}?tab=backups`);
  await ready(page);

  await page.getByRole('button', { name: 'Back up now' }).click();
  await expect(page.getByRole('progressbar', { name: /Backing up/ })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Back up now' })).toHaveAttribute('aria-busy', 'true');
  await expect(page.getByText('Backup created')).toBeVisible({ timeout: SLOW_BACKUP_MS });

  await page.getByRole('button', { name: /Actions for the backup from/ }).first().click();
  await page.getByRole('menuitem', { name: 'Restore as new profile…' }).click();
  const dialog = page.getByRole('dialog', { name: 'Restore as new profile' });
  const name = dialog.getByLabel('New profile name');
  await expect(name).toHaveValue(new RegExp(`^${DELETE_PROFILE}-restored-\\d{4}-\\d{2}-\\d{2}$`));
  await name.fill(RESTORED_PROFILE);
  await dialog.getByRole('button', { name: 'Restore' }).click();
  await expect(dialog.getByRole('progressbar', { name: 'Restoring' })).toBeVisible();

  await expect(page).toHaveURL(/\/profiles\/[0-9A-Z]{26}$/, { timeout: SLOW_BACKUP_MS });
  await expect(page.getByRole('heading', { level: 1, name: RESTORED_PROFILE })).toBeVisible();
  await request.delete(`/v1/profiles/${RESTORED_PROFILE}`);
  expect(pageErrors).toEqual([]);
});

test('settings storage backups: restore pre-fills a valid name, rejects an invalid one inline, and creates the profile', async ({ page, request, pageErrors }) => {
  test.setTimeout(SLOW_BACKUP_MS);
  await page.goto('/?settings=storage');
  await ready(page);
  const settings = page.locator('dialog[open]').filter({ has: page.getByRole('navigation', { name: 'Settings sections' }) });
  await settings.getByRole('tab', { name: /Backups/ }).click();

  const row = settings.getByRole('row', { name: new RegExp(DELETE_PROFILE) }).first();
  await expect(row.getByText(/deleted profile/)).toHaveCount(0);
  await row.getByRole('button', { name: `Actions for the ${DELETE_PROFILE} backup` }).click();
  await page.getByRole('menuitem', { name: 'Restore as new profile…' }).click();

  const dialog = page.getByRole('dialog', { name: 'Restore as new profile' });
  const name = dialog.getByLabel('New profile name');
  await expect(name).toHaveValue(new RegExp(`^${DELETE_PROFILE}-restored-\\d{4}-\\d{2}-\\d{2}$`));

  await name.fill('not valid!');
  await expect(dialog.getByText(/Use 1 to 64 characters/)).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Restore profile' })).toBeDisabled();

  await name.fill(RESTORED_PROFILE);
  await dialog.getByRole('button', { name: 'Restore profile' }).click();
  await expect(page.getByText(`Restored as ${RESTORED_PROFILE}`)).toBeVisible({ timeout: SLOW_BACKUP_MS });

  const restored = await request.get(`/v1/profiles/${RESTORED_PROFILE}`);
  expect(restored.ok()).toBe(true);
  await request.delete(`/v1/profiles/${RESTORED_PROFILE}`);
  expect(pageErrors).toEqual([]);
});

test('delete profile: the dialog lists the consequences', async ({ page, request, pageErrors }) => {
  const { profiles } = await (await request.get('/v1/profiles')).json();
  const { id } = profiles.find((p: { name: string }) => p.name === DELETE_PROFILE);

  await page.goto(`/profiles/${id}`);
  await ready(page);

  await page.getByRole('button', { name: 'More actions' }).click();
  await page.getByRole('menuitem', { name: 'Delete…' }).click();
  const dialog = page.getByRole('dialog', { name: `Delete ${DELETE_PROFILE}?` });
  await expect(dialog.getByText(/of browser data/)).toBeVisible();
  await expect(dialog.getByText(/no backups|backups? (is|are) kept/)).toBeVisible();

  await dialog.getByRole('button', { name: 'Delete profile' }).click();
  await expect(page).toHaveURL(/\/profiles$/);
  await expect(page.getByRole('link', { name: DELETE_PROFILE })).toHaveCount(0);
  expect(pageErrors).toEqual([]);
});
