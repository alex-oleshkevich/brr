<script lang="ts">
  import { goto } from '$app/navigation';
  import { ApiError, api, unwrap } from '$lib/api/client';
  import { session } from '$lib/api/session';
  import type { components } from '$lib/api/schema';
  import { clock } from '$lib/clock.svelte';
  import { relative } from '$lib/format';
  import Badge from '$lib/ui/Badge.svelte';
  import { confirm } from '$lib/ui/confirm.svelte';
  import Menu from '$lib/ui/Menu.svelte';
  import StateBadge from '$lib/ui/StateBadge.svelte';
  import Table from '$lib/ui/Table.svelte';
  import { refocusIfLost } from '$lib/ui/focus';
  import { toast } from '$lib/ui/toast.svelte';

  type Tab = components['schemas']['Tab'];

  let {
    tabs,
    loading,
    error,
    onretry,
    selected,
    onchange,
  }: {
    tabs: Tab[] | undefined;
    loading: boolean;
    error: ApiError | null;
    onretry: () => void;
    selected?: string | null;
    onchange: () => void;
  } = $props();

  const columns = [
    { key: 'page', label: 'Page' },
    { key: 'state', label: 'State', width: '140px' },
    { key: 'held', label: 'In use by', width: '22%' },
    { key: 'pinned', label: 'Pinned', width: '88px' },
    { key: 'activity', label: 'Last activity', width: '120px' },
    { key: 'actions', label: 'Actions', width: '48px', srOnly: true },
  ];

  const open = $derived(tabs?.filter((t) => t.state !== 'closed' && t.closed_at == null));

  const title = (tab: Tab) => tab.title || tab.url || 'Untitled';

  const show = (tab: Tab) => goto(`?tab=${tab.id}`, { replaceState: true, noScroll: true });

  async function pin(tab: Tab) {
    try {
      const path = { params: { path: { tab_id: tab.id } }, body: {} };
      await unwrap(tab.pinned ? api.POST('/v1/tabs/{tab_id}/unpin', path) : api.POST('/v1/tabs/{tab_id}/pin', path));
      onchange();
    } catch (e) {
      toast.apiError(e, tab.pinned ? "Couldn't unpin the tab" : "Couldn't pin the tab");
    }
  }

  async function close(tab: Tab) {
    const ok = await confirm({
      title: 'Close this tab?',
      description: `${title(tab)} closes in the browser.`,
      confirmLabel: 'Close tab',
      tone: 'danger',
    });
    if (!ok) return;

    try {
      await unwrap(api.DELETE('/v1/tabs/{tab_id}', { params: { path: { tab_id: tab.id } } }));
      onchange();
      refocusIfLost('tabs-title');
    } catch (e) {
      toast.apiError(e, "Couldn't close the tab");
    }
  }
</script>

<Table
  caption="Tabs"
  {columns}
  rows={open}
  key={(row: Tab) => row.id}
  {loading}
  {error}
  {onretry}
  empty={{ title: 'No open tabs' }}
  rowClass={(row: Tab) => (row.id === selected ? 'tab-row tab-highlight' : 'tab-row')}
  rowCurrent={(row: Tab) => row.id === selected}
  flush
  rowId={(row: Tab) => `tab-row-${row.id}`}
>
  {#snippet row(tab: Tab)}
    <td>
      <div class="page">
        <a class="title truncate" href="?tab={tab.id}" data-sveltekit-noscroll data-sveltekit-replacestate title={title(tab)}>{title(tab)}</a>
        {#if tab.title && tab.title !== tab.url}<span class="url truncate" title={tab.url}>{tab.url}</span>{/if}
      </div>
    </td>
    <td><StateBadge kind="tab" state={tab.state} /></td>
    <td>
      {#if tab.lease}
        <span class="held">
          <span class="truncate" title={tab.lease.client_label}>{tab.lease.client_id === session.clientId ? 'You' : tab.lease.client_label}</span>
        </span>
      {:else}
        —
      {/if}
    </td>
    <td>{#if tab.pinned}<Badge>Pinned</Badge>{/if}</td>
    <td>{relative(tab.last_activity_at, clock.now)}</td>
    <td class="menu">
      <Menu
        label="Actions for {title(tab)}"
        items={[
          { label: 'Show in live view', onselect: () => show(tab) },
          { label: tab.pinned ? 'Unpin' : 'Pin', onselect: () => pin(tab) },
          { label: 'Close tab…', tone: 'danger', onselect: () => close(tab) },
        ]}
      />
    </td>
  {/snippet}
</Table>

<style>
  .page {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .title {
    align-self: flex-start;
    max-width: 100%;
    font-weight: 500;
    color: var(--text);
    text-decoration: none;
  }

  /* The whole row shows the tab: the title link stretches over it, and the menu stays above. */
  :global(tr.tab-row) {
    position: relative;
    cursor: pointer;
  }

  .title::after {
    content: '';
    position: absolute;
    inset: 0;
  }

  .menu {
    position: relative;
    z-index: 1;
  }

  .title:hover {
    color: var(--accent-text);
    text-decoration: underline;
  }

  .url {
    color: var(--text-tertiary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }

  .held {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  :global(tr.tab-highlight td) {
    background: var(--accent-subtle);
  }
</style>
