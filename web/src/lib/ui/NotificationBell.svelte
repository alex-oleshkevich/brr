<script lang="ts">
  import { clock } from '$lib/clock.svelte';
  import { relative } from '$lib/format';
  import { goto } from '$app/navigation';
  import { toast } from './toast.svelte';
  import { notificationHref, notificationTitle, notifications } from '$lib/notifications.svelte';
  import Button from './Button.svelte';
  import EmptyState from './EmptyState.svelte';
  import Icon from './Icon.svelte';
  import { placePopover } from './popover';

  let anchor: HTMLSpanElement;
  let panel: HTMLDivElement;
  let expanded = $state(false);
  let wasOpen = false;

  const label = $derived(notifications.unread > 0 ? `Notifications, ${notifications.unread} unread` : 'Notifications');

  function show() {
    panel.showPopover();
    placePopover(anchor, panel, 'end');
  }

  async function follow(e: Parameters<typeof notificationHref>[0]) {
    try {
      const href = await notificationHref(e);
      if (!href) return;
      close();
      await goto(href);
    } catch (err) {
      toast.apiError(err, "Couldn't open this notification");
    }
  }

  function close() {
    panel.hidePopover();
    anchor.querySelector('button')?.focus();
  }
</script>

<span class="anchor" bind:this={anchor}>
  <Button
    variant="ghost"
    iconOnly
    icon="bell"
    aria-label={label}
    aria-haspopup="dialog"
    aria-expanded={expanded}
    onpointerdown={() => (wasOpen = expanded)}
    onclick={() => {
      if (expanded) close();
      else if (!wasOpen) show();
      wasOpen = false;
    }}
  />
  {#if notifications.unread > 0}<span class="pill num" class:needs-you={notifications.needsYou} aria-hidden="true">{notifications.unread}</span>{/if}
</span>

<div
  class="panel"
  popover="auto"
  role="dialog"
  tabindex="-1"
  aria-label="Notifications"
  bind:this={panel}
  ontoggle={(e) => (expanded = (e as ToggleEvent).newState === 'open')}
  onkeydown={(e) => e.key === 'Escape' && (e.preventDefault(), close())}
>
  <div class="head">
    <h2>Notifications</h2>
    <Button variant="ghost" size="sm" onclick={() => notifications.markAllRead()}>
      Mark all read
    </Button>
  </div>

  {#if notifications.error && notifications.items.length === 0}
    <EmptyState tone="danger" title="Couldn't load notifications" description={notifications.error.message}>
      {#snippet action()}<Button onclick={() => notifications.load()}>Retry</Button>{/snippet}
    </EmptyState>
  {:else if notifications.items.length === 0}
    <EmptyState compact title="You're all caught up" />
  {:else}
    <ul>
      {#each notifications.items as item (item.seq)}
        {@const meta = notificationTitle(item)}
        {#snippet body()}
          <span class="icon tone-{meta.tone}"><Icon name="alert" /></span>
          <div class="text">
            <p class="title">{meta.title}</p>
            <p class="summary">{notifications.text(item)}</p>
          </div>
          <time class="time" datetime={new Date(item.ts).toISOString()}>{notifications.isResolved(item) ? 'Resolved' : relative(item.ts, clock.now)}</time>
        {/snippet}
        <li class:unread={notifications.isUnread(item)} class:resolved={notifications.isResolved(item)}>
          <button type="button" class="entry" onclick={() => follow(item)}>{@render body()}</button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .anchor {
    position: relative;
    display: inline-flex;
  }

  .pill {
    position: absolute;
    top: 0;
    right: 0;
    min-width: 16px;
    height: 16px;
    padding: 0 var(--space-1);
    border-radius: var(--radius-full);
    background: var(--accent);
    color: var(--accent-fg);
    font-size: var(--text-xs-size);
    line-height: 16px;
    font-weight: 500;
    text-align: center;
    pointer-events: none;
  }

  .pill.needs-you {
    background: var(--warning-fg);
    color: var(--bg);
  }

  .resolved .title,
  .resolved .summary {
    color: var(--text-tertiary);
  }

  .panel {
    position: fixed;
    inset: auto;
    margin: 0;
    width: var(--toast-w);
    max-width: calc(100vw - var(--space-4));
    padding: 0;
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

  .panel:popover-open {
    opacity: 1;
    transform: none;
  }

  @starting-style {
    .panel:popover-open {
      opacity: 0;
      transform: translateY(4px);
    }
  }

  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--border);
  }

  h2 {
    font-size: var(--text-md-size);
    line-height: var(--text-md-line);
    letter-spacing: 0;
  }

  ul {
    max-height: 420px;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    list-style: none;
  }

  li {
    border-bottom: 1px solid var(--border);
  }

  .entry {
    display: grid;
    grid-template-columns: auto 1fr auto;
    gap: var(--space-3);
    width: 100%;
    padding: var(--space-3) var(--space-4);
    background: none;
    border: 0;
    text-align: left;
  }

  button.entry {
    cursor: pointer;
  }

  button.entry:hover {
    background: var(--bg-muted);
  }

  li:last-child {
    border-bottom: 0;
  }

  li.unread {
    background: var(--bg-subtle);
  }

  .entry:focus-visible {
    outline-offset: -2px;
  }

  .icon {
    display: flex;
    margin-top: 2px;
  }

  .tone-warning {
    color: var(--warning-solid);
  }

  .tone-danger {
    color: var(--danger-solid);
  }

  .tone-neutral {
    color: var(--neutral-solid);
  }

  .text {
    min-width: 0;
  }

  .title {
    font-weight: 500;
  }

  .summary,
  .time {
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }

  .summary {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .time {
    color: var(--text-tertiary);
    white-space: nowrap;
  }
</style>
