<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { ApiError } from '$lib/api/client';
  import type { components } from '$lib/api/schema';
  import { buildLabel } from '$lib/builds/labels';
  import { clock } from '$lib/clock.svelte';
  import { absolute, duration, relative } from '$lib/format';
  import { outcome, profileLabel } from '$lib/instances/actions';
  import Badge from '$lib/ui/Badge.svelte';
  import StateBadge from '$lib/ui/StateBadge.svelte';
  import Table from '$lib/ui/Table.svelte';
  import { atLeastZero, isFinal, runHref, runResult, templateLabel } from './runs';

  type Instance = components['schemas']['Instance'];

  let {
    runs,
    loading = false,
    error = null,
    compact = false,
    hideProfile = false,
    empty,
    onretry = () => {},
  }: {
    runs: Instance[] | undefined;
    loading?: boolean;
    error?: ApiError | null;
    compact?: boolean;
    hideProfile?: boolean;
    empty?: Snippet;
    onretry?: () => void;
  } = $props();

  const columns = $derived([
    ...(hideProfile ? [] : [{ key: 'profile', label: 'Profile', width: compact ? '34%' : '20%' }]),
    ...(compact && !hideProfile ? [] : [{ key: 'template', label: 'Template', width: '16%' }, { key: 'build', label: 'Browser version', width: '22%' }]),
    { key: 'started', label: 'Started', width: '120px' },
    { key: 'time', label: 'Duration', width: '90px' },
    { key: 'result', label: 'Result' },
  ]);

  // A finished run shows how long it ran; a live one how long it has been running.
  const ranFor = (run: Instance) => {
    if (!run.started_at) return '—';
    return duration(atLeastZero((isFinal(run) ? (run.ended_at ?? run.started_at) : clock.now) - run.started_at));
  };

</script>

<Table
  caption="Runs"
  {columns}
  rows={runs}
  key={(row: Instance) => row.id}
  {loading}
  {error}
  {onretry}
  empty={{ icon: 'play', title: 'No runs', compact, action: empty }}
  rowClass={() => 'clickable'}
>
  {#snippet row(run: Instance)}
    {@const failed = outcome(run)}
    {#if !hideProfile}
      <td>
        <span class="profile">
          {#if run.kind === 'live'}
            <a href={runHref(run)}><Badge>Attached</Badge></a>
          {:else}
            <a class="truncate" href={runHref(run)} title={profileLabel(run)}>{profileLabel(run)}</a>
          {/if}
        </span>
      </td>
    {/if}
    {#if !compact || hideProfile}
      <td class="truncate" title={run.template_name ? templateLabel(run.template_name) : undefined}>
        {#if hideProfile}<a href={runHref(run)}>{templateLabel(run.template_name)}</a>{:else}{templateLabel(run.template_name)}{/if}
      </td>
      <td class="truncate" title={run.build ? buildLabel(run.build) : undefined}>{run.build ? buildLabel(run.build) : '—'}</td>
    {/if}
    {@const began = run.started_at ?? run.queued_at}
    <td class="num" title={absolute(began)}>{relative(Math.min(began, clock.now), clock.now)}</td>
    <td class="num">{ranFor(run)}</td>
    <td class="truncate">
      {#if failed}
        <Badge dot tone={failed.tone}>{failed.label}</Badge>
      {:else if isFinal(run)}
        {runResult(run)}
      {:else}
        <StateBadge kind="instance" state={run.state} label={run.kind === 'live' && run.state === 'attached' ? 'Connected' : undefined} />
      {/if}
    </td>
  {/snippet}
</Table>

<style>
  .profile {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  /* The whole row opens the run: the name link stretches over it. */
  :global(tr.clickable) {
    position: relative;
    cursor: pointer;
  }

  .profile a::after {
    content: '';
    position: absolute;
    inset: 0;
  }
</style>
