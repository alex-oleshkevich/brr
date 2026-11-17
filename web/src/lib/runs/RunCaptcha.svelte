<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import { fetchAll } from '$lib/api/paged.svelte';
  import type { components } from '$lib/api/schema';
  import { captchaType } from '$lib/events/summary';
  import { clock } from '$lib/clock.svelte';
  import { absolute, duration, relative, usd } from '$lib/format';
  import Badge from '$lib/ui/Badge.svelte';
  import Table from '$lib/ui/Table.svelte';

  type Instance = components['schemas']['Instance'];
  type Solve = components['schemas']['CaptchaSolve'];

  const PAGE = 100;

  let { run }: { run: Instance } = $props();

  const solves = liveQuery(
    () =>
      fetchAll(async (cursor) => {
        const out = await unwrap(api.GET('/v1/captcha/solves', { params: { query: { instance_id: run.id, limit: PAGE, cursor } } }));
        return { items: out.solves, next: out.next_cursor };
      }),
    { kinds: ['captcha.*'] },
  );


  const total = $derived((solves.data ?? []).reduce((sum, s) => sum + s.cost_usd_micros, 0));

  const columns = [
    { key: 'site', label: 'Site', width: '28%' },
    { key: 'type', label: 'Type', width: '18%' },
    { key: 'result', label: 'Result', width: '14%' },
    { key: 'cost', label: 'Cost', align: 'right' as const, width: '12%' },
    { key: 'took', label: 'Took', align: 'right' as const, width: '10%' },
    { key: 'time', label: 'Time' },
  ];
</script>

{#snippet totalFooter()}
  <p>Total cost: <strong class="num">{usd(total)}</strong></p>
{/snippet}

<Table
  caption="Captcha solves"
  {columns}
  rows={solves.data}
  key={(row: Solve) => row.id}
  loading={solves.loading}
  error={solves.error}
  onretry={() => solves.reload()}
  empty={{ icon: 'activity', title: 'No captchas', description: 'Captchas met in this run appear here.', compact: true }}
  footer={solves.data?.length ? totalFooter : undefined}
>
  {#snippet row(item: Solve)}
    <td class="truncate" title={item.site}>{item.site}</td>
    <td class="truncate">{captchaType(item.type)}</td>
    <td><Badge dot tone={item.result === 'solved' ? 'success' : 'danger'}>{item.result === 'solved' ? 'Solved' : 'Failed'}</Badge></td>
    <td class="num right">{usd(item.cost_usd_micros)}</td>
    <td class="num right">{duration(item.duration_ms)}</td>
    <td title={absolute(item.ts)}>{relative(item.ts, clock.now)}</td>
  {/snippet}
</Table>

<style>
  p {
    margin: 0;
    text-align: right;
  }
</style>
