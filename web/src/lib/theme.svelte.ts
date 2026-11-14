import { api, unwrap } from './api/client';
import { events } from './api/events.svelte';
import { toast } from './ui/toast.svelte';

export type ThemePref = 'system' | 'light' | 'dark';

const STORAGE_KEY = 'brr.theme';
const dark = matchMedia('(prefers-color-scheme: dark)');

class Theme {
  pref = $state<ThemePref>('system');
  resolved = $state<'light' | 'dark'>('light');
  lockedByEnv = $state(false);

  async init(): Promise<void> {
    this.apply((localStorage.getItem(STORAGE_KEY) as ThemePref | null) ?? 'system');
    dark.addEventListener('change', () => this.apply(this.pref));

    events.on(['settings.changed'], (e) => {
      const keys = (e.data as { keys?: string[] } | null)?.keys;
      if (keys?.includes('ui.theme')) void this.read();
    });
    await this.read();
  }

  async set(pref: ThemePref): Promise<void> {
    this.apply(pref);
    try {
      await unwrap(api.PATCH('/v1/settings', { body: { 'ui.theme': pref } }));
    } catch (e) {
      toast.apiError(e, "Couldn't save the theme");
    }
  }

  private async read(): Promise<void> {
    try {
      const { settings, sources } = await unwrap(api.GET('/v1/settings'));
      this.lockedByEnv = sources['ui.theme'] === 'env';
      this.apply(settings.ui?.theme ?? 'system');
    } catch {
      // The cached preference stays; the shell reports an unreachable daemon.
    }
  }

  private apply(pref: ThemePref): void {
    this.pref = pref;
    this.resolved = pref === 'system' ? (dark.matches ? 'dark' : 'light') : pref;
    document.documentElement.dataset.theme = this.resolved;
    localStorage.setItem(STORAGE_KEY, pref);
  }
}

export const theme = new Theme();
