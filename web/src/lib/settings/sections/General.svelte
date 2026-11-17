<script lang="ts">
  import { onMount } from 'svelte';
  import { ApiError, api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import { theme, type ThemePref } from '$lib/theme.svelte';
  import Button from '$lib/ui/Button.svelte';
  import Disclosure from '$lib/ui/Disclosure.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Input from '$lib/ui/Input.svelte';
  import KeyValue from '$lib/ui/KeyValue.svelte';
  import SaveBar from '$lib/ui/SaveBar.svelte';
  import SegmentedControl from '$lib/ui/SegmentedControl.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import StatusDot from '$lib/ui/StatusDot.svelte';
  import { SettingsForm } from '../form.svelte';
  import type { components } from '$lib/api/schema';

  let { dirty = $bindable(false) }: { dirty?: boolean } = $props();

  type DoctorCheck = components['schemas']['Check'];

  const RESTART_HINT = 'Takes effect after the daemon restarts';
  const THEMES = [
    { value: 'system', label: 'System' },
    { value: 'light', label: 'Light' },
    { value: 'dark', label: 'Dark' },
  ];
  const CHECK_LABELS: Record<string, string> = {
    data_dir: 'Data folder',
    config_dir: 'Settings folder',
    listen: 'Network port',
    keyring: 'Saved-secrets key',
    xvfb: 'Hidden display support',
    display: 'Desktop display',
    dev_shm: 'Shared memory',
    user_namespaces: 'Browser sandbox',
    chromium_build: 'Browser version',
    disk_floor: 'Free disk space',
    docker: 'Docker',
    xz: 'Archive support',
  };
  const checkLabel = (name: string) => CHECK_LABELS[name] ?? name.replaceAll('_', ' ').replace(/^./, (c) => c.toUpperCase());

  const form = new SettingsForm(['server.bind', 'server.socket']);
  const info = liveQuery(() => unwrap(api.GET('/v1/system/info')), { kinds: [] });

  let checks = $state<DoctorCheck[] | null>(null);
  let doctorError = $state<ApiError | null>(null);
  let doctorLoading = $state(false);

  $effect(() => {
    dirty = form.dirty;
  });

  async function runDoctor() {
    doctorLoading = true;
    try {
      checks = (await unwrap(api.GET('/v1/system/doctor'))).checks;
      doctorError = null;
    } catch (e) {
      doctorError = e instanceof ApiError ? e : new ApiError(0, 'Unreachable', String(e), {});
    } finally {
      doctorLoading = false;
    }
  }

  onMount(() => void runDoctor());

  const bind = $derived(String(form.values['server.bind'] ?? '').trim().toLowerCase());
  const exposed = $derived.by(() => {
    const match = bind.match(/^(?:\[([^\]]+)\]|([^:]+)):\d{1,5}$/);
    if (!match) return false;
    const host = match[1] ?? match[2];
    return !(host === 'localhost' || host === '::1' || host.startsWith('127.'));
  });
</script>

<div class="section">
  <div class="field">
    <h3>Theme</h3>
    <SegmentedControl name="ui.theme" label="Theme" options={THEMES} bind:value={() => theme.pref, (pref) => void theme.set(pref as ThemePref)} disabled={theme.lockedByEnv} />
    {#if theme.lockedByEnv}<p class="note">Set by BRR_UI__THEME. Remove the environment variable to change it here.</p>{/if}
  </div>

  <div class="field">
    <h3>About</h3>
    {#if info.error}
      <EmptyState tone="danger" title="Couldn't load system info" description={info.error.message}>
        {#snippet action()}<Button onclick={info.reload}>Retry</Button>{/snippet}
      </EmptyState>
    {:else if info.data}
      <KeyValue
        items={[
          { label: 'Version', value: info.data.version },
          { label: 'Data directory', value: info.data.data_dir, mono: true, copy: true },
          { label: 'Config file', value: info.data.config_path, mono: true, copy: true },
        ]}
      />
    {:else}
      <div aria-busy="true" class="stack"><Skeleton height="20px" /><Skeleton height="20px" /><Skeleton height="20px" /></div>
    {/if}
  </div>

  <div class="field">
    <h3>Health checks</h3>
    {#if doctorError}
      <EmptyState tone="danger" title="Couldn't run the health checks" description={doctorError.message}>
        {#snippet action()}<Button onclick={runDoctor}>Retry</Button>{/snippet}
      </EmptyState>
    {:else if checks === null}
      <div aria-busy="true" class="stack"><Skeleton height="20px" /><Skeleton height="20px" /><Skeleton height="20px" /></div>
    {:else}
      <ul class="checks">
        {#each checks as check (check.name)}
          <li>
            <StatusDot tone={check.status === 'ok' ? 'success' : check.status === 'warn' ? 'warning' : 'danger'} label={checkLabel(check.name)} />
            <span class="detail">{check.detail}</span>
          </li>
        {/each}
      </ul>
      <div><Button size="sm" loading={doctorLoading} onclick={runDoctor}>Run checks again</Button></div>
    {/if}
  </div>

  <Disclosure summary="Advanced">
    {#if form.loadError}
      <EmptyState tone="danger" title="Couldn't load settings" description={form.loadError.message}>
        {#snippet action()}<Button onclick={form.load}>Retry</Button>{/snippet}
      </EmptyState>
    {:else if form.loading && form.settings === undefined}
      <div aria-busy="true" class="stack"><Skeleton height="56px" /><Skeleton height="56px" /></div>
    {:else}
      <Input id="server-bind" name="server.bind" label="Listen address" bind:value={form.values['server.bind']} readonly={form.isEnv('server.bind')} badge={form.badge('server.bind')} hint={form.hint('server.bind', `Where brr accepts connections, as host:port. ${RESTART_HINT}`)} error={form.error('server.bind')} />
      {#if exposed}
        <p class="warning">Anyone who can reach this address can control brr. brr has no authentication (ADR 0002).</p>
      {/if}
      <Input id="server-socket" name="server.socket" label="Unix socket" bind:value={form.values['server.socket']} readonly={form.isEnv('server.socket')} badge={form.badge('server.socket')} hint={form.hint('server.socket', `Optional path of a Unix socket to listen on. ${RESTART_HINT}`)} error={form.error('server.socket')} />
      <fieldset class="vault" name="vault.key_source"><KeyValue items={[{ label: 'Vault key source', value: form.settings?.vault.key_source ?? 'Not set yet' }]} /></fieldset>
    {/if}
  </Disclosure>
  <SaveBar dirty={form.dirty} saving={form.saving} disabled={form.invalid} onsave={form.save} ondiscard={form.discard} />
</div>

<style>
  .section {
    display: flex;
    flex-direction: column;
    gap: var(--space-6);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    align-items: flex-start;
  }

  .vault {
    min-inline-size: 0;
    margin: 0;
    padding: 0;
    border: 0;
  }

  h3 {
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
    font-weight: 500;
    letter-spacing: 0;
  }

  .note,
  .detail {
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }

  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    width: 100%;
  }

  .warning {
    padding: var(--space-3);
    background: var(--warning-bg);
    border: 1px solid var(--warning-border);
    border-radius: var(--radius-md);
    color: var(--warning-fg);
  }

  .checks {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    width: 100%;
  }

  .checks li {
    display: grid;
    grid-template-columns: 180px 1fr;
    align-items: baseline;
    gap: var(--space-3);
  }
</style>
