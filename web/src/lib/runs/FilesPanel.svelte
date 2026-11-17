<script lang="ts">
  import { page } from '$app/state';
  import { goto } from '$app/navigation';
  import { api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import { pagedQuery } from '$lib/api/paged.svelte';
  import { clock } from '$lib/clock.svelte';
  import type { components } from '$lib/api/schema';
  import { absolute, bytes, relative } from '$lib/format';
  import Button from '$lib/ui/Button.svelte';
  import Badge from '$lib/ui/Badge.svelte';
  import Menu from '$lib/ui/Menu.svelte';
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
  const kindSingular = (k: string) => (k === 'har' || k === 'mhtml' || k === 'html' ? kindLabel(k) : kindLabel(k).replace(/s$/, ''));
  const LIMIT = 100;
  const FILE_KINDS = ['artifact.*', 'download.completed', 'download.cancelled', 'retention.sweep_completed'];
  const ALL = 'all';

  let kind = $state(page.url.searchParams.get('kind') ?? ALL);
  let profile = $state(page.url.searchParams.get('profile') ?? ALL);
  let browser = $state(page.url.searchParams.get('browser') ?? ALL);

  const profiles = liveQuery(async () => (await unwrap(api.GET('/v1/profiles'))).profiles, { kinds: ['profile.*'] });
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
    const url = new URL(page.url);
    for (const [key, value] of [['kind', kind], ['profile', profile], ['browser', browser]]) {
      if (value === ALL) url.searchParams.delete(key);
      else url.searchParams.set(key, value);
    }
    if (url.search !== page.url.search) void goto(url, { replaceState: true, keepFocus: true, noScroll: true });
  });

  const profileName = $derived(new Map((profiles.data ?? []).map((p) => [p.id, p.name])));
  // The built-in one-off profile is brr's, not the user's: it stays out of the pickers.
  const ownProfiles = $derived((profiles.data ?? []).filter((p) => !p.builtin));
  const kindOptions = [{ value: ALL, label: 'All types' }, ...KINDS];
  const profileOptions = $derived([{ value: ALL, label: 'All profiles' }, ...ownProfiles.map((p) => ({ value: p.id, label: p.name }))]);
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

<div class="stack">
  <div class="filters">
    <Select label="Type" bind:value={kind} options={kindOptions} />
    <Select label="Profile" bind:value={profile} options={profileOptions} />
    {#if browser !== ALL}
      <div class="one-run"><Button onclick={() => (browser = ALL)}>Showing one run · Show all</Button></div>
    {/if}
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
      <td><Badge>{kindSingular(a.kind)}</Badge></td>
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
    align-items: end;
    gap: var(--space-4);
  }
</style>
