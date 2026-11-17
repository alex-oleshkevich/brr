<script lang="ts">
  import { goto } from '$app/navigation';
  import { ApiError, api, unwrap } from '$lib/api/client';
  import Button from '$lib/ui/Button.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import Input from '$lib/ui/Input.svelte';
  import SegmentedControl from '$lib/ui/SegmentedControl.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  let { open = $bindable(false) }: { open?: boolean } = $props();

  const modes = [
    { value: 'cdp', label: 'Debugging address' },
    { value: 'install', label: 'Installed browser' },
  ];

  let mode = $state('cdp');
  let target = $state('');
  let error = $state<string | null>(null);
  let attaching = $state(false);

  $effect(() => {
    if (open) {
      mode = 'cdp';
      target = '';
      error = null;
    }
  });

  // A stale error goes away as soon as the person edits the field.
  $effect(() => {
    void target;
    void mode;
    error = null;
  });

  // "127.0.0.1:9222" is what people type; the daemon wants a full URL.
  const withScheme = (value: string) => (/^[a-z][a-z0-9+.-]*:\/\//i.test(value) ? value : `http://${value}`);

  async function attach(e: SubmitEvent) {
    e.preventDefault();
    attaching = true;
    error = null;
    const value = target.trim();
    try {
      const out = await unwrap(
        api.POST('/v1/instances/attach', { body: mode === 'cdp' ? { cdp_url: withScheme(value) } : { install_name: value } }),
      );
      toast.success('Attached to the browser');
      open = false;
      void goto(`/browsers/${out.instance.id}`);
    } catch (err) {
      if (err instanceof ApiError && ['InvalidArgument', 'Conflict', 'NotFound'].includes(err.code)) error = err.message;
      else toast.apiError(err, "Couldn't attach to the browser");
    } finally {
      attaching = false;
    }
  }
</script>

<Dialog bind:open size="sm" title="Attach a browser" description="Use a browser you already run. brr watches it and never stops it.">
  <form id="attach-form" onsubmit={attach}>
    <SegmentedControl label="How to find it" bind:value={mode} options={modes} />
    {#if mode === 'cdp'}
      <Input
        label="Debugging address"
        bind:value={target}
        type="text"
        required
        {error}
        placeholder="http://127.0.0.1:9222"
        hint="The address the browser prints when started with --remote-debugging-port."
        autocomplete="off"
      />
    {:else}
      <Input label="Browser name" bind:value={target} required {error} placeholder="chrome" hint="The name of a browser installed on this machine." autocomplete="off" />
    {/if}
  </form>
  {#snippet footer()}
    <Button onclick={() => (open = false)}>Cancel</Button>
    <Button variant="primary" type="submit" loading={attaching} disabled={!target.trim()} form="attach-form">Attach</Button>
  {/snippet}
</Dialog>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
</style>
