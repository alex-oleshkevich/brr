<script lang="ts">
  import { ApiError, api, unwrap } from '$lib/api/client';
  import Button from '$lib/ui/Button.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import { toast } from '$lib/ui/toast.svelte';
  import type { components } from '$lib/api/schema';

  type Preview = components['schemas']['ProfileReapplyTemplateOutput'];

  let { open = $bindable(false), profileId, onapplied }: { open?: boolean; profileId: string; onapplied?: () => void } = $props();

  let preview = $state<Preview | null>(null);
  let loading = $state(false);
  let applying = $state(false);
  let error = $state<string | null>(null);

  const nothingToChange = $derived(preview !== null && preview.changes.length === 0 && preview.extensions.added.length === 0 && preview.extensions.removed.length === 0);

  const SECTION_LABELS: Record<string, string> = {
    build: 'Browser version',
    display: 'Display',
    screen: 'Display',
    proxy: 'Proxy',
    locale: 'Language & time zone',
    timezone: 'Language & time zone',
    viewport: 'Location, window & fingerprint',
    geolocation: 'Location, window & fingerprint',
    fingerprint: 'Location, window & fingerprint',
    user_agent: 'Location, window & fingerprint',
    args: 'Launch options',
    env: 'Launch options',
    prefs: 'Launch options',
    webrtc: 'Network privacy',
    geoip: 'Network privacy',
    companion: 'Extensions',
    captcha: 'Captcha',
    block_ads: 'Content',
    dismiss_cookies: 'Content',
    download_max_mb: 'Storage & retention',
    retention: 'Storage & retention',
    idle: 'Idle & pinning',
  };

  function fieldLabel(field: string): string {
    const parts = field.split('.');
    const leaf = parts[parts.length - 1].replaceAll('_', ' ');
    const section = SECTION_LABELS[parts[0]];
    const leafLabel = leaf.charAt(0).toUpperCase() + leaf.slice(1);
    return section && section.toLowerCase() !== leafLabel.toLowerCase() ? `${section} — ${leafLabel}` : leafLabel;
  }

  let library = $state<Map<string, string>>(new Map());
  const extName = (id: string) => library.get(id) ?? id;

  function describe(e: unknown): string {
    if (e instanceof ApiError && e.code === 'Conflict' && e.details.reason === 'template_gone') {
      return 'The template this profile came from was deleted, so there is nothing to re-apply.';
    }
    if (e instanceof ApiError && e.code === 'Conflict' && e.details.reason === 'profile_running') {
      return 'Stop the profile first, then try again.';
    }
    return e instanceof Error ? e.message : "Couldn't check what would change.";
  }

  function format(value: unknown): string {
    if (value === null || value === undefined) return 'Not set';
    if (typeof value === 'boolean') return value ? 'On' : 'Off';
    if (typeof value === 'string') return value === '' ? 'Not set' : value;
    if (typeof value === 'number') return String(value);
    if (Array.isArray(value)) return value.length === 0 ? 'Not set' : value.map((v) => format(v)).join(', ');
    if (typeof value === 'object') {
      const size = value as { width?: unknown; height?: unknown };
      if (typeof size.width === 'number' && typeof size.height === 'number') return `${size.width} × ${size.height}`;
      return JSON.stringify(value);
    }
    return JSON.stringify(value);
  }

  $effect(() => {
    if (!open) return;
    preview = null;
    error = null;
    loading = true;
    void unwrap(api.GET('/v1/extensions')).then(
      (out) => (library = new Map(out.extensions.map((e) => [e.id, e.name]))),
      () => {},
    );
    void unwrap(api.POST('/v1/profiles/{profile}/reapply-template', { params: { path: { profile: profileId } }, body: { dry_run: true } }))
      .then((out) => (preview = out))
      .catch((e: unknown) => (error = describe(e)))
      .finally(() => (loading = false));
  });

  async function apply() {
    if (applying || nothingToChange) return;
    applying = true;
    error = null;
    try {
      await unwrap(api.POST('/v1/profiles/{profile}/reapply-template', { params: { path: { profile: profileId } }, body: { dry_run: false } }));
      open = false;
      toast.success('Template re-applied');
      onapplied?.();
    } catch (e) {
      error = describe(e);
    } finally {
      applying = false;
    }
  }
</script>

<Dialog bind:open title="Re-apply template" description="Copy the template's settings and extensions over this profile's. This replaces the values below; profile data like cookies and history is kept.">
  {#if loading}
    <div aria-busy="true" class="stack"><Skeleton height="20px" /><Skeleton height="20px" /><Skeleton height="20px" /></div>
  {:else if error}
    <p class="error" role="alert">{error}</p>
  {:else if preview}
    {#if nothingToChange}
      <p>Nothing to change</p>
    {:else}
      {#if preview.changes.length > 0}
        <table>
          <caption class="sr-only">Settings that will change</caption>
          <thead><tr><th scope="col">Field</th><th scope="col">From</th><th scope="col">To</th></tr></thead>
          <tbody>
            {#each preview.changes as change (change.field)}
              <tr><td>{fieldLabel(change.field)}</td><td>{format(change.from)}</td><td>{format(change.to)}</td></tr>
            {/each}
          </tbody>
        </table>
      {/if}
      {#if preview.extensions.added.length > 0}
        <p><strong>Extensions added:</strong> {preview.extensions.added.map(extName).join(', ')}</p>
      {/if}
      {#if preview.extensions.removed.length > 0}
        <p><strong>Extensions removed:</strong> {preview.extensions.removed.map(extName).join(', ')}</p>
      {/if}
    {/if}
  {/if}
  {#snippet footer()}
    <Button onclick={() => (open = false)} disabled={applying}>Cancel</Button>
    <Button variant="primary" loading={applying} disabled={loading || error !== null || nothingToChange} onclick={apply}>Re-apply template</Button>
  {/snippet}
</Dialog>

<style>
  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .error {
    color: var(--danger);
  }

  table {
    width: 100%;
    border-collapse: collapse;
  }

  th,
  td {
    padding: var(--space-2);
    text-align: left;
    overflow-wrap: anywhere;
    border-bottom: 1px solid var(--border);
  }
</style>
