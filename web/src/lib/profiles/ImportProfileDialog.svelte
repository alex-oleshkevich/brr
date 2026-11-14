<script lang="ts">
  import { goto } from '$app/navigation';
  import { ApiError } from '$lib/api/client';
  import Button from '$lib/ui/Button.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import Input from '$lib/ui/Input.svelte';
  import { toast } from '$lib/ui/toast.svelte';
  import { importArchive } from './archive';

  let { open = $bindable(false) }: { open?: boolean } = $props();

  let passphrase = $state('');
  let busy = $state(false);
  let error = $state<string | null>(null);
  let fileInput = $state<HTMLInputElement | undefined>(undefined);
  let hasFile = $state(false);

  $effect(() => {
    if (open) return;
    passphrase = '';
    error = null;
    hasFile = false;
    if (fileInput) fileInput.value = '';
  });

  async function run(event: SubmitEvent) {
    event.preventDefault();
    const file = fileInput?.files?.[0];
    if (!file || !passphrase || busy) return;
    busy = true;
    error = null;
    try {
      const out = await importArchive(file, passphrase);
      const missing = out.missing_extensions.length;
      toast.show({
        tone: 'success',
        title: 'Profile imported',
        description: missing > 0 ? `${missing} ${missing === 1 ? 'extension was' : 'extensions were'} not installed here.` : undefined,
      });
      open = false;
      await goto(`/profiles/${out.profile.id}`);
    } catch (e) {
      if (e instanceof ApiError && e.code === 'InvalidArgument') error = 'Wrong passphrase, or the file is not a brr archive.';
      else toast.apiError(e, "Couldn't import the profile");
    } finally {
      busy = false;
      passphrase = '';
    }
  }
</script>

<Dialog bind:open title="Import profile" description="The archive always becomes a new profile." dismissible={!busy}>
  <form id="import-form" onsubmit={run} novalidate>
    <div class="fields">
      <div class="field">
        <label class="field-label" for="archive-file">Archive (.age)</label>
        <input class="control" id="archive-file" type="file" accept=".age" bind:this={fileInput} onchange={() => (hasFile = !!fileInput?.files?.length)} />
      </div>
      <Input label="Passphrase" type="password" autocomplete="off" bind:value={passphrase} required {error} />
    </div>
  </form>
  {#snippet footer()}
    <Button onclick={() => (open = false)} disabled={busy}>Cancel</Button>
    <Button variant="primary" type="submit" form="import-form" loading={busy} disabled={!hasFile || !passphrase}>Import</Button>
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
