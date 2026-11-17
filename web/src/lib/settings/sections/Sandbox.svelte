<script lang="ts">
  import { untrack } from 'svelte';
  import { ApiError, api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import type { components } from '$lib/api/schema';
  import Button from '$lib/ui/Button.svelte';
  import Card from '$lib/ui/Card.svelte';
  import Disclosure from '$lib/ui/Disclosure.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Input from '$lib/ui/Input.svelte';
  import SaveBar from '$lib/ui/SaveBar.svelte';
  import SegmentedControl from '$lib/ui/SegmentedControl.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  type Num = number | null;
  type Form = {
    mode: components['schemas']['SandboxMode'];
    image: string;
    memory_mb: Num;
    cpus: Num;
    pids: Num;
    shm_mb: Num;
  };

  const MODE_OPTIONS = [
    { value: 'none', label: 'None', hint: "None — browsers run as the brr user, with brr's network and files" },
    {
      value: 'docker',
      label: 'Docker',
      hint: 'Docker — each browser runs in its own container: no network except through brr, sees only its own profile and downloads, memory/CPU/process limits',
    },
  ];
  const DOCKER_NOTES = ['Needs the Docker socket. Anyone who controls brr then controls this machine — keep brr behind authentication.', "Visible display isn't available in a container."];
  const BOUNDS: Record<string, [number, number]> = {
    memory_mb: [256, 65_536],
    cpus: [0.1, 64],
    pids: [64, 65_536],
    shm_mb: [64, 16_384],
  };
  const WHOLE = ['memory_mb', 'pids', 'shm_mb'];
  const SETTING_KEYS: Record<string, string> = {
    mode: 'sandbox.mode',
    image: 'sandbox.image',
    memory_mb: 'sandbox.memory_mb',
    cpus: 'sandbox.cpus',
    pids: 'sandbox.pids',
    shm_mb: 'sandbox.shm_mb',
  };

  let { dirty = $bindable(false) }: { dirty?: boolean } = $props();

  const settings = liveQuery(() => unwrap(api.GET('/v1/settings')), { kinds: ['settings.changed'] });

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

  function build(s: components['schemas']['Settings']): Form {
    return {
      mode: s.sandbox.mode,
      image: s.sandbox.image ?? '',
      memory_mb: s.sandbox.memory_mb,
      cpus: s.sandbox.cpus,
      pids: s.sandbox.pids,
      shm_mb: s.sandbox.shm_mb,
    };
  }

  function apply() {
    const s = settings.data;
    if (!s) return;
    form = build(s.settings);
    baseline = JSON.stringify(form);
    loaded = JSON.stringify(s.settings);
    changedElsewhere = false;
    serverErrors = {};
  }

  $effect(() => {
    const s = settings.data;
    if (!s || JSON.stringify(s.settings) === loaded) return;
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
      else if (WHOLE.includes(key) && !Number.isInteger(value)) e[key] = 'Enter a whole number.';
      else if (value < min) e[key] = `Must be at least ${min}.`;
      else if (value > max) e[key] = `Must be at most ${max.toLocaleString()}.`;
    }
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
    try {
      const patch: Record<string, unknown> = {};
      for (const [field, dotted] of Object.entries(SETTING_KEYS)) {
        const key = field as keyof Form;
        if (isEnv(dotted) || f[key] === before[key]) continue;
        patch[dotted] = key === 'image' ? f.image.trim() || null : f[key];
      }
      if (Object.keys(patch).length > 0) await unwrap(api.PATCH('/v1/settings', { body: patch }));

      baseline = JSON.stringify(f);
      loaded = '';
      settings.reload();
      toast.success('Settings saved');
    } catch (e) {
      const field = e instanceof ApiError && e.code === 'InvalidArgument' ? (e.details as { field?: string }).field : undefined;
      const known = field ? Object.entries(SETTING_KEYS).find(([, dotted]) => dotted === field)?.[0] : undefined;
      if (known) serverErrors = { [known]: (e as ApiError).message };
      else toast.apiError(e, "Couldn't save the settings");
    } finally {
      saving = false;
    }
  }
</script>

{#if settings.loading}
  <div class="stack" aria-busy="true">
    <Skeleton width="30%" />
    <Skeleton height="64px" radius="var(--radius-md)" />
  </div>
{:else if settings.error && !settings.data}
  <EmptyState tone="danger" title="Couldn't load these settings" description={settings.error.message}>
    {#snippet action()}<Button onclick={settings.reload}>Retry</Button>{/snippet}
  </EmptyState>
{:else if form}
  <div class="stack">
    {#if changedElsewhere}
      <p class="notice" role="status">
        Settings changed elsewhere.
        <Button variant="ghost" size="sm" onclick={apply}>Reload</Button>
      </p>
    {/if}

    <Card title="Isolation" description="Applies to browsers started from now on. Running browsers keep the mode they started with.">
      <SegmentedControl name="sandbox.mode" label="Sandbox mode" bind:value={form.mode} options={MODE_OPTIONS} disabled={isEnv('sandbox.mode')} />
      {#if isEnv('sandbox.mode')}
        <p class="note">{badge('sandbox.mode')}. {envHint('sandbox.mode')}</p>
      {/if}
      {#if form.mode === 'docker'}
        {#each DOCKER_NOTES as note (note)}
          <p class="note">{note}</p>
        {/each}
      {/if}
    </Card>

    <Disclosure summary="Advanced">
      <div class="grid advanced">
        <Input name="sandbox.image" label="Docker image" placeholder="Image brr runs in" bind:value={form.image} error={err('image')} readonly={isEnv('sandbox.image')} badge={badge('sandbox.image')} hint={envHint('sandbox.image', 'Leave empty to use the image brr is running in.')} />
        <Input name="sandbox.memory_mb" label="Memory limit per browser (MB)" type="number" min={256} max={65536} step={1} bind:value={form.memory_mb} error={err('memory_mb')} readonly={isEnv('sandbox.memory_mb')} badge={badge('sandbox.memory_mb')} hint={envHint('sandbox.memory_mb')} />
        <Input name="sandbox.cpus" label="CPU limit per browser" type="number" min={0.1} max={64} step={0.1} bind:value={form.cpus} error={err('cpus')} readonly={isEnv('sandbox.cpus')} badge={badge('sandbox.cpus')} hint={envHint('sandbox.cpus', 'Number of cores a browser may use.')} />
        <Input name="sandbox.pids" label="Process limit per browser" type="number" min={64} max={65536} step={1} bind:value={form.pids} error={err('pids')} readonly={isEnv('sandbox.pids')} badge={badge('sandbox.pids')} hint={envHint('sandbox.pids')} />
        <Input name="sandbox.shm_mb" label="Shared memory per browser (MB)" type="number" min={64} max={16384} step={1} bind:value={form.shm_mb} error={err('shm_mb')} readonly={isEnv('sandbox.shm_mb')} badge={badge('sandbox.shm_mb')} hint={envHint('sandbox.shm_mb', 'Chromium needs this for rendering.')} />
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

  .note {
    margin: var(--space-2) 0 0;
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
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
