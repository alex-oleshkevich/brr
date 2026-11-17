import { page } from '$app/state';

export type SettingsSectionId = 'general' | 'proxies' | 'captcha' | 'resources' | 'logins' | 'sandbox' | 'hosts' | 'capacity' | 'storage' | 'agents' | 'updates';

export const settingsSections: { id: SettingsSectionId; label: string }[] = [
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
];

export const settingsParam = 'settings';

export const settingsHref = (id: SettingsSectionId | null) => {
  const url = new URL(page.url);
  if (id) url.searchParams.set(settingsParam, id);
  else url.searchParams.delete(settingsParam);
  return url.pathname + url.search + url.hash;
};
