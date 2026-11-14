<script lang="ts">
  import { untrack } from 'svelte';
  import { ApiError, api, unwrap } from '$lib/api/client';
  import type { components } from '$lib/api/schema';
  import Button from '$lib/ui/Button.svelte';
  import Checkbox from '$lib/ui/Checkbox.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import Input from '$lib/ui/Input.svelte';
  import SegmentedControl from '$lib/ui/SegmentedControl.svelte';
  import { toast } from '$lib/ui/toast.svelte';
  import HealthDot from './HealthDot.svelte';
  import { proxyLabel, type Proxy } from './format';

  type Pool = components['schemas']['Pool'];

  let {
    open = $bindable(false),
    pool,
    proxies,
    onsaved,
  }: { open?: boolean; pool?: Pool; proxies: Proxy[]; onsaved: () => void } = $props();

  const modes = [
    { value: 'sticky', label: 'Sticky per profile' },
    { value: 'rotate', label: 'Rotate per launch' },
  ];

  let name = $state('');
  let mode = $state('sticky');
  let selected = $state<Record<string, boolean>>({});
  let nameError = $state<string | null>(null);
  let saving = $state(false);

  $effect(() => {
    if (!open) return;
    untrack(() => {
      name = pool?.name ?? '';
      mode = pool?.mode ?? 'sticky';
      selected = Object.fromEntries((pool?.proxy_ids ?? []).map((id) => [id, true]));
      nameError = null;
    });
  });

  async function save(event: SubmitEvent) {
    event.preventDefault();
    nameError = name.trim() ? null : 'Enter a name.';
    if (nameError) return;

    saving = true;
    const body = {
      name: name.trim(),
      mode: mode as 'sticky' | 'rotate',
      proxy_ids: proxies.filter((p) => selected[p.id]).map((p) => p.id),
    };
    try {
      if (pool) await unwrap(api.PATCH('/v1/proxy-pools/{pool}', { params: { path: { pool: pool.id } }, body }));
      else await unwrap(api.POST('/v1/proxy-pools', { body }));
      toast.success(pool ? 'Pool saved' : 'Pool created');
      onsaved();
      open = false;
    } catch (e) {
      if (e instanceof ApiError && ['Conflict', 'InvalidArgument'].includes(e.code)) nameError = e.message;
      else toast.apiError(e, "Couldn't save the pool");
    } finally {
      saving = false;
    }
  }
</script>

<Dialog bind:open title={pool ? 'Edit pool' : 'New pool'} dismissible={!saving}>
  <form id="pool-form" onsubmit={save} novalidate>
    <div class="fields">
      <Input label="Name" bind:value={name} required error={nameError} autocomplete="off" />
      <div class="field">
        <span class="field-label">Mode</span>
        <SegmentedControl label="Mode" bind:value={mode} options={modes} />
      </div>
      <div class="field">
        <span class="field-label" id="members-label">Members</span>
        <div role="group" aria-labelledby="members-label" class="members">
        {#each proxies as proxy (proxy.id)}
          <div class="member">
            <Checkbox label={proxyLabel(proxy)} bind:checked={() => !!selected[proxy.id], (checked) => (selected[proxy.id] = checked)} />
            <HealthDot {proxy} />
          </div>
        {:else}
          <p class="none">Add a proxy first.</p>
        {/each}
        </div>
      </div>
    </div>
  </form>
  {#snippet footer()}
    <Button onclick={() => (open = false)} disabled={saving}>Cancel</Button>
    <Button variant="primary" type="submit" form="pool-form" loading={saving}>Save</Button>
  {/snippet}
</Dialog>

<style>
  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .members {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    max-height: 280px;
    padding: var(--space-3);
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }

  .member {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .none {
    color: var(--text-secondary);
  }
</style>
