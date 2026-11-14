<script lang="ts">
  let {
    value,
    label,
    tone = 'accent',
    size = 'sm',
  }: {
    value: number | null;
    label: string;
    tone?: 'accent' | 'success' | 'warning' | 'danger';
    size?: 'sm' | 'md';
  } = $props();

  const percent = $derived(value === null ? null : Math.min(100, Math.max(0, value)));
</script>

<div
  class="track"
  class:md={size === 'md'}
  role="progressbar"
  aria-label={label}
  aria-valuemin="0"
  aria-valuemax="100"
  aria-valuenow={percent ?? undefined}
>
  <div
    class="bar"
    class:progress-indeterminate={percent === null}
    style:background={tone === 'accent' ? 'var(--accent)' : `var(--${tone}-solid)`}
    style:width={percent === null ? undefined : `${percent}%`}
  ></div>
</div>

<style>
  .track {
    height: 4px;
    overflow: hidden;
    border-radius: var(--radius-full);
    background: var(--bg-muted);
  }

  .track.md {
    height: 6px;
  }

  .bar {
    height: 100%;
    border-radius: var(--radius-full);
  }

  .progress-indeterminate {
    width: 30%;
    animation: slide 1.4s var(--ease-standard) infinite;
  }

  @keyframes slide {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(333%);
    }
  }
</style>
