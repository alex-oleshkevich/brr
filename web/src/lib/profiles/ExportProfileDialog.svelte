<script lang="ts">
  import { ApiError, api, unwrap } from '$lib/api/client';
  import { saveBlob } from '$lib/download';
  import Button from '$lib/ui/Button.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import Input from '$lib/ui/Input.svelte';
  import SegmentedControl from '$lib/ui/SegmentedControl.svelte';
  import { toast } from '$lib/ui/toast.svelte';
  import { exportArchive } from './archive';

  let {
    open = $bindable(false),
    profileId,
    profileName,
  }: { open?: boolean; profileId: string; profileName: string } = $props();

  let passphrase = $state('');
  let confirmation = $state('');
  let destination = $state<'download' | 'storage'>('download');
  let busy = $state(false);
  let error = $state<string | null>(null);

  // A passphrase must not outlive the dialog.
  $effect(() => {
    if (!open) reset();
  });

  function reset() {
    passphrase = '';
    confirmation = '';
    destination = 'download';
    error = null;
  }

  const mismatch = $derived(confirmation !== '' && confirmation !== passphrase ? 'The passphrases do not match.' : null);
  const ready = $derived(passphrase !== '' && passphrase === confirmation);

  async function run(event: SubmitEvent) {
    event.preventDefault();
    if (!ready || busy) return;
    busy = true;
    error = null;
    try {
      if (destination === 'download') {
        const out = await exportArchive(profileId, passphrase);
        saveBlob(out.blob, out.filename ?? `${profileName}.tar.zst.age`);
        toast.success('Profile exported');
      } else {
        await unwrap(api.POST('/v1/profiles/{profile}/export', { params: { path: { profile: profileId } }, body: { passphrase, destination } }));
        toast.success('Saved to the storage backend');
      }
      open = false;
    } catch (e) {
      if (e instanceof ApiError && e.code === 'InvalidArgument' && e.details.field === 'passphrase') error = e.message;
      else toast.apiError(e, "Couldn't export the profile");
    } finally {
      busy = false;
      passphrase = '';
      confirmation = '';
    }
  }
</script>

<Dialog bind:open title="Export profile" description="Credentials are not included." dismissible={!busy}>
  <form id="export-form" onsubmit={run} novalidate>
    <div class="fields">
      <Input label="Passphrase" type="password" autocomplete="new-password" bind:value={passphrase} required error={error} />
      <Input label="Confirm passphrase" type="password" autocomplete="new-password" bind:value={confirmation} required error={mismatch} />
      <div class="field">
        <span class="field-label">Destination</span>
        <SegmentedControl
          label="Destination"
          bind:value={destination}
          options={[
            { value: 'download', label: 'Download' },
            { value: 'storage', label: 'Storage backend' },
          ]}
        />
      </div>
    </div>
  </form>
  {#snippet footer()}
    <Button onclick={() => (open = false)} disabled={busy}>Cancel</Button>
    <Button variant="primary" type="submit" form="export-form" loading={busy} disabled={!ready}>Export</Button>
  {/snippet}
</Dialog>

<style>
  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    align-items: flex-start;
  }
</style>
