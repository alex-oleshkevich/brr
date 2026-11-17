<script lang="ts">
  import EmptyState from './EmptyState.svelte';

  const PAD = { top: 8, right: 8, bottom: 24, left: 48 };
  const GRIDLINES = 3;
  const SERIES_COLORS = 4;
  const LABEL_CHAR_W = 7;
  const LABEL_GAP = 16;
  const BAR_MAX_WIDTH = 48;
  const BAR_FILL_RATIO = 0.6;
  const STEPS = [1, 2, 2.5, 5, 10];
  const INTEGER_STEPS = [1, 2, 5, 10];

  let {
    label,
    series,
    data,
    format = (n: number) => n.toLocaleString(),
    height = 200,
  }: {
    label: string;
    series: { key: string; label: string }[];
    data: { x: string; label?: string; values: Record<string, number> }[];
    format?: (n: number) => string;
    height?: number;
  } = $props();

  let width = $state(600);

  const innerW = $derived(Math.max(width - PAD.left - PAD.right, 0));
  const innerH = $derived(height - PAD.top - PAD.bottom);
  const slot = $derived(data.length ? innerW / data.length : 0);
  const barW = $derived(Math.min(slot * BAR_FILL_RATIO, BAR_MAX_WIDTH));
  const labelW = $derived(Math.max(...data.map((d) => (d.label ?? d.x).length)) * LABEL_CHAR_W + LABEL_GAP);
  const every = $derived(Math.max(1, Math.ceil(labelW / slot)));
  const integral = $derived(data.every((d) => series.every((s) => Number.isInteger(d.values[s.key] ?? 0))));

  // Segments of each bar, bottom-up, as value ranges.
  const stacks = $derived(
    data.map((d) => {
      let base = 0;
      return series.map((s) => {
        const v = Math.max(d.values[s.key] ?? 0, 0);
        const seg = { v, y0: base, y1: base + v };
        base += v;
        return seg;
      });
    }),
  );
  const peakTotal = $derived(Math.max(...stacks.map((segs) => segs.at(-1)?.y1 ?? 0), 0));

  // A nice step (1, 2, 2.5, 5 x 10^n; integers only for integer data) gives at most GRIDLINES ticks.
  const step = $derived.by(() => {
    if (peakTotal <= 0) return 1;
    const raw = peakTotal / GRIDLINES;
    const mag = 10 ** Math.floor(Math.log10(integral ? Math.max(raw, 1) : raw));
    return (integral ? INTEGER_STEPS : STEPS)
      .map((f) => f * mag)
      .find((s) => s >= raw) ?? mag * 10;
  });
  const count = $derived(Math.max(1, Math.ceil(peakTotal / step)));
  const peak = $derived(count * step);
  const ticks = $derived(Array.from({ length: count }, (_, i) => +((i + 1) * step).toPrecision(12)));

  const yAt = (v: number) => PAD.top + innerH - (v / peak) * innerH;
  const cx = (i: number) => PAD.left + (i + 0.5) * slot;
  const fill = (i: number) => `var(--chart-${(i % SERIES_COLORS) + 1})`;
</script>

<div class="chart" bind:clientWidth={width}>
  {#if data.length === 0}
    <EmptyState compact title="No data yet" />
  {:else}
    <svg viewBox="0 0 {width} {height}" width="100%" {height} role="img" aria-label={label}>
      {#each ticks as t}
        <line class="grid" x1={PAD.left} x2={PAD.left + innerW} y1={yAt(t)} y2={yAt(t)} />
        <text class="axis" x={PAD.left - 8} y={yAt(t)} dy="0.32em" text-anchor="end">{format(t)}</text>
      {/each}
      <line class="baseline" x1={PAD.left} x2={PAD.left + innerW} y1={yAt(0)} y2={yAt(0)} />
      <text class="axis" x={PAD.left - 8} y={yAt(0)} dy="0.32em" text-anchor="end">{format(0)}</text>

      {#each data as d, i}
        {#each stacks[i] as seg, s}
          {#if seg.v > 0}
            <rect class="segment" x={cx(i) - barW / 2} y={yAt(seg.y1)} width={barW} height={yAt(seg.y0) - yAt(seg.y1)} style="fill: {fill(s)}">
              <title>{d.x} · {series[s].label}: {format(seg.v)}</title>
            </rect>
          {/if}
        {/each}
        {#if i % every === 0}
          <text class="axis" x={cx(i)} y={height - 6} text-anchor="middle">{d.label ?? d.x}</text>
        {/if}
      {/each}
    </svg>

    <ul class="legend">
      {#each series as s, i}
        <li><span class="swatch" style="background: {fill(i)}"></span>{s.label}</li>
      {/each}
    </ul>

    <table class="sr-only">
      <caption>{label}</caption>
      <thead>
        <tr>
          <th scope="col">Period</th>
          {#each series as s}<th scope="col">{s.label}</th>{/each}
        </tr>
      </thead>
      <tbody>
        {#each data as d}
          <tr>
            <th scope="row">{d.x}</th>
            {#each series as s}<td>{format(d.values[s.key] ?? 0)}</td>{/each}
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

<style>
  .chart {
    position: relative;
    min-width: 0;
  }

  svg {
    display: block;
    overflow: visible;
  }

  .grid {
    stroke: var(--border);
    stroke-width: 1;
  }

  .baseline {
    stroke: var(--border-strong);
    stroke-width: 1;
  }

  .axis {
    fill: var(--text-tertiary);
    font-size: var(--text-xs-size);
    font-variant-numeric: tabular-nums;
  }

  .segment:hover {
    opacity: 0.8;
  }

  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2) var(--space-4);
    margin-top: var(--space-3);
    padding: 0;
    list-style: none;
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }

  .legend li {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }

  .swatch {
    width: 8px;
    height: 8px;
    border-radius: 2px;
  }
</style>
