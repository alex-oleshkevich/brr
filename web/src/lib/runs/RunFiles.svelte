<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import { pagedQuery } from '$lib/api/paged.svelte';
  import type { components } from '$lib/api/schema';
  import { clock } from '$lib/clock.svelte';
  import { absolute, bytes, relative } from '$lib/format';
  import Badge from '$lib/ui/Badge.svelte';
  import PagedFooter from '$lib/ui/PagedFooter.svelte';
  import Table from '$lib/ui/Table.svelte';

  type Instance = components['schemas']['Instance'];
  type Artifact = components['schemas']['Artifact'];

  const LIMIT = 50;

  let { run }: { run: Instance } = $props();

  const list = pagedQuery(
    async (cursor) => {
      const out = await unwrap(api.GET('/v1/artifacts', { params: { query: { instance_id: run.id, limit: LIMIT, cursor } } }));
      return { items: out.artifacts, next: out.next_cursor };
    },
    { kinds: ['artifact.*', 'download.completed'], key: (a) => a.id, moreError: "Couldn't load more files" },
  );

  const columns = [
    { key: 'name', label: 'Name', width: '40%' },
    { key: 'kind', label: 'Type', width: '14%' },
    { key: 'size', label: 'Size', width: '12%', align: 'right' as const },
    { key: 'saved', label: 'Saved', width: '16%' },
    { key: 'actions', label: 'Download', width: '96px', srOnly: true, keepWidth: true },
  ];

  const fileName = (a: Artifact) => a.filename || `${a.kind} · ${absolute(a.created_at)}`;
  const contentUrl = (a: Artifact) => `/v1/artifacts/${a.id}/content`;
</script>

{#snippet more()}
  <PagedFooter {list} />
{/snippet}

<Table
  caption="Files"
  {columns}
  rows={list.items}
  key={(row: Artifact) => row.id}
  loading={list.loading}
  error={list.error}
  onretry={() => list.reload()}
  empty={{ icon: 'file', title: 'No files', description: 'Screenshots, PDFs and downloads from this run appear here.', compact: true }}
  footer={list.hasMore || list.capped ? more : undefined}
>
  {#snippet row(a: Artifact)}
    <td class="truncate"><a href={contentUrl(a)} target="_blank" rel="noopener" title={fileName(a)}>{fileName(a)}</a></td>
    <td><Badge>{a.kind}</Badge></td>
    <td class="num right">{bytes(a.size_bytes)}</td>
    <td title={absolute(a.created_at)}>{relative(a.created_at, clock.now)}</td>
    <td><a href={contentUrl(a)} download={fileName(a)}>Download</a></td>
  {/snippet}
</Table>
