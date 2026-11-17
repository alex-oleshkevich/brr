<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import type { components } from '$lib/api/schema';
  import Card from '$lib/ui/Card.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import KeyValue from '$lib/ui/KeyValue.svelte';
  import PanelState from './PanelState.svelte';
  import { NOT_RECORDED } from './runs';

  type Instance = components['schemas']['Instance'];

  let { run }: { run: Instance } = $props();

  // The Template link only works while the Template exists.
  const template = liveQuery(
    async () => {
      if (!run.template_id) return null;
      try {
        return await unwrap(api.GET('/v1/templates/{template}', { params: { path: { template: run.template_id } } }));
      } catch {
        return null;
      }
    },
    { kinds: ['template.*'] },
  );
</script>

{#if !run.template_name && run.extensions === null}
  <EmptyState icon="file" title={NOT_RECORDED} compact />
{:else}
  <PanelState loading={template.loading} error={template.error} onretry={() => template.reload()} title="the template">
    <div class="stack">
      <Card title="Template">
        <KeyValue
          items={[
            {
              label: 'Template',
              value: run.template_name ?? null,
              href: template.data ? `/templates/${template.data.id}` : undefined,
            },
          ]}
        />
        {#if run.template_name && !template.data}<p class="note">This template has since been deleted.</p>{/if}
      </Card>
      <Card title="Extensions loaded" flush={(run.extensions?.length ?? 0) > 0}>
        {#if !run.extensions?.length}
          <p class="note">No extensions were loaded.</p>
        {:else}
          <ul>
            {#each run.extensions as extension (extension.id)}
              <li><span>{extension.name}</span><span class="version">{extension.version}</span></li>
            {/each}
          </ul>
        {/if}
      </Card>
    </div>
  </PanelState>
{/if}

<style>
  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }

  .note {
    color: var(--text-secondary);
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: flex;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-3) var(--space-5);
    border-top: 1px solid var(--border);
  }

  li:first-child {
    border-top: 0;
  }

  .version {
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
  }
</style>
