<script lang="ts">
  import { ApiError, api, unwrap } from '$lib/api/client';
  import Button from '$lib/ui/Button.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import Input from '$lib/ui/Input.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  let { open = $bindable(false), instanceId, onpromoted }: { open?: boolean; instanceId: string; onpromoted?: () => void } = $props();

  const uid = $props.id();
  const formId = `promote-form-${uid}`;

  let name = $state('');
  let error = $state<string | null>(null);
  let saving = $state(false);

  $effect(() => {
    if (open) {
      name = '';
      error = null;
    }
  });

  async function promote(e: SubmitEvent) {
    e.preventDefault();
    saving = true;
    error = null;
    try {
      await unwrap(
        api.POST('/v1/instances/{instance_id}/promote', { params: { path: { instance_id: instanceId } }, body: { name: name.trim() } }),
      );
      toast.success(`This run's data will be kept as ${name.trim()}`, 'The profile is saved when this browser stops.');
      open = false;
      onpromoted?.();
    } catch (err) {
      if (err instanceof ApiError && ['Conflict', 'InvalidArgument'].includes(err.code)) error = err.message;
      else toast.apiError(err, "Couldn't promote the browser");
    } finally {
      saving = false;
    }
  }
</script>

<Dialog
  bind:open
  size="sm"
  title="Keep this run's data"
  description="When this browser stops, its data is kept as a new profile that keeps its data."
>
  <form id={formId} onsubmit={promote}>
    <Input label="Name" bind:value={name} required {error} autocomplete="off" />
  </form>
  {#snippet footer()}
    <Button onclick={() => (open = false)}>Cancel</Button>
    <Button variant="primary" type="submit" loading={saving} disabled={!name.trim()} form={formId}>Keep data</Button>
  {/snippet}
</Dialog>
