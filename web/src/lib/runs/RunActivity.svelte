<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import { events, type BrrEvent } from '$lib/api/events.svelte';
  import { pagedQuery } from '$lib/api/paged.svelte';
  import type { components } from '$lib/api/schema';
  import { clock } from '$lib/clock.svelte';
  import { KINDS, kindLabel } from '$lib/events/kinds';
  import { summarize } from '$lib/events/summary';
  import { absolute, relative } from '$lib/format';
  import PagedFooter from '$lib/ui/PagedFooter.svelte';
  import Table from '$lib/ui/Table.svelte';

  type Instance = components['schemas']['Instance'];

  const LIMIT = 100;

  let { run }: { run: Instance } = $props();

  const mentionsRun = (e: BrrEvent) =>
    (e.data as { instance_id?: string } | null)?.instance_id === run.id || (e.subject.type === 'instance' && e.subject.id === run.id);

  // The daemon matches the Instance on both its own events and those of its Tabs and leases.
  const list = pagedQuery(
    async (cursor) => {
      const out = await unwrap(api.GET('/v1/events', { params: { query: { instance_id: run.id, order: 'desc', limit: LIMIT, cursor } } }));
      return { items: out.events, next: out.next_cursor };
    },
    { kinds: [], key: (e) => e.seq, moreError: "Couldn't load more activity" },
  );

  // A finished run gets no new events of its own, but a late file or login can still arrive.
  $effect(() => {
    return events.on('*', (e) => {
      const persisted = KINDS.find((k) => k.kind === e.kind)?.transient === false;
      if (persisted && mentionsRun(e) && !list.items?.some((row) => row.seq === e.seq)) list.prepend(e);
    });
  });

  const columns = [
    { key: 'time', label: 'Time', width: '140px' },
    { key: 'what', label: 'What happened', width: '26%' },
    { key: 'summary', label: 'Details' },
  ];
</script>

{#snippet more()}
  <PagedFooter {list} />
{/snippet}

<Table
  caption="Activity"
  {columns}
  rows={list.items}
  key={(row: BrrEvent) => String(row.seq)}
  loading={list.loading}
  error={list.error}
  onretry={() => list.reload()}
  empty={{ icon: 'activity', title: 'No activity', compact: true }}
  footer={list.hasMore || list.capped ? more : undefined}
>
  {#snippet row(item: BrrEvent)}
    <td><time class="truncate" datetime={new Date(item.ts).toISOString()} title={absolute(item.ts)}>{relative(item.ts, clock.now)}</time></td>
    <td class="truncate">{kindLabel(item.kind)}</td>
    <td class="truncate" title={summarize(item)}>{summarize(item)}</td>
  {/snippet}
</Table>
