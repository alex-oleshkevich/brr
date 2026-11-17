<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import { channelsFor, INSTALLABLE_SOURCES, SOURCE_NAMES, type BuildSource } from '$lib/builds/labels';
  import Dialog from '$lib/ui/Dialog.svelte';
  import SegmentedControl from '$lib/ui/SegmentedControl.svelte';
  import Select from '$lib/ui/Select.svelte';
  import Button from '$lib/ui/Button.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  let { open = $bindable(false), oninstalled }: { open?: boolean; oninstalled?: () => void } = $props();

  let source = $state<BuildSource>('cft');
  let mode = $state('channel');
  let channel = $state('stable');
  let version = $state('');
  let versions = $state<string[]>([]);
  let installing = $state(false);

  let loading = $state(false);
  let loadError = $state('');
  let latestRequest = 0;

  async function loadVersions() {
    const request = ++latestRequest;
    versions = [];
    version = '';
    loadError = '';
    loading = true;
    try {
      const out = await unwrap(api.GET('/v1/builds/available', { params: { query: { source } } }));
      if (request !== latestRequest) return;
      versions = [...new Set(out.available.map((a) => a.version))];
      version = versions[0] ?? '';
    } catch (e) {
      if (request === latestRequest) loadError = e instanceof Error ? e.message : String(e);
    } finally {
      if (request === latestRequest) loading = false;
    }
  }

  $effect(() => {
    if (!open) return;
    channel = channelsFor(source)[0].value;
    if (mode === 'version') void loadVersions();
  });

  async function install(event: SubmitEvent) {
    event.preventDefault();
    if (installing) return;
    installing = true;
    try {
      await unwrap(
        api.POST('/v1/builds', { body: mode === 'channel' ? { source, channel } : { source, version } }),
      );
      open = false;
      oninstalled?.();
      toast.success(`Installing ${SOURCE_NAMES[source]}`);
    } catch (e) {
      toast.apiError(e, "Couldn't start the install");
    } finally {
      installing = false;
    }
  }
</script>

<Dialog bind:open title="Install a browser version" submit={{ label: 'Install browser', form: 'install-form', loading: installing, disabled: mode === 'version' && !version }}>
  <form id="install-form" onsubmit={install}>
    <div class="fields">
      <Select
        label="Source"
        bind:value={source}
        options={INSTALLABLE_SOURCES.map((s) => ({ value: s, label: SOURCE_NAMES[s] }))}
      />
      <div class="field">
        <span class="field-label">Version</span>
        <SegmentedControl
          label="Version"
          bind:value={mode}
          options={[
            { value: 'channel', label: 'Channel' },
            { value: 'version', label: 'Exact version' },
          ]}
        />
      </div>
      {#if mode === 'channel'}
        <Select label="Channel" bind:value={channel} options={channelsFor(source)} />
      {:else}
        <Select label="Version" bind:value={version} options={versions.map((v) => ({ value: v, label: v }))} hint={!loading && !loadError && versions.length === 0 ? 'No versions available for this source.' : undefined} />
        {#if loadError}
          <div class="load-error" role="alert">
            <span>Couldn't load the available versions: {loadError}</span>
            <Button size="sm" onclick={() => void loadVersions()}>Retry</Button>
          </div>
        {/if}
      {/if}
    </div>
  </form>
</Dialog>

<style>
  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .load-error {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    color: var(--danger-fg);
    font-size: var(--text-sm-size);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    align-items: flex-start;
  }
</style>
