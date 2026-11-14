<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { ApiError, api, unwrap } from '$lib/api/client';
  import Badge from '$lib/ui/Badge.svelte';
  import Button from '$lib/ui/Button.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Icon from '$lib/ui/Icon.svelte';
  import PageHeader from '$lib/ui/PageHeader.svelte';
  import SaveBar from '$lib/ui/SaveBar.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import { guardUnsaved } from '$lib/ui/unsaved';
  import { confirm } from '$lib/ui/confirm.svelte';
  import { toast } from '$lib/ui/toast.svelte';
  import type { components } from '$lib/api/schema';
  import SettingsForm from '$lib/profiles/SettingsForm.svelte';
  import GeneralCard from '$lib/profiles/GeneralCard.svelte';
  import { templatePurpose, templateTitle, type Template } from '$lib/profiles/templates';
  import { canonical, engineOf, type GlobalSettings, type Settings } from '$lib/profiles/form';

  type PatchBody = components['schemas']['TemplateUpdateInput'];

  const templateId = $derived(page.params.id ?? '');

  let template = $state<Template | null>(null);
  let loadError = $state<ApiError | null>(null);
  let loading = $state(true);
  let settings = $state<Settings | null>(null);
  let name = $state('');
  let snapshot = $state('');
  let revision = $state(0);
  let saving = $state(false);
  let formInvalid = $state(false);
  let duplicating = $state(false);
  let errors = $state<Record<string, string>>({});
  let defaults = $state<GlobalSettings | undefined>(undefined);
  let builds = $state<components['schemas']['BuildView'][]>([]);
  let proxies = $state<components['schemas']['Proxy'][]>([]);
  let pools = $state<components['schemas']['Pool'][]>([]);
  let library = $state<components['schemas']['ExtensionView'][]>([]);

  const dirty = $derived(snapshot !== '' && canonical({ name, settings }) !== snapshot);

  function applyTemplate(next: Template) {
    template = next;
    settings = JSON.parse(JSON.stringify(next.settings)) as Settings;
    name = next.name;
    snapshot = canonical({ name, settings: next.settings });
    errors = {};
    revision += 1;
  }

  async function load() {
    loading = true;
    try {
      applyTemplate(await unwrap(api.GET('/v1/templates/{template}', { params: { path: { template: templateId } } })));
      loadError = null;
    } catch (e) {
      loadError = e instanceof ApiError ? e : new ApiError(0, 'Unreachable', String(e), {});
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void templateId;
    void load();
  });

  onMount(() => {
    void unwrap(api.GET('/v1/settings')).then((out) => (defaults = out.settings), (e) => toast.apiError(e, "Couldn't load settings"));
    void unwrap(api.GET('/v1/builds')).then((out) => (builds = out.builds), (e) => toast.apiError(e, "Couldn't load builds"));
    void unwrap(api.GET('/v1/proxies')).then((out) => (proxies = out.proxies), (e) => toast.apiError(e, "Couldn't load proxies"));
    void unwrap(api.GET('/v1/proxy-pools')).then((out) => (pools = out.pools), (e) => toast.apiError(e, "Couldn't load proxy pools"));
    void unwrap(api.GET('/v1/extensions')).then((out) => (library = out.extensions), (e) => toast.apiError(e, "Couldn't load extensions"));
  });

  guardUnsaved(() => dirty, 'Your edits to this template will be lost.');

  async function save() {
    if (!template || !settings || saving) return;
    saving = true;
    errors = {};
    try {
      const before = JSON.parse(snapshot) as { name: string; settings: Settings };
      const body: PatchBody = { template: templateId };
      if (name !== before.name) body.name = name;
      if (canonical(settings) !== canonical(before.settings)) body.settings = settings;
      if (Object.keys(body).length > 1) {
        applyTemplate(await unwrap(api.PATCH('/v1/templates/{template}', { params: { path: { template: templateId } }, body })));
      }
      toast.success('Template saved');
    } catch (e) {
      if (e instanceof ApiError && e.code === 'InvalidArgument' && typeof e.details?.field === 'string') {
        errors = { [e.details.field]: e.message };
        await tick();
        if (!document.querySelector('[aria-invalid="true"]')) toast.apiError(e, "Couldn't save the template");
        else document.querySelector('[aria-invalid="true"]')?.scrollIntoView({ behavior: 'smooth', block: 'center' });
      } else {
        toast.apiError(e, "Couldn't save the template");
      }
    } finally {
      saving = false;
    }
  }

  async function duplicate() {
    if (!template || duplicating) return;
    duplicating = true;
    try {
      const created = await unwrap(api.POST('/v1/templates', { body: { name: `${template.name}-copy`, settings: template.settings } }));
      toast.success('Template duplicated');
      await goto(`/templates/${created.id}`);
    } catch (e) {
      toast.apiError(e, "Couldn't duplicate the template");
    } finally {
      duplicating = false;
    }
  }

  const engine = $derived(settings ? engineOf(settings, builds) : 'chromium');
</script>

{#if loading && !template}
  <div aria-busy="true" class="stack">
    <Skeleton width="240px" height="28px" />
    <Skeleton height="180px" radius="var(--radius-lg)" />
    <Skeleton height="180px" radius="var(--radius-lg)" />
  </div>
{:else if loadError?.code === 'NotFound' && !template}
  <EmptyState level={1} title="Template not found" description="It may have been deleted.">
    {#snippet action()}<Button href="/profiles?tab=templates">Back to templates</Button>{/snippet}
  </EmptyState>
{:else if loadError && !template}
  <EmptyState tone="danger" title="Couldn't load this template" description={loadError.message}>
    {#snippet action()}<Button onclick={load}>Retry</Button>{/snippet}
  </EmptyState>
{:else if template && settings}
  {@const current = template}
  <PageHeader
    title={templateTitle(current)}
    description={templatePurpose(current)}
    breadcrumb={[
      { label: 'Profiles', href: '/profiles' },
      { label: 'Templates', href: '/profiles?tab=templates' },
      { label: templateTitle(current), href: `/templates/${current.id}` },
    ]}
  >
    {#snippet badge()}
      {#if current.builtin}<Badge>Built-in</Badge>{/if}
      <Badge>{engine === 'camoufox' ? 'Camoufox' : 'Chromium'}</Badge>
    {/snippet}
    {#snippet actions()}
      {#if !current.builtin}
        <Button variant="primary" onclick={save} loading={saving} disabled={!dirty || formInvalid}>Save</Button>
      {/if}
    {/snippet}
  </PageHeader>

  {#if current.builtin}
    <p class="banner">
      <Icon name="info" /> Built-in templates are read-only. Duplicate it to make changes.
      <Button size="sm" loading={duplicating} onclick={duplicate}>Duplicate</Button>
    </p>
  {/if}

  {#key revision}
    <SettingsForm bind:settings {errors} {builds} {proxies} {pools} {library} {defaults} isTemplate readonly={current.builtin} oninvalidchange={(invalid) => (formInvalid = invalid)}>
      {#snippet general()}
        <GeneralCard bind:name showMeta={false} readonly={current.builtin} nameError={errors['name'] ?? null} />
      {/snippet}
    </SettingsForm>
  {/key}

  <SaveBar dirty={dirty && !current.builtin} {saving} disabled={formInvalid} onsave={save} ondiscard={load} />
{/if}

<style>
  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .banner {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-4);
    padding: var(--space-3) var(--space-4);
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    color: var(--text-secondary);
  }
</style>
