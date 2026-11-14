<script lang="ts" generics="T">
  import type { Snippet } from 'svelte';
  import type { ApiError } from '$lib/api/client';
  import Button from './Button.svelte';
  import EmptyState from './EmptyState.svelte';
  import type { IconName } from './Icon.svelte';
  import Skeleton from './Skeleton.svelte';

  type Column = { key: string; label: string; width?: string; align?: 'left' | 'right'; srOnly?: boolean; keepWidth?: boolean };

  let {
    caption,
    columns,
    rows,
    key,
    loading,
    error,
    onretry,
    empty,
    row,
    footer,
    expanded,
    detail,
    rowClass,
    rowId,
    rowCurrent,
    flush = false,
  }: {
    caption: string;
    columns: Column[];
    rows: T[] | undefined;
    key: (row: T) => string;
    loading: boolean;
    error: ApiError | null;
    onretry: () => void;
    empty: { title: string; description?: string; icon?: IconName; action?: Snippet; compact?: boolean };
    row: Snippet<[T]>;
    footer?: Snippet;
    expanded?: (row: T) => boolean;
    detail?: Snippet<[T]>;
    rowClass?: (row: T) => string | undefined;
    rowId?: (row: T) => string;
    rowCurrent?: (row: T) => boolean;
    flush?: boolean;
  } = $props();

  const SKELETON_ROWS = 5;
  // Room for one small icon button between the cell paddings, so every table's menu lines up. `keepWidth` opts out for a text action.
  const ACTIONS_COL_W = '56px';
</script>

<div class="wrap" class:flush class:has-actions={columns[columns.length - 1]?.srOnly}>
  <table aria-busy={loading || undefined}>
    <caption class="sr-only">{caption}</caption>
    <colgroup>
      {#each columns as column, index}<col style:width={index === columns.length - 1 && column.srOnly && !column.keepWidth ? ACTIONS_COL_W : column.width} />{/each}
    </colgroup>
    <thead>
      <tr>
        {#each columns as column}
          <th scope="col" class:num={column.align === 'right'} class:right={column.align === 'right'}>
            {#if column.srOnly}<span class="sr-only">{column.label}</span>{:else}{column.label}{/if}
          </th>
        {/each}
      </tr>
    </thead>
    <tbody>
      {#if loading}
        {#each { length: SKELETON_ROWS } as _, index (index)}
          <tr>
            {#each columns as column}<td><Skeleton width={column.srOnly ? '16px' : '70%'} /></td>{/each}
          </tr>
        {/each}
      {:else if error}
        <tr class="plain">
          <td colspan={columns.length}>
            <EmptyState tone="danger" title="Couldn't load {caption.toLowerCase()}" description={error.message}>
              {#snippet action()}<Button onclick={onretry}>Retry</Button>{/snippet}
            </EmptyState>
          </td>
        </tr>
      {:else if rows && rows.length === 0}
        <tr class="plain">
          <td colspan={columns.length}>
            <EmptyState title={empty.title} description={empty.description} icon={empty.icon} action={empty.action} compact={empty.compact} />
          </td>
        </tr>
      {:else if rows}
        {#each rows as item (key(item))}
          <tr class={rowClass?.(item)} id={rowId?.(item)} aria-current={rowCurrent?.(item) ? 'true' : undefined}>{@render row(item)}</tr>
          {#if detail && expanded?.(item)}
            <tr class="plain detail"><td colspan={columns.length}>{@render detail(item)}</td></tr>
          {/if}
        {/each}
      {/if}
    </tbody>
  </table>
  {#if footer}<div class="footer">{@render footer()}</div>{/if}
</div>

<style>
  .wrap {
    overflow: hidden;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
  }

  .wrap.flush {
    background: none;
    border: 0;
    border-radius: 0;
  }

  table {
    width: 100%;
    table-layout: fixed;
    border-collapse: collapse;
  }

  th {
    height: var(--row-h-header);
    padding: 0 var(--space-3);
    background: var(--bg-subtle);
    border-bottom: 1px solid var(--border);
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
    font-weight: 500;
    text-align: left;
  }

  th.right {
    text-align: right;
  }

  th:first-child,
  tbody :global(td:first-child) {
    padding-left: var(--space-4);
  }

  th:last-child,
  tbody :global(td:last-child) {
    padding-right: var(--space-4);
  }

  tbody :global(td) {
    height: var(--row-h);
    padding: 0 var(--space-3);
    border-bottom: 1px solid var(--border);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* The trailing actions column keeps its menu at the same right edge on every table. */
  .has-actions tbody :global(tr:not(.plain) td:last-child) {
    text-align: right;
  }

  tbody tr:last-child :global(td) {
    border-bottom: 0;
  }

  tbody tr:not(.plain):hover {
    background: var(--bg-subtle);
  }

  .plain :global(td) {
    padding: 0;
    height: auto;
  }

  .detail :global(td) {
    padding: var(--space-3) var(--space-4);
    background: var(--bg-subtle);
  }

  .footer {
    padding: var(--space-3) var(--space-4);
    border-top: 1px solid var(--border);
    background: var(--bg-subtle);
  }
</style>
