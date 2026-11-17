<script lang="ts">
  import EmptyState from './EmptyState.svelte';

  const PAD = { top: 8, right: 8, bottom: 24, left: 48 };
  const GRIDLINES = 3;
  const SERIES_COLORS = 4;
  const LABEL_CHAR_W = 7;
  const LABEL_GAP = 16;
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
  const column = $derived(data.length > 1 ? innerW / (data.length - 1) : innerW);
  const labelW = $derived(Math.max(...data.map((d) => (d.label ?? d.x).length)) * LABEL_CHAR_W + LABEL_GAP);
  const every = $derived(Math.max(1, Math.ceil(labelW / column)));
  const integral = $derived(data.every((d) => series.every((s) => Number.isInteger(d.values[s.key] ?? 0))));

  // Bands of each point, bottom-up, as value ranges.
  const stacks = $derived(
    data.map((d) => {
      let base = 0;
      return series.map((s) => {
        const v = Math.max(d.values[s.key] ?? 0, 0);
        const band = { y0: base, y1: base + v };
        base += v;
        return band;
      });
    }),
  );
  const peakTotal = $derived(Math.max(...stacks.map((bands) => bands.at(-1)?.y1 ?? 0), 0));

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
  const xAt = (i: number) => (data.length > 1 ? PAD.left + (i / (data.length - 1)) * innerW : PAD.left + innerW / 2);
  const fill = (i: number) => `var(--chart-${(i % SERIES_COLORS) + 1})`;

  // Each series' edge is drawn only where the series has a value, so zero runs leave no line behind.
  const areas = $derived(
    series.map((s, k) => {
      const upper = data.map((_, i) => `${xAt(i)},${yAt(stacks[i][k].y1)}`);
      const lower = data.map((_, i) => `${xAt(i)},${yAt(stacks[i][k].y0)}`).reverse();
      let edge = '';
      let drawing = false;
      data.forEach((_, i) => {
        const band = stacks[i][k];
        const on = band.y1 > band.y0;
        if (on) edge += `${drawing ? ' L' : ' M'}${xAt(i)},${yAt(band.y1)}`;
        drawing = on;
      });
      return { s, k, edge, body: `M${upper.join(' L')} L${lower.join(' L')} Z` };
    }),
  );

  const tip = (d: { x: string; values: Record<string, number> }) =>
    `${d.x}: ${series.map((s) => `${s.label} ${format(d.values[s.key] ?? 0)}`).join(', ')}`;
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

      {#each areas as area (area.s.key)}
        <path class="band" d={area.body} style="fill: {fill(area.k)}" />
        <path class="edge" d={area.edge} style="stroke: {fill(area.k)}" />
      {/each}

      {#each data as d, i}
        <rect class="hit" x={xAt(i) - column / 2} y={PAD.top} width={column} height={innerH}>
          <title>{tip(d)}</title>
        </rect>
        {#if i % every === 0}
          <text class="axis" x={xAt(i)} y={height - 6} text-anchor="middle">{d.label ?? d.x}</text>
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

  .band {
    fill-opacity: 0.25;
  }

  .edge {
    fill: none;
    stroke-width: 1.5;
    stroke-linejoin: round;
  }

  .hit {
    fill: transparent;
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
