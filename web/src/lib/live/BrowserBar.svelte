<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import Button from '$lib/ui/Button.svelte';
  import Input from '$lib/ui/Input.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  let { tabId, url, enabled }: { tabId: string; url: string; enabled: boolean } = $props();

  type Move = 'back' | 'forward' | 'reload' | 'go';

  let address = $state('');
  let editing = $state(false);
  let busy = $state<Move | null>(null);

  // The field follows the page until the person starts typing.
  $effect(() => {
    if (!editing) address = url;
  });

  const path = $derived({ params: { path: { tab_id: tabId } } });

  // "example.com" is what people type; navigation needs a full URL.
  const withScheme = (value: string) => (/^[a-z][a-z0-9+.-]*:/i.test(value) ? value : `https://${value}`);

  async function move(kind: Move) {
    const target = address.trim();
    if (busy || (kind === 'go' && !target)) return;

    busy = kind;
    try {
      if (kind === 'back') await unwrap(api.POST('/v1/tabs/{tab_id}/go_back', { ...path, body: {} }));
      else if (kind === 'forward') await unwrap(api.POST('/v1/tabs/{tab_id}/go_forward', { ...path, body: {} }));
      else if (kind === 'reload') await unwrap(api.POST('/v1/tabs/{tab_id}/reload', { ...path, body: {} }));
      else await unwrap(api.POST('/v1/tabs/{tab_id}/navigate', { ...path, body: { url: withScheme(target) } }));
      editing = false;
    } catch (e) {
      toast.apiError(e, kind === 'go' ? "Couldn't open that address" : `Couldn't ${kind === 'reload' ? 'reload' : `go ${kind}`}`);
    } finally {
      busy = null;
    }
  }
</script>

<form class="bar" role="search" aria-label="Browser bar" onsubmit={(e) => (e.preventDefault(), void move('go'))}>
  <Button size="sm" variant="ghost" aria-label="Back" disabled={!enabled} loading={busy === 'back'} onclick={() => move('back')}>←</Button>
  <Button size="sm" variant="ghost" aria-label="Forward" disabled={!enabled} loading={busy === 'forward'} onclick={() => move('forward')}>→</Button>
  <Button size="sm" variant="ghost" iconOnly icon="refresh" aria-label="Reload" disabled={!enabled} loading={busy === 'reload'} onclick={() => move('reload')} />
  <div class="address" onfocusin={() => (editing = true)} onfocusout={() => (editing = false)}>
    <Input
      label="Address"
      hideLabel
      type="text"
      bind:value={address}
      placeholder="Type an address and press Enter"
      autocomplete="off"
      disabled={!enabled}
    />
  </div>
</form>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    margin-bottom: var(--space-3);
  }

  .address {
    flex: 1;
    min-width: 0;
    margin-left: var(--space-2);
  }
</style>
