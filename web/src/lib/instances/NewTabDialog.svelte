<script lang="ts">
  import { goto } from '$app/navigation';
  import { api, unwrap } from '$lib/api/client';
  import Button from '$lib/ui/Button.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import Input from '$lib/ui/Input.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  let { open = $bindable(false), instanceId }: { open?: boolean; instanceId: string } = $props();

  let url = $state('');
  let opening = $state(false);

  $effect(() => {
    if (open) url = '';
  });

  // "example.com" is what people type; the daemon wants a full URL.
  const withScheme = (value: string) => (/^[a-z][a-z0-9+.-]*:/i.test(value) ? value : `https://${value}`);

  async function openTab(event: SubmitEvent) {
    event.preventDefault();
    if (opening) return;

    opening = true;
    try {
      const target = url.trim();
      const { tab } = await unwrap(
        api.POST('/v1/instances/{instance_id}/tabs', {
          params: { path: { instance_id: instanceId } },
          body: { url: target ? withScheme(target) : null },
        }),
      );
      open = false;
      toast.success("Opened a new tab. You're in control of it.");
      await goto(`?tab=${tab.id}`, { noScroll: false, replaceState: true });
    } catch (e) {
      toast.apiError(e, "Couldn't open a new tab");
    } finally {
      opening = false;
    }
  }
</script>

<Dialog bind:open size="sm" title="New tab" description="Opens a tab in this browser and puts you in control of it." dismissible={!opening}>
  <form id="new-tab-form" onsubmit={openTab} novalidate>
    <Input label="Address (optional)" type="text" bind:value={url} placeholder="https://example.com" autocomplete="off" />
  </form>
  {#snippet footer()}
    <Button onclick={() => (open = false)} disabled={opening}>Cancel</Button>
    <Button variant="primary" type="submit" form="new-tab-form" loading={opening}>Open tab</Button>
  {/snippet}
</Dialog>
