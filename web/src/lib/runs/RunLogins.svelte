<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import { fetchAll } from '$lib/api/paged.svelte';
  import { liveQuery } from '$lib/api/live.svelte';
  import type { components } from '$lib/api/schema';
  import { clock } from '$lib/clock.svelte';
  import { absolute, relative } from '$lib/format';
  import Badge from '$lib/ui/Badge.svelte';
  import Table from '$lib/ui/Table.svelte';

  type Instance = components['schemas']['Instance'];
  type Row = { seq: number; ts: number; ok: boolean; site: string; username: string };

  const LIMIT = 200;

  let { run }: { run: Instance } = $props();

  const rows = liveQuery(
    async (): Promise<Row[]> => {
      const [{ events }, credentials] = await Promise.all([
        unwrap(api.GET('/v1/events', { params: { query: { kinds: 'login.performed', instance_id: run.id, order: 'desc', limit: LIMIT } } })),
        fetchAll(async (cursor) => {
          const out = await unwrap(api.GET('/v1/credentials', { params: { query: { cursor } } }));
          return { items: out.credentials, next: out.next_cursor };
        }),
      ]);
      const byId = new Map(credentials.map((c) => [c.id, c]));

      return events.map((e) => {
        const data = e.data as { credential_id?: string; ok?: boolean } | null;
        const credential = data?.credential_id ? byId.get(data.credential_id) : undefined;

        return { seq: e.seq, ts: e.ts, ok: Boolean(data?.ok), site: credential?.site ?? 'Deleted login', username: credential?.username ?? '—' };
      });
    },
    { kinds: ['login.performed'], match: (e) => (e.data as { instance_id?: string } | null)?.instance_id === run.id },
  );

  const columns = [
    { key: 'site', label: 'Site', width: '34%' },
    { key: 'username', label: 'Username', width: '30%' },
    { key: 'result', label: 'Result', width: '16%' },
    { key: 'time', label: 'Time' },
  ];
</script>

<Table
  caption="Logins"
  {columns}
  rows={rows.data}
  key={(row: Row) => String(row.seq)}
  loading={rows.loading}
  error={rows.error}
  onretry={() => rows.reload()}
  empty={{ icon: 'activity', title: 'No logins', description: 'Logins brr performed in this run appear here.', compact: true }}
>
  {#snippet row(item: Row)}
    <td class="truncate" title={item.site}>{item.site}</td>
    <td class="truncate" title={item.username}>{item.username}</td>
    <td><Badge dot tone={item.ok ? 'success' : 'danger'}>{item.ok ? 'Succeeded' : 'Failed'}</Badge></td>
    <td title={absolute(item.ts)}>{relative(item.ts, clock.now)}</td>
  {/snippet}
</Table>
