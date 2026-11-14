<script lang="ts">
  import type { components } from '$lib/api/schema';
  import Button from '$lib/ui/Button.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import { profileLabel, stopInstance } from './actions';
  import PromoteDialog from './PromoteDialog.svelte';

  type Instance = components['schemas']['Instance'];

  // The page's live tab list when it has one: the instance snapshot can be older than the tabs on screen.
  let { open = $bindable(false), instance, tabCount }: { open?: boolean; instance: Instance; tabCount?: number } = $props();

  let stopping = $state(false);
  let promoting = $state(false);

  const name = $derived(profileLabel(instance));
  const count = $derived(tabCount ?? instance.tab_count);
  const effect = $derived(
    `${count === 0 ? 'Any' : count === 1 ? 'Its tab closes and any' : `Its ${count} tabs close and any`} agent using it is disconnected.`,
  );
  const ephemeral = $derived(instance.ephemeral);

  async function confirm() {
    stopping = true;
    try {
      await stopInstance(instance);
      open = false;
    } finally {
      stopping = false;
    }
  }

  function promote() {
    open = false;
    promoting = true;
  }
</script>

<Dialog
  bind:open
  size="sm"
  title="Stop {name}?"
  description={ephemeral
    ? `${effect} This browser is temporary: its data is deleted when it stops. Promote it first to keep it.`
    : `${effect} Cookies and logins are kept.`}
  dismissible={!stopping}
>
  {#snippet footer()}
    <Button onclick={() => (open = false)} disabled={stopping}>Cancel</Button>
    {#if ephemeral}<Button onclick={promote} disabled={stopping}>Promote…</Button>{/if}
    <Button variant="danger" loading={stopping} onclick={confirm}>{ephemeral ? 'Stop and discard' : 'Stop browser'}</Button>
  {/snippet}
</Dialog>

<PromoteDialog bind:open={promoting} instanceId={instance.id} />
