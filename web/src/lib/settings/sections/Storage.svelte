<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { ApiError, api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import { clock } from '$lib/clock.svelte';
  import type { components } from '$lib/api/schema';
  import { absolute, bytes, count, relative } from '$lib/format';
  import { canonical } from '$lib/config/form';
  import BackendForm from '$lib/storage/BackendForm.svelte';
  import CleanDialog from '$lib/storage/CleanDialog.svelte';
  import RestoreDialog from '$lib/storage/RestoreDialog.svelte';
  import Badge from '$lib/ui/Badge.svelte';
  import Button from '$lib/ui/Button.svelte';
  import Card from '$lib/ui/Card.svelte';
  import Disclosure from '$lib/ui/Disclosure.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Input from '$lib/ui/Input.svelte';
  import Menu from '$lib/ui/Menu.svelte';
  import Progress from '$lib/ui/Progress.svelte';
  import SaveBar from '$lib/ui/SaveBar.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import Table from '$lib/ui/Table.svelte';
  import Tabs from '$lib/ui/Tabs.svelte';
  import { confirm } from '$lib/ui/confirm.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  type Backup = components['schemas']['Backup'];
  type Retention = components['schemas']['RetentionSettings'];
  type Num = number | null;

  const FILE_KINDS = ['artifact.*', 'download.completed', 'download.cancelled', 'retention.sweep_completed'];
  const KIND_LABELS: Record<string, string> = {
    download: 'Downloads',
    screenshot: 'Screenshots',
    pdf: 'PDFs',
    mhtml: 'MHTML',
    har: 'HAR',
    html: 'HTML',
    markdown: 'Markdown',
  };
  const BACKEND_NAMES: Record<string, string> = { local: 'This computer', s3: 'S3 storage' };
  const kindLabel = (k: string) => KIND_LABELS[k] ?? k;

  let { dirty = $bindable(false) }: { dirty?: boolean } = $props();

  let cleaning = $state(false);
  let view = $state('overview');
  let tab = $state('profiles');
  let restoring = $state<Backup | null>(null);
  let backendDirty = $state(false);

  const backend = liveQuery(() => unwrap(api.GET('/v1/storage/backend')), { kinds: [] });
  const usage = liveQuery(() => unwrap(api.GET('/v1/storage/usage')), {
    kinds: [...FILE_KINDS, 'build.*', 'extension.*', 'backup.*'],
  });
  const backups = liveQuery(async () => (await unwrap(api.GET('/v1/backups'))).backups, { kinds: ['backup.*', 'profile.*'] });
  const profiles = liveQuery(async () => (await unwrap(api.GET('/v1/profiles'))).profiles, { kinds: ['profile.*'] });
  const general = liveQuery(() => unwrap(api.GET('/v1/settings')), { kinds: ['settings.changed'] });
  const retentionQuery = liveQuery(() => unwrap(api.GET('/v1/retention')), { kinds: ['settings.changed'] });

  const liveNames = $derived(new Set((profiles.data ?? []).map((p) => p.name)));

  type Row = { key: string; name: string; bytes: number; count?: number };
  const panels = $derived.by<Record<string, Row[]>>(() => {
    const u = usage.data;
    const sorted = (rows: Row[]) => rows.sort((a, b) => b.bytes - a.bytes);
    return {
      profiles: sorted((u?.profiles ?? []).map((r) => ({ key: r.profile_id, name: r.name, bytes: r.bytes }))),
      builds: sorted((u?.builds ?? []).map((r) => ({ key: r.build_id, name: r.label, bytes: r.bytes }))),
      extensions: sorted((u?.extensions ?? []).map((r) => ({ key: r.extension_id, name: r.name, bytes: r.bytes }))),
      artifacts: sorted((u?.artifacts ?? []).map((r) => ({ key: r.kind, name: kindLabel(r.kind), bytes: r.bytes, count: r.count }))),
    };
  });
  const total = $derived(
    usage.data ? Object.values(panels).reduce((sum, rows) => sum + rows.reduce((n, r) => n + r.bytes, 0), 0) : null,
  );
  const tabItems = [
    { id: 'profiles', label: 'Profiles' },
    { id: 'builds', label: 'Browser versions' },
    { id: 'extensions', label: 'Extensions' },
    { id: 'artifacts', label: 'Files' },
  ];
  const usageEmpty: Record<string, string> = {
    profiles: 'No profile data yet',
    builds: 'No browser versions installed',
    extensions: 'No extensions installed',
    artifacts: 'No files stored',
  };
  const columns = $derived(
    tab === 'artifacts'
      ? [
          { key: 'name', label: 'Type', width: '30%' },
          { key: 'count', label: 'Count', width: '15%', align: 'right' as const },
          { key: 'size', label: 'Size', width: '15%', align: 'right' as const },
          { key: 'share', label: 'Share', width: '40%' },
        ]
      : [
          { key: 'name', label: 'Name', width: '45%' },
          { key: 'size', label: 'Size', width: '15%', align: 'right' as const },
          { key: 'share', label: 'Share', width: '40%' },
        ],
  );
  const rows = $derived(usage.data ? panels[tab] : undefined);
  const largest = $derived(Math.max(1, ...(rows ?? []).map((r) => r.bytes)));

  async function removeBackup(b: Backup) {
    const ok = await confirm({
      title: `Delete the ${b.profile_name} backup?`,
      description: `The ${bytes(b.size_bytes)} backup from ${absolute(b.created_at)} is deleted from storage. This can't be undone.`,
      confirmLabel: 'Delete backup',
      tone: 'danger',
    });
    if (!ok) return;
    try {
      await unwrap(api.DELETE('/v1/backups/{backup_id}', { params: { path: { backup_id: b.id } } }));
      toast.success('Backup deleted');
      backups.reload();
      usage.reload();
    } catch (e) {
      toast.apiError(e, "Couldn't delete the backup");
    }
  }

  function swept() {
    backups.reload();
    usage.reload();
  }

  let retDays = $state<Num>(null);
  let retGb = $state<Num>(null);
  let retDownloadMb = $state<Num>(null);
  let retFreeGb = $state<Num>(null);
  let retUnusedDays = $state<Num>(null);
  let retEventsDays = $state<Num>(null);
  let backupKeep = $state<Num>(null);
  let urlTtl = $state<Num>(null);
  let inlineMax = $state<Num>(null);
  let passStatus = $state<{ set: boolean; hint?: string | null }>({ set: false });
  let replacing = $state(false);
  let pass1 = $state('');
  let pass2 = $state('');
  let snapshot = $state('');
  let retSnapshot = $state<Retention | null>(null);
  let saving = $state(false);
  let changedElsewhere = $state(false);
  let serverErrors = $state<Record<string, string>>({});
  let loaded = '';

  const form = $derived(
    canonical({ retDays, retGb, retDownloadMb, retFreeGb, retUnusedDays, retEventsDays, backupKeep, urlTtl, inlineMax }),
  );
  const passMismatch = $derived(pass2 !== '' && pass1 !== pass2 ? 'The passphrases do not match.' : null);
  const passInvalid = $derived(pass1 !== '' && pass1 !== pass2);
  const formDirty = $derived(snapshot !== '' && (form !== snapshot || pass1 !== ''));
  const isDirty = $derived(formDirty || backendDirty);
  $effect(() => {
    dirty = isDirty;
  });

  const sources = $derived(general.data?.sources ?? {});
  const isEnv = (key: string) => sources[key] === 'env';
  const envName = (key: string) => 'BRR_' + key.replace('.', '__').toUpperCase();
  const badge = (key: string) => (isEnv(key) ? `Set by ${envName(key)}` : undefined);
  const envHint = (key: string, hint?: string) => (isEnv(key) ? 'Remove the environment variable to edit it here.' : hint);
  const err = (key: string) => serverErrors[key] ?? null;

  const settingsLoading = $derived((general.loading || retentionQuery.loading) && snapshot === '');
  const settingsError = $derived(snapshot === '' ? (general.error ?? retentionQuery.error) : null);

  function applySettings() {
    const out = general.data;
    const retention = retentionQuery.data;
    if (!out || !retention) return;
    passStatus = out.secrets['backup.passphrase'] ?? { set: false };
    retDays = retention.artifacts_max_age_days;
    retGb = retention.artifacts_max_total_gb;
    retDownloadMb = retention.download_max_mb;
    retFreeGb = retention.free_disk_floor_gb;
    retUnusedDays = retention.unused_days;
    retEventsDays = retention.events_days;
    backupKeep = out.settings.backup.keep;
    urlTtl = out.settings.artifacts.url_ttl_s;
    inlineMax = out.settings.results.inline_max_mb;
    retSnapshot = retention;
    snapshot = form;
    loaded = JSON.stringify([out.settings, retention]);
    changedElsewhere = false;
    serverErrors = {};
  }

  $effect(() => {
    const out = general.data;
    const retention = retentionQuery.data;
    if (!out || !retention || JSON.stringify([out.settings, retention]) === loaded) return;
    if (snapshot === '' || !untrack(() => formDirty)) untrack(applySettings);
    else changedElsewhere = true;
  });

  $effect(() => {
    void form;
    void pass1;
    untrack(() => (serverErrors = {}));
  });

  function localError(value: Num): string | null {
    if (value === null) return 'Required';
    return Number.isInteger(value) && value >= 0 ? null : 'Enter a whole number, 0 or more.';
  }
  const required = (value: Num, key: string) => localError(value) ?? err(key);
  const numbers: [Num, string][] = $derived([
    [retDays, 'retention.artifacts_max_age_days'],
    [retGb, 'retention.artifacts_max_total_gb'],
    [retDownloadMb, 'retention.download_max_mb'],
    [retFreeGb, 'retention.free_disk_floor_gb'],
    [retUnusedDays, 'retention.unused_days'],
    [retEventsDays, 'retention.events_days'],
    [backupKeep, 'backup.keep'],
    [urlTtl, 'artifacts.url_ttl_s'],
    [inlineMax, 'results.inline_max_mb'],
  ]);
  const hasInvalid = $derived(numbers.some(([value]) => localError(value) !== null));

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

  function reloadSettings() {
    general.reload();
    retentionQuery.reload();
  }

  async function save() {
    if (saving || !retSnapshot || passInvalid || hasInvalid) return;
    saving = true;
    serverErrors = {};
    try {
      const before = JSON.parse(snapshot) as Record<string, unknown>;
      const patch: Record<string, unknown> = {};
      const maybe = (key: string, dotted: string, value: unknown) => {
        if (!isEnv(dotted) && canonical(value) !== canonical(before[key])) patch[dotted] = value;
      };
      maybe('backupKeep', 'backup.keep', backupKeep);
      maybe('urlTtl', 'artifacts.url_ttl_s', urlTtl);
      maybe('inlineMax', 'results.inline_max_mb', inlineMax);
      if (pass1 !== '') patch['backup.passphrase'] = pass1;
      if (Object.keys(patch).length > 0) await unwrap(api.PATCH('/v1/settings', { body: patch }));

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
      loaded = '';
      if (pass1 !== '') {
        pass1 = '';
        pass2 = '';
        replacing = false;
      }
      general.reload();
      retentionQuery.reload();
      toast.success('Settings saved');
    } catch (e) {
      const field = e instanceof ApiError && e.code === 'InvalidArgument' ? (e.details as { field?: string }).field : undefined;
      if (field) serverErrors = { [field]: (e as ApiError).message };
      else toast.apiError(e, "Couldn't save the settings");
    } finally {
      saving = false;
    }
  }
</script>

<CleanDialog bind:open={cleaning} onswept={swept} />
<RestoreDialog bind:backup={restoring} onrestored={() => backups.reload()} />

<div class="stack">
  <div class="head">
    <Button onclick={() => (cleaning = true)}>Clean up…</Button>
  </div>

  <Tabs
    items={[
      { id: 'overview', label: 'Overview' },
      { id: 'backups', label: 'Backups', count: backups.data?.length },
    ]}
    bind:value={view}
    label="Storage"
    idPrefix="storage"
  />

  {#if view === 'overview'}
    <div role="tabpanel" id="storage-panel-overview" aria-labelledby="storage-tab-overview" class="stack">
      <Card title="Backend">
        {#if backend.loading}
          <div aria-busy="true" class="skeletons">
            <Skeleton width="40%" height="32px" radius="var(--radius-md)" />
            <Skeleton height="64px" radius="var(--radius-md)" />
          </div>
        {:else if backend.error && !backend.data}
          <EmptyState tone="danger" title="Couldn't load the storage backend" description={backend.error.message}>
            {#snippet action()}<Button onclick={() => backend.reload()}>Retry</Button>{/snippet}
          </EmptyState>
        {:else if backend.data}
          <BackendForm backend={backend.data} bind:dirty={backendDirty} onsaved={() => backend.reload()} />
        {/if}
      </Card>

      <Card title="Usage">
        {#snippet actions()}
          {#if total !== null}<span class="total" aria-label="Total used">{bytes(total)}</span>{/if}
        {/snippet}
        <div class="usage">
          <Tabs items={tabItems} bind:value={tab} label="Usage by" idPrefix="usage" />
          <div role="tabpanel" id="usage-panel-{tab}" aria-labelledby="usage-tab-{tab}">
            <Table
              caption="Usage"
              {columns}
              {rows}
              key={(row: Row) => row.key}
              loading={usage.loading}
              error={usage.error}
              onretry={() => usage.reload()}
              empty={{ icon: 'hard-drive', title: usageEmpty[tab] }}
            >
              {#snippet row(r: Row)}
                <td class="truncate">{r.name}</td>
                {#if tab === 'artifacts'}<td class="num right">{count(r.count ?? 0)}</td>{/if}
                <td class="num right">{bytes(r.bytes)}</td>
                <td><Progress value={(r.bytes / largest) * 100} label="{r.name} share" /></td>
              {/snippet}
            </Table>
          </div>
        </div>
      </Card>

      {#if changedElsewhere}
        <p class="banner" role="status">
          <span>Settings changed elsewhere.</span>
          <Button variant="ghost" size="sm" onclick={() => applySettings()}>Reload</Button>
        </p>
      {/if}

      <Card title="Cleanup" description="Clean up previews what is past these rules and deletes nothing until you confirm.">
        {#snippet actions()}
          <Button size="sm" onclick={() => (cleaning = true)}>Clean now</Button>
        {/snippet}
        {#if settingsLoading}
          <div aria-busy="true" class="skeletons">
            <Skeleton height="64px" radius="var(--radius-md)" />
            <Skeleton height="64px" radius="var(--radius-md)" />
          </div>
        {:else if settingsError}
          <EmptyState tone="danger" title="Couldn't load cleanup settings" description={settingsError.message}>
            {#snippet action()}<Button onclick={reloadSettings}>Retry</Button>{/snippet}
          </EmptyState>
        {:else}
          <div class="fields">
            <div class="pair">
              <Input name="retention.artifacts_max_age_days" label="Keep saved files for (days)" type="number" min={0} bind:value={retDays} readonly={isEnv('retention.artifacts_max_age_days')} badge={badge('retention.artifacts_max_age_days')} hint={envHint('retention.artifacts_max_age_days', 'Screenshots, PDFs and downloads older than this are deleted.')} error={required(retDays, 'retention.artifacts_max_age_days')} />
              <Input name="retention.artifacts_max_total_gb" label="Total size of saved files (GB)" type="number" min={0} bind:value={retGb} readonly={isEnv('retention.artifacts_max_total_gb')} badge={badge('retention.artifacts_max_total_gb')} hint={envHint('retention.artifacts_max_total_gb', 'When saved files grow past this, the oldest are deleted.')} error={required(retGb, 'retention.artifacts_max_total_gb')} />
            </div>
            <div class="pair">
              <Input name="retention.download_max_mb" label="Largest download to keep (MB)" type="number" min={0} bind:value={retDownloadMb} readonly={isEnv('retention.download_max_mb')} badge={badge('retention.download_max_mb')} hint={envHint('retention.download_max_mb', 'Bigger downloads are not saved.')} error={required(retDownloadMb, 'retention.download_max_mb')} />
              <Input name="retention.free_disk_floor_gb" label="Keep this much disk free (GB)" type="number" min={0} bind:value={retFreeGb} readonly={isEnv('retention.free_disk_floor_gb')} badge={badge('retention.free_disk_floor_gb')} hint={envHint('retention.free_disk_floor_gb', 'Saving new files is refused when free disk space is below this.')} error={required(retFreeGb, 'retention.free_disk_floor_gb')} />
            </div>
            <div class="pair">
              <Input name="retention.unused_days" label="Remove unused browser versions and extensions after (days)" type="number" min={0} bind:value={retUnusedDays} readonly={isEnv('retention.unused_days')} badge={badge('retention.unused_days')} hint={envHint('retention.unused_days', 'Ones no profile uses are deleted after this long.')} error={required(retUnusedDays, 'retention.unused_days')} />
              <Input name="retention.events_days" label="Keep history and activity for (days)" type="number" min={0} bind:value={retEventsDays} readonly={isEnv('retention.events_days')} badge={badge('retention.events_days')} hint={envHint('retention.events_days', 'Older entries are deleted.')} error={required(retEventsDays, 'retention.events_days')} />
            </div>
          </div>
        {/if}
      </Card>

      <Card title="Backups" description="Profile backups are encrypted with this passphrase. Logins are not included; re-link them after restoring.">
        {#if settingsLoading}
          <div aria-busy="true" class="skeletons">
            <Skeleton height="64px" radius="var(--radius-md)" />
          </div>
        {:else if settingsError}
          <EmptyState tone="danger" title="Couldn't load backup settings" description={settingsError.message}>
            {#snippet action()}<Button onclick={reloadSettings}>Retry</Button>{/snippet}
          </EmptyState>
        {:else}
          <div class="fields">
            {#if passStatus.set && !replacing}
              <div class="field">
                <span class="field-label" id="passphrase-label">Backup passphrase</span>
                <div class="secret" role="group" aria-labelledby="passphrase-label">
                  <span>Set</span>
                  <Button variant="ghost" size="sm" onclick={replacePassphrase}>Replace</Button>
                </div>
              </div>
            {:else}
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
            {/if}
            <Input name="backup.keep" label="Backups to keep per profile" type="number" min={0} bind:value={backupKeep} readonly={isEnv('backup.keep')} badge={badge('backup.keep')} hint={envHint('backup.keep', 'Older backups of a profile are deleted after a new one is made.')} error={required(backupKeep, 'backup.keep')} />
          </div>
        {/if}
      </Card>

      {#if !settingsLoading && !settingsError}
        <Disclosure summary="Advanced">
          <div class="pair">
            <Input name="artifacts.url_ttl_s" label="Download link lifetime (s)" type="number" min={0} bind:value={urlTtl} readonly={isEnv('artifacts.url_ttl_s')} badge={badge('artifacts.url_ttl_s')} hint={envHint('artifacts.url_ttl_s', 'How long a link to a saved file stays valid.')} error={required(urlTtl, 'artifacts.url_ttl_s')} />
            <Input name="results.inline_max_mb" label="Largest result shown inline (MB)" type="number" min={0} bind:value={inlineMax} readonly={isEnv('results.inline_max_mb')} badge={badge('results.inline_max_mb')} hint={envHint('results.inline_max_mb', 'Bigger results are saved as files instead of returned inline.')} error={required(inlineMax, 'results.inline_max_mb')} />
          </div>
        </Disclosure>
      {/if}
    </div>
  {:else}
    <div role="tabpanel" id="storage-panel-backups" aria-labelledby="storage-tab-backups" class="stack">
      <Table
        caption="Backups"
        columns={[
          { key: 'profile', label: 'Profile', width: '30%' },
          { key: 'created', label: 'Backed up', width: '22%' },
          { key: 'size', label: 'Size', width: '13%', align: 'right' },
          { key: 'where', label: 'Stored in', width: '27%' },
          { key: 'actions', label: 'Actions', width: '48px', srOnly: true },
        ]}
        rows={backups.data}
        key={(row: Backup) => row.id}
        loading={backups.loading}
        error={backups.error}
        onretry={() => backups.reload()}
        empty={{
          icon: 'hard-drive',
          title: 'No backups yet',
          description: 'Back up a profile from its page. Backups of deleted profiles stay here until you delete them.',
        }}
      >
        {#snippet row(b: Backup)}
          <td class="truncate">
            {#if b.profile_deleted && liveNames.has(b.profile_name)}
              From a deleted profile named {b.profile_name}
            {:else}
              {b.profile_name}
              {#if b.profile_deleted}<Badge tone="warning">Original profile deleted</Badge>{/if}
            {/if}
          </td>
          <td title={absolute(b.created_at)}>{relative(b.created_at, clock.now)}</td>
          <td class="num right">{bytes(b.size_bytes)}</td>
          <td class="truncate">{BACKEND_NAMES[b.backend] ?? b.backend}</td>
          <td>
            <Menu
              label="Actions for the {b.profile_name} backup"
              items={[
                { label: 'Restore as new profile…', onselect: () => (restoring = b) },
                'separator',
                { label: 'Delete…', tone: 'danger', onselect: () => removeBackup(b) },
              ]}
            />
          </td>
        {/snippet}
      </Table>
    </div>
  {/if}
</div>
<SaveBar dirty={formDirty} {saving} disabled={passInvalid || hasInvalid} onsave={save} ondiscard={() => { cancelReplace(); applySettings(); }} />

<style>
  .stack,
  .usage {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .skeletons,
  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .head {
    display: flex;
    justify-content: flex-end;
  }

  .pair {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--space-3);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    align-items: flex-start;
  }

  .secret {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    margin: 0;
    padding: var(--space-2) var(--space-3) var(--space-2) var(--space-4);
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }

  .total {
    font-size: var(--text-lg-size);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  @media (max-width: 1023px) {
    .pair {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
