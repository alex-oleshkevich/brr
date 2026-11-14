<script lang="ts">
  import { onMount, tick, untrack } from 'svelte';
  import { ApiError, api, unwrap } from '$lib/api/client';
  import { events } from '$lib/api/events.svelte';
  import { revealInvalid, scrollToHash } from '$lib/scroll';
  import { canonical } from '$lib/profiles/form';
  import Button from '$lib/ui/Button.svelte';
  import Card from '$lib/ui/Card.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Icon from '$lib/ui/Icon.svelte';
  import Input from '$lib/ui/Input.svelte';
  import KeyValue from '$lib/ui/KeyValue.svelte';
  import PageHeader from '$lib/ui/PageHeader.svelte';
  import SaveBar from '$lib/ui/SaveBar.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import { guardUnsaved } from '$lib/ui/unsaved';
  import StatusDot from '$lib/ui/StatusDot.svelte';
  import { confirm } from '$lib/ui/confirm.svelte';
  import { toast } from '$lib/ui/toast.svelte';
  import type { components } from '$lib/api/schema';

  type Settings = components['schemas']['Settings'];
  type Retention = components['schemas']['RetentionSettings'];
  type DoctorCheck = components['schemas']['Check'];

  const NAV = [
    { id: 'server', label: 'General' },
    { id: 'idle', label: 'Idle browsers' },
    { id: 'retention', label: 'Cleanup' },
    { id: 'oneshot', label: 'One-off jobs' },
    { id: 'backups', label: 'Backups' },
    { id: 'about', label: 'About' },
  ];

  let loading = $state(true);
  let loadError = $state<ApiError | null>(null);
  let sources = $state<Record<string, string>>({});
  let version = $state<string | null>(null);
  let dataDir = $state<string | null>(null);
  let configPath = $state<string | null>(null);

  let bind = $state('');
  let idleTab = $state<number | null>(null);
  let idleInstance = $state<number | null>(null);
  let idleGrace = $state<number | null>(null);
  let retDays = $state<number | null>(null);
  let retGb = $state<number | null>(null);
  let retDownloadMb = $state<number | null>(null);
  let retFreeGb = $state<number | null>(null);
  let retUnusedDays = $state<number | null>(null);
  let retEventsDays = $state<number | null>(null);
  let poolSize = $state<number | null>(null);
  let linger = $state<number | null>(null);
  // The passphrase stays out of `form`, so it is never copied into the snapshot string.
  let passStatus = $state<{ set: boolean; hint?: string | null }>({ set: false });
  let replacing = $state(false);
  let pass1 = $state('');
  let pass2 = $state('');
  let snapshot = $state('');
  let retSnapshot = $state<Retention | null>(null);
  let saving = $state(false);
  let changedElsewhere = $state(false);
  let active = $state('server');

  let checks = $state<DoctorCheck[] | null>(null);
  let doctorError = $state<ApiError | null>(null);
  let doctorLoading = $state(false);

  const form = $derived(
    canonical({
      bind,
      idleTab,
      idleInstance,
      idleGrace,
      retDays,
      retGb,
      retDownloadMb,
      retFreeGb,
      retUnusedDays,
      retEventsDays,
      poolSize,
      linger,
    }),
  );
  const passMismatch = $derived(pass2 !== '' && pass1 !== pass2 ? 'The passphrases do not match.' : null);
  const passInvalid = $derived(pass1 !== '' && pass1 !== pass2);
  const dirty = $derived(snapshot !== '' && (form !== snapshot || pass1 !== ''));

  const isEnv = (key: string) => sources[key] === 'env';
  const envName = (key: string) => 'BRR_' + key.replace('.', '__').toUpperCase();
  const badge = (key: string) => (isEnv(key) ? `Set by ${envName(key)}` : undefined);
  const envHint = (key: string, hint?: string) => (isEnv(key) ? 'Remove the environment variable to edit it here.' : hint);
  let serverErrors = $state<Record<string, string>>({});
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
  const err = (key: string) => serverErrors[key] ?? null;

  // A server error belongs to the value that caused it; editing anything clears it.
  $effect(() => {
    void form;
    void pass1;
    untrack(() => (serverErrors = {}));
  });
  const loopback = $derived.by(() => {
    const value = bind.trim().toLowerCase();
    const host = value.startsWith('[') ? value.slice(1, value.indexOf(']')) : value.slice(0, value.lastIndexOf(':') || undefined);
    return host === 'localhost' || host === '::1' || host.startsWith('127.');
  });

  function apply(settings: Settings, retention: Retention, src: Record<string, string>) {
    sources = src;
    bind = settings.server.bind;
    idleTab = settings.idle.tab_min;
    idleInstance = settings.idle.instance_min;
    idleGrace = settings.idle.grace_s;
    retDays = retention.artifacts_max_age_days;
    retGb = retention.artifacts_max_total_gb;
    retDownloadMb = retention.download_max_mb;
    retFreeGb = retention.free_disk_floor_gb;
    retUnusedDays = retention.unused_days;
    retEventsDays = retention.events_days;
    poolSize = settings.oneshot.pool_size;
    linger = settings.oneshot.linger_s;
    retSnapshot = retention;
    snapshot = form;
    changedElsewhere = false;
  }

  async function load() {
    loading = true;
    try {
      const [out, retention] = await Promise.all([unwrap(api.GET('/v1/settings')), unwrap(api.GET('/v1/retention'))]);
      passStatus = out.secrets['backup.passphrase'] ?? { set: false };
      apply(out.settings, retention, out.sources);
      loadError = null;
    } catch (e) {
      loadError = e instanceof ApiError ? e : new ApiError(0, 'Unreachable', String(e), {});
    } finally {
      loading = false;
    }
  }

  async function runDoctor() {
    doctorLoading = true;
    try {
      const out = await unwrap(api.GET('/v1/system/doctor'));
      checks = out.checks;
      doctorError = null;
    } catch (e) {
      doctorError = e instanceof ApiError ? e : new ApiError(0, 'Unreachable', String(e), {});
    } finally {
      doctorLoading = false;
    }
  }

  onMount(() => {
    void load().then(scrollToHash);
    void unwrap(api.GET('/v1/system/info')).then((info) => {
      version = info.version;
      dataDir = info.data_dir;
      configPath = info.config_path;
    }, (e) => toast.apiError(e, "Couldn't load system info"));
    void runDoctor();

    const off = events.on(['settings.changed'], () => {
      if (untrack(() => saving)) return;
      if (untrack(() => dirty)) changedElsewhere = true;
      else void load();
    });
    return off;
  });

  $effect(() => {
    if (loading) return;
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) if (entry.isIntersecting) active = entry.target.id;
      },
      { rootMargin: '-80px 0px -60% 0px' },
    );
    for (const el of document.querySelectorAll('.settings-page .card[id]')) observer.observe(el);
    return () => observer.disconnect();
  });

  guardUnsaved(() => dirty, 'Your edits to these settings will be lost.');

  function jump(event: MouseEvent, id: string) {
    event.preventDefault();
    document.getElementById(id)?.scrollIntoView({ behavior: 'smooth', block: 'start' });
    document.getElementById(`${id}-title`)?.focus({ preventScroll: true });
    active = id;
  }

  function cancelReplace() {
    replacing = false;
    pass1 = '';
    pass2 = '';
  }

  async function replacePassphrase() {
    replacing = true;
    await tick();
    document.getElementById('backup-passphrase')?.focus();
  }

  async function save() {
    if (saving || !retSnapshot || passInvalid) return;
    saving = true;
    serverErrors = {};
    try {
      const before = JSON.parse(snapshot) as Record<string, unknown>;
      const patch: Record<string, unknown> = {};
      const maybe = (key: string, dotted: string, value: unknown) => {
        if (!isEnv(dotted) && canonical(value) !== canonical(before[key])) patch[dotted] = value;
      };
      maybe('bind', 'server.bind', bind);
      maybe('idleTab', 'idle.tab_min', idleTab);
      maybe('idleInstance', 'idle.instance_min', idleInstance);
      maybe('idleGrace', 'idle.grace_s', idleGrace);
      maybe('poolSize', 'oneshot.pool_size', poolSize);
      maybe('linger', 'oneshot.linger_s', linger);
      if (pass1 !== '') patch['backup.passphrase'] = pass1;

      if (Object.keys(patch).length > 0) {
        await unwrap(api.PATCH('/v1/settings', { body: patch }));
      }

      const retentionBody: Record<string, unknown> = {};
      const rMaybe = (key: keyof Retention, value: unknown) => {
        if (!isEnv(`retention.${key}`) && canonical(value) !== canonical(retSnapshot?.[key])) retentionBody[key] = value;
      };
      rMaybe('artifacts_max_age_days', retDays);
      rMaybe('artifacts_max_total_gb', retGb);
      rMaybe('download_max_mb', retDownloadMb);
      rMaybe('free_disk_floor_gb', retFreeGb);
      rMaybe('unused_days', retUnusedDays);
      rMaybe('events_days', retEventsDays);
      if (Object.keys(retentionBody).length > 0) {
        retSnapshot = await unwrap(api.PUT('/v1/retention', { body: retentionBody }));
      }

      snapshot = form;
      if (pass1 !== '') {
        pass1 = '';
        pass2 = '';
        replacing = false;
        await load();
      }
      toast.success('Settings saved');
    } catch (e) {
      const field = e instanceof ApiError && e.code === 'InvalidArgument' ? (e.details as { field?: string }).field : undefined;
      if (field) {
        serverErrors = { [field]: (e as ApiError).message };
        await tick();
        if (document.querySelector('[aria-invalid="true"]')) revealInvalid();
        else toast.apiError(e, "Couldn't save the settings");
      } else {
        toast.apiError(e, "Couldn't save the settings");
      }
    } finally {
      saving = false;
    }
  }

  // The daemon would reject these too, but only with a TOML parse error.
  const MIN_ONE = ['idle.tab_min', 'idle.instance_min'];
  function localError(value: number | null, key: string): string | null {
    if (value === null) return 'Required';
    const min = MIN_ONE.includes(key) ? 1 : 0;
    return Number.isInteger(value) && value >= min ? null : `Enter a whole number, ${min} or more.`;
  }
  const required = (value: number | null, key: string) => localError(value, key) ?? err(key);
  const numbers: [number | null, string][] = $derived([
    [idleTab, 'idle.tab_min'],
    [idleInstance, 'idle.instance_min'],
    [idleGrace, 'idle.grace_s'],
    [retDays, 'retention.artifacts_max_age_days'],
    [retGb, 'retention.artifacts_max_total_gb'],
    [retDownloadMb, 'retention.download_max_mb'],
    [retFreeGb, 'retention.free_disk_floor_gb'],
    [retUnusedDays, 'retention.unused_days'],
    [retEventsDays, 'retention.events_days'],
    [poolSize, 'oneshot.pool_size'],
    [linger, 'oneshot.linger_s'],
  ]);
  const hasInvalid = $derived(numbers.some(([value, key]) => localError(value, key) !== null));
</script>

<PageHeader title="Settings">
  {#snippet actions()}
    <Button variant="primary" onclick={save} loading={saving} disabled={!dirty || passInvalid || hasInvalid}>Save</Button>
  {/snippet}
</PageHeader>

{#if loading && snapshot === ''}
  <div aria-busy="true" class="stack">
    <Skeleton width="240px" height="28px" />
    <Skeleton height="160px" radius="var(--radius-lg)" />
    <Skeleton height="160px" radius="var(--radius-lg)" />
  </div>
{:else if loadError}
  <EmptyState tone="danger" title="Couldn't load settings" description={loadError.message}>
    {#snippet action()}<Button onclick={load}>Retry</Button>{/snippet}
  </EmptyState>
{:else}
  {#if changedElsewhere}
    <p class="banner">
      <Icon name="info" /> Settings changed elsewhere.
      <Button variant="ghost" size="sm" onclick={load}>Reload</Button>
    </p>
  {/if}

  <div class="settings-page">
    <nav class="section-nav" aria-label="Settings sections">
      {#each NAV as item (item.id)}
        <a href="#{item.id}" aria-current={active === item.id ? 'true' : undefined} onclick={(e) => jump(e, item.id)}>{item.label}</a>
      {/each}
    </nav>

    <div class="content">
      <Card id="server" title="Server">
        <div class="fields">
          <Input id="server-bind" label="Listen address" bind:value={bind} readonly={isEnv('server.bind')} badge={badge('server.bind')} hint={envHint('server.bind', 'Where brr accepts connections, as host:port. Takes effect after brr restarts.')} error={err('server.bind')} />
          {#if !loopback}
            <p class="warning">Anyone who can reach this address can control brr. brr has no authentication (ADR 0002).</p>
          {/if}
        </div>
      </Card>

      <Card id="idle" title="Idle browsers">
        <div class="fields">
          <div class="triple">
            <Input id="idle-tab" label="Close an idle tab after (min)" type="number" min={1} bind:value={idleTab} readonly={isEnv('idle.tab_min')} badge={badge('idle.tab_min')} hint={envHint('idle.tab_min', "A tab nobody has used for this long is closed.")} error={required(idleTab, 'idle.tab_min')} />
            <Input id="idle-instance" label="Close an idle browser after (min)" type="number" min={1} bind:value={idleInstance} readonly={isEnv('idle.instance_min')} badge={badge('idle.instance_min')} hint={envHint('idle.instance_min', "A browser with no activity for this long is stopped.")} error={required(idleInstance, 'idle.instance_min')} />
            <Input id="idle-grace" label="Warning period before closing an idle browser (s)" type="number" min={0} bind:value={idleGrace} readonly={isEnv('idle.grace_s')} badge={badge('idle.grace_s')} hint={envHint('idle.grace_s', "Agents get this long to react before the browser closes.")} error={required(idleGrace, 'idle.grace_s')} />
          </div>
        </div>
      </Card>

      <Card id="retention" title="Cleanup">
        <div class="fields">
          <div class="pair">
            <Input label="Keep saved files for (days)" type="number" min={0} bind:value={retDays} readonly={isEnv('retention.artifacts_max_age_days')} badge={badge('retention.artifacts_max_age_days')} hint={envHint('retention.artifacts_max_age_days', "Screenshots, PDFs and downloads older than this are deleted.")} error={required(retDays, 'retention.artifacts_max_age_days')} />
            <Input label="Total size of saved files (GB)" type="number" min={0} bind:value={retGb} readonly={isEnv('retention.artifacts_max_total_gb')} badge={badge('retention.artifacts_max_total_gb')} hint={envHint('retention.artifacts_max_total_gb', "When saved files grow past this, the oldest are deleted.")} error={required(retGb, 'retention.artifacts_max_total_gb')} />
          </div>
          <div class="pair">
            <Input label="Largest download to keep (MB)" type="number" min={0} bind:value={retDownloadMb} readonly={isEnv('retention.download_max_mb')} badge={badge('retention.download_max_mb')} hint={envHint('retention.download_max_mb', "Bigger downloads are not saved.")} error={required(retDownloadMb, 'retention.download_max_mb')} />
            <Input label="Keep this much disk free (GB)" type="number" min={0} bind:value={retFreeGb} readonly={isEnv('retention.free_disk_floor_gb')} badge={badge('retention.free_disk_floor_gb')} hint={envHint('retention.free_disk_floor_gb', "Saving new files is refused when free disk space is below this.")} error={required(retFreeGb, 'retention.free_disk_floor_gb')} />
          </div>
          <div class="pair">
            <Input label="Remove unused browser versions and extensions after (days)" type="number" min={0} bind:value={retUnusedDays} readonly={isEnv('retention.unused_days')} badge={badge('retention.unused_days')} hint={envHint('retention.unused_days', "Ones no profile uses are deleted after this long.")} error={required(retUnusedDays, 'retention.unused_days')} />
            <Input label="Keep activity and captcha history for (days)" type="number" min={0} bind:value={retEventsDays} readonly={isEnv('retention.events_days')} badge={badge('retention.events_days')} hint={envHint('retention.events_days', "Older entries are deleted.")} error={required(retEventsDays, 'retention.events_days')} />
          </div>
        </div>
      </Card>

      <Card id="oneshot" title="One-off jobs">
        <div class="fields">
          <div class="pair">
            <Input id="pool-size" label="Pre-started browsers for one-off jobs" type="number" min={0} bind:value={poolSize} readonly={isEnv('oneshot.pool_size')} badge={badge('oneshot.pool_size')} hint={envHint('oneshot.pool_size', "Kept ready so one-off jobs start instantly.")} error={required(poolSize, 'oneshot.pool_size')} />
            <Input id="linger" label="Keep a one-off browser open after its job (s)" type="number" min={0} bind:value={linger} readonly={isEnv('oneshot.linger_s')} badge={badge('oneshot.linger_s')} hint={envHint('oneshot.linger_s', "Lets a quick follow-up job reuse it.")} error={required(linger, 'oneshot.linger_s')} />
          </div>
        </div>
      </Card>

      <Card id="backups" title="Backups" description="Profile backups are encrypted with this passphrase. Logins are not included; re-link them after restoring.">
        {#if passStatus.set && !replacing}
          <div class="field">
            <span class="field-label" id="passphrase-label">Backup passphrase</span>
            <div class="secret" role="group" aria-labelledby="passphrase-label">
              <span>Set</span>
              <Button variant="ghost" size="sm" onclick={replacePassphrase}>Replace</Button>
            </div>
          </div>
        {:else}
          <div class="fields">
            <Input
              id="backup-passphrase"
              label="Backup passphrase"
              type="password"
              autocomplete="new-password"
              bind:value={pass1}
              hint="Encrypts profile backups. Keep it safe: backups can only be restored with it."
            />
            <Input label="Confirm passphrase" hint="Type it again to be sure." type="password" autocomplete="new-password" bind:value={pass2} error={passMismatch} />
            {#if passStatus.set}
              <div>
                <Button variant="ghost" size="sm" onclick={cancelReplace}>Cancel</Button>
              </div>
            {/if}
          </div>
        {/if}
      </Card>

      <Card id="about" title="About">
        <div class="fields">
          <KeyValue
            items={[
              { label: 'Version', value: version },
              { label: 'Data directory', value: dataDir, mono: true, copy: true },
              { label: 'Config file', value: configPath, mono: true, copy: true },
            ]}
          />
          <div class="field">
            <span class="field-label">Health checks</span>
            {#if doctorError}
              <EmptyState tone="danger" title="Couldn't run the health checks" description={doctorError.message}>
                {#snippet action()}<Button onclick={runDoctor}>Retry</Button>{/snippet}
              </EmptyState>
            {:else if checks === null}
              <div aria-busy="true" class="checks-loading">
                <Skeleton height="20px" />
                <Skeleton height="20px" />
                <Skeleton height="20px" />
              </div>
            {:else}
              <ul class="checks">
                {#each checks as check (check.name)}
                  <li>
                    <StatusDot tone={check.status === 'ok' ? 'success' : check.status === 'warn' ? 'warning' : 'danger'} label={checkLabel(check.name)} />
                    <span class="detail">{check.detail}</span>
                  </li>
                {/each}
              </ul>
            {/if}
            {#if !doctorError}
              <div>
                <Button size="sm" loading={doctorLoading} onclick={runDoctor}>Run checks again</Button>
              </div>
            {/if}
          </div>
        </div>
      </Card>
    </div>
  </div>
  <SaveBar {dirty} {saving} disabled={passInvalid || hasInvalid} onsave={save} ondiscard={() => { cancelReplace(); void load(); }} />
{/if}

<style>
  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .banner {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-4);
    padding: var(--space-3) var(--space-4);
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    color: var(--text-secondary);
  }

  .settings-page {
    display: grid;
    grid-template-columns: 200px minmax(0, 1fr);
    gap: var(--space-6);
    align-items: start;
  }

  .section-nav {
    position: sticky;
    top: calc(var(--topbar-h) + var(--space-6));
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .section-nav a {
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-md);
    color: var(--text-secondary);
  }

  .section-nav a:hover {
    background: var(--bg-muted);
    color: var(--text);
  }

  .section-nav a[aria-current='true'] {
    background: var(--bg-muted);
    color: var(--text);
    font-weight: 500;
  }

  .content {
    display: flex;
    flex-direction: column;
    gap: var(--space-6);
    min-width: 0;
    padding-bottom: 40vh;
  }

  :global(.settings-page .card) {
    scroll-margin-top: calc(var(--topbar-h) + var(--space-6));
  }

  .secret {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    min-height: var(--control-h-md);
  }

  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    align-items: flex-start;
  }

  .pair {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-3);
  }

  .triple {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: var(--space-4);
  }

  @media (max-width: 1279px) {
    .triple {
      grid-template-columns: minmax(0, 1fr);
    }
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

  .detail {
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }

  .checks-loading {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    width: 100%;
  }

  @media (max-width: 1023px) {
    .settings-page {
      grid-template-columns: minmax(0, 1fr);
    }

    .section-nav {
      display: none;
    }
  }
</style>
