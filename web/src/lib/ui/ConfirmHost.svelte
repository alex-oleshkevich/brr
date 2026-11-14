<script lang="ts">
  import Button from './Button.svelte';
  import Dialog from './Dialog.svelte';
  import { confirmState } from './confirm.svelte';

  const current = $derived(confirmState.current);
  let open = $state(false);
  let shown = $state<typeof confirmState.current>(null);

  $effect(() => {
    if (current) {
      shown = current;
      open = true;
    }
  });

  function settle(ok: boolean) {
    if (!shown) return;
    shown.resolve(ok);
    confirmState.current = null;
    shown = null;
    open = false;
  }
</script>

<Dialog bind:open size="sm" title={shown?.title ?? ''} description={shown?.description} onclose={() => settle(false)}>
  {#snippet footer()}
    <Button autofocus={shown?.tone === 'danger'} onclick={() => settle(false)}>Cancel</Button>
    <Button variant={shown?.tone === 'danger' ? 'danger' : 'primary'} onclick={() => settle(true)}>
      {shown?.confirmLabel}
    </Button>
  {/snippet}
</Dialog>
