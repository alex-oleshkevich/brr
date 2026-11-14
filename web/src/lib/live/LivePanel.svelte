<script lang="ts">
  import { tick } from 'svelte';
  import { ApiError, api, unwrap } from '$lib/api/client';
  import { session } from '$lib/api/session';
  import type { components } from '$lib/api/schema';
  import { everySecond } from '$lib/clock.svelte';
  import { names } from '$lib/events/names.svelte';
  import { countdown } from '$lib/events/summary';
  import Button from '$lib/ui/Button.svelte';
  import Card from '$lib/ui/Card.svelte';
  import Icon from '$lib/ui/Icon.svelte';
  import { confirm } from '$lib/ui/confirm.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import StateBadge from '$lib/ui/StateBadge.svelte';
  import { toast } from '$lib/ui/toast.svelte';
  import StatusDot from '$lib/ui/StatusDot.svelte';
  import BrowserBar from './BrowserBar.svelte';
  import LiveView, { type Status } from './LiveView.svelte';
  import { capped } from './protocol';

  type Tab = components['schemas']['Tab'];

  let {
    tab,
    awaiting,
    focused,
    loading,
    error,
    onretry,
  }: {
    tab: Tab | undefined;
    // Why the Tab waits and until when, from its tab.awaiting_human event.
    awaiting: { reason: string; deadline: number | null } | null;
    focused: boolean;
    loading: boolean;
    error: ApiError | null;
    onretry: () => void;
  } = $props();

  const HUMAN_VERBS: Record<string, string> = {
    login: 'log in',
    captcha: 'solve a captcha',
    dialog: 'answer a dialog',
  };
  const FALLBACK_AGENT = 'the agent';

  let busy = $state<'take' | 'back' | 'done' | 'release' | null>(null);
  let status = $state<Status>('connecting');
  let now = $state(Date.now());
  // The lease record names who the tab came from only by id, so the agent's label is remembered from when it was seen holding it.
  let seenAgent = $state<{ tabId: string; label: string } | null>(null);

  const lease = $derived(tab?.lease ?? null);
  const mine = $derived(lease !== null && lease.client_id === session.clientId);
  const tookOver = $derived(mine && lease?.transferred_from_client_id != null);
  const isAwaiting = $derived(tab?.state === 'awaiting_human');
  const interactive = $derived(isAwaiting || mine);
  // A primitive: the connection restarts when the Tab's id changes, not each time its record is replaced.
  const tabId = $derived(tab?.id);
  const title = $derived(tab ? capped(tab.title || tab.url || 'Untitled') : '');
  const agent = $derived(
    lease && !mine
      ? lease.client_label
      : seenAgent && seenAgent.tabId === tabId
        ? seenAgent.label
        : ((lease?.transferred_from_client_id && names.client(lease.transferred_from_client_id)) || FALLBACK_AGENT),
  );
  const host = $derived.by(() => {
    try {
      return new URL(tab?.url ?? '').host || tab?.url || 'this page';
    } catch {
      return tab?.url || 'this page';
    }
  });
  const need = $derived(HUMAN_VERBS[awaiting?.reason ?? ''] ?? awaiting?.reason ?? 'take a look');
  const holder = $derived(
    isAwaiting ? `Your turn · ${agent} is waiting` : lease ? (mine ? "You're in control" : `In use by ${lease.client_label}`) : 'Not in use',
  );

  $effect(() => {
    if (tabId && lease && !mine) seenAgent = { tabId, label: lease.client_label };
  });

  // Deep links land here: bring the view on screen, not just focus it.
  $effect(() => {
    if (!focused || !tabId) return;
    void tick().then(() => document.getElementById('live-view')?.scrollIntoView({ block: 'start' }));
  });

  // A one-second clock only while a countdown is on screen.
  $effect(() => {
    if (!isAwaiting || awaiting?.deadline == null) return;

    return everySecond((tickNow) => (now = tickNow));
  });

  const left = $derived(awaiting?.deadline == null ? null : countdown(awaiting.deadline, now));

  async function takeOver() {
    if (!tab) return;
    const ok = await confirm({
      title: 'Take control of this tab?',
      description: lease ? `${lease.client_label} can't use it until you hand it back.` : "Nobody else can use it until you release it.",
      confirmLabel: 'Take control',
    });
    if (!ok) return;

    busy = 'take';
    try {
      await unwrap(api.POST('/v1/tabs/{tab_id}/take-over', { params: { path: { tab_id: tab.id } }, body: {} }));
      toast.success("You're in control of this tab");
    } catch (e) {
      toast.apiError(e, "Couldn't take control of the tab");
    } finally {
      busy = null;
    }
  }

  async function handBack() {
    if (!tab) return;
    busy = 'back';
    try {
      await unwrap(api.POST('/v1/tabs/{tab_id}/hand-back', { params: { path: { tab_id: tab.id } }, body: {} }));
      toast.success(`Handed back to ${agent}`);
    } catch (e) {
      if (e instanceof ApiError && e.details.reason === 'client_gone') {
        // Nobody to hand it back to, and the daemon keeps the Lease: free it so the Tab isn't stuck with you.
        toast.error('The agent that used this tab has ended.');
        return await release();
      } else {
        toast.apiError(e, "Couldn't hand the tab back");
      }
    } finally {
      busy = null;
    }
  }

  async function release() {
    if (!lease) return;
    busy = 'release';
    try {
      await unwrap(api.DELETE('/v1/leases/{lease_id}', { params: { path: { lease_id: lease.lease_id } } }));
      toast.success('Released. Agents can use this browser again.');
    } catch (e) {
      toast.apiError(e, "Couldn't release the tab");
    } finally {
      busy = null;
    }
  }

  async function done() {
    if (!tab) return;
    busy = 'done';
    try {
      await unwrap(api.POST('/v1/tabs/{tab_id}/human-done', { params: { path: { tab_id: tab.id } }, body: {} }));
      toast.success(`Handed back to ${agent}`);
    } catch (e) {
      toast.apiError(e, "Couldn't mark the step as done");
    } finally {
      busy = null;
    }
  }
</script>

<Card title="Live view" id="live-view">
  {#snippet actions()}
    {#if tab && tab.state !== 'closed'}
      {#if isAwaiting}
        <Button variant="primary" loading={busy === 'done'} onclick={done}>Done — hand back to {agent}</Button>
      {:else if mine}
        <!-- One control for giving the tab up: back to the agent it came from if there was a transfer, else just released. -->
        <Button loading={busy === 'back' || busy === 'release'} onclick={tookOver ? handBack : release}>{tookOver ? `Hand back to ${agent}` : 'Release'}</Button>
      {:else}
        <Button loading={busy === 'take'} onclick={takeOver}>Take control</Button>
      {/if}
    {/if}
  {/snippet}

  {#if isAwaiting}
    <p class="banner" role="status">
      <Icon name="alert" />
      <span>
        <strong>{agent}</strong> needs you to {need} on <strong>{host}</strong>. Do it below, then click Done.
        {#if left}<span class="left">{left}</span>{/if}
      </span>
    </p>
  {/if}

  {#if tab && tab.state !== 'closed'}
    <BrowserBar tabId={tab.id} url={tab.url} enabled={interactive} />
  {/if}

  {#if tab}
    <div class="head">
      <span class="name truncate" {title}>{title}</span>
      <!-- The banner already says why it waits. -->
      {#if !isAwaiting}<StateBadge kind="tab" state={tab.state} />{/if}
      <span class="mode">
        <span class="held">{holder}</span>
        {#if status === 'live'}<StatusDot tone="success" label="Live" />{/if}
      </span>
    </div>
  {/if}

  {#if loading}
    <div class="loading" aria-busy="true"><Skeleton height="100%" radius="var(--radius-lg)" /></div>
  {:else if error}
    <EmptyState tone="danger" title="Couldn't load the live view" description={error.message}>
      {#snippet action()}<Button onclick={onretry}>Retry</Button>{/snippet}
    </EmptyState>
  {:else if tabId}
    <LiveView {tabId} {interactive} {title} {focused} bind:status />
    {#if mine && !tookOver && !isAwaiting}
      <p class="control-note">You're in control. Release so agents can use this browser.</p>
    {/if}
  {:else}
    <EmptyState title="No tab to show" description="Open a tab in this browser to watch it here." />
  {/if}
</Card>

<style>
  :global(#live-view) {
    scroll-margin-top: calc(var(--topbar-h) + var(--space-4));
  }

  .loading {
    aspect-ratio: 16 / 9;
    max-height: 70vh;
  }

  .head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-width: 0;
    margin-bottom: var(--space-3);
  }

  .name {
    min-width: 0;
    font-weight: 500;
  }

  .mode {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-left: auto;
  }

  .held {
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    white-space: nowrap;
  }

  .banner {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    margin-bottom: var(--space-3);
    padding: var(--space-3) var(--space-4);
    background: var(--warning-bg);
    border: 1px solid var(--warning-border);
    border-radius: var(--radius-md);
    color: var(--warning-fg);
  }

  .banner .left {
    margin-left: var(--space-1);
    font-weight: 500;
    font-variant-numeric: tabular-nums;
  }

  .control-note {
    margin-top: var(--space-2);
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
  }
</style>
