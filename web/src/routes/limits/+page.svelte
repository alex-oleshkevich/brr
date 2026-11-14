<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { ApiError, api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import type { components, paths } from '$lib/api/schema';
  import Button from '$lib/ui/Button.svelte';
  import Badge from '$lib/ui/Badge.svelte';
  import Card from '$lib/ui/Card.svelte';
  import { confirm } from '$lib/ui/confirm.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Input from '$lib/ui/Input.svelte';
  import PageHeader from '$lib/ui/PageHeader.svelte';
  import SegmentedControl from '$lib/ui/SegmentedControl.svelte';
  import Select from '$lib/ui/Select.svelte';
  import SaveBar from '$lib/ui/SaveBar.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import { guardUnsaved } from '$lib/ui/unsaved';
  import { toast } from '$lib/ui/toast.svelte';

  type Limits = components['schemas']['LimitsView'];
  type LimitsBody = paths['/v1/limits']['put']['requestBody']['content']['application/json'];
  type Num = number | null;

  type Form = {
    mode: string;
    max_instances: Num;
    chromium: Num;
    camoufox: Num;
    max_tabs_per_instance: Num;
    max_tabs_total: Num;
    tags: { tag: string; max: Num }[];
    min_free_ram_mb: Num;
    max_load_per_core: Num;
    reserve_headless: Num;
    reserve_display: Num;
    reserve_camoufox: Num;
    pressure_check_s: Num;
    admission: string;
    queue_timeout_s: Num;
  };

  const fromLimits = (l: Limits): Form => ({
    mode: l.mode,
    max_instances: l.max_instances,
    chromium: l.max_instances_per_engine.chromium,
    camoufox: l.max_instances_per_engine.camoufox,
    max_tabs_per_instance: l.max_tabs_per_instance,
    max_tabs_total: l.max_tabs_total,
    tags: Object.entries(l.max_instances_per_tag).map(([tag, max]) => ({ tag, max })),
    min_free_ram_mb: l.adaptive.min_free_ram_mb,
    max_load_per_core: l.adaptive.max_load_per_core,
    reserve_headless: l.adaptive.reserve_mb.chromium_headless,
    reserve_display: l.adaptive.reserve_mb.chromium_display,
    reserve_camoufox: l.adaptive.reserve_mb.camoufox,
    pressure_check_s: l.adaptive.pressure_check_s,
    admission: l.admission.default,
    queue_timeout_s: l.admission.queue_timeout_s,
  });

  // settings.set publishes only settings.changed, even for limits.*, adaptive.* and admission.* keys.
  const limits = liveQuery(() => unwrap(api.GET('/v1/limits')), {
    kinds: ['limits.changed', 'settings.changed'],
    match: (e) => e.kind === 'limits.changed' || ((e.data as { keys?: string[] } | null)?.keys ?? []).some((k) => /^(limits|adaptive|admission)\./.test(k)),
  });
  const settings = liveQuery(() => unwrap(api.GET('/v1/settings')), { kinds: ['settings.changed'] });

  let form = $state<Form>();
  let baseline = $state('');
  let loaded = '';
  let changedElsewhere = $state(false);
  let saving = $state(false);
  let serverErrors = $state<Record<string, string>>({});

  const dirty = $derived(form !== undefined && JSON.stringify(form) !== baseline);

  function apply(l: Limits) {
    form = fromLimits(l);
    baseline = JSON.stringify(form);
    loaded = JSON.stringify(l);
    changedElsewhere = false;
    serverErrors = {};
  }

  $effect(() => {
    const data = limits.data;
    if (!data || JSON.stringify(data) === loaded) return;
    if (form === undefined || !dirty) apply(data);
    else changedElsewhere = true;
  });

  const sources = $derived(settings.data?.sources ?? {});
  const isEnv = (key: string) => sources[key] === 'env';
  const envName = (key: string) => 'BRR_' + key.replace('.', '__').toUpperCase();
  const badge = (key: string) => (isEnv(key) ? `Set by ${envName(key)}` : undefined);
  const envHint = (key: string, hint?: string) => (isEnv(key) ? 'Remove the environment variable to edit it here.' : hint);

  const ADAPTIVE_FIELDS = ['min_free_ram_mb', 'max_load_per_core', 'reserve_headless', 'reserve_display', 'reserve_camoufox', 'pressure_check_s'];

  const isInt = (n: Num) => n !== null && Number.isInteger(n) && n >= 0;

  const errors = $derived.by(() => {
    const f = form;
    const e: Record<string, string> = {};
    if (!f) return e;

    const ints: [keyof Form, number][] = [
      ['max_instances', 1],
      ['chromium', 0],
      ['camoufox', 0],
      ['max_tabs_per_instance', 0],
      ['max_tabs_total', 0],
      ['min_free_ram_mb', 0],
      ['reserve_headless', 0],
      ['reserve_display', 0],
      ['reserve_camoufox', 0],
      ['pressure_check_s', 1],
      ['queue_timeout_s', 0],
    ];
    for (const [key, min] of ints) {
      const value = f[key] as Num;
      if (value === null) e[key] = 'Enter a number.';
      else if (!isInt(value) || value < min) e[key] = min === 1 ? 'Enter a whole number, 1 or more.' : 'Enter a whole number, 0 or more.';
    }
    if (f.max_load_per_core === null) e.max_load_per_core = 'Enter a number.';
    else if (!(f.max_load_per_core > 0 && f.max_load_per_core <= 4)) e.max_load_per_core = 'Enter a number above 0 and up to 4.';
    const seen = new Set<string>();
    f.tags.forEach((row, i) => {
      const tag = row.tag.trim();
      if (!tag) e[`tag-${i}`] = 'Enter a tag.';
      else if (seen.has(tag)) e[`tag-${i}`] = 'Tag already listed.';
      seen.add(tag);
      if (!isInt(row.max)) e[`tag-max-${i}`] = 'Enter a whole number, 0 or more.';
    });
    // Hidden fields cannot be fixed by the user, so they never block saving.
    if (f.mode !== 'adaptive') for (const key of ADAPTIVE_FIELDS) delete e[key];
    return e;
  });

  const invalid = $derived(Object.keys(errors).length > 0);

  // A server error belongs to the value that caused it; editing anything clears it.
  $effect(() => {
    void JSON.stringify(form);
    untrack(() => (serverErrors = {}));
  });

  function body(f: Form) {
    const b: LimitsBody = {};
    const n = (v: Num) => v as number;
    if (!isEnv('limits.mode')) b.mode = f.mode as 'static' | 'adaptive';
    if (!isEnv('limits.max_instances')) b.max_instances = n(f.max_instances);
    if (!isEnv('limits.max_instances_per_engine')) b.max_instances_per_engine = { chromium: n(f.chromium), camoufox: n(f.camoufox) };
    if (!isEnv('limits.max_instances_per_tag')) {
      b.max_instances_per_tag = Object.fromEntries(f.tags.map((row) => [row.tag.trim(), n(row.max)]));
    }
    if (!isEnv('limits.max_tabs_per_instance')) b.max_tabs_per_instance = n(f.max_tabs_per_instance);
    if (!isEnv('limits.max_tabs_total')) b.max_tabs_total = n(f.max_tabs_total);

    const adaptive: components['schemas']['SetAdaptive'] = {};
    if (!isEnv('adaptive.min_free_ram_mb')) adaptive.min_free_ram_mb = n(f.min_free_ram_mb);
    if (!isEnv('adaptive.max_load_per_core')) adaptive.max_load_per_core = n(f.max_load_per_core);
    if (!isEnv('adaptive.pressure_check_s')) adaptive.pressure_check_s = n(f.pressure_check_s);
    if (!isEnv('adaptive.reserve_mb')) {
      adaptive.reserve_mb = { chromium_headless: n(f.reserve_headless), chromium_display: n(f.reserve_display), camoufox: n(f.reserve_camoufox) };
    }
    if (Object.keys(adaptive).length) b.adaptive = adaptive;

    const admission: components['schemas']['SetAdmission'] = {};
    if (!isEnv('admission.default')) admission.default = f.admission as 'queue' | 'reject' | 'evict';
    if (!isEnv('admission.queue_timeout_s')) admission.queue_timeout_s = n(f.queue_timeout_s);
    if (Object.keys(admission).length) b.admission = admission;
    return b;
  }

  async function save() {
    if (!form || invalid || saving) return;
    saving = true;
    serverErrors = {};
    try {
      apply(await unwrap(api.PUT('/v1/limits', { body: body(form) })));
      toast.success('Capacity saved');
    } catch (e) {
      const field = e instanceof ApiError && e.code === 'InvalidArgument' ? (e.details as { field?: string }).field : undefined;
      if (field && form && field.split('.').pop()! in form) serverErrors = { [field.split('.').pop()!]: (e as ApiError).message };
      else toast.apiError(e, "Couldn't save capacity");
    } finally {
      saving = false;
    }
  }

  guardUnsaved(() => dirty, 'Your edits to capacity are not saved.');

  const err = (key: string) => errors[key] ?? serverErrors[key] ?? null;

  const modeOptions = [
    { value: 'static', label: 'Fixed limits', hint: 'Run up to the maximums below.' },
    {
      value: 'adaptive',
      label: 'Based on free memory',
      hint: 'Start browsers while memory and CPU allow it. The maximums below still apply.',
    },
  ];
  const admissionOptions = [
    { value: 'queue', label: 'Wait to start' },
    { value: 'reject', label: 'Refuse the launch' },
    { value: 'evict', label: 'Make room (stop the least recently used idle browser)' },
  ];

  let tagList: HTMLElement | undefined = $state();

  async function addTag() {
    form!.tags.push({ tag: '', max: 1 });
    await tick();
    tagList?.querySelectorAll<HTMLInputElement>('input')[(form!.tags.length - 1) * 2]?.focus();
  }

  async function removeTag(index: number) {
    form!.tags = form!.tags.filter((_, j) => j !== index);
    await tick();
    const buttons = tagList?.querySelectorAll<HTMLElement>('[data-remove]') ?? [];
    (buttons[Math.min(index, buttons.length - 1)] ?? tagList?.querySelector<HTMLElement>('[data-add]'))?.focus();
  }
</script>

 <div class="page">
<PageHeader title="Capacity" description="How many browsers brr runs at once, and what happens when it is full.">
  {#snippet actions()}
    <Button variant="primary" loading={saving} disabled={!dirty || invalid} onclick={save}>Save</Button>
  {/snippet}
</PageHeader>

{#if limits.loading || settings.loading}
  <div class="stack" aria-busy="true">
    {#each { length: 3 } as _, i (i)}
      <Card><Skeleton width="30%" /><div class="gap"></div><Skeleton height="64px" radius="var(--radius-md)" /></Card>
    {/each}
  </div>
{:else if (limits.error && !limits.data) || (settings.error && !settings.data)}
  {@const failure = limits.error ?? settings.error}
  <Card>
    <EmptyState tone="danger" title="Couldn't load capacity" description={failure?.message}>
      {#snippet action()}<Button onclick={() => (limits.reload(), settings.reload())}>Retry</Button>{/snippet}
    </EmptyState>
  </Card>
{:else if form}
  <div class="stack">
    {#if changedElsewhere}
      <p class="notice" role="status">
        Capacity changed elsewhere.
        <Button variant="ghost" size="sm" onclick={() => limits.data && apply(limits.data)}>Reload</Button>
      </p>
    {/if}

    <Card>
      <SegmentedControl label="Mode" bind:value={form.mode} options={modeOptions} disabled={isEnv('limits.mode')} />
      {#if isEnv('limits.mode')}
        <p class="env"><Badge>{badge('limits.mode')}</Badge> {envHint('limits.mode')}</p>
      {/if}
    </Card>

    <Card title="Maximums">
      <div class="grid">
        <Input label="Browsers running at once" type="number" min={1} step={1} bind:value={form.max_instances} error={err('max_instances')} readonly={isEnv('limits.max_instances')} badge={badge('limits.max_instances')} hint={envHint('limits.max_instances', "Further launches wait or are refused, see below.")} />
        <Input label="Chromium browsers at once" type="number" min={0} step={1} bind:value={form.chromium} error={err('chromium')} readonly={isEnv('limits.max_instances_per_engine')} badge={badge('limits.max_instances_per_engine')} hint={envHint('limits.max_instances_per_engine', "Limit for Chrome-based browsers.")} />
        <Input label="Camoufox browsers at once" type="number" min={0} step={1} bind:value={form.camoufox} error={err('camoufox')} readonly={isEnv('limits.max_instances_per_engine')} badge={badge('limits.max_instances_per_engine')} hint={envHint('limits.max_instances_per_engine', "Limit for Firefox-based (Camoufox) browsers.")} />
        <Input label="Tabs per browser" type="number" min={0} step={1} bind:value={form.max_tabs_per_instance} error={err('max_tabs_per_instance')} readonly={isEnv('limits.max_tabs_per_instance')} badge={badge('limits.max_tabs_per_instance')} hint={envHint('limits.max_tabs_per_instance', "The most tabs one browser can have open.")} />
        <Input label="Tabs across all browsers" type="number" min={0} step={1} bind:value={form.max_tabs_total} error={err('max_tabs_total')} readonly={isEnv('limits.max_tabs_total')} badge={badge('limits.max_tabs_total')} hint={envHint('limits.max_tabs_total', "The most tabs open at once, counting every browser.")} />
      </div>

      <div class="tags" bind:this={tagList}>
        <h3>Limits by profile tag {#if isEnv('limits.max_instances_per_tag')}<Badge>{badge('limits.max_instances_per_tag')}</Badge>{/if}</h3>
        {#each form.tags as row, i (i)}
          <div class="tag-row">
            <Input label="Tag" hideLabel={i > 0} bind:value={row.tag} error={err(`tag-${i}`)} readonly={isEnv('limits.max_instances_per_tag')} />
            <Input label="Max" hideLabel={i > 0} type="number" min={0} step={1} bind:value={row.max} error={err(`tag-max-${i}`)} readonly={isEnv('limits.max_instances_per_tag')} />
            {#if !isEnv('limits.max_instances_per_tag')}
              <div class="remove" class:first={i === 0}>
                <Button variant="ghost" iconOnly icon="x" data-remove aria-label="Remove tag limit {row.tag}" onclick={() => removeTag(i)} />
              </div>
            {/if}
          </div>
        {/each}
        {#if isEnv('limits.max_instances_per_tag')}
          <p class="env">{envHint('limits.max_instances_per_tag')}</p>
        {:else}
          <div><Button variant="ghost" icon="plus" data-add onclick={addTag}>Add a tag limit</Button></div>
        {/if}
      </div>
    </Card>

    {#if form.mode === 'adaptive'}
    <Card title="Free memory and CPU" description="Browsers only start while the machine has room.">
      <div class="grid">
        <Input label="Keep this much memory free (MB)" type="number" min={0} step={1} bind:value={form.min_free_ram_mb} error={err('min_free_ram_mb')} readonly={isEnv('adaptive.min_free_ram_mb')} badge={badge('adaptive.min_free_ram_mb')} hint={envHint('adaptive.min_free_ram_mb', "Stops starting browsers when free memory would drop below this.")} />
        <Input label="Stop starting browsers above this CPU load" type="number" min={0} max={4} step={0.05} bind:value={form.max_load_per_core} error={err('max_load_per_core')} readonly={isEnv('adaptive.max_load_per_core')} badge={badge('adaptive.max_load_per_core')} hint={envHint('adaptive.max_load_per_core', "1 means every core is fully busy.")} />
        <Input label="Memory set aside per headless Chromium (MB)" type="number" min={0} step={1} bind:value={form.reserve_headless} error={err('reserve_headless')} readonly={isEnv('adaptive.reserve_mb')} badge={badge('adaptive.reserve_mb')} hint={envHint('adaptive.reserve_mb', "Estimate used when deciding if another one fits.")} />
        <Input label="Memory set aside per Chromium with a window (MB)" type="number" min={0} step={1} bind:value={form.reserve_display} error={err('reserve_display')} readonly={isEnv('adaptive.reserve_mb')} badge={badge('adaptive.reserve_mb')} hint={envHint('adaptive.reserve_mb', "Estimate used when deciding if another one fits.")} />
        <Input label="Memory set aside per Camoufox (MB)" type="number" min={0} step={1} bind:value={form.reserve_camoufox} error={err('reserve_camoufox')} readonly={isEnv('adaptive.reserve_mb')} badge={badge('adaptive.reserve_mb')} hint={envHint('adaptive.reserve_mb', "Estimate used when deciding if another one fits.")} />
        <Input label="Re-check memory and CPU every (s)" type="number" min={0} step={1} bind:value={form.pressure_check_s} error={err('pressure_check_s')} readonly={isEnv('adaptive.pressure_check_s')} badge={badge('adaptive.pressure_check_s')} hint={envHint('adaptive.pressure_check_s', "How often the free memory and CPU load are measured.")} />
      </div>
    </Card>

    {/if}

    <Card title="When capacity is full">
      <div class="grid">
        {#if isEnv('admission.default')}
          {@const current = form.admission}
          <Input label="What happens to a launch" value={admissionOptions.find((o) => o.value === current)?.label} readonly badge={badge('admission.default')} hint={envHint('admission.default')} />
        {:else}
          <Select label="What happens to a launch" bind:value={form.admission} options={admissionOptions} />
        {/if}
        <Input label="Give up waiting after (s)" type="number" min={0} step={1} bind:value={form.queue_timeout_s} error={err('queue_timeout_s')} readonly={isEnv('admission.queue_timeout_s')} badge={badge('admission.queue_timeout_s')} hint={envHint('admission.queue_timeout_s', "How long a launch waits for a free slot.")} />
      </div>
    </Card>
  </div>
  <SaveBar {dirty} {saving} disabled={invalid} onsave={save} ondiscard={() => limits.data && apply(limits.data)} />
{/if}
</div>

<style>
  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .gap {
    height: var(--space-3);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--space-4);
  }

  .tags {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    margin-top: var(--space-6);
    padding-top: var(--space-5);
    border-top: 1px solid var(--border);
  }

  h3 {
    font-size: var(--text-base-size);
    line-height: var(--text-base-line);
  }

  .tag-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 120px auto;
    gap: var(--space-3);
    align-items: start;
  }

  .remove.first {
    padding-top: calc(var(--text-sm-line) + var(--space-2));
  }

  .notice {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-2) var(--space-3) var(--space-2) var(--space-4);
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }

  .env {
    margin-top: var(--space-3);
    color: var(--text-tertiary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }

  @media (max-width: 1023px) {
    .grid {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
