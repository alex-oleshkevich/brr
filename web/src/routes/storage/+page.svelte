<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import { clock } from '$lib/clock.svelte';
  import type { components } from '$lib/api/schema';
  import { absolute, bytes, count, relative } from '$lib/format';
  import BackendForm from '$lib/storage/BackendForm.svelte';
  import CleanDialog from '$lib/storage/CleanDialog.svelte';
  import RestoreDialog from '$lib/storage/RestoreDialog.svelte';
  import Badge from '$lib/ui/Badge.svelte';
  import Button from '$lib/ui/Button.svelte';
  import Card from '$lib/ui/Card.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Menu from '$lib/ui/Menu.svelte';
  import PageHeader from '$lib/ui/PageHeader.svelte';
  import Progress from '$lib/ui/Progress.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import Table from '$lib/ui/Table.svelte';
  import Tabs from '$lib/ui/Tabs.svelte';
  import { confirm } from '$lib/ui/confirm.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  type Backup = components['schemas']['Backup'];

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

  let cleaning = $state(false);
  let view = $state('overview');
  let tab = $state('profiles');
  let restoring = $state<Backup | null>(null);

  const backend = liveQuery(() => unwrap(api.GET('/v1/storage/backend')), { kinds: [] });
  const usage = liveQuery(() => unwrap(api.GET('/v1/storage/usage')), {
    kinds: [...FILE_KINDS, 'build.*', 'extension.*', 'backup.*'],
  });
  const backups = liveQuery(async () => (await unwrap(api.GET('/v1/backups'))).backups, { kinds: ['backup.*', 'profile.*'] });

  const profiles = liveQuery(async () => (await unwrap(api.GET('/v1/profiles'))).profiles, { kinds: ['profile.*'] });
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
</script>

<PageHeader title="Storage" description="Where files and backups live. Clean up previews what is past your retention rules and deletes nothing until you confirm.">
  {#snippet actions()}
    <Button onclick={() => (cleaning = true)}>Clean up…</Button>
  {/snippet}
</PageHeader>

<CleanDialog bind:open={cleaning} onswept={swept} />
<RestoreDialog bind:backup={restoring} onrestored={() => backups.reload()} />

<div class="stack">
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
          <BackendForm backend={backend.data} onsaved={() => backend.reload()} />
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
    </div>
  {:else}
    <div role="tabpanel" id="storage-panel-backups" aria-labelledby="storage-tab-backups" class="stack">
      <Table
        caption="Backups"
        columns={[
          { key: 'profile', label: 'Profile', width: '32%' },
          { key: 'created', label: 'Backed up', width: '24%' },
          { key: 'size', label: 'Size', width: '14%', align: 'right' },
          { key: 'where', label: 'Stored in', width: '22%' },
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

<style>
  .stack,
  .usage {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .skeletons {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .total {
    font-size: var(--text-lg-size);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
</style>
