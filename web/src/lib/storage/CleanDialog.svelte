<script lang="ts">
  import { untrack } from 'svelte';
  import { ApiError, api, unwrap } from '$lib/api/client';
  import type { components } from '$lib/api/schema';
  import { bytes, count } from '$lib/format';
  import Button from '$lib/ui/Button.svelte';
  import Icon from '$lib/ui/Icon.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  type Report = components['schemas']['SweepReport'];

  let { open = $bindable(false), onswept }: { open?: boolean; onswept: () => void } = $props();

  const RULES: Record<components['schemas']['SweepRule'], string> = {
    artifacts_age: 'Files older than the retention age',
    artifacts_size: 'Files over the size limit',
    ephemeral_leftovers: 'Ephemeral leftovers',
    unused_builds: 'Unused browser versions',
    unused_extensions: 'Unused extensions',
    events_age: 'Old activity',
    captcha_solves_age: 'Old captcha log',
  };
  const SHOWN = 50;

  let report = $state<Report>();
  let failure = $state<ApiError | null>(null);
  let deleting = $state(false);
  let run = 0;

  const itemCount = $derived(report?.groups.reduce((n, g) => n + g.items.length, 0) ?? 0);

  // The preview is what a sweep would delete right now. Confirming runs a new sweep with the current rules, which the
  // daemon cannot restrict to the previewed items, so the toast reports what it actually deleted.
  async function preview() {
    const mine = ++run;
    report = undefined;
    failure = null;
    try {
      const next = await unwrap(api.POST('/v1/retention/sweep', { body: { dry_run: true } }));
      if (mine === run) report = next;
    } catch (e) {
      if (mine === run) failure = e instanceof ApiError ? e : new ApiError(0, 'Unreachable', String(e), {});
    }
  }

  $effect(() => {
    if (open) untrack(preview);
  });

  async function confirmSweep() {
    deleting = true;
    try {
      const done = await unwrap(api.POST('/v1/retention/sweep', { body: { dry_run: false } }));
      const deleted = done.groups.reduce((n, g) => n + g.items.length, 0);
      toast.success(`Deleted ${count(deleted)} ${deleted === 1 ? 'item' : 'items'}, freed ${bytes(done.bytes)}`);
      open = false;
      onswept();
    } catch (e) {
      toast.apiError(e, "Couldn't clean up");
    } finally {
      deleting = false;
    }
  }
</script>

<Dialog bind:open size="lg" title="Clean up" description={itemCount > 0 ? "These items are past your retention rules now. Cleaning runs the sweep again and deletes what qualifies at that moment. This can't be undone." : undefined} dismissible={!deleting}>
  {#if failure}
    <EmptyState tone="danger" title="Couldn't preview the cleanup" description={failure.message}>
      {#snippet action()}<Button onclick={preview}>Retry</Button>{/snippet}
    </EmptyState>
  {:else if !report}
    <div class="groups" aria-busy="true">
      {#each { length: 3 } as _, i (i)}<Skeleton height="36px" radius="var(--radius-md)" />{/each}
    </div>
  {:else if report.groups.length === 0}
    <EmptyState title="Nothing to clean" description="No files, browser versions or logs are past their retention rules." />
  {:else}
    <div class="groups">
      {#each report.groups as group (group.rule)}
        <details>
          <summary>
            <span class="name"><span class="chevron"><Icon name="chevron-right" /></span>{(RULES as Record<string, string>)[group.rule] ?? group.rule}</span>
            <span class="meta">{count(group.items.length)} {group.items.length === 1 ? 'item' : 'items'} · {bytes(group.bytes)}</span>
          </summary>
          <ul>
            {#each group.items.slice(0, SHOWN) as item (item.kind + item.id)}
              <li><span class="label">{item.label}</span><span class="size">{bytes(item.bytes)}</span></li>
            {/each}
            {#if group.items.length > SHOWN}<li class="more">and {count(group.items.length - SHOWN)} more</li>{/if}
          </ul>
        </details>
      {/each}
    </div>
  {/if}
  {#snippet footer()}
    {#if report && report.groups.length > 0}
      <span class="total">{bytes(report.bytes)} from {count(itemCount)} {itemCount === 1 ? 'item' : 'items'} right now</span>
      <Button onclick={() => (open = false)} disabled={deleting}>Cancel</Button>
      <Button variant="danger" loading={deleting} onclick={confirmSweep}>Delete these items</Button>
    {:else if report}
      <Button onclick={() => (open = false)}>Close</Button>
    {:else}
      <Button onclick={() => (open = false)}>Cancel</Button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .groups {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  details {
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }

  summary {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-3) var(--space-4);
    cursor: pointer;
    font-weight: 500;
  }

  .name {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }

  .chevron {
    display: inline-flex;
    color: var(--text-tertiary);
    transition: transform var(--duration-fast, 120ms) var(--ease-standard);
  }

  details[open] .chevron {
    transform: rotate(90deg);
  }

  summary::-webkit-details-marker {
    display: none;
  }

  .meta {
    color: var(--text-secondary);
    font-weight: 400;
    font-size: var(--text-sm-size);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  ul {
    margin: 0;
    padding: var(--space-1) var(--space-4) var(--space-3);
    list-style: none;
    border-top: 1px solid var(--border);
  }

  li {
    display: flex;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-1) 0;
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }

  .label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .size,
  .more {
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .total {
    flex: 1;
    color: var(--text-secondary);
  }
</style>
