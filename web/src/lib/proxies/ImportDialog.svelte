<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import { importLineError } from '$lib/proxies/format';
  import Button from '$lib/ui/Button.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import Select from '$lib/ui/Select.svelte';
  import Textarea from '$lib/ui/Textarea.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  let { open = $bindable(false), onimported }: { open?: boolean; onimported: () => void } = $props();

  const schemes = [
    { value: 'http', label: 'HTTP' },
    { value: 'https', label: 'HTTPS' },
    { value: 'socks5', label: 'SOCKS5' },
  ];

  let scheme = $state('http');
  let text = $state('');
  let importing = $state(false);

  // The pasted list holds passwords, so it is cleared whenever the dialog opens or closes.
  $effect(() => {
    void open;
    text = '';
  });

  const parsed = $derived.by(() => {
    let valid = 0;
    const invalid: { line: number; reason: string }[] = [];
    text.split('\n').forEach((raw, index) => {
      const line = raw.trim();
      if (!line) return;
      const reason = importLineError(line);
      if (reason) invalid.push({ line: index + 1, reason });
      else valid++;
    });
    return { valid, invalid };
  });

  const SHOWN_ERRORS = 5;

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (parsed.valid === 0) return;
    importing = true;
    try {
      const result = await unwrap(api.POST('/v1/proxies/import', { body: { scheme: scheme as 'http' | 'https' | 'socks5', text } }));
      toast.success(
        `Imported ${result.imported} ${result.imported === 1 ? 'proxy' : 'proxies'}`,
        result.duplicates > 0 ? `${result.duplicates} already existed.` : undefined,
      );
      text = '';
      onimported();
      open = false;
    } catch (e) {
      toast.apiError(e, "Couldn't import proxies");
    } finally {
      importing = false;
    }
  }
</script>

<Dialog bind:open size="lg" title="Bulk import proxies" dismissible={!importing}>
  <form id="import-form" onsubmit={submit}>
    <div class="fields">
      <Select label="Scheme" bind:value={scheme} options={schemes} />
      <div>
        <Textarea label="Proxies" bind:value={text} rows={10} placeholder={'host:port\nhost:port:user:pass\nsocks5://user:pass@host:port'} />
        <p class="help">One per line: <code>host:port</code>, <code>host:port:user:pass</code> or <code>scheme://user:pass@host:port</code>. A scheme in the line overrides the one above.</p>
        <p class="count" aria-live="polite">
          {parsed.valid} {parsed.valid === 1 ? 'proxy' : 'proxies'}{parsed.invalid.length ? ` · ${parsed.invalid.length} invalid ${parsed.invalid.length === 1 ? 'line' : 'lines'}` : ''}
        </p>
        {#if parsed.invalid.length}
          <ul class="errors">
            {#each parsed.invalid.slice(0, SHOWN_ERRORS) as { line, reason } (line)}<li>Line {line}: {reason}</li>{/each}
            {#if parsed.invalid.length > SHOWN_ERRORS}<li>and {parsed.invalid.length - SHOWN_ERRORS} more</li>{/if}
          </ul>
        {/if}
      </div>
    </div>
  </form>
  {#snippet footer()}
    <Button onclick={() => (open = false)} disabled={importing}>Cancel</Button>
    <Button variant="primary" type="submit" form="import-form" loading={importing} disabled={parsed.valid === 0}>
      Import {parsed.valid} {parsed.valid === 1 ? 'proxy' : 'proxies'}
    </Button>
  {/snippet}
</Dialog>

<style>
  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .help {
    margin-top: var(--space-2);
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }

  .errors {
    margin: var(--space-1) 0 0;
    padding: 0;
    list-style: none;
    color: var(--danger-fg);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }

  .count {
    margin-top: var(--space-1);
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }
</style>
