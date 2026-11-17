<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import { clock } from '$lib/clock.svelte';
  import { relative } from '$lib/format';
  import Button from '$lib/ui/Button.svelte';
  import Card from '$lib/ui/Card.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Input from '$lib/ui/Input.svelte';
  import KeyValue from '$lib/ui/KeyValue.svelte';
  import SaveBar from '$lib/ui/SaveBar.svelte';
  import SegmentedControl from '$lib/ui/SegmentedControl.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import { confirm } from '$lib/ui/confirm.svelte';
  import { toast } from '$lib/ui/toast.svelte';
  import { SettingsForm, type FieldValue } from '../form.svelte';

  const SERVERS = {
    com: 'https://vault.bitwarden.com',
    eu: 'https://vault.bitwarden.eu',
  };
  const SERVER_OPTIONS = [
    { value: 'com', label: 'Bitwarden.com' },
    { value: 'eu', label: 'Bitwarden EU' },
    { value: 'custom', label: 'Self-hosted' },
  ];
  const INTERVAL_KEY = 'bitwarden.sync_interval_min';
  const KEY_PATH = 'Bitwarden web vault → Account settings → Security → Keys → View API key';

  let { dirty = $bindable(false) }: { dirty?: boolean } = $props();

  const form = new SettingsForm([INTERVAL_KEY], [], (key, value: FieldValue) =>
    key === INTERVAL_KEY && !(Number.isInteger(value) && (value as number) >= 1 && (value as number) <= 1440) ? 'Enter a whole number from 1 to 1440.' : null,
  );
  const status = liveQuery(() => unwrap(api.GET('/v1/bitwarden/status')), { kinds: ['bitwarden.synced', 'bitwarden.sync_failed'] });

  let server = $state('com');
  let customUrl = $state('');
  let clientId = $state('');
  let clientSecret = $state('');
  let masterPassword = $state('');
  let seeded = false;
  let connecting = $state(false);
  let syncing = $state(false);
  let disconnecting = $state(false);

  $effect(() => {
    dirty = form.dirty;
  });

  // The saved server picks the preset once; after that the user's choice wins.
  $effect(() => {
    const saved = form.settings?.bitwarden.server_url;
    if (saved === undefined || seeded) return;
    seeded = true;
    const preset = Object.entries(SERVERS).find(([, url]) => url === saved)?.[0];
    server = preset ?? 'custom';
    if (!preset) customUrl = saved;
  });

  const serverUrl = $derived(server === 'custom' ? customUrl.trim() : SERVERS[server as keyof typeof SERVERS]);
  const serverError = $derived(server === 'custom' && customUrl.trim() !== '' && !customUrl.trim().startsWith('https://') ? 'Use an https:// address.' : null);
  const canConnect = $derived(serverUrl !== '' && !serverError && clientId.trim() !== '' && clientSecret !== '' && masterPassword !== '');
  const serverDisplay = $derived(form.settings?.bitwarden.server_url ?? '');
  const connected = $derived(status.data?.connected === true);

  async function connect(event: SubmitEvent) {
    event.preventDefault();
    if (connecting || !canConnect) return;
    connecting = true;
    try {
      const out = await unwrap(
        api.POST('/v1/bitwarden/connect', {
          body: { server_url: serverUrl, client_id: clientId.trim(), client_secret: clientSecret, master_password: masterPassword },
        }),
      );
      clientSecret = '';
      masterPassword = '';
      clientId = '';
      toast.success('Bitwarden connected', `${out.items} login ${out.items === 1 ? 'item' : 'items'} available.`);
      status.reload();
      void form.load();
    } catch (e) {
      toast.apiError(e, "Couldn't connect to Bitwarden");
    } finally {
      connecting = false;
    }
  }

  async function sync() {
    if (syncing) return;
    syncing = true;
    try {
      const out = await unwrap(api.POST('/v1/bitwarden/sync', { body: {} }));
      if (out.error) toast.error("Couldn't sync Bitwarden", out.error);
      else toast.success('Bitwarden synced');
      status.reload();
    } catch (e) {
      toast.apiError(e, "Couldn't sync Bitwarden");
    } finally {
      syncing = false;
    }
  }

  async function disconnect() {
    const ok = await confirm({
      title: 'Disconnect Bitwarden?',
      description: 'brr forgets the API key and master password. Logins linked from Bitwarden stop working until you reconnect.',
      confirmLabel: 'Disconnect',
      tone: 'danger',
    });
    if (!ok || disconnecting) return;
    disconnecting = true;
    try {
      await unwrap(api.POST('/v1/bitwarden/disconnect', { body: {} }));
      toast.success('Bitwarden disconnected');
      status.reload();
    } catch (e) {
      toast.apiError(e, "Couldn't disconnect Bitwarden");
    } finally {
      disconnecting = false;
    }
  }
</script>

{#if (form.loading && form.settings === undefined) || (status.loading && !status.data)}
  <div class="stack" aria-busy="true">
    <Skeleton width="30%" />
    <Skeleton height="160px" radius="var(--radius-lg)" />
  </div>
{:else if form.loadError || (status.error && !status.data)}
  <EmptyState tone="danger" title="Couldn't load Bitwarden settings" description={form.loadError?.message ?? status.error?.message}>
    {#snippet action()}
      <Button
        onclick={() => {
          void form.load();
          status.reload();
        }}>Retry</Button
      >
    {/snippet}
  </EmptyState>
{:else if status.data}
  {@const bw = status.data}
  <div class="stack">
    {#if connected}
      <Card title="Bitwarden" description="brr decrypts linked logins in memory when it types them. Passwords and codes are never saved in brr.">
        <KeyValue
          items={[
            { label: 'Account', value: bw.email ?? '—' },
            { label: 'Status', value: bw.unlocked ? 'Unlocked' : 'Locked', tone: bw.unlocked ? undefined : ('danger' as const) },
            { label: 'Login items', value: String(bw.items) },
            { label: 'Last sync', value: bw.synced_at ? relative(bw.synced_at, clock.now) : 'Never' },
          ]}
        />
        <div class="server">
          <Input name="bitwarden.server_url" label="Server" value={serverDisplay} readonly />
        </div>
        {#if bw.error}
          <p class="error" role="alert">{bw.error}</p>
        {/if}
        <div class="actions">
          <Button loading={syncing} onclick={sync}>Sync now</Button>
          <Button variant="ghost" loading={disconnecting} onclick={disconnect}>Disconnect…</Button>
        </div>
      </Card>
    {:else}
      <Card title="Bitwarden" description="Type logins stored in Bitwarden or Vaultwarden. brr keeps the API key and master password sealed in its own vault.">
        <form id="bitwarden-form" class="fields" onsubmit={connect}>
          <SegmentedControl name="bitwarden.server_url" label="Server" bind:value={server} options={SERVER_OPTIONS} />
          {#if server === 'custom'}
            <Input label="Server URL" type="url" bind:value={customUrl} placeholder="https://vault.example.com" error={serverError} autocomplete="off" />
          {/if}
          <Input label="Client ID" bind:value={clientId} placeholder="user.xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx" autocomplete="off" required />
          <Input label="Client secret" type="password" bind:value={clientSecret} autocomplete="off" required />
          <Input label="Master password" type="password" bind:value={masterPassword} autocomplete="off" required />
          <p class="note">Find the API key in the {KEY_PATH}. Use a dedicated Bitwarden account or organization that holds only the logins you want agents to use.</p>
          {#if bw.error}
            <p class="error" role="alert">{bw.error}</p>
          {/if}
          <div class="actions">
            <Button variant="primary" type="submit" loading={connecting} disabled={!canConnect}>Connect</Button>
          </div>
        </form>
      </Card>
    {/if}

    <Card title="Sync">
      <Input
        name={INTERVAL_KEY}
        label="Sync every (min)"
        type="number"
        min={1}
        max={1440}
        step={1}
        bind:value={form.values[INTERVAL_KEY]}
        error={form.error(INTERVAL_KEY)}
        readonly={form.isEnv(INTERVAL_KEY)}
        badge={form.badge(INTERVAL_KEY)}
        hint={form.hint(INTERVAL_KEY, 'How often brr re-reads the vault, so changed passwords are picked up. 1 to 1440.')}
      />
    </Card>
  </div>
  <SaveBar dirty={form.dirty} saving={form.saving} disabled={form.invalid} onsave={form.save} ondiscard={form.discard} />
{/if}

<style>
  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding-bottom: 80px;
  }

  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    max-width: 520px;
  }

  .actions {
    display: flex;
    gap: var(--space-2);
    margin-top: var(--space-3);
  }

  .server {
    max-width: 520px;
    margin-top: var(--space-3);
  }

  .note {
    margin: 0;
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }

  .error {
    margin: var(--space-3) 0 0;
    color: var(--danger-fg);
  }
</style>
