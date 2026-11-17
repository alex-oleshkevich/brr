<script lang="ts">
  let {
    label,
    value,
    sub,
    tone = 'neutral',
    href,
  }: {
    label: string;
    value: string;
    sub?: string;
    tone?: 'neutral' | 'success' | 'warning' | 'danger';
    href?: string;
  } = $props();
</script>

<svelte:element this={href ? 'a' : 'div'} class="card tone-{tone}" class:link={href} {href}>
  <span class="label">{label}</span>
  <span class="value num">{value}</span>
  {#if sub}<span class="sub" title={sub}>{sub}</span>{/if}
</svelte:element>

<style>
  .card {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
    padding: var(--space-4) var(--space-5);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    color: var(--text);
    text-decoration: none;
  }

  .link {
    transition: border-color var(--duration-fast) var(--ease-standard);
  }

  .link:hover {
    border-color: var(--border-strong);
  }

  .label {
    overflow: hidden;
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .value {
    overflow: hidden;
    font-size: var(--text-2xl-size);
    line-height: var(--text-2xl-line);
    font-weight: 600;
    letter-spacing: -0.01em;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sub {
    overflow: hidden;
    color: var(--text-tertiary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .tone-success .value {
    color: var(--success-fg);
  }

  .tone-warning .value {
    color: var(--warning-fg);
  }

  .tone-danger .value {
    color: var(--danger-fg);
  }
</style>
