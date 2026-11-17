<script lang="ts">
  import type { Snippet } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import Icon from './Icon.svelte';
  import Tabs from './Tabs.svelte';

  let {
    title,
    description,
    back,
    tabs,
    tab = $bindable(),
    actions,
    side,
    children,
  }: {
    title: string;
    description?: string;
    back?: { href: string; label: string };
    tabs?: { id: string; label: string }[];
    tab?: string;
    actions?: Snippet;
    side?: Snippet;
    children: Snippet;
  } = $props();

  // The URL is the source of truth; a missing or unknown ?tab= falls back to the first tab, which is never written.
  // `synced` is the tab the URL currently implies, so only selections made in the page rewrite the URL.
  let synced: string | undefined;

  $effect(() => {
    if (!tabs?.length) return;
    const requested = page.url.searchParams.get('tab');
    synced = (tabs.find((t) => t.id === requested) ?? tabs[0]).id;
    tab = synced;
  });

  $effect(() => {
    if (!tabs?.length || tab === undefined || tab === synced) return;
    synced = tab;
    const param = tab === tabs[0].id ? '' : `?tab=${tab}`;
    goto(`${page.url.pathname}${param}`, { replaceState: true, noScroll: true, keepFocus: true });
  });

  const panel = $derived(tabs?.length ? tab : undefined);
</script>

<svelte:head><title>{title} · brr</title></svelte:head>

<div class="detail">
  <header>
    {#if back}
      <a class="back" href={back.href}><span class="flip" aria-hidden="true"><Icon name="chevron-right" /></span>{back.label}</a>
    {/if}
    <div class="head">
      <div class="text">
        <h1>{title}</h1>
        {#if description}<p>{description}</p>{/if}
      </div>
      {#if actions}<div class="actions">{@render actions()}</div>{/if}
    </div>
  </header>

  {#if tabs?.length}
    <Tabs items={tabs} bind:value={tab} label={title} idPrefix="detail" />
  {/if}

  <div class="body" class:with-side={Boolean(side)}>
    <div
      class="main"
      role={panel ? 'tabpanel' : undefined}
      id={panel ? `detail-panel-${panel}` : undefined}
      aria-labelledby={panel ? `detail-tab-${panel}` : undefined}
    >
      {@render children()}
    </div>
    {#if side}<div class="side">{@render side()}</div>{/if}
  </div>
</div>

<style>
  .detail {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    min-width: 0;
  }

  header {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .back {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    width: fit-content;
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
    text-decoration: none;
  }

  .back:hover {
    color: var(--text);
  }

  .flip {
    display: inline-flex;
    transform: rotate(180deg);
  }

  .head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--space-4);
  }

  h1 {
    font-size: var(--text-xl-size);
    line-height: var(--text-xl-line);
    letter-spacing: -0.005em;
  }

  p {
    margin-top: var(--space-1);
    color: var(--text-secondary);
  }

  .actions {
    display: flex;
    flex-shrink: 0;
    gap: var(--space-2);
  }

  .body {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    min-width: 0;
  }

  .body.with-side {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 300px;
    align-items: start;
    gap: var(--space-6);
  }

  .main,
  .side {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    min-width: 0;
  }

  @media (max-width: 1023px) {
    .body.with-side {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
