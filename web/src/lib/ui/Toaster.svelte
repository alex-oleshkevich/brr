<script lang="ts">
  import Button from './Button.svelte';
  import Icon, { type IconName } from './Icon.svelte';
  import { toast, type ToastItem } from './toast.svelte';

  const icons: Record<ToastItem['tone'], IconName> = {
    neutral: 'info',
    success: 'circle-check',
    warning: 'alert',
    danger: 'alert',
  };

  let region: HTMLElement;

  // Showing the popover again puts it back on top of any modal dialog opened since the last toast.
  $effect(() => {
    if (toast.items.length === 0) {
      if (region.matches(':popover-open')) region.hidePopover();
      return;
    }
    if (region.matches(':popover-open')) region.hidePopover();
    region.showPopover();
  });

  function expire(node: HTMLElement, item: ToastItem) {
    let remaining = item.timeoutMs ?? (item.tone === 'danger' ? 8000 : 5000);
    let started = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const start = () => {
      started = Date.now();
      timer = setTimeout(() => toast.dismiss(item.id), remaining);
    };
    const pause = () => {
      if (timer === undefined) return;
      clearTimeout(timer);
      timer = undefined;
      remaining -= Date.now() - started;
    };
    const resume = () => {
      if (timer === undefined && !node.matches(':hover, :focus-within')) start();
    };

    node.addEventListener('pointerenter', pause);
    node.addEventListener('focusin', pause);
    node.addEventListener('pointerleave', resume);
    node.addEventListener('focusout', resume);
    start();

    return { destroy: () => clearTimeout(timer) };
  }
</script>

<section class="region" popover="manual" aria-label="Status messages" aria-live="polite" bind:this={region}>
  {#each toast.items as item (item.id)}
    <div class="toast" role={item.tone === 'danger' ? 'alert' : undefined} use:expire={item}>
      <span class="icon tone-{item.tone}"><Icon name={icons[item.tone]} /></span>
      <div class="text">
        <p class="title">{item.title}</p>
        {#if item.description}<p class="description">{item.description}</p>{/if}
        {#if item.action}
          <div class="action">
            <Button size="sm" onclick={() => { item.action?.onclick(); toast.dismiss(item.id); }}>{item.action.label}</Button>
          </div>
        {/if}
      </div>
      <Button variant="ghost" size="sm" iconOnly icon="x" aria-label="Dismiss" onclick={() => toast.dismiss(item.id)} />
    </div>
  {/each}
</section>

<style>
  .region {
    position: fixed;
    inset: auto var(--space-6) var(--space-6) auto;
    margin: 0;
    padding: 0;
    border: 0;
    background: transparent;
    color: inherit;
    overflow: visible;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    width: var(--toast-w);
    max-width: calc(100vw - var(--space-8));
    pointer-events: none;
  }

  .region:not(:popover-open) {
    display: none;
  }

  .toast {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    background: var(--surface-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-md);
    pointer-events: auto;
    transition:
      opacity var(--duration-base) var(--ease-out),
      transform var(--duration-base) var(--ease-out);
  }

  @starting-style {
    .toast {
      opacity: 0;
      transform: translateY(4px);
    }
  }

  .icon {
    display: flex;
    margin-top: 2px;
  }

  .tone-neutral {
    color: var(--neutral-solid);
  }

  .tone-success {
    color: var(--success-solid);
  }

  .tone-warning {
    color: var(--warning-solid);
  }

  .tone-danger {
    color: var(--danger-solid);
  }

  .text {
    flex: 1;
    min-width: 0;
  }

  .title {
    font-weight: 500;
  }

  .description {
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }

  .action {
    margin-top: var(--space-2);
  }
</style>
