<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import { session } from '$lib/api/session';
  import type { components } from '$lib/api/schema';
  import Button from '$lib/ui/Button.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import { profileLabel, stopInstance } from './actions';
  import PromoteDialog from './PromoteDialog.svelte';

  type Instance = components['schemas']['Instance'];

  // The page's live tab list when it has one: the instance snapshot can be older than the tabs on screen.
  let {
    open = $bindable(false),
    instance,
    tabCount,
    onpromoted,
  }: { open?: boolean; instance: Instance; tabCount?: number; onpromoted?: () => void } = $props();

  let stopping = $state(false);
  let promoting = $state(false);

  const leases = liveQuery(
    async () => (await unwrap(api.GET('/v1/leases', { params: { query: { instance_id: instance.id } } }))).leases,
    { kinds: ['lease.*'] },
  );
  // Other clients are the ones a stop would cut off; the UI's own sessions are the person stopping.
  const others = $derived.by(() => {
    const byClient = new Map<string, string>();
    for (const lease of leases.data ?? []) {
      if (lease.released_at != null || session.uiClientIds.has(lease.client_id)) continue;
      byClient.set(lease.client_id, `${lease.client_label} (${lease.client_kind})`);
    }

    return [...byClient.values()];
  });

  const name = $derived(profileLabel(instance));
  const count = $derived(tabCount ?? instance.tab_count);
  const effect = $derived(
    `${count === 0 ? 'Any' : count === 1 ? 'Its tab closes and any' : `Its ${count} tabs close and any`} agent using it is disconnected.`,
  );
  const keeping = $derived(instance.promote_to_name ?? null);
  const discards = $derived(instance.ephemeral && !keeping);
  const consequence = $derived(
    keeping
      ? `This run's data will be kept as profile ${keeping}.`
      : instance.ephemeral
        ? "This browser starts fresh each time: its data is deleted when it stops. Keep this run's data first to save it."
        : 'Cookies and logins are kept.',
  );

  async function confirm() {
    stopping = true;
    try {
      await stopInstance(instance);
      open = false;
    } finally {
      stopping = false;
    }
  }

  function keep() {
    open = false;
    promoting = true;
  }
</script>

<Dialog bind:open size="sm" title="Stop {name}?" description="{effect} {consequence}" dismissible={!stopping}>
  {#if others.length > 0}
    <p class="others">Also in use by: {others.join(', ')}. They will be disconnected.</p>
  {/if}
  {#snippet footer()}
    <Button onclick={() => (open = false)} disabled={stopping}>Cancel</Button>
    {#if discards}<Button onclick={keep} disabled={stopping}>Keep this run's data…</Button>{/if}
    <Button variant="danger" loading={stopping} onclick={confirm}>{discards ? 'Stop and discard' : 'Stop browser'}</Button>
  {/snippet}
</Dialog>

<PromoteDialog bind:open={promoting} instanceId={instance.id} {onpromoted} />

<style>
  .others {
    margin: 0;
    color: var(--text-secondary);
  }
</style>
