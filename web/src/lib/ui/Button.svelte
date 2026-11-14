<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLButtonAttributes } from 'svelte/elements';
  import Icon, { type IconName } from './Icon.svelte';
  import { tooltip } from './tooltip';

  type Props = Omit<HTMLButtonAttributes, 'children' | 'type' | 'onclick' | 'autofocus'> & {
    variant?: 'primary' | 'secondary' | 'ghost' | 'danger';
    size?: 'sm' | 'md';
    type?: 'button' | 'submit';
    href?: string;
    disabled?: boolean;
    loading?: boolean;
    icon?: IconName;
    iconOnly?: boolean;
    autofocus?: boolean;
    'aria-label'?: string;
    onclick?: (e: MouseEvent) => void;
    children?: Snippet;
  };

  let {
    variant = 'secondary',
    size = 'md',
    type = 'button',
    href,
    disabled = false,
    loading = false,
    icon,
    iconOnly = false,
    autofocus = false,
    children,
    onclick,
    ...rest
  }: Props = $props();

  const inactive = $derived(disabled || loading);

  // A loading button stays enabled so keyboard focus survives the request; its clicks are swallowed instead.
  function handleClick(e: MouseEvent) {
    if (loading) e.preventDefault();
    else onclick?.(e);
  }
  const tip = $derived(iconOnly ? rest['aria-label'] : undefined);
  const classes = $derived(`btn btn-${variant}${size === 'sm' ? ' btn-sm' : ''}${iconOnly ? ' btn-icon' : ''}`);
</script>

{#snippet content()}
  {#if loading}
    <span class="spinner" aria-hidden="true"></span>
  {:else if icon}
    <Icon name={icon} />
  {/if}
  {#if !iconOnly}{@render children?.()}{/if}
{/snippet}

{#if href && !inactive}
  <a class={classes} {href} use:tooltip={tip} data-autofocus={autofocus || undefined} {...rest as object}>{@render content()}</a>
{:else}
  <button
    class={classes}
    {type}
    {disabled}
    aria-disabled={loading || undefined}
    aria-busy={loading || undefined}
    onclick={handleClick}
    use:tooltip={tip}
    data-autofocus={autofocus || undefined}
    {...rest}
  >
    {@render content()}
  </button>
{/if}

<style>
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    height: var(--control-h-md);
    padding: 0 var(--space-3);
    border: 1px solid transparent;
    border-radius: var(--radius-md);
    font-size: var(--text-base-size);
    line-height: var(--text-base-line);
    font-weight: 500;
    white-space: nowrap;
    cursor: pointer;
    text-decoration: none;
    transition:
      background-color var(--duration-fast) var(--ease-standard),
      border-color var(--duration-fast) var(--ease-standard),
      color var(--duration-fast) var(--ease-standard);
  }

  .btn-sm {
    height: var(--control-h-sm);
    padding: 0 var(--space-2);
  }

  .btn-icon {
    width: var(--control-h-md);
    padding: 0;
  }

  .btn-icon.btn-sm {
    width: var(--control-h-sm);
  }

  .btn-primary {
    background: var(--accent);
    color: var(--accent-fg);
    box-shadow: var(--shadow-sm);
  }

  .btn-primary:hover {
    background: var(--accent-hover);
  }

  .btn-secondary {
    background: var(--surface);
    color: var(--text);
    border-color: var(--border-strong);
    box-shadow: var(--shadow-sm);
  }

  .btn-secondary:hover {
    background: var(--bg-muted);
  }

  .btn-ghost {
    background: transparent;
    color: var(--text-secondary);
  }

  .btn-ghost:hover {
    background: var(--bg-muted);
    color: var(--text);
  }

  .btn-danger {
    background: var(--danger);
    color: #fff;
  }

  .btn-danger:hover {
    background: var(--danger-hover);
  }

  .btn-secondary:active:not(:disabled, [aria-disabled='true']),
  .btn-ghost:active:not(:disabled, [aria-disabled='true']) {
    background: var(--bg-emphasis);
  }

  .btn:disabled,
  .btn[aria-disabled='true'] {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* Opacity alone dissolves a filled button into the dark background; a visible boundary and a readable label keep it findable. */
  :is(.btn-primary, .btn-secondary, .btn-danger):disabled {
    opacity: 1;
    background: var(--bg-muted);
    color: var(--text-tertiary);
    border-color: var(--border-control);
    box-shadow: none;
  }

  .spinner {
    width: 14px;
    height: 14px;
    border: 2px solid currentColor;
    border-right-color: transparent;
    border-radius: var(--radius-full);
    animation: spin 0.7s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
