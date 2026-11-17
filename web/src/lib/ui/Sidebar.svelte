<script lang="ts">
  import type { Snippet } from 'svelte';
  import { page } from '$app/state';
  import { goto } from '$app/navigation';
  import { events } from '$lib/api/events.svelte';
  import type { NavItem } from '$lib/nav';
  import { settingsHref } from '$lib/settings/sections';
  import Icon from './Icon.svelte';
  import StatusDot from './StatusDot.svelte';

  let { items, version, footer }: { items: NavItem[]; version?: string; footer?: Snippet } = $props();

  const sections = ['Work', 'Library'] as const;
  const groups = $derived(
    sections.map((section) => ({ section, items: items.filter((item) => item.section === section) })).filter((g) => g.items.length > 0),
  );

  const active = (item: NavItem) =>
    item.href === '/' ? page.url.pathname === '/' : page.url.pathname.startsWith(item.href);
  const live = $derived(events.status === 'open');
</script>

<aside>
  <div class="brand">
    <span class="name">brr</span>
    {#if version}<span class="version">{version}</span>{/if}
  </div>

  <nav aria-label="Main">
    {#each groups as group (group.section)}
      <p class="group">{group.section}</p>
      {#each group.items as item (item.href)}
        <a class="nav-item" href={item.href} aria-current={active(item) ? 'page' : undefined}>{item.label}</a>
      {/each}
    {/each}
  </nav>

  <div class="footer">
    {@render footer?.()}
    <button type="button" class="nav-item settings" onclick={() => goto(settingsHref('general'), { noScroll: true, keepFocus: true })}><Icon name="settings" />Settings</button>
    <StatusDot tone={live ? 'success' : 'warning'} label={live ? 'Live' : 'Reconnecting…'} />
  </div>
</aside>

<style>
  aside {
    display: flex;
    flex-direction: column;
    width: var(--sidebar-w);
    height: 100%;
    padding: var(--space-3);
    background: var(--bg-subtle);
    border-right: 1px solid var(--border);
  }

  .brand {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: var(--topbar-h);
    margin-top: calc(var(--space-3) * -1);
    padding: 0 var(--space-2);
  }

  .name {
    font-size: var(--text-md-size);
    line-height: var(--text-md-line);
    font-weight: 600;
    letter-spacing: 0;
  }

  .version {
    color: var(--text-tertiary);
    font-size: var(--text-xs-size);
    line-height: var(--text-xs-line);
  }

  nav {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 0;
    overflow-y: auto;
  }

  .group {
    padding: var(--space-4) var(--space-2) var(--space-1);
    color: var(--text-tertiary);
    font-size: var(--text-xs-size);
    line-height: var(--text-xs-line);
    font-weight: 500;
  }

  .nav-item {
    display: flex;
    align-items: center;
    height: var(--control-h-md);
    padding: 0 var(--space-2);
    border-radius: var(--radius-md);
    color: var(--text-secondary);
    font-weight: 500;
    transition:
      background-color var(--duration-fast) var(--ease-standard),
      color var(--duration-fast) var(--ease-standard);
  }

  .nav-item:hover,
  .nav-item[aria-current='page'] {
    background: var(--bg-muted);
    color: var(--text);
  }

  .footer {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding-top: var(--space-3);
  }

  .settings {
    gap: var(--space-2);
    border: 0;
    background: none;
    font: inherit;
    font-weight: 500;
    text-align: left;
    cursor: pointer;
  }

  .footer > :global(:not(.nav-item)) {
    margin-left: var(--space-2);
  }
</style>
