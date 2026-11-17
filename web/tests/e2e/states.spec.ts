// Last by file name: a crash leaves an unread notification that the bell checks in shell.spec.ts do not expect.
import { expect, ready, test } from './fixtures';

test('overview: a crashed browser needs attention until it is dismissed', async ({ page, request, pageErrors }) => {
  const name = 'e2e-crash';
  const profile = await (await request.post('/v1/profiles', { data: { name, template: 'chromium-headless' } })).json();
  const { instance } = await (await request.post(`/v1/profiles/${profile.id}/launch`, { data: { display: 'headless' } })).json();
  expect(instance.pid, 'a running browser has a process').toBeTruthy();
  process.kill(instance.pid, 'SIGKILL');

  await page.goto('/');
  await ready(page);
  const crashAlert = () => page.locator('li').filter({ hasText: `${name} crashed` }).filter({ has: page.getByRole('button', { name: /^Dismiss/ }) });
  const alert = crashAlert();
  await expect(alert).toContainText('killed');
  await expect(alert).not.toContainText('signal 9');

  await alert.getByRole('button', { name: /^Dismiss/ }).click();
  await expect(alert).toHaveCount(0);
  await page.reload();
  await ready(page);
  await expect(crashAlert()).toHaveCount(0);

  expect(pageErrors).toEqual([]);

  await request.delete(`/v1/profiles/${profile.id}`);
});
