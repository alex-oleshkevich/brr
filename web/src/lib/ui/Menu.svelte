<script lang="ts" module>
  export type MenuItem = {
    label: string;
    onselect: () => void;
    tone?: 'danger';
    disabled?: boolean;
    hint?: string;
    checked?: boolean;
  };

  export type TriggerProps = {
    onclick: () => void;
    onpointerdown: () => void;
    onkeydown: (e: KeyboardEvent) => void;
    'aria-haspopup': 'menu';
    'aria-expanded': boolean;
    'aria-label': string;
  };
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';
  import Button from './Button.svelte';
  import Icon from './Icon.svelte';
  import { placePopover } from './popover';

  let {
    label,
    items,
    align = 'end',
    trigger,
  }: {
    label: string;
    items: (MenuItem | 'separator')[];
    align?: 'start' | 'end';
    trigger?: Snippet<[TriggerProps]>;
  } = $props();

  let anchor: HTMLSpanElement;
  let panel: HTMLDivElement;
  let expanded = $state(false);
  // Light dismiss closes the panel on pointerdown, before the trigger's click would toggle it.
  let wasOpen = false;

  const entries = () => [...panel.querySelectorAll<HTMLElement>('[role^="menuitem"]')];
  const triggerEl = () => anchor.querySelector<HTMLElement>('button, a');

  function show(focus: 'first' | 'last' = 'first') {
    panel.showPopover();
    placePopover(anchor, panel, align);
    const all = entries();
    (focus === 'first' ? all[0] : all[all.length - 1])?.focus();
  }

  function close() {
    panel.hidePopover();
    triggerEl()?.focus();
  }

  const triggerProps: TriggerProps = {
    onpointerdown: () => (wasOpen = expanded),
    onclick() {
      if (expanded) close();
      else if (!wasOpen) show();
      wasOpen = false;
    },
    onkeydown(e) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        show(e.key === 'ArrowDown' ? 'first' : 'last');
      }
    },
    get 'aria-haspopup'() {
      return 'menu' as const;
    },
    get 'aria-expanded'() {
      return expanded;
    },
    get 'aria-label'() {
      return label;
    },
  };

  function select(item: MenuItem) {
    if (item.disabled) return;
    close();
    item.onselect();
  }

  function onkeydown(e: KeyboardEvent) {
    const all = entries();
    const current = all.indexOf(document.activeElement as HTMLElement);
    const move = (index: number) => {
      e.preventDefault();
      all[(index + all.length) % all.length]?.focus();
    };

    if (e.key === 'ArrowDown') move(current + 1);
    else if (e.key === 'ArrowUp') move(current - 1);
    else if (e.key === 'Home') move(0);
    else if (e.key === 'End') move(all.length - 1);
    else if (e.key === 'Tab') {
      e.preventDefault();
      close();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      close();
    }
  }
</script>

<span class="anchor" bind:this={anchor}>
  {#if trigger}
    {@render trigger(triggerProps)}
  {:else}
    <Button variant="ghost" size="sm" iconOnly icon="more" {...triggerProps} />
  {/if}
</span>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class="menu"
  role="menu"
  aria-label={label}
  popover="auto"
  tabindex="-1"
  bind:this={panel}
  {onkeydown}
  ontoggle={(e) => (expanded = (e as ToggleEvent).newState === 'open')}
>
  {#each items as item}
    {#if item === 'separator'}
      <div class="separator" role="separator"></div>
    {:else}
      <button
        type="button"
        class="item"
        class:danger={item.tone === 'danger'}
        role={item.checked === undefined ? 'menuitem' : 'menuitemradio'}
        aria-checked={item.checked}
        aria-disabled={item.disabled || undefined}
        {...item.disabled && item.hint ? { 'aria-description': item.hint } : {}}
        tabindex="-1"
        onclick={() => select(item)}
      >
        {#if item.checked !== undefined}
          <span class="check">{#if item.checked}<Icon name="check" />{/if}</span>
        {/if}
        <span class="text">{item.label}{#if item.disabled && item.hint}<span class="reason">{item.hint}</span>{/if}</span>
      </button>
    {/if}
  {/each}
</div>

<style>
  .anchor {
    display: inline-flex;
  }

  .menu {
    position: fixed;
    inset: auto;
    margin: 0;
    min-width: var(--menu-min-w);
    padding: var(--space-1);
    background: var(--surface-raised);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-md);
    opacity: 0;
    transform: translateY(4px);
    transition:
      opacity var(--duration-base) var(--ease-out),
      transform var(--duration-base) var(--ease-out);
  }

  .menu:popover-open {
    opacity: 1;
    transform: none;
  }

  @starting-style {
    .menu:popover-open {
      opacity: 0;
      transform: translateY(4px);
    }
  }

  .item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    min-height: var(--control-h-md);
    padding: var(--space-1) var(--space-2);
    background: none;
    border: 0;
    border-radius: var(--radius-sm);
    text-align: left;
    cursor: pointer;
  }

  .item:hover,
  .item:focus-visible {
    background: var(--bg-muted);
  }

  .item:focus-visible {
    outline-offset: -2px;
  }

  .item.danger {
    color: var(--danger-fg);
  }

  .item[aria-disabled='true'] {
    color: var(--text-disabled);
    cursor: not-allowed;
  }

  .text {
    display: flex;
    flex-direction: column;
  }

  .reason {
    max-width: 260px;
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
    color: var(--text-tertiary);
    white-space: normal;
  }

  .check {
    display: flex;
    width: 16px;
    color: var(--text-secondary);
  }

  .separator {
    height: 1px;
    margin: var(--space-1) calc(var(--space-1) * -1);
    background: var(--border);
  }
</style>
