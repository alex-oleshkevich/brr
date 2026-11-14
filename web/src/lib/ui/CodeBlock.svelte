<script lang="ts">
  import { toast } from './toast.svelte';
  import Button from './Button.svelte';

  let { code, label, maxHeight = '320px' }: { code: string; label: string; maxHeight?: string } = $props();

  const canCopy = typeof navigator !== 'undefined' && !!navigator.clipboard;

  async function copy() {
    await navigator.clipboard.writeText(code);
    toast.success('Copied');
  }
</script>

<div class="block">
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <pre tabindex="0" aria-label={label} style:max-height={maxHeight}><code>{code}</code></pre>
  {#if canCopy}
    <div class="copy"><Button variant="ghost" size="sm" iconOnly icon="copy" aria-label="Copy {label}" onclick={copy} /></div>
  {/if}
</div>

<style>
  .block {
    position: relative;
  }

  pre {
    margin: 0;
    padding: var(--space-3) var(--space-4);
    overflow: auto;
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
  }

  code {
    padding: 0;
    background: none;
    font-size: var(--text-sm-size);
    line-height: 18px;
    white-space: pre;
  }

  .copy {
    position: absolute;
    top: var(--space-2);
    right: var(--space-2);
  }
</style>
