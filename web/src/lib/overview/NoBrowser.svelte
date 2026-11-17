<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import { events } from '$lib/api/events.svelte';
  import type { components } from '$lib/api/schema';
  import { detectBrowsers } from '$lib/builds/detect';
  import { bytes } from '$lib/format';
  import Button from '$lib/ui/Button.svelte';
  import Card from '$lib/ui/Card.svelte';
  import Progress from '$lib/ui/Progress.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  type Build = components['schemas']['BuildView'];

  let { builds, onchange }: { builds: Build[]; onchange: () => void } = $props();

  let detecting = $state(false);
  let installId = $state<string | null>(null);
  let notFound = $state(false);
  let installError = $state<string | null>(null);
  let progress = $state<{ bytes: number; total: number | null; extracting: boolean } | null>(null);

  const installing = $derived(builds.find((b) => b.state === 'installing'));
  const watchedId = $derived(installId ?? installing?.id ?? null);
  const showProgress = $derived(installId !== null || installing !== undefined);

  $effect(() => {
    const offProgress = events.on(['build.install_progress'], (e) => {
      const data = e.data as { build_id: string; bytes: number; total?: number | null; phase: 'downloading' | 'extracting' };
      if (data.build_id === watchedId) progress = { bytes: data.bytes, total: data.total ?? null, extracting: data.phase === 'extracting' };
    });
    const offFailed = events.on(['build.install_failed'], (e) => {
      const data = e.data as { build_id: string; error?: string };
      if (data.build_id !== watchedId) return;
      installError = data.error ?? 'The install failed.';
      installId = null;
      progress = null;
    });
    const offInstalled = events.on(['build.installed'], (e) => {
      if ((e.data as { build_id: string }).build_id === watchedId) installId = null;
    });
    return () => {
      offProgress();
      offFailed();
      offInstalled();
    };
  });

  async function detect() {
    detecting = true;
    notFound = false;
    const found = await detectBrowsers(builds);
    if (found) notFound = !found.some((b) => b.engine === 'chromium');
    onchange();
    detecting = false;
  }

  async function install() {
    installError = null;
    progress = null;
    try {
      installId = (await unwrap(api.POST('/v1/builds', { body: { source: 'cft', channel: 'stable' } }))).id;
      onchange();
    } catch (e) {
      toast.apiError(e, "Couldn't start the install");
    }
  }
</script>

<Card title="No browser yet" description="Use the Chrome already on this machine, or install Chrome for Testing.">
  {#if showProgress}
    <div class="install">
      <Progress label="Installing Chrome for Testing" size="md" value={progress?.total && !progress.extracting ? (progress.bytes / progress.total) * 100 : null} />
      <p class="note num">
        {#if progress?.extracting}Extracting…{:else if progress}{bytes(progress.bytes)}{progress.total ? ` of ${bytes(progress.total)}` : ''}{:else}Starting the download{/if}
      </p>
    </div>
  {:else}
    <div class="actions">
      <Button variant="primary" loading={detecting} onclick={detect}>Use system Chrome</Button>
      <Button onclick={install}>{installError ? 'Retry' : 'Install Chrome for Testing'}</Button>
    </div>
    {#if notFound}<p class="error">No system Chrome, Chromium, Brave or Edge found.</p>{/if}
    {#if installError}<p class="error">{installError}</p>{/if}
  {/if}
</Card>

<style>
  .actions {
    display: flex;
    gap: var(--space-2);
  }

  .install {
    max-width: 320px;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .note {
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }

  .error {
    margin-top: var(--space-3);
    color: var(--danger-fg);
  }
</style>
