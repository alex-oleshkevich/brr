<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import Dialog from '$lib/ui/Dialog.svelte';
  import Input from '$lib/ui/Input.svelte';
  import SegmentedControl from '$lib/ui/SegmentedControl.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  let { open = $bindable(false), onadded }: { open?: boolean; onadded?: () => void } = $props();

  let mode = $state<'cws' | 'amo' | 'file'>('cws');
  let ref = $state('');
  let error = $state<string | null>(null);
  let adding = $state(false);
  let fileInput = $state<HTMLInputElement | undefined>(undefined);
  let hasFile = $state(false);

  $effect(() => {
    if (open) {
      ref = '';
      error = null;
      hasFile = false;
      if (fileInput) fileInput.value = '';
    }
  });

  function cwsId(value: string): string | null {
    if (/^[a-p]{32}$/.test(value)) return value;
    try {
      const url = new URL(value);
      if (url.hostname !== 'chromewebstore.google.com' && !(url.hostname === 'chrome.google.com' && url.pathname.startsWith('/webstore'))) return null;
      const match = url.pathname.match(/([a-p]{32})(?:[/?#]|$)/);
      return match?.[1] ?? null;
    } catch {
      return null;
    }
  }

  async function toBase64(file: File): Promise<string> {
    const bytes = new Uint8Array(await file.arrayBuffer());
    let binary = '';
    for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    return btoa(binary);
  }

  async function add(event: SubmitEvent) {
    event.preventDefault();
    if (adding) return;
    error = null;

    let body: { source: 'cws' | 'amo' | 'local'; ref?: string; file?: string };
    if (mode === 'cws') {
      const id = cwsId(ref.trim());
      if (!id) {
        error = 'Enter a 32-character extension ID (a–p) or a Chrome Web Store URL.';
        return;
      }
      body = { source: 'cws', ref: id };
    } else if (mode === 'amo') {
      if (!ref.trim()) {
        error = 'Enter an add-on slug or URL.';
        return;
      }
      body = { source: 'amo', ref: ref.trim() };
    } else {
      const file = fileInput?.files?.[0];
      if (!file) {
        error = 'Choose an extension file.';
        return;
      }
      // Guard before the read: a large file takes long enough for a second submit.
      adding = true;
      try {
        body = { source: 'local', file: await toBase64(file) };
      } catch (e) {
        adding = false;
        toast.apiError(e, "Couldn't read the file");
        return;
      }
    }

    adding = true;
    try {
      const known = new Set((await unwrap(api.GET('/v1/extensions'))).extensions.map((e) => e.id));
      const ext = await unwrap(api.POST('/v1/extensions', { body }));
      open = false;
      onadded?.();
      toast.success(known.has(ext.id) ? 'Already in your library' : `Added ${ext.name}`);
    } catch (e) {
      toast.apiError(e, "Couldn't add the extension");
    } finally {
      adding = false;
      if (fileInput) fileInput.value = '';
    }
  }
</script>

<Dialog bind:open title="Add an extension" submit={{ label: 'Add extension', form: 'add-extension-form', loading: adding, disabled: mode === 'file' && !hasFile }}>
  <form id="add-extension-form" onsubmit={add}>
    <div class="fields">
      <SegmentedControl
        label="Source"
        bind:value={mode}
        options={[
          { value: 'cws', label: 'Chrome Web Store' },
          { value: 'amo', label: 'Firefox Add-ons' },
          { value: 'file', label: 'File' },
        ]}
      />
      {#if mode === 'cws'}
        <Input label="Extension ID or Web Store URL" bind:value={ref} error={error} />
      {:else if mode === 'amo'}
        <Input label="Add-on slug or URL" bind:value={ref} error={error} />
      {:else}
        <div class="field">
          <label class="field-label" for="extension-file">Extension file</label>
          <input class="control" id="extension-file" type="file" accept=".crx,.xpi,.zip" bind:this={fileInput} onchange={() => (hasFile = !!fileInput?.files?.length)} />
          {#if error}<p class="field-error">{error}</p>{/if}
        </div>
      {/if}
      <p class="field-hint">A .zip is a packed unpacked-folder extension. The engine (Chromium or Camoufox) is detected from the file. To add an unpacked folder directly, run brr extension add &lt;path&gt; in a terminal.</p>
    </div>
  </form>
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

  input[type='file'] {
    width: 100%;
    height: auto;
    padding: var(--space-2) var(--space-3);
  }

  input[type='file']::file-selector-button {
    margin-right: var(--space-3);
    padding: var(--space-1) var(--space-3);
    background: var(--surface);
    border: 1px solid var(--border-control);
    border-radius: var(--radius-md);
    font: inherit;
    cursor: pointer;
  }
</style>
