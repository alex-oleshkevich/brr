<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import Button from '$lib/ui/Button.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import SegmentedControl from '$lib/ui/SegmentedControl.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  let { open = $bindable(false), onsaved }: { open?: boolean; onsaved?: () => void } = $props();

  let format = $state<'chrome' | 'firefox'>('chrome');
  let importing = $state(false);
  let fileInput = $state<HTMLInputElement | undefined>(undefined);
  let hasFile = $state(false);

  $effect(() => {
    if (open) {
      hasFile = false;
      if (fileInput) fileInput.value = '';
    }
  });

  async function runImport() {
    const file = fileInput?.files?.[0];
    if (!file || importing) return;
    importing = true;
    try {
      const out = await unwrap(api.POST('/v1/credentials/import', { body: { format, file: await file.text() } }));
      toast.success(`Imported ${out.imported} credentials, skipped ${out.skipped}`);
      open = false;
      onsaved?.();
    } catch (e) {
      toast.apiError(e, "Couldn't import the CSV");
    } finally {
      importing = false;
    }
  }
</script>

<Dialog bind:open title="Import CSV">
  <div class="fields">
    <div class="field">
      <span class="field-label">Format</span>
      <SegmentedControl
        label="Format"
        bind:value={format}
        options={[
          { value: 'chrome', label: 'Chrome' },
          { value: 'firefox', label: 'Firefox' },
        ]}
      />
    </div>
    <div class="field">
      <label class="field-label" for="csv-file">Password export (.csv)</label>
      <input class="control" id="csv-file" type="file" accept=".csv" bind:this={fileInput} onchange={() => (hasFile = !!fileInput?.files?.length)} />
    </div>
  </div>
  {#snippet footer()}
    <Button onclick={() => (open = false)} disabled={importing}>Cancel</Button>
    <Button variant="primary" loading={importing} disabled={!hasFile} onclick={runImport}>Import</Button>
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
