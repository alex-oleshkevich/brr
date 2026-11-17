<script lang="ts">
  import '$lib/styles/tokens.css';
  import '$lib/styles/base.css';
  import type { Snippet } from 'svelte';
  import { onMount } from 'svelte';
  import { afterNavigate } from '$app/navigation';
  import { api, unwrap } from '$lib/api/client';
  import { events } from '$lib/api/events.svelte';
  import { startSession } from '$lib/api/session';
  import { nav } from '$lib/nav';
  import SettingsDialog from '$lib/settings/SettingsDialog.svelte';
  import { theme } from '$lib/theme.svelte';
  import Button from '$lib/ui/Button.svelte';
  import Icon from '$lib/ui/Icon.svelte';
  import ConfirmHost from '$lib/ui/ConfirmHost.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Sidebar from '$lib/ui/Sidebar.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import CapacityPill from '$lib/ui/CapacityPill.svelte';
  import NotificationBell from '$lib/ui/NotificationBell.svelte';
  import NeedsYouBar from '$lib/attention/NeedsYouBar.svelte';
  import { notifications } from '$lib/notifications.svelte';
  import ThemeToggle from '$lib/ui/ThemeToggle.svelte';
  import Toaster from '$lib/ui/Toaster.svelte';
  import Topbar from '$lib/ui/Topbar.svelte';

  let { children }: { children: Snippet } = $props();

  let status = $state<'starting' | 'ready' | 'failed'>('starting');
  let version = $state<string>();
  let drawer: HTMLDialogElement;
  let drawerOpen = $state(false);

  async function start() {
    status = 'starting';
    try {
      await startSession();
    } catch {
      status = 'failed';
      return;
    }
    status = 'ready';
    events.connect();
    void notifications.start();
    version = (await unwrap(api.GET('/v1/system/info')).catch(() => undefined))?.version;
  }

  onMount(() => {
    void theme.init().then(start);
  });

  afterNavigate(() => drawer?.open && drawer.close());
</script>

<a class="skip" href="#main">Skip to content</a>

<div class="shell">
  <div class="static-sidebar"><Sidebar items={nav} {version} /></div>

  <div class="column">
    <Topbar menuExpanded={drawerOpen} onmenu={() => drawer.showModal()}>
      {#if status === 'ready'}<CapacityPill />{/if}
      <NotificationBell />
      <ThemeToggle />
    </Topbar>

    {#if status === 'ready'}
      <div class="notices">
        {#if events.status === 'reconnecting'}
          <p class="offline" role="status">
            <Icon name="alert" />
            Can't reach the brr daemon. Showing the last data it sent; reconnecting…
          </p>
        {/if}
        <NeedsYouBar />
      </div>
    {/if}

    <main id="main" tabindex="-1">
      {#if status === 'ready'}
        {@render children()}
      {:else if status === 'failed'}
        <EmptyState tone="danger" title="Can't reach the brr daemon" description="Start it with brr daemon start, then retry.">
          {#snippet action()}<Button variant="primary" onclick={start}>Retry</Button>{/snippet}
        </EmptyState>
      {:else}
        <div class="loading">
          <Skeleton width="200px" height="28px" />
          <Skeleton height="160px" radius="var(--radius-lg)" />
          <Skeleton height="160px" radius="var(--radius-lg)" />
        </div>
      {/if}
    </main>
  </div>
</div>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog
  class="drawer"
  aria-label="Navigation"
  bind:this={drawer}
  onclose={() => (drawerOpen = false)}
  onclick={(e) => e.target === drawer && drawer.close()}
>
  <Sidebar items={nav} {version} />
</dialog>

{#if status === 'ready'}<SettingsDialog />{/if}
<Toaster />
<ConfirmHost />

<style>
  .skip {
    position: absolute;
    left: var(--space-2);
    top: var(--space-2);
    z-index: 200;
    padding: var(--space-2) var(--space-3);
    background: var(--surface-raised);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-md);
    font-weight: 500;
    transform: translateY(-200%);
  }

  .skip:focus {
    transform: none;
  }

  .shell {
    display: grid;
    grid-template-columns: var(--sidebar-w) minmax(0, 1fr);
    min-height: 100vh;
  }

  .static-sidebar {
    position: sticky;
    top: 0;
    height: 100vh;
  }

  main {
    max-width: var(--content-max);
    margin: 0 auto;
    padding: var(--space-6) var(--space-8) var(--space-12);
  }

  main:focus {
    outline: none;
  }

  .notices {
    position: sticky;
    top: var(--topbar-h);
    z-index: 9;
  }

  .offline {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-8);
    background: var(--warning-bg);
    border-bottom: 1px solid var(--warning-border);
    color: var(--warning-fg);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }

  .loading {
    display: flex;
    flex-direction: column;
    gap: var(--space-6);
  }

  .drawer {
    width: var(--sidebar-w);
    height: 100vh;
    max-width: 100vw;
    max-height: none;
    margin: 0;
    padding: 0;
    overflow: hidden;
    background: var(--bg-subtle);
    color: var(--text);
    border: 0;
    box-shadow: var(--shadow-lg);
    transform: translateX(-100%);
    transition: transform var(--duration-slow) var(--ease-out);
  }

  .drawer[open] {
    transform: none;
  }

  @starting-style {
    .drawer[open] {
      transform: translateX(-100%);
    }
  }

  .drawer::backdrop {
    background: var(--overlay);
    transition: opacity var(--duration-base) var(--ease-standard);
  }

  @starting-style {
    .drawer[open]::backdrop {
      opacity: 0;
    }
  }

  @media (max-width: 1023px) {
    .shell {
      grid-template-columns: minmax(0, 1fr);
    }

    .static-sidebar {
      display: none;
    }
  }

  @media (min-width: 1024px) {
    .drawer {
      display: none;
    }
  }
</style>
