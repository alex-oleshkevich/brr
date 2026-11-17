<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { ApiError, api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import type { components, paths } from '$lib/api/schema';
  import { clock } from '$lib/clock.svelte';
  import { relative } from '$lib/format';
  import Badge from '$lib/ui/Badge.svelte';
  import Button from '$lib/ui/Button.svelte';
  import Card from '$lib/ui/Card.svelte';
  import Disclosure from '$lib/ui/Disclosure.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Input from '$lib/ui/Input.svelte';
  import SaveBar from '$lib/ui/SaveBar.svelte';
  import SegmentedControl from '$lib/ui/SegmentedControl.svelte';
  import Select from '$lib/ui/Select.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
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
    admission: string;
    priority: Num;
    queue_timeout_s: Num;
    exclusive_wait_s: Num;
  };

  const EXCLUSIVE_WAIT_KEY = 'lease.exclusive_wait_s';
  const MAX_SECONDS = 86_400;
  const INT_FIELDS: [keyof Form, number, number][] = [
    ['max_instances', 1, Number.MAX_SAFE_INTEGER],
    ['chromium', 0, Number.MAX_SAFE_INTEGER],
    ['camoufox', 0, Number.MAX_SAFE_INTEGER],
    ['max_tabs_per_instance', 0, Number.MAX_SAFE_INTEGER],
    ['max_tabs_total', 0, Number.MAX_SAFE_INTEGER],
    ['queue_timeout_s', 0, MAX_SECONDS],
    ['priority', 0, 255],
    ['exclusive_wait_s', 0, MAX_SECONDS],
  ];

  let { dirty = $bindable(false) }: { dirty?: boolean } = $props();

  // settings.set publishes only settings.changed, even for limits.* and admission.* keys.
  const limits = liveQuery(() => unwrap(api.GET('/v1/limits')), {
    kinds: ['limits.changed', 'settings.changed'],
    match: (e) => e.kind === 'limits.changed' || ((e.data as { keys?: string[] } | null)?.keys ?? []).some((k) => /^(limits|admission)\./.test(k)),
  });
  const settings = liveQuery(() => unwrap(api.GET('/v1/settings')), { kinds: ['settings.changed'] });
  const queue = liveQuery(async () => (await unwrap(api.GET('/v1/admission/queue'))).queue, {
    kinds: ['instance.queued', 'instance.state_changed', 'admission.decided'],
  });

  let form = $state<Form>();
  let baseline = $state('');
  let loaded = '';
  let changedElsewhere = $state(false);
  let saving = $state(false);
  let serverErrors = $state<Record<string, string>>({});

  const isDirty = $derived(form !== undefined && JSON.stringify(form) !== baseline);
  $effect(() => {
    dirty = isDirty;
  });

  function apply() {
    const l: Limits | undefined = limits.data;
    const s = settings.data;
    if (!l || !s) return;
    form = {
      mode: l.mode,
      max_instances: l.max_instances,
      chromium: l.max_instances_per_engine.chromium,
      camoufox: l.max_instances_per_engine.camoufox,
      max_tabs_per_instance: l.max_tabs_per_instance,
      max_tabs_total: l.max_tabs_total,
      tags: Object.entries(l.max_instances_per_tag).map(([tag, max]) => ({ tag, max })),
      admission: l.admission.default,
      priority: l.admission.priority,
      queue_timeout_s: l.admission.queue_timeout_s,
      exclusive_wait_s: s.settings.lease.exclusive_wait_s,
    };
    baseline = JSON.stringify(form);
    loaded = JSON.stringify([l, s.settings.lease]);
    changedElsewhere = false;
    serverErrors = {};
  }

  $effect(() => {
    const l = limits.data;
    const s = settings.data;
    if (!l || !s || JSON.stringify([l, s.settings.lease]) === loaded) return;
    if (form === undefined || !untrack(() => isDirty)) apply();
    else changedElsewhere = true;
  });

  const sources = $derived(settings.data?.sources ?? {});
  const isEnv = (key: string) => sources[key] === 'env';
  const envName = (key: string) => 'BRR_' + key.replace('.', '__').toUpperCase();
  const badge = (key: string) => (isEnv(key) ? `Set by ${envName(key)}` : undefined);
  const envHint = (key: string, hint?: string) => (isEnv(key) ? 'Remove the environment variable to edit it here.' : hint);

  const isInt = (n: Num) => n !== null && Number.isInteger(n) && n >= 0;

  const errors = $derived.by(() => {
    const f = form;
    const e: Record<string, string> = {};
    if (!f) return e;
    for (const [key, min, max] of INT_FIELDS) {
      const value = f[key] as Num;
      if (value === null) e[key] = 'Enter a number.';
      else if (!Number.isInteger(value)) e[key] = 'Enter a whole number.';
      else if (value < min) e[key] = `Must be at least ${min}.`;
      else if (value > max) e[key] = `Must be at most ${max.toLocaleString()}.`;
    }
    const seen = new Set<string>();
    f.tags.forEach((row, i) => {
      const tag = row.tag.trim();
      if (!tag) e[`tag-${i}`] = 'Enter a tag.';
      else if (seen.has(tag)) e[`tag-${i}`] = 'Tag already listed.';
      seen.add(tag);
      if (row.max === null) e[`tag-max-${i}`] = 'Enter a number.';
      else if (!isInt(row.max)) e[`tag-max-${i}`] = 'Enter a whole number, 0 or more.';
    });
    return e;
  });
  const engineWarning = $derived.by(() => {
    const f = form;
    if (!f || f.max_instances === null) return null;
    const limit = f.max_instances;
    const over = [
      ['Chromium', f.chromium],
      ['Camoufox', f.camoufox],
    ].filter(([, n]) => n !== null && (n as number) > limit);
    if (!over.length) return null;
    return `${over.map(([name]) => name).join(' and ')} ${over.length > 1 ? 'limits are' : 'limit is'} above the overall limit of ${limit}, so the overall limit applies first.`;
  });
  const invalid = $derived(Object.keys(errors).length > 0);
  const err = (key: string) => errors[key] ?? serverErrors[key] ?? null;

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

    const admission: components['schemas']['SetAdmission'] = {};
    if (!isEnv('admission.default')) admission.default = f.admission as 'queue' | 'reject' | 'evict';
    if (!isEnv('admission.queue_timeout_s')) admission.queue_timeout_s = n(f.queue_timeout_s);
    if (!isEnv('admission.priority')) admission.priority = n(f.priority);
    if (Object.keys(admission).length) b.admission = admission;
    return b;
  }

  async function save() {
    if (!form || invalid || saving) return;
    saving = true;
    serverErrors = {};
    const f = form;
    const before = JSON.parse(baseline) as Form;
    try {
      await unwrap(api.PUT('/v1/limits', { body: body(f) }));
      if (f.exclusive_wait_s !== before.exclusive_wait_s && !isEnv(EXCLUSIVE_WAIT_KEY)) {
        baseline = JSON.stringify({ ...f, exclusive_wait_s: before.exclusive_wait_s });
        try {
          await unwrap(api.PATCH('/v1/settings', { body: { [EXCLUSIVE_WAIT_KEY]: f.exclusive_wait_s } }));
        } catch (e) {
          loaded = '';
          limits.reload();
          settings.reload();
          if (e instanceof ApiError && e.code === 'InvalidArgument') serverErrors = { exclusive_wait_s: e.message };
          toast.apiError(e, 'Limits and admission were saved, but the exclusive lease wait was not');
          return;
        }
      }
      baseline = JSON.stringify(f);
      loaded = '';
      limits.reload();
      settings.reload();
      toast.success('Capacity saved');
    } catch (e) {
      const field = e instanceof ApiError && e.code === 'InvalidArgument' ? (e.details as { field?: string }).field : undefined;
      const key = field === EXCLUSIVE_WAIT_KEY ? 'exclusive_wait_s' : field?.split('.').pop();
      if (key && form && key in form) serverErrors = { [key]: (e as ApiError).message };
      else toast.apiError(e, "Couldn't save capacity");
    } finally {
      saving = false;
    }
  }

  const reload = () => {
    limits.reload();
    settings.reload();
  };

  const modeOptions = [
    { value: 'static', label: 'Fixed limits', hint: 'Run up to the maximums below.' },
    {
      value: 'adaptive',
      label: 'Based on free memory',
      hint: 'Start browsers while memory and CPU allow it. The maximums below still apply. Tune the thresholds under Idle & resources.',
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

{#if limits.loading || settings.loading}
  <div class="stack" aria-busy="true">
    {#each { length: 3 } as _, i (i)}
      <Card><Skeleton width="30%" /><div class="gap"></div><Skeleton height="64px" radius="var(--radius-md)" /></Card>
    {/each}
  </div>
{:else if (limits.error && !limits.data) || (settings.error && !settings.data)}
  {@const failure = limits.error ?? settings.error}
  <EmptyState tone="danger" title="Couldn't load capacity" description={failure?.message}>
    {#snippet action()}<Button onclick={reload}>Retry</Button>{/snippet}
  </EmptyState>
{:else if form}
  <div class="stack">
    {#if changedElsewhere}
      <p class="notice" role="status">
        Capacity changed elsewhere.
        <Button variant="ghost" size="sm" onclick={apply}>Reload</Button>
      </p>
    {/if}

    <Card>
      <SegmentedControl name="limits.mode" label="Mode" bind:value={form.mode} options={modeOptions} disabled={isEnv('limits.mode')} />
      {#if isEnv('limits.mode')}
        <p class="env"><Badge>{badge('limits.mode')}</Badge> {envHint('limits.mode')}</p>
      {/if}
    </Card>

    <Card title="Maximums">
      <div class="grid">
        <Input name="limits.max_instances" label="Browsers running at once" type="number" min={1} step={1} bind:value={form.max_instances} error={err('max_instances')} readonly={isEnv('limits.max_instances')} badge={badge('limits.max_instances')} hint={envHint('limits.max_instances', 'Further launches wait or are refused, see below.')} />
        <Input name="limits.max_instances_per_engine.chromium" label="Chromium browsers at once" type="number" min={0} step={1} bind:value={form.chromium} error={err('chromium')} readonly={isEnv('limits.max_instances_per_engine')} badge={badge('limits.max_instances_per_engine')} hint={envHint('limits.max_instances_per_engine', 'Limit for Chrome-based browsers.')} />
        <Input name="limits.max_instances_per_engine.camoufox" label="Camoufox browsers at once" type="number" min={0} step={1} bind:value={form.camoufox} error={err('camoufox')} readonly={isEnv('limits.max_instances_per_engine')} badge={badge('limits.max_instances_per_engine')} hint={envHint('limits.max_instances_per_engine', 'Limit for Firefox-based (Camoufox) browsers.')} />
        <Input name="limits.max_tabs_per_instance" label="Tabs per browser" type="number" min={0} step={1} bind:value={form.max_tabs_per_instance} error={err('max_tabs_per_instance')} readonly={isEnv('limits.max_tabs_per_instance')} badge={badge('limits.max_tabs_per_instance')} hint={envHint('limits.max_tabs_per_instance', 'The most tabs one browser can have open.')} />
        <Input name="limits.max_tabs_total" label="Tabs across all browsers" type="number" min={0} step={1} bind:value={form.max_tabs_total} error={err('max_tabs_total')} readonly={isEnv('limits.max_tabs_total')} badge={badge('limits.max_tabs_total')} hint={envHint('limits.max_tabs_total', 'The most tabs open at once, counting every browser.')} />
      </div>

      {#if engineWarning}<p class="warn" role="status">{engineWarning}</p>{/if}

      <fieldset class="tags" name="limits.max_instances_per_tag" bind:this={tagList}>
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
      </fieldset>
    </Card>

    <Card title="When capacity is full">
      <div class="grid">
        {#if isEnv('admission.default')}
          {@const current = form.admission}
          <Input name="admission.default" label="What happens to a launch" value={admissionOptions.find((o) => o.value === current)?.label} readonly badge={badge('admission.default')} hint={envHint('admission.default')} />
        {:else}
          <Select name="admission.default" label="What happens to a launch" bind:value={form.admission} options={admissionOptions} />
        {/if}
        <Input name="admission.queue_timeout_s" label="Give up waiting after (s)" type="number" min={0} step={1} bind:value={form.queue_timeout_s} error={err('queue_timeout_s')} readonly={isEnv('admission.queue_timeout_s')} badge={badge('admission.queue_timeout_s')} hint={envHint('admission.queue_timeout_s', 'How long a launch waits for a free slot.')} />
        <Input name="admission.priority" label="Default launch priority" type="number" min={0} max={255} step={1} bind:value={form.priority} error={err('priority')} readonly={isEnv('admission.priority')} badge={badge('admission.priority')} hint={envHint('admission.priority', 'Higher numbers start first when launches are waiting.')} />
      </div>
    </Card>

    <Card title="Waiting to start" description="Launches queued until there is room.">
      {#if queue.error && !queue.data}
        <EmptyState tone="danger" title="Couldn't load the queue" description={queue.error.message}>
          {#snippet action()}<Button onclick={() => queue.reload()}>Retry</Button>{/snippet}
        </EmptyState>
      {:else if queue.loading}
        <Skeleton height="40px" radius="var(--radius-md)" />
      {:else if !queue.data?.length}
        <p class="env">Nothing is waiting.</p>
      {:else}
        <ol class="queue">
          {#each queue.data as entry (entry.instance_id)}
            <li><span class="pos">{entry.queue_pos}</span> <span>{entry.profile_name}</span> <Badge>Priority {entry.priority}</Badge> <span class="since">waiting since {relative(entry.queued_at, clock.now)}</span></li>
          {/each}
        </ol>
      {/if}
    </Card>

    <Disclosure summary="Advanced">
      <div class="advanced">
        <Input name="lease.exclusive_wait_s" label="Wait for an exclusive lease (s)" type="number" min={0} step={1} bind:value={form.exclusive_wait_s} error={err('exclusive_wait_s')} readonly={isEnv(EXCLUSIVE_WAIT_KEY)} badge={badge(EXCLUSIVE_WAIT_KEY)} hint={envHint(EXCLUSIVE_WAIT_KEY, 'How long an agent waits for others to release a browser it asks to use alone.')} />
      </div>
    </Disclosure>
  </div>
  <SaveBar dirty={isDirty} {saving} disabled={invalid} onsave={save} ondiscard={apply} />
{/if}

<style>
  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding-bottom: 80px;
  }

  .warn {
    margin-top: var(--space-4);
    padding: var(--space-2) var(--space-3);
    background: var(--warning-bg);
    border: 1px solid var(--warning-border);
    border-radius: var(--radius-md);
    color: var(--warning-fg);
  }

  .since {
    color: var(--text-tertiary);
    font-size: var(--text-sm-size);
  }

  .gap {
    height: var(--space-3);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--space-4);
  }

  .advanced {
    padding-top: var(--space-3);
    max-width: 50%;
  }

  .tags {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-inline-size: 0;
    margin: var(--space-6) 0 0;
    padding: var(--space-5) 0 0;
    border: 0;
    border-top: 1px solid var(--border);
  }

  h3 {
    font-size: var(--text-base-size);
    line-height: var(--text-base-line);
  }

  .tag-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(140px, 200px) auto;
    gap: var(--space-3);
    align-items: start;
  }

  .remove.first {
    padding-top: calc(var(--text-sm-line) + var(--space-2));
  }

  .queue {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .queue li {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .pos {
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
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

    .advanced {
      max-width: none;
    }
  }
</style>
