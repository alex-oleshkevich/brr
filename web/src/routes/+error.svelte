<script lang="ts">
  import { untrack } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { events } from '$lib/api/events.svelte';
  import Button from '$lib/ui/Button.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';

  // Navigating while the daemon is down fails to fetch the page's code, which SvelteKit reports as a bare "Internal Error".
  const offline = $derived(events.status === 'reconnecting' || (page.status >= 500 && page.error?.message === 'Internal Error'));
  let retrying = $state(false);

  async function retry() {
    retrying = true;
    try {
      await goto(page.url.pathname + page.url.search, { replaceState: true, invalidateAll: true });
    } finally {
      retrying = false;
    }
  }

  // The live stream flipping back to open means the daemon is up again: retry without waiting for a click.
  let sawDown = false;
  $effect(() => {
    if (events.status === 'reconnecting') sawDown = true;
    else if (events.status === 'open' && sawDown) {
      sawDown = false;
      untrack(() => void retry());
    }
  });
</script>

{#if offline}
  <EmptyState
    level={1}
    tone="danger"
    title="Can't reach the brr daemon"
    description="It may have stopped or restarted. Start it with brr daemon start if it isn't running, then retry."
  >
    {#snippet action()}<Button variant="primary" loading={retrying} onclick={retry}>Retry</Button>{/snippet}
  </EmptyState>
{:else if page.status === 404}
  <EmptyState level={1} title="Page not found" description="This page doesn't exist or has moved.">
    {#snippet action()}<Button href="/">Go to dashboard</Button>{/snippet}
  </EmptyState>
{:else}
  <EmptyState level={1} tone="danger" title="Something went wrong" description={page.error?.message}>
    {#snippet action()}<Button onclick={retry} loading={retrying}>Try again</Button>{/snippet}
  </EmptyState>
{/if}
