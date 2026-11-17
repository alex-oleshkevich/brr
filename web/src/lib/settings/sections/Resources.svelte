<script lang="ts">
  import { untrack } from 'svelte';
  import { ApiError, api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import type { components, paths } from '$lib/api/schema';
  import Button from '$lib/ui/Button.svelte';
  import Card from '$lib/ui/Card.svelte';
  import Disclosure from '$lib/ui/Disclosure.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Input from '$lib/ui/Input.svelte';
  import SaveBar from '$lib/ui/SaveBar.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import Switch from '$lib/ui/Switch.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  type Limits = components['schemas']['LimitsView'];
  type LimitsBody = paths['/v1/limits']['put']['requestBody']['content']['application/json'];
  type Num = number | null;

  type Form = {
    idle_tab: Num;
    idle_instance: Num;
    idle_grace: Num;
    min_free_ram_mb: Num;
    max_load_per_core: Num;
    reserve_headless: Num;
    reserve_display: Num;
    reserve_camoufox: Num;
    pressure_check_s: Num;
    auto_restart: boolean;
    start_timeout_s: Num;
    stop_timeout_s: Num;
    console_buffer: Num;
    network_buffer: Num;
    login_interval: Num;
  };

  const BOUNDS: Record<string, [number, number]> = {
    idle_tab: [1, Number.MAX_SAFE_INTEGER],
    idle_instance: [1, Number.MAX_SAFE_INTEGER],
    idle_grace: [0, Number.MAX_SAFE_INTEGER],
    min_free_ram_mb: [0, 1_048_576],
    reserve_headless: [0, Number.MAX_SAFE_INTEGER],
    reserve_display: [0, Number.MAX_SAFE_INTEGER],
    reserve_camoufox: [0, Number.MAX_SAFE_INTEGER],
    pressure_check_s: [1, Number.MAX_SAFE_INTEGER],
    start_timeout_s: [1, 3600],
    stop_timeout_s: [1, 3600],
    console_buffer: [1, 100_000],
    network_buffer: [1, 100_000],
    login_interval: [1, Number.MAX_SAFE_INTEGER],
  };
  const SETTING_KEYS: Record<string, string> = {
    idle_tab: 'idle.tab_min',
    idle_instance: 'idle.instance_min',
    idle_grace: 'idle.grace_s',
    auto_restart: 'instance.auto_restart',
    start_timeout_s: 'instance.start_timeout_s',
    stop_timeout_s: 'instance.stop_timeout_s',
    console_buffer: 'tab.console_buffer',
    network_buffer: 'tab.network_buffer',
    login_interval: 'login_check.interval_min',
  };

  let { dirty = $bindable(false) }: { dirty?: boolean } = $props();

  const settings = liveQuery(() => unwrap(api.GET('/v1/settings')), { kinds: ['settings.changed'] });
  // settings.set publishes only settings.changed, even for adaptive.* keys.
  const limits = liveQuery(() => unwrap(api.GET('/v1/limits')), {
    kinds: ['limits.changed', 'settings.changed'],
    match: (e) => e.kind === 'limits.changed' || ((e.data as { keys?: string[] } | null)?.keys ?? []).some((k) => /^adaptive\./.test(k)),
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

  function build(s: components['schemas']['Settings'], l: Limits): Form {
    return {
      idle_tab: s.idle.tab_min,
      idle_instance: s.idle.instance_min,
      idle_grace: s.idle.grace_s,
      min_free_ram_mb: l.adaptive.min_free_ram_mb,
      max_load_per_core: l.adaptive.max_load_per_core,
      reserve_headless: l.adaptive.reserve_mb.chromium_headless,
      reserve_display: l.adaptive.reserve_mb.chromium_display,
      reserve_camoufox: l.adaptive.reserve_mb.camoufox,
      pressure_check_s: l.adaptive.pressure_check_s,
      auto_restart: s.instance.auto_restart,
      start_timeout_s: s.instance.start_timeout_s,
      stop_timeout_s: s.instance.stop_timeout_s,
      console_buffer: s.tab.console_buffer,
      network_buffer: s.tab.network_buffer,
      login_interval: s.login_check.interval_min,
    };
  }

  function apply() {
    const s = settings.data;
    const l = limits.data;
    if (!s || !l) return;
    form = build(s.settings, l);
    baseline = JSON.stringify(form);
    loaded = JSON.stringify([s.settings, l]);
    changedElsewhere = false;
    serverErrors = {};
  }

  $effect(() => {
    const s = settings.data;
    const l = limits.data;
    if (!s || !l || JSON.stringify([s.settings, l]) === loaded) return;
    if (form === undefined || !untrack(() => isDirty)) apply();
    else changedElsewhere = true;
  });

  const sources = $derived(settings.data?.sources ?? {});
  const isEnv = (key: string) => sources[key] === 'env';
  const envName = (key: string) => 'BRR_' + key.replace('.', '__').toUpperCase();
  const badge = (key: string) => (isEnv(key) ? `Set by ${envName(key)}` : undefined);
  const envHint = (key: string, hint?: string) => (isEnv(key) ? 'Remove the environment variable to edit it here.' : hint);

  const errors = $derived.by(() => {
    const e: Record<string, string> = {};
    const f = form;
    if (!f) return e;
    for (const [key, [min, max]] of Object.entries(BOUNDS)) {
      const value = f[key as keyof Form] as Num;
      if (value === null) e[key] = 'Enter a number.';
      else if (!Number.isInteger(value)) e[key] = 'Enter a whole number.';
      else if (value < min) e[key] = `Must be at least ${min}.`;
      else if (value > max) e[key] = `Must be at most ${max.toLocaleString()}.`;
    }
    if (f.max_load_per_core === null) e.max_load_per_core = 'Enter a number.';
    else if (!(f.max_load_per_core > 0 && f.max_load_per_core <= 4)) e.max_load_per_core = 'Enter a number above 0 and up to 4.';
    return e;
  });
  const invalid = $derived(Object.keys(errors).length > 0);
  const err = (key: string) => errors[key] ?? serverErrors[key] ?? null;

  // A server error belongs to the value that caused it; editing anything clears it.
  $effect(() => {
    void JSON.stringify(form);
    untrack(() => (serverErrors = {}));
  });

  async function save() {
    if (!form || invalid || saving) return;
    saving = true;
    serverErrors = {};
    const f = form;
    const before = JSON.parse(baseline) as Form;
    const n = (v: Num) => v as number;
    try {
      const patch: Record<string, unknown> = {};
      for (const [field, dotted] of Object.entries(SETTING_KEYS)) {
        const key = field as keyof Form;
        if (!isEnv(dotted) && f[key] !== before[key]) patch[dotted] = f[key];
      }
      if (Object.keys(patch).length > 0) await unwrap(api.PATCH('/v1/settings', { body: patch }));

      const adaptive: components['schemas']['SetAdaptive'] = {};
      const changed = (...keys: (keyof Form)[]) => keys.some((k) => f[k] !== before[k]);
      if (changed('min_free_ram_mb') && !isEnv('adaptive.min_free_ram_mb')) adaptive.min_free_ram_mb = n(f.min_free_ram_mb);
      if (changed('max_load_per_core') && !isEnv('adaptive.max_load_per_core')) adaptive.max_load_per_core = n(f.max_load_per_core);
      if (changed('pressure_check_s') && !isEnv('adaptive.pressure_check_s')) adaptive.pressure_check_s = n(f.pressure_check_s);
      if (changed('reserve_headless', 'reserve_display', 'reserve_camoufox') && !isEnv('adaptive.reserve_mb')) {
        adaptive.reserve_mb = { chromium_headless: n(f.reserve_headless), chromium_display: n(f.reserve_display), camoufox: n(f.reserve_camoufox) };
      }
      if (Object.keys(adaptive).length > 0) {
        const body: LimitsBody = { adaptive };
        await unwrap(api.PUT('/v1/limits', { body }));
      }

      baseline = JSON.stringify(f);
      loaded = '';
      settings.reload();
      limits.reload();
      toast.success('Settings saved');
    } catch (e) {
      const field = e instanceof ApiError && e.code === 'InvalidArgument' ? (e.details as { field?: string }).field : undefined;
      const known = field ? Object.entries({ ...SETTING_KEYS }).find(([, dotted]) => dotted === field)?.[0] ?? field.split('.').pop()! : undefined;
      if (known && form && known in form) serverErrors = { [known]: (e as ApiError).message };
      else toast.apiError(e, "Couldn't save the settings");
    } finally {
      saving = false;
    }
  }

  const reload = () => {
    settings.reload();
    limits.reload();
  };
</script>

{#if settings.loading || limits.loading}
  <div class="stack" aria-busy="true">
    <Skeleton width="30%" />
    <Skeleton height="64px" radius="var(--radius-md)" />
    <Skeleton height="64px" radius="var(--radius-md)" />
  </div>
{:else if (settings.error && !settings.data) || (limits.error && !limits.data)}
  {@const failure = settings.error ?? limits.error}
  <EmptyState tone="danger" title="Couldn't load these settings" description={failure?.message}>
    {#snippet action()}<Button onclick={reload}>Retry</Button>{/snippet}
  </EmptyState>
{:else if form}
  <div class="stack">
    {#if changedElsewhere}
      <p class="notice" role="status">
        These settings changed elsewhere.
        <Button variant="ghost" size="sm" onclick={apply}>Reload</Button>
      </p>
    {/if}

    <Card title="Idle browsers">
      <div class="grid">
        <Input name="idle.tab_min" label="Close an idle tab after (min)" type="number" min={1} step={1} bind:value={form.idle_tab} error={err('idle_tab')} readonly={isEnv('idle.tab_min')} badge={badge('idle.tab_min')} hint={envHint('idle.tab_min', 'A tab nobody has used for this long is closed.')} />
        <Input name="idle.instance_min" label="Close an idle browser after (min)" type="number" min={1} step={1} bind:value={form.idle_instance} error={err('idle_instance')} readonly={isEnv('idle.instance_min')} badge={badge('idle.instance_min')} hint={envHint('idle.instance_min', 'A browser with no activity for this long is stopped.')} />
        <Input name="idle.grace_s" label="Warning period before closing an idle browser (s)" type="number" min={0} step={1} bind:value={form.idle_grace} error={err('idle_grace')} readonly={isEnv('idle.grace_s')} badge={badge('idle.grace_s')} hint={envHint('idle.grace_s', 'Agents get this long to react before the browser closes.')} />
      </div>
    </Card>

    <Card title="Free memory and CPU" description="Used when capacity is set to start browsers based on free memory.">
      <div class="grid">
        <Input name="adaptive.min_free_ram_mb" label="Keep this much memory free (MB)" type="number" min={0} step={1} bind:value={form.min_free_ram_mb} error={err('min_free_ram_mb')} readonly={isEnv('adaptive.min_free_ram_mb')} badge={badge('adaptive.min_free_ram_mb')} hint={envHint('adaptive.min_free_ram_mb', 'Stops starting browsers when free memory would drop below this.')} />
        <Input name="adaptive.max_load_per_core" label="Stop starting browsers above this CPU load" type="number" min={0} max={4} step={0.05} bind:value={form.max_load_per_core} error={err('max_load_per_core')} readonly={isEnv('adaptive.max_load_per_core')} badge={badge('adaptive.max_load_per_core')} hint={envHint('adaptive.max_load_per_core', '1 means every core is fully busy.')} />
        <Input name="adaptive.reserve_mb.chromium_headless" label="Memory set aside per headless Chromium (MB)" type="number" min={0} step={1} bind:value={form.reserve_headless} error={err('reserve_headless')} readonly={isEnv('adaptive.reserve_mb')} badge={badge('adaptive.reserve_mb')} hint={envHint('adaptive.reserve_mb', 'Estimate used when deciding if another one fits.')} />
        <Input name="adaptive.reserve_mb.chromium_display" label="Memory set aside per Chromium with a window (MB)" type="number" min={0} step={1} bind:value={form.reserve_display} error={err('reserve_display')} readonly={isEnv('adaptive.reserve_mb')} badge={badge('adaptive.reserve_mb')} hint={envHint('adaptive.reserve_mb', 'Estimate used when deciding if another one fits.')} />
        <Input name="adaptive.reserve_mb.camoufox" label="Memory set aside per Camoufox (MB)" type="number" min={0} step={1} bind:value={form.reserve_camoufox} error={err('reserve_camoufox')} readonly={isEnv('adaptive.reserve_mb')} badge={badge('adaptive.reserve_mb')} hint={envHint('adaptive.reserve_mb', 'Estimate used when deciding if another one fits.')} />
        <Input name="adaptive.pressure_check_s" label="Re-check memory and CPU every (s)" type="number" min={1} step={1} bind:value={form.pressure_check_s} error={err('pressure_check_s')} readonly={isEnv('adaptive.pressure_check_s')} badge={badge('adaptive.pressure_check_s')} hint={envHint('adaptive.pressure_check_s', 'How often the free memory and CPU load are measured.')} />
      </div>
    </Card>

    <Card title="Crashes">
      <Switch name="instance.auto_restart" label="Restart a browser that crashes" bind:checked={form.auto_restart} disabled={isEnv('instance.auto_restart')} hint={envHint('instance.auto_restart', 'Tries again a few times with a growing delay.') ?? badge('instance.auto_restart')} />
    </Card>

    <Disclosure summary="Advanced">
      <div class="grid advanced">
        <Input name="instance.start_timeout_s" label="Wait for a browser to start (s)" type="number" min={0} step={1} bind:value={form.start_timeout_s} error={err('start_timeout_s')} readonly={isEnv('instance.start_timeout_s')} badge={badge('instance.start_timeout_s')} hint={envHint('instance.start_timeout_s', 'A browser that is not ready by then is treated as failed.')} />
        <Input name="instance.stop_timeout_s" label="Wait for a browser to stop (s)" type="number" min={0} step={1} bind:value={form.stop_timeout_s} error={err('stop_timeout_s')} readonly={isEnv('instance.stop_timeout_s')} badge={badge('instance.stop_timeout_s')} hint={envHint('instance.stop_timeout_s', 'After this long a browser that will not close is killed.')} />
        <Input name="tab.console_buffer" label="Console messages kept per tab" type="number" min={0} step={1} bind:value={form.console_buffer} error={err('console_buffer')} readonly={isEnv('tab.console_buffer')} badge={badge('tab.console_buffer')} hint={envHint('tab.console_buffer', 'The oldest messages are dropped first.')} />
        <Input name="tab.network_buffer" label="Network requests kept per tab" type="number" min={0} step={1} bind:value={form.network_buffer} error={err('network_buffer')} readonly={isEnv('tab.network_buffer')} badge={badge('tab.network_buffer')} hint={envHint('tab.network_buffer', 'The oldest requests are dropped first.')} />
        <Input name="login_check.interval_min" label="Check saved logins every (min)" type="number" min={1} step={1} bind:value={form.login_interval} error={err('login_interval')} readonly={isEnv('login_check.interval_min')} badge={badge('login_check.interval_min')} hint={envHint('login_check.interval_min', 'How often brr checks that profiles are still logged in.')} />
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

  .grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--space-4);
  }

  .advanced {
    padding-top: var(--space-3);
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

  @media (max-width: 1023px) {
    .grid {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
