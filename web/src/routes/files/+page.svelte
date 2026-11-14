<script lang="ts">
  import { page } from '$app/state';
  import { goto } from '$app/navigation';
  import { api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import { pagedQuery } from '$lib/api/paged.svelte';
  import { clock } from '$lib/clock.svelte';
  import type { components } from '$lib/api/schema';
  import { absolute, bytes, relative } from '$lib/format';
  import Badge from '$lib/ui/Badge.svelte';
  import Menu from '$lib/ui/Menu.svelte';
  import PageHeader from '$lib/ui/PageHeader.svelte';
  import PagedFooter from '$lib/ui/PagedFooter.svelte';
  import Select from '$lib/ui/Select.svelte';
  import Table from '$lib/ui/Table.svelte';
  import { confirm } from '$lib/ui/confirm.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  type Artifact = components['schemas']['Artifact'];
  type Kind = components['schemas']['ArtifactKind'];

  const KINDS: { value: Kind; label: string }[] = [
    { value: 'download', label: 'Downloads' },
    { value: 'screenshot', label: 'Screenshots' },
    { value: 'pdf', label: 'PDFs' },
    { value: 'mhtml', label: 'MHTML' },
    { value: 'har', label: 'HAR' },
    { value: 'html', label: 'HTML' },
    { value: 'markdown', label: 'Markdown' },
  ];
  const kindLabel = (k: string) => KINDS.find((o) => o.value === k)?.label ?? k;
  const LIMIT = 100;
  const FILE_KINDS = ['artifact.*', 'download.completed', 'download.cancelled', 'retention.sweep_completed'];
  const ALL = 'all';

  let kind = $state(page.url.searchParams.get('kind') ?? ALL);
  let profile = $state(page.url.searchParams.get('profile') ?? ALL);
  let browser = $state(page.url.searchParams.get('browser') ?? ALL);

  const profiles = liveQuery(async () => (await unwrap(api.GET('/v1/profiles'))).profiles, { kinds: ['profile.*'] });
  const browsers = liveQuery(async () => (await unwrap(api.GET('/v1/instances', { params: { query: { limit: LIMIT } } }))).instances, { kinds: ['instance.*'] });
  const list = pagedQuery(
    async (cursor) => {
      const out = await unwrap(
        api.GET('/v1/artifacts', {
          params: {
            query: {
              limit: LIMIT,
              cursor,
              kind: kind === ALL ? undefined : (kind as Kind),
              profile_id: profile === ALL ? undefined : profile,
              instance_id: browser === ALL ? undefined : browser,
            },
          },
        }),
      );
      return { items: out.artifacts, next: out.next_cursor };
    },
    { kinds: FILE_KINDS, key: (a) => a.id, moreError: "Couldn't load more files" },
  );

  $effect(() => {
    const params = new URLSearchParams();
    if (kind !== ALL) params.set('kind', kind);
    if (profile !== ALL) params.set('profile', profile);
    if (browser !== ALL) params.set('browser', browser);
    const query = params.toString();
    void goto(query ? `/files?${query}` : '/files', { replaceState: true, keepFocus: true, noScroll: true });
  });

  const profileName = $derived(new Map((profiles.data ?? []).map((p) => [p.id, p.name])));
  // The built-in one-off profile is brr's, not the user's: it stays out of the pickers.
  const ownProfiles = $derived((profiles.data ?? []).filter((p) => !p.builtin));
  const builtinIds = $derived(new Set((profiles.data ?? []).filter((p) => p.builtin).map((p) => p.id)));
  const kindOptions = [{ value: ALL, label: 'All types' }, ...KINDS];
  const profileOptions = $derived([{ value: ALL, label: 'All profiles' }, ...ownProfiles.map((p) => ({ value: p.id, label: p.name }))]);
  const browserOptions = $derived([
    { value: ALL, label: 'All browsers' },
    ...(browsers.data ?? [])
      .filter((i) => !(i.profile_id && builtinIds.has(i.profile_id)))
      .map((i) => ({ value: i.id, label: `${i.profile_name ?? 'Browser'} · started ${relative(i.started_at ?? i.queued_at, clock.now)}` })),
  ]);

  const fileName = (a: Artifact) => a.filename || `${kindLabel(a.kind).replace(/s$/, '')} · ${absolute(a.created_at)}`;
  const contentUrl = (a: Artifact) => `/v1/artifacts/${a.id}/content`;

  function download(a: Artifact) {
    const link = document.createElement('a');
    link.href = contentUrl(a);
    link.download = fileName(a);
    link.click();
  }

  async function remove(a: Artifact) {
    const ok = await confirm({
      title: `Delete ${fileName(a)}?`,
      description: "The stored file is deleted. This can't be undone.",
      confirmLabel: 'Delete file',
      tone: 'danger',
    });
    if (!ok) return;
    try {
      await unwrap(api.DELETE('/v1/artifacts/{artifact_id}', { params: { path: { artifact_id: a.id } } }));
      toast.success('File deleted');
      list.reload();
    } catch (e) {
      toast.apiError(e, "Couldn't delete the file");
    }
  }
</script>

<PageHeader title="Files" description="Screenshots, PDFs, recordings and downloads that agents saved." />

<div class="stack">
  <div class="filters">
    <Select label="Type" bind:value={kind} options={kindOptions} />
    <Select label="Profile" bind:value={profile} options={profileOptions} />
    <Select label="Browser" bind:value={browser} options={browserOptions} />
  </div>
  <Table
    caption="Files"
    columns={[
      { key: 'name', label: 'Name', width: '32%' },
      { key: 'kind', label: 'Type', width: '14%' },
      { key: 'profile', label: 'Profile', width: '20%' },
      { key: 'size', label: 'Size', width: '12%', align: 'right' },
      { key: 'created', label: 'Saved', width: '16%' },
      { key: 'actions', label: 'Actions', width: '48px', srOnly: true },
    ]}
    rows={list.items}
    key={(row: Artifact) => row.id}
    loading={list.loading}
    error={list.error}
    onretry={() => list.reload()}
    empty={{
      icon: 'file',
      title: 'No files yet',
      description: 'Agents save screenshots, PDFs and recordings here (return=url). `brr shot -o` saves locally only.',
    }}
    footer={list.hasMore || list.capped ? loadMoreFooter : undefined}
  >
    {#snippet row(a: Artifact)}
      <td class="truncate">
        <a href={contentUrl(a)} target="_blank" rel="noopener" download={a.kind === 'download' ? '' : undefined}>{fileName(a)}</a>
      </td>
      <td><Badge>{kindLabel(a.kind)}</Badge></td>
      <td class="truncate">{a.profile_id ? (profileName.get(a.profile_id) ?? 'Deleted profile') : '—'}</td>
      <td class="num right">{bytes(a.size_bytes)}</td>
      <td title={absolute(a.created_at)}>{relative(a.created_at, clock.now)}</td>
      <td>
        <Menu
          label="Actions for {fileName(a)}"
          items={[
            { label: 'Open', onselect: () => window.open(contentUrl(a), '_blank', 'noopener') },
            { label: 'Download', onselect: () => download(a) },
            'separator',
            { label: 'Delete…', tone: 'danger', onselect: () => remove(a) },
          ]}
        />
      </td>
    {/snippet}
  </Table>
</div>

{#snippet loadMoreFooter()}
  <PagedFooter {list} />
{/snippet}

<style>
  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .filters {
    display: grid;
    grid-template-columns: repeat(3, 200px);
    gap: var(--space-4);
  }
</style>
