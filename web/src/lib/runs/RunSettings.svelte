<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import type { components } from '$lib/api/schema';
  import SettingsView from '$lib/config/SettingsView.svelte';
  import Checkbox from '$lib/ui/Checkbox.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import PanelState from './PanelState.svelte';
  import { NOT_RECORDED } from './runs';

  type Instance = components['schemas']['Instance'];

  let { run }: { run: Instance } = $props();

  let changes = $state(false);

  // The Profile as it is now; a deleted Profile leaves nothing to compare with.
  const current = liveQuery(
    async () => {
      if (!run.profile_id) return null;
      try {
        return await unwrap(api.GET('/v1/profiles/{profile}', { params: { path: { profile: run.profile_id } } }));
      } catch {
        return null;
      }
    },
    { kinds: ['profile.*'] },
  );
</script>

{#if !run.settings}
  <EmptyState icon="file" title={NOT_RECORDED} description="These are the settings the browser ran with." compact />
{:else}
  <PanelState loading={current.loading} error={current.error} onretry={() => current.reload()} title="the profile">
    <div class="stack">
      {#if current.data}
        <Checkbox bind:checked={changes} label="Show changes since this run" hint="Marks each changed field and shows the run's value next to the current one in {current.data.name}." />
      {/if}
      <SettingsView settings={run.settings} extensions={run.extensions ?? undefined} compare={changes && current.data ? current.data.settings : undefined} />
    </div>
  </PanelState>
{/if}

<style>
  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
</style>
