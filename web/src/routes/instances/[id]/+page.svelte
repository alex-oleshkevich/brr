<script lang="ts">
  import { untrack } from 'svelte';
  import { page } from '$app/state';
  import { api, unwrap } from '$lib/api/client';
  import { events, type BrrEvent } from '$lib/api/events.svelte';
  import { liveQuery } from '$lib/api/live.svelte';
  import { endReason, plainError } from '$lib/events/summary';
  import { clock } from '$lib/clock.svelte';
  import { KINDS } from '$lib/events/kinds';
    import Skeleton from '$lib/ui/Skeleton.svelte';
  import { absolute, bytes, relative } from '$lib/format';
  import { toast } from '$lib/ui/toast.svelte';
  import { buildLabel } from '$lib/builds/labels';
  import { canPromote, canRelaunch, canStop, hasEnded, outcome, profileLabel } from '$lib/instances/actions';
  import LivePanel from '$lib/live/LivePanel.svelte';
  import LaunchDialog from '$lib/instances/LaunchDialog.svelte';
  import ConnectDialog from '$lib/instances/ConnectDialog.svelte';
  import NewTabDialog from '$lib/instances/NewTabDialog.svelte';
  import StopDialog from '$lib/instances/StopDialog.svelte';
  import PromoteDialog from '$lib/instances/PromoteDialog.svelte';
  import TabsTable from '$lib/instances/TabsTable.svelte';
  import Timeline from '$lib/instances/Timeline.svelte';
  import Badge from '$lib/ui/Badge.svelte';
  import Button from '$lib/ui/Button.svelte';
  import Card from '$lib/ui/Card.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import KeyValue from '$lib/ui/KeyValue.svelte';
  import Menu from '$lib/ui/Menu.svelte';
  import PageHeader from '$lib/ui/PageHeader.svelte';
  import Icon from '$lib/ui/Icon.svelte';
  import StateBadge from '$lib/ui/StateBadge.svelte';

  const TIMELINE_LIMIT = 50;
  const FILES_LIMIT = 50;
  const instanceId = $derived(page.params.id ?? '');
  const forThisInstance = (e: BrrEvent) =>
    (e.data as { instance_id?: string } | null)?.instance_id === instanceId || (e.subject.type === 'instance' && e.subject.id === instanceId);

  const instance = liveQuery(() => unwrap(api.GET('/v1/instances/{instance_id}', { params: { path: { instance_id: instanceId } } })), {
    kinds: ['instance.state_changed'],
    match: forThisInstance,
    refreshMs: 15_000,
  });
  const tabs = liveQuery(
    async () => (await unwrap(api.GET('/v1/instances/{instance_id}/tabs', { params: { path: { instance_id: instanceId } } }))).tabs,
    // lease.* covers Held by, which changes without a tab event.
    { kinds: ['tab.*', 'lease.*'], match: forThisInstance },
  );
  const files = liveQuery(
    async () => (await unwrap(api.GET('/v1/artifacts', { params: { query: { instance_id: instanceId, limit: FILES_LIMIT } } }))).artifacts,
    { kinds: ['artifact.*', 'download.completed'], match: forThisInstance },
  );
  // The timeline appends from the feed and refetches only on resync, so a busy Instance does not flood the daemon.
  const history = liveQuery(
    async () => (await unwrap(api.GET('/v1/events', { params: { query: { instance_id: instanceId, order: 'desc', limit: TIMELINE_LIMIT } } }))).events,
    { kinds: [] },
  );

  let timeline = $state<BrrEvent[]>([]);
  let promoting = $state(false);
  let stopOpen = $state(false);
  let connecting = $state(false);
  let newTab = $state(false);
  let relaunching = $state(false);

  // Events that arrive while the history request is in flight are kept and merged in.
  $effect(() => {
    const loaded = history.data ?? [];
    const seen = new Set(loaded.map((e) => e.seq));
    const kept = untrack(() => timeline).filter((e) => forThisInstance(e) && !seen.has(e.seq));
    timeline = [...kept, ...loaded].sort((a, b) => b.seq - a.seq).slice(0, TIMELINE_LIMIT);
  });

  $effect(() => {
    return events.on('*', (e) => {
      const persisted = KINDS.find((k) => k.kind === e.kind)?.transient === false;
      if (!persisted || !forThisInstance(e) || timeline.some((t) => t.seq === e.seq)) return;
      timeline = [e, ...timeline].slice(0, TIMELINE_LIMIT);
    });
  });

  const inst = $derived(instance.data);
  const requestedTab = $derived(page.url.searchParams.get('tab'));
  const openTabs = $derived((tabs.data ?? []).filter((t) => t.state !== 'closed' && t.closed_at == null));
  // ?tab, else the first ready Tab, else the first open one.
  const selectedTab = $derived(
    openTabs.find((t) => t.id === requestedTab) ?? openTabs.find((t) => t.state === 'ready') ?? openTabs[0],
  );
  // A Tab that closes while shown stays on screen as closed (the live view says so) until another one can be shown.
  let lastShown = $state<(typeof openTabs)[number]>();
  $effect(() => {
    if (selectedTab) lastShown = selectedTab;
  });
  const shownTab = $derived(
    selectedTab ?? (lastShown?.instance_id === instanceId ? { ...lastShown, state: 'closed' as const, lease: null } : undefined),
  );
  // The Tab record has no deadline; it comes from the latest tab.awaiting_human event.
  const awaitingMarks = liveQuery(
    async () =>
      (await unwrap(api.GET('/v1/events', { params: { query: { kinds: 'tab.awaiting_human', instance_id: instanceId, order: 'desc', limit: 20 } } }))).events,
    { kinds: ['tab.awaiting_human'], match: forThisInstance },
  );
  const awaiting = $derived.by(() => {
    const mark = (awaitingMarks.data ?? []).find((e) => (e.data as { tab_id?: string } | null)?.tab_id === selectedTab?.id);
    const data = mark?.data as { reason?: string; deadline?: number } | undefined;

    return data ? { reason: data.reason ?? 'human', deadline: data.deadline ?? null } : null;
  });

  const ended = $derived(inst ? hasEnded(inst) || inst.state === 'crashed' : false);
  const failed = $derived(inst ? outcome(inst) : null);
  // One line of what matters; the rest sits behind Details.
  const meta = $derived(
    inst
      ? [
          inst.build ? buildLabel(inst.build) : null,
          inst.display,
          inst.started_at ? `started ${relative(inst.started_at, clock.now)}` : null,
        ]
          .filter(Boolean)
          .join(' · ')
      : '',
  );
  const details = $derived(
    inst
      ? [
          { label: 'Browser ID', value: inst.id, mono: true, copy: true },
          { label: 'Process ID', value: inst.pid && !ended ? inst.pid : null },
          { label: 'Profile', value: inst.kind === 'live' ? 'Live browser' : (inst.profile_name ?? null) },
          { label: 'Started', value: inst.started_at ? absolute(inst.started_at) : null },
          { label: 'Ended', value: inst.ended_at ? absolute(inst.ended_at) : null },
          { label: 'Last activity', value: relative(inst.last_activity_at, clock.now) },
          { label: 'Restarts', value: inst.restart_count > 0 ? inst.restart_count : null },
          { label: 'Why it ended', value: inst.end_reason ? endReason(inst.end_reason) : null },
          { label: 'Error', value: inst.error ? plainError(inst.error) : null, tone: 'danger' as const },
          { label: 'Browser log', value: inst.log_path ?? null, mono: true, copy: true },
        ].filter((row) => row.value !== null)
      : [],
  );

  async function copyPath(path: string) {
    await navigator.clipboard.writeText(path);
    toast.success('Copied');
  }

  const artifactName = (a: { filename: string; kind: string }) => a.filename || a.kind;
</script>

{#if instance.loading}
  <PageHeader title="Browser" />
  <div class="stack" aria-busy="true">
    <Card title="Live view">
      <div class="skeletons">
        {#each { length: 5 } as _, i (i)}<Skeleton width="{40 + ((i * 17) % 40)}%" />{/each}
      </div>
    </Card>
    <Card title="Tabs" flush>
      <div class="skeletons">
        {#each { length: 3 } as _, i (i)}<Skeleton />{/each}
      </div>
    </Card>
  </div>
{:else if instance.error && !inst}
  {#if instance.error.code === 'NotFound'}
    <EmptyState level={1} title="Browser not found" description="It may have been removed.">
      {#snippet action()}<Button href="/instances">Back to browsers</Button>{/snippet}
    </EmptyState>
  {:else}
    <EmptyState tone="danger" title="Couldn't load this browser" description={instance.error.message}>
      {#snippet action()}<Button onclick={() => instance.reload()}>Retry</Button>{/snippet}
    </EmptyState>
  {/if}
{:else if inst}
  <PageHeader title={profileLabel(inst)} description={meta} breadcrumb={[{ label: 'Browsers', href: '/instances' }]}>
    {#snippet badge()}
      {#if failed}<Badge dot tone={failed.tone}>{failed.label}</Badge>{:else}<StateBadge kind="instance" state={inst.state} />{/if}
    {/snippet}
    {#snippet actions()}
      {#if canRelaunch(inst)}<Button variant="primary" onclick={() => (relaunching = true)}>Launch again</Button>{/if}
      {#if !ended}<Button onclick={() => (connecting = true)}>Connect…</Button>{/if}
      {#if canStop(inst)}<Button onclick={() => (stopOpen = true)}>Stop</Button>{/if}
      {#if canPromote(inst)}
        <Menu label="More actions" items={[{ label: 'Promote…', onselect: () => (promoting = true) }]} />
      {/if}
    {/snippet}
  </PageHeader>

  <div class="stack">
    {#if ended}
      <Card title="This browser has ended">
        <p class="ended">
          {inst.end_reason ? endReason(inst.end_reason) : 'It is no longer running.'}{inst.ended_at ? ` · ${absolute(inst.ended_at)}` : ''}
        </p>
        {#if inst.error}<p class="error">{plainError(inst.error)}</p>{/if}
        {#if inst.log_path}
          <p class="log">
            Browser log: <code>{inst.log_path}</code>
            <Button variant="ghost" size="sm" iconOnly icon="copy" aria-label="Copy browser log path" onclick={() => copyPath(inst.log_path!)} />
          </p>
        {/if}
      </Card>
    {:else}
      <LivePanel
        tab={shownTab}
        {awaiting}
        focused={requestedTab !== null && selectedTab?.id === requestedTab}
        loading={tabs.loading}
        error={tabs.error}
        onretry={() => tabs.reload()}
      />

      <Card title="Tabs" id="tabs" flush>
        {#snippet actions()}<Button size="sm" icon="plus" onclick={() => (newTab = true)}>New tab</Button>{/snippet}
        <TabsTable
          tabs={tabs.data}
          loading={tabs.loading}
          error={tabs.error}
          onretry={() => tabs.reload()}
          selected={selectedTab?.id}
          onchange={() => tabs.reload()}
        />
      </Card>
    {/if}

    <Card title="Files" id="files" flush={(files.data?.length ?? 0) > 0}>
      {#snippet actions()}<Button variant="ghost" size="sm" href="/files">All files</Button>{/snippet}
      {#if files.error}
        <EmptyState tone="danger" title="Couldn't load files" description={files.error.message}>
          {#snippet action()}<Button onclick={() => files.reload()}>Retry</Button>{/snippet}
        </EmptyState>
      {:else if !files.loading && (files.data?.length ?? 0) === 0}
        <p class="none">No files yet. Screenshots, PDFs and downloads from this browser appear here.</p>
      {:else}
        <ul class="files">
          {#each files.data ?? [] as file (file.id)}
            <li>
              <a href="/v1/artifacts/{file.id}/content" target="_blank" rel="noopener">{artifactName(file)}</a>
              <span class="size">{bytes(file.size_bytes)} · {relative(file.created_at, clock.now)}</span>
            </li>
          {/each}
        </ul>
      {/if}
    </Card>

    <details class="details">
      <summary><span class="chevron"><Icon name="chevron-right" /></span>Details</summary>
      <KeyValue items={details} grid />
    </details>

    <Card title="Activity">
      {#snippet actions()}<Button variant="ghost" size="sm" href="/events?instance={instanceId}">View all activity</Button>{/snippet}
      <Timeline events={timeline} loading={history.loading} error={history.error} onretry={() => history.reload()} />
    </Card>
  </div>

  <PromoteDialog bind:open={promoting} instanceId={inst.id} />
  <StopDialog bind:open={stopOpen} instance={inst} tabCount={openTabs.length} />
  {#if inst.profile_id}<LaunchDialog bind:open={relaunching} profileId={inst.profile_id} />{/if}
  <ConnectDialog bind:open={connecting} instance={inst} />
  <NewTabDialog bind:open={newTab} instanceId={inst.id} />
{/if}

<style>
  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-6);
  }

  .skeletons {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-5);
  }

  .ended,
  .none {
    color: var(--text-secondary);
  }

  .error {
    margin-top: var(--space-2);
    color: var(--danger-fg);
  }

  .log {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-2);
    min-width: 0;
  }

  .log code {
    overflow-wrap: anywhere;
  }

  .files {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .files li {
    display: flex;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-3) var(--space-5);
    border-top: 1px solid var(--border);
  }

  .files li:first-child {
    border-top: 0;
  }

  .size {
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .details {
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    padding: var(--space-3) var(--space-5);
  }

  .details summary {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    cursor: pointer;
    list-style: none;
    font-weight: 500;
  }

  .details summary::-webkit-details-marker {
    display: none;
  }

  .details[open] summary {
    margin-bottom: var(--space-3);
  }

  .chevron {
    display: flex;
    transition: transform var(--duration-base) var(--ease-out);
  }

  .details[open] .chevron {
    transform: rotate(90deg);
  }
</style>
