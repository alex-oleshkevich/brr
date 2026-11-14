<script lang="ts">
  import { goto } from '$app/navigation';
  import { ApiError, api, unwrap } from '$lib/api/client';
  import type { components } from '$lib/api/schema';
  import { bytes } from '$lib/format';
  import { profileNameError, restoredName } from '$lib/profiles/restore';
  import Dialog from '$lib/ui/Dialog.svelte';
  import Input from '$lib/ui/Input.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  type Backup = components['schemas']['Backup'];

  let { backup = $bindable(null), onrestored }: { backup?: Backup | null; onrestored: () => void } = $props();

  const FORM_ID = 'restore-backup-form';

  let open = $state(false);
  let name = $state('');
  let restoring = $state(false);
  let serverError = $state<string | null>(null);
  const nameError = $derived(profileNameError(name.trim()));

  $effect(() => {
    if (backup && !open) {
      name = restoredName(backup.profile_name, backup.created_at);
      serverError = null;
      open = true;
    }
  });

  $effect(() => {
    if (!open) backup = null;
  });

  async function restore(event: SubmitEvent) {
    event.preventDefault();
    if (!backup || restoring || nameError) return;
    restoring = true;
    try {
      const out = await unwrap(api.POST('/v1/backups/{backup_id}/restore', { params: { path: { backup_id: backup.id } }, body: { name: name.trim() } }));
      open = false;
      onrestored();
      toast.show({ tone: 'success', title: `Restored as ${out.profile.name}`, action: { label: 'Open', onclick: () => goto(`/profiles/${out.profile.id}`) } });
    } catch (e) {
      if (e instanceof ApiError && (e.code === 'Conflict' || (e.code === 'InvalidArgument' && e.details?.field === 'name'))) serverError = e.message;
      else toast.apiError(e, "Couldn't restore the backup");
    } finally {
      restoring = false;
    }
  }
</script>

<Dialog
  bind:open
  title="Restore as new profile"
  description="This downloads {bytes(backup?.size_bytes ?? 0)}, decrypts it and creates a new profile. It can take a minute. Logins aren't included in backups; re-link them after restoring."
  dismissible={!restoring}
  submit={{ label: restoring ? 'Restoring…' : 'Restore profile', form: FORM_ID, loading: restoring, disabled: !!nameError }}
>
  <form id={FORM_ID} onsubmit={restore}>
    <Input label="New profile name" bind:value={name} required error={name.trim() ? (nameError ?? serverError) : null} disabled={restoring} />
  </form>
</Dialog>
