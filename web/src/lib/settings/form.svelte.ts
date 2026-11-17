import { onMount, tick } from 'svelte';
import { ApiError, api, unwrap } from '$lib/api/client';
import { events } from '$lib/api/events.svelte';
import { canonical } from '$lib/config/form';
import { revealInvalid } from '$lib/scroll';
import { toast } from '$lib/ui/toast.svelte';
import type { components } from '$lib/api/schema';

type Settings = components['schemas']['Settings'];
export type FieldValue = string | number | null;

const pick = (root: unknown, key: string) => key.split('.').reduce<unknown>((o, k) => (o as Record<string, unknown> | null)?.[k], root);

// Dotted setting keys held as editable values. List keys edit as one entry per line; keys that are null on the daemon edit as '' and save back as null.
export class SettingsForm {
  values = $state<Record<string, FieldValue>>({});
  settings = $state<Settings>();
  sources = $state<Record<string, string>>({});
  loading = $state(true);
  loadError = $state<ApiError | null>(null);
  saving = $state(false);
  serverErrors = $state<Record<string, { message: string; value: FieldValue }>>({});
  private snapshot = $state('');
  private numeric = new Set<string>();
  private nullable = new Set<string>();

  constructor(
    private keys: string[],
    private lists: string[] = [],
    private validate: (key: string, value: FieldValue) => string | null = () => null,
  ) {
    onMount(() => {
      void this.load();
      return events.on(['settings.changed'], () => {
        if (!this.saving && !this.dirty) void this.load();
      });
    });
  }

  get dirty() {
    return this.snapshot !== '' && canonical(this.values) !== this.snapshot;
  }

  get invalid() {
    return this.keys.some((key) => this.localError(key) !== null);
  }

  isEnv = (key: string) => this.sources[key] === 'env';
  badge = (key: string) => (this.isEnv(key) ? `Set by BRR_${key.replace('.', '__').toUpperCase()}` : undefined);
  hint = (key: string, hint?: string) => (this.isEnv(key) ? 'Remove the environment variable to edit it here.' : hint);

  private localError(key: string): string | null {
    const value = this.values[key];
    const custom = this.validate(key, value);
    if (custom) return custom;
    if (!this.numeric.has(key)) return null;
    if (value === null || value === undefined || value === '') return 'Required';
    return Number.isInteger(value) && (value as number) >= 0 ? null : 'Enter a whole number, 0 or more.';
  }

  // A server error belongs to the value that caused it; editing the field clears it.
  error = (key: string) => {
    const server = this.serverErrors[key];
    return this.localError(key) ?? (server && canonical(server.value) === canonical(this.values[key]) ? server.message : null);
  };

  load = async () => {
    this.loading = true;
    try {
      const out = await unwrap(api.GET('/v1/settings'));
      this.settings = out.settings;
      this.sources = out.sources;
      const next: Record<string, FieldValue> = {};
      for (const key of this.keys) {
        const raw = pick(out.settings, key);
        if (this.lists.includes(key)) next[key] = ((raw as string[] | undefined) ?? []).join('\n');
        else if (raw === null || raw === undefined) {
          this.nullable.add(key);
          next[key] = '';
        } else {
          if (typeof raw === 'number') this.numeric.add(key);
          next[key] = raw as FieldValue;
        }
      }
      this.values = next;
      this.snapshot = canonical(next);
      this.serverErrors = {};
      this.loadError = null;
    } catch (e) {
      this.loadError = e instanceof ApiError ? e : new ApiError(0, 'Unreachable', String(e), {});
    } finally {
      this.loading = false;
    }
  };

  discard = () => {
    this.values = JSON.parse(this.snapshot) as Record<string, FieldValue>;
    this.serverErrors = {};
  };

  save = async () => {
    if (this.saving || this.invalid) return;
    const before = JSON.parse(this.snapshot) as Record<string, FieldValue>;
    const patch: Record<string, unknown> = {};
    for (const key of this.keys) {
      const value = this.values[key];
      if (this.isEnv(key) || canonical(value) === canonical(before[key])) continue;
      if (this.lists.includes(key)) {
        patch[key] = String(value)
          .split('\n')
          .map((line) => line.trim())
          .filter(Boolean);
      } else patch[key] = value === '' && this.nullable.has(key) ? null : value;
    }
    this.saving = true;
    this.serverErrors = {};
    try {
      const out = Object.keys(patch).length > 0 ? await unwrap(api.PATCH('/v1/settings', { body: patch })) : null;
      this.snapshot = canonical(this.values);
      const restart = out?.restart_required ?? [];
      toast.success('Settings saved', restart.length ? `Takes effect after the daemon restarts: ${restart.join(', ')}.` : undefined);
    } catch (e) {
      const field = e instanceof ApiError && e.code === 'InvalidArgument' ? (e.details as { field?: string }).field : undefined;
      if (field) {
        this.serverErrors = { [field]: { message: (e as ApiError).message, value: this.values[field] } };
        await tick();
        if (document.querySelector('[aria-invalid="true"]')) revealInvalid();
        else toast.apiError(e, "Couldn't save the settings");
      } else toast.apiError(e, "Couldn't save the settings");
    } finally {
      this.saving = false;
    }
  };
}
