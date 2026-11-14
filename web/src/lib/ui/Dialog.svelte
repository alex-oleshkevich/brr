<script lang="ts">
  import type { Snippet } from 'svelte';
  import Button from './Button.svelte';

  let {
    open = $bindable(false),
    title,
    description,
    size = 'md',
    dismissible = true,
    children,
    footer,
    submit,
    onclose,
  }: {
    open?: boolean;
    title: string;
    description?: string;
    size?: 'sm' | 'md' | 'lg';
    dismissible?: boolean;
    children?: Snippet;
    footer?: Snippet;
    // Standard Cancel + primary submit footer. `label` is "<Verb> <object>" ("Add proxy"), never "Save" or "OK".
    submit?: { label: string; form: string; loading?: boolean; disabled?: boolean; tone?: 'primary' | 'danger' };
    onclose?: () => void;
  } = $props();

  const uid = $props.id();
  let dialog: HTMLDialogElement;

  $effect(() => {
    if (open && !dialog.open) {
      dialog.showModal();
      dialog.querySelector<HTMLElement>('[data-autofocus], [autofocus], .body input, .body select, .body textarea')?.focus();
    } else if (!open && dialog.open) {
      dialog.close();
    }
  });

  function handleClose() {
    open = false;
    onclose?.();
  }
</script>

<!-- Esc and focus handling are native to <dialog>; the click handler only adds backdrop dismissal. -->
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog
  bind:this={dialog}
  class="dialog dialog-{size}"
  aria-labelledby="dialog-{uid}-title"
  aria-describedby={description ? `dialog-${uid}-desc` : undefined}
  onclose={handleClose}
  oncancel={(e) => !dismissible && e.preventDefault()}
  onclick={(e) => dismissible && e.target === dialog && dialog.close()}
>
  <div class="panel">
    <header>
      <div>
        <h2 id="dialog-{uid}-title">{title}</h2>
        {#if description}<p id="dialog-{uid}-desc">{description}</p>{/if}
      </div>
      {#if dismissible}
        <Button variant="ghost" size="sm" iconOnly icon="x" aria-label="Close" onclick={() => dialog.close()} />
      {/if}
    </header>
    {#if children}<div class="body">{@render children()}</div>{/if}
    {#if footer}
      <footer>{@render footer()}</footer>
    {:else if submit}
      <footer>
        <Button onclick={() => dialog.close()} disabled={submit.loading}>Cancel</Button>
        <Button variant={submit.tone ?? 'primary'} type="submit" form={submit.form} loading={submit.loading} disabled={submit.disabled}>{submit.label}</Button>
      </footer>
    {/if}
  </div>
</dialog>

<style>
  dialog {
    width: calc(100vw - var(--space-8));
    max-width: var(--dialog-w-md);
    max-height: 85vh;
    padding: 0;
    overflow: hidden;
    background: var(--surface-raised);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-xl);
    box-shadow: var(--shadow-lg);
    opacity: 0;
    transform: scale(0.98);
    transition:
      opacity var(--duration-slow) var(--ease-out),
      transform var(--duration-slow) var(--ease-out);
  }

  dialog[open] {
    opacity: 1;
    transform: none;
  }

  @starting-style {
    dialog[open] {
      opacity: 0;
      transform: scale(0.98);
    }
  }

  dialog::backdrop {
    background: var(--overlay);
    transition: opacity var(--duration-base) var(--ease-standard);
  }

  @starting-style {
    dialog[open]::backdrop {
      opacity: 0;
    }
  }

  .dialog-sm {
    max-width: var(--dialog-w-sm);
  }

  .dialog-lg {
    max-width: var(--dialog-w-lg);
  }

  .panel {
    display: flex;
    flex-direction: column;
    max-height: 85vh;
    padding: var(--space-6);
  }

  header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-4);
  }

  h2 {
    font-size: var(--text-lg-size);
    line-height: var(--text-lg-line);
    letter-spacing: 0;
  }

  p {
    margin-top: var(--space-1);
    color: var(--text-secondary);
  }

  .body {
    min-height: 0;
    /* The padding keeps focus rings of edge controls inside the scrolling box. */
    margin: var(--space-4) calc(var(--space-1) * -1) 0;
    padding: var(--space-1);
    overflow: auto;
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    margin-top: var(--space-5);
  }
</style>
