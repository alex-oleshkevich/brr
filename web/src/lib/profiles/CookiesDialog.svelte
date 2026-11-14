<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import { saveBlob } from '$lib/download';
  import Button from '$lib/ui/Button.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import SegmentedControl from '$lib/ui/SegmentedControl.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  let {
    open = $bindable(false),
    mode,
    profileId,
    profileName,
  }: { open?: boolean; mode: 'export' | 'import'; profileId: string; profileName: string } = $props();

  let format = $state<'json' | 'netscape'>('json');
  let busy = $state(false);
  let fileInput = $state<HTMLInputElement | undefined>(undefined);
  let hasFile = $state(false);

  $effect(() => {
    if (open) {
      format = 'json';
      hasFile = false;
      if (fileInput) fileInput.value = '';
    }
  });

  async function exportCookies() {
    busy = true;
    try {
      const out = await unwrap(api.GET('/v1/profiles/{profile}/cookies', { params: { path: { profile: profileId }, query: { format } } }));
      saveBlob(new Blob([out.data], { type: format === 'json' ? 'application/json' : 'text/plain' }), `${profileName}-cookies.${format === 'json' ? 'json' : 'txt'}`);
      open = false;
    } catch (e) {
      toast.apiError(e, "Couldn't export cookies");
    } finally {
      busy = false;
    }
  }

  async function importCookies() {
    const file = fileInput?.files?.[0];
    if (!file || busy) return;
    busy = true;
    try {
      const data = await file.text();
      await unwrap(api.PUT('/v1/profiles/{profile}/cookies', { params: { path: { profile: profileId } }, body: { data, format } }));
      toast.success('Cookies imported');
      open = false;
    } catch (e) {
      toast.apiError(e, "Couldn't import cookies");
    } finally {
      busy = false;
      if (fileInput) fileInput.value = '';
    }
  }
</script>

<Dialog bind:open title={mode === 'export' ? 'Export cookies' : 'Import cookies'}>
  <div class="fields">
    <div class="field">
      <span class="field-label">Format</span>
      <SegmentedControl
        label="Format"
        bind:value={format}
        options={[
          { value: 'json', label: 'JSON (Playwright)' },
          { value: 'netscape', label: 'cookies.txt (curl, wget)' },
        ]}
      />
    </div>
    {#if mode === 'import'}
      <div class="field">
        <label class="field-label" for="cookies-file">Cookie file</label>
        <input class="control" id="cookies-file" type="file" accept=".json,.txt" bind:this={fileInput} onchange={() => (hasFile = !!fileInput?.files?.length)} />
      </div>
    {/if}
  </div>
  {#snippet footer()}
    <Button onclick={() => (open = false)} disabled={busy}>Cancel</Button>
    {#if mode === 'export'}
      <Button variant="primary" loading={busy} onclick={exportCookies}>Download</Button>
    {:else}
      <Button variant="primary" loading={busy} disabled={!hasFile} onclick={importCookies}>Import</Button>
    {/if}
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
