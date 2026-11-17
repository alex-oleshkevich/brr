<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import Button from '$lib/ui/Button.svelte';
  import Checkbox from '$lib/ui/Checkbox.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import { toast } from '$lib/ui/toast.svelte';
  import { engineOf } from '$lib/config/form';
  import type { components } from '$lib/api/schema';

  type ExtensionView = components['schemas']['ExtensionView'];
  type Template = components['schemas']['Template'];

  let { open = $bindable(false), extension, onsaved }: { open?: boolean; extension: ExtensionView | null; onsaved?: () => void } = $props();

  const templates = liveQuery(async () => (await unwrap(api.GET('/v1/templates'))).templates, { kinds: [] });
  const builds = liveQuery(async () => (await unwrap(api.GET('/v1/builds'))).builds, { kinds: [] });

  let checked = $state<Set<string>>(new Set());
  let saving = $state(false);

  const matching = $derived((templates.data ?? []).filter((t: Template) => !t.builtin));

  const eligible = $derived(extension ? matching.filter((t) => engineOf(t.settings, builds.data ?? []) === extension.engine) : []);



  $effect(() => {
    if (!open || !extension) return;
    checked = new Set((templates.data ?? []).filter((t) => t.extensions.some((e) => e.extension_id === extension.id)).map((t) => t.id));
  });

  function toggle(id: string, on: boolean) {
    const next = new Set(checked);
    if (on) next.add(id);
    else next.delete(id);
    checked = next;
  }

  async function save() {
    if (!extension || saving) return;
    saving = true;
    try {
      for (const t of eligible) {
        const has = t.extensions.some((e) => e.extension_id === extension.id);
        const want = checked.has(t.id);
        if (has === want) continue;
        const extensions = want
          ? [...t.extensions, { extension_id: extension.id, auto_update: true, pinned_version: null }]
          : t.extensions.filter((e) => e.extension_id !== extension.id);
        await unwrap(api.PATCH('/v1/templates/{template}', { params: { path: { template: t.id } }, body: { template: t.id, extensions } }));
      }
      toast.success('Templates updated');
      open = false;
      templates.reload();
      onsaved?.();
    } catch (e) {
      toast.apiError(e, "Couldn't update the templates");
    } finally {
      saving = false;
    }
  }
</script>

<Dialog bind:open title="Add to templates">
  {#if extension}
    <p class="intro">Choose the templates that should include {extension.name}. New profiles made from them get it too.</p>
    {#if templates.loading || builds.loading}
      <p class="field-hint" role="status">Loading templates…</p>
    {:else if templates.error}
      <p class="danger" role="alert">{templates.error.message}</p>
    {:else if eligible.length === 0}
      <p class="field-hint">No templates of yours use {extension.engine === 'camoufox' ? 'Camoufox' : 'Chromium'} yet.</p>
    {:else}
      <div class="list">
        {#each eligible as t (t.id)}
          <Checkbox label={t.name} checked={checked.has(t.id)} onchange={(on) => toggle(t.id, on)} />
        {/each}
      </div>
    {/if}
  {/if}
  {#snippet footer()}
    <Button onclick={() => (open = false)} disabled={saving}>Cancel</Button>
    <Button variant="primary" loading={saving} disabled={!extension || eligible.length === 0} onclick={save}>Save</Button>
  {/snippet}
</Dialog>

<style>
  .intro {
    margin-bottom: var(--space-4);
    color: var(--text-secondary);
  }

  .list {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .danger {
    padding: var(--space-3);
    background: var(--danger-bg);
    border: 1px solid var(--danger-border);
    border-radius: var(--radius-md);
    color: var(--danger-fg);
  }
</style>
