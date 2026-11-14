<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import { saveBlob } from '$lib/download';
  import Button from '$lib/ui/Button.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  let {
    open = $bindable(false),
    mode,
    profileId,
    profileName,
  }: { open?: boolean; mode: 'export' | 'import'; profileId: string; profileName: string } = $props();

  let busy = $state(false);

  $effect(() => {
    if (open) {
      hasFile = false;
      if (fileInput) fileInput.value = '';
    }
  });
  let fileInput = $state<HTMLInputElement | undefined>(undefined);
  let hasFile = $state(false);

  async function exportBookmarks() {
    busy = true;
    try {
      const out = await unwrap(api.GET('/v1/profiles/{profile}/bookmarks', { params: { path: { profile: profileId } } }));
      saveBlob(new Blob([out.data], { type: 'text/html' }), `${profileName}-bookmarks.html`);
      open = false;
    } catch (e) {
      toast.apiError(e, "Couldn't export bookmarks");
    } finally {
      busy = false;
    }
  }

  async function importBookmarks() {
    const file = fileInput?.files?.[0];
    if (!file || busy) return;
    busy = true;
    try {
      const data = await file.text();
      await unwrap(api.PUT('/v1/profiles/{profile}/bookmarks', { params: { path: { profile: profileId } }, body: { data } }));
      toast.success('Bookmarks imported');
      open = false;
    } catch (e) {
      toast.apiError(e, "Couldn't import bookmarks");
    } finally {
      busy = false;
      if (fileInput) fileInput.value = '';
    }
  }
</script>

<Dialog bind:open title={mode === 'export' ? 'Export bookmarks' : 'Import bookmarks'}>
  {#if mode === 'import'}
    <div class="field">
      <label class="field-label" for="bookmarks-file">Bookmarks file</label>
      <input class="control" id="bookmarks-file" type="file" accept=".html" bind:this={fileInput} onchange={() => (hasFile = !!fileInput?.files?.length)} />
    </div>
  {:else}
    <p class="field-hint">Downloads this profile's bookmarks as an HTML file.</p>
  {/if}
  {#snippet footer()}
    <Button onclick={() => (open = false)} disabled={busy}>Cancel</Button>
    {#if mode === 'export'}
      <Button variant="primary" loading={busy} onclick={exportBookmarks}>Download</Button>
    {:else}
      <Button variant="primary" loading={busy} disabled={!hasFile} onclick={importBookmarks}>Import</Button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    align-items: flex-start;
  }
</style>
