<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import type { components } from '$lib/api/schema';
  import Button from '$lib/ui/Button.svelte';
  import Checkbox from '$lib/ui/Checkbox.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  type Credential = components['schemas']['Credential'];
  type Profile = components['schemas']['Profile'];

  let {
    open = $bindable(false),
    credential = null,
    profiles = [],
    onsaved,
  }: { open?: boolean; credential?: Credential | null; profiles?: Profile[]; onsaved?: () => void } = $props();

  let linked = $state<string[]>([]);
  let saving = $state(false);

  $effect(() => {
    if (open && credential) linked = [...credential.profile_ids];
  });

  function toggle(id: string, checked: boolean) {
    linked = checked ? [...linked, id] : linked.filter((p) => p !== id);
  }

  async function save() {
    if (!credential || saving) return;
    saving = true;
    try {
      await unwrap(
        api.PATCH('/v1/credentials/{credential_id}', { params: { path: { credential_id: credential.id } }, body: { profile_ids: linked } }),
      );
      toast.success('Links saved');
      open = false;
      onsaved?.();
    } catch (e) {
      toast.apiError(e, "Couldn't save the links");
    } finally {
      saving = false;
    }
  }
</script>

<Dialog bind:open title="Link profiles">
  <div class="profiles">
    {#each profiles as profile (profile.id)}
      <Checkbox label={profile.name} checked={linked.includes(profile.id)} onchange={(checked) => toggle(profile.id, checked)} />
    {/each}
    {#if profiles.length === 0}<p class="field-hint">No profiles yet.</p>{/if}
  </div>
  {#snippet footer()}
    <Button onclick={() => (open = false)} disabled={saving}>Cancel</Button>
    <Button variant="primary" loading={saving} onclick={save}>Save links</Button>
  {/snippet}
</Dialog>

<style>
  .profiles {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    max-height: 200px;
    overflow: auto;
  }
</style>
