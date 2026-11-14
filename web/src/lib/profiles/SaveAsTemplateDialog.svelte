<script lang="ts">
  import { goto } from '$app/navigation';
  import { api, unwrap } from '$lib/api/client';
  import Button from '$lib/ui/Button.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import Input from '$lib/ui/Input.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  let { open = $bindable(false), profileId, profileName }: { open?: boolean; profileId: string; profileName: string } = $props();

  let name = $state('');
  let saving = $state(false);

  $effect(() => {
    if (open) name = profileName;
  });

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (!name.trim() || saving) return;
    saving = true;
    try {
      const template = await unwrap(
        api.POST('/v1/profiles/{profile}/save-as-template', { params: { path: { profile: profileId } }, body: { name: name.trim() } }),
      );
      open = false;
      toast.show({ tone: 'success', title: 'Template saved', action: { label: 'Open', onclick: () => goto(`/templates/${template.id}`) } });
    } catch (e) {
      toast.apiError(e, "Couldn't save the template");
    } finally {
      saving = false;
    }
  }
</script>

<Dialog bind:open title="Save as template">
  <form id="save-template-form" onsubmit={save}>
    <Input label="Name" bind:value={name} required />
  </form>
  {#snippet footer()}
    <Button onclick={() => (open = false)} disabled={saving}>Cancel</Button>
    <Button variant="primary" type="submit" form="save-template-form" loading={saving} disabled={!name.trim()}>Save template</Button>
  {/snippet}
</Dialog>
