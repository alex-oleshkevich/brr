<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import { buildLabel } from '$lib/builds/labels';
  import { clock } from '$lib/clock.svelte';
  import { absolute, duration, relative } from '$lib/format';
  import { canRelaunch, outcome, profileLabel } from '$lib/instances/actions';
  import LaunchDialog from '$lib/instances/LaunchDialog.svelte';
  import RunActivity from '$lib/runs/RunActivity.svelte';
  import RunCaptcha from '$lib/runs/RunCaptcha.svelte';
  import RunFiles from '$lib/runs/RunFiles.svelte';
  import RunLogins from '$lib/runs/RunLogins.svelte';
  import RunNetwork from '$lib/runs/RunNetwork.svelte';
  import RunSettings from '$lib/runs/RunSettings.svelte';
  import RunSummary from '$lib/runs/RunSummary.svelte';
  import RunTemplate from '$lib/runs/RunTemplate.svelte';
  import { isFinal, runResult } from '$lib/runs/runs';
  import Badge from '$lib/ui/Badge.svelte';
  import Button from '$lib/ui/Button.svelte';
  import Card from '$lib/ui/Card.svelte';
  import DetailLayout from '$lib/ui/DetailLayout.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';

  const FULL_TABS = [
    { id: 'summary', label: 'Summary' },
    { id: 'settings', label: 'Settings' },
    { id: 'template', label: 'Template & extensions' },
    { id: 'network', label: 'Network' },
    { id: 'logins', label: 'Logins' },
    { id: 'captcha', label: 'Captcha' },
    { id: 'files', label: 'Files' },
    { id: 'activity', label: 'Activity' },
  ];
  const ATTACHED_TABS = FULL_TABS.filter((t) => ['summary', 'network', 'files', 'activity'].includes(t.id));

  const instanceId = $derived(page.params.id ?? '');
  const instance = liveQuery(() => unwrap(api.GET('/v1/instances/{instance_id}', { params: { path: { instance_id: instanceId } } })), {
    kinds: ['instance.state_changed'],
    match: (e) => (e.data as { instance_id?: string } | null)?.instance_id === instanceId || (e.subject.type === 'instance' && e.subject.id === instanceId),
  });

  const run = $derived(instance.data);
  const failed = $derived(run ? outcome(run) : null);
  const tabs = $derived(run ? (run.kind === 'live' ? ATTACHED_TABS : FULL_TABS) : undefined);
  const meta = $derived(
    run
      ? [
          run.build ? buildLabel(run.build) : null,
          run.started_at ? `started ${relative(run.started_at, clock.now)}` : null,
          run.started_at && run.ended_at ? `ran ${duration(run.ended_at - run.started_at)}` : null,
        ]
          .filter(Boolean)
          .join(' · ')
      : '',
  );

  let tab = $state<string>();
  let relaunching = $state(false);

  // A run that is still going lives on the Browsers page.
  $effect(() => {
    if (run && !isFinal(run)) void goto(`/browsers/${run.id}`, { replaceState: true });
  });
</script>

{#if instance.loading}
  <DetailLayout title="Run" back={{ href: '/history', label: 'History' }}>
    <div class="skeletons" aria-busy="true">
      {#each { length: 5 } as _, i (i)}<Skeleton width="{40 + ((i * 17) % 40)}%" />{/each}
    </div>
  </DetailLayout>
{:else if instance.error && !run}
  {#if instance.error.code === 'NotFound'}
    <EmptyState level={1} title="Run not found" description="It may have been removed after its retention period.">
      {#snippet action()}<Button href="/history">Back to history</Button>{/snippet}
    </EmptyState>
  {:else}
    <EmptyState tone="danger" title="Couldn't load this run" description={instance.error.message}>
      {#snippet action()}<Button onclick={() => instance.reload()}>Retry</Button>{/snippet}
    </EmptyState>
  {/if}
{:else if run}
  <DetailLayout title={run.kind === 'live' ? 'Attached browser' : profileLabel(run)} description={meta} back={{ href: '/history', label: 'History' }} {tabs} bind:tab>
    {#snippet actions()}
      {#if failed}<Badge dot tone={failed.tone}>{failed.label}</Badge>{:else}<Badge>{runResult(run)}</Badge>{/if}
      {#if canRelaunch(run)}<Button variant="primary" onclick={() => (relaunching = true)}>Launch again</Button>{/if}
    {/snippet}

    {#if tab === 'settings'}
      <RunSettings {run} />
    {:else if tab === 'template'}
      <RunTemplate {run} />
    {:else if tab === 'network'}
      <RunNetwork {run} />
    {:else if tab === 'logins'}
      <RunLogins {run} />
    {:else if tab === 'captcha'}
      <RunCaptcha {run} />
    {:else if tab === 'files'}
      <RunFiles {run} />
    {:else if tab === 'activity'}
      <RunActivity {run} />
    {:else}
      <Card title="Summary" description={run.ended_at ? `Ended ${absolute(run.ended_at)}` : undefined}>
        <RunSummary {run} />
      </Card>
    {/if}
  </DetailLayout>
  {#if run.profile_id}<LaunchDialog bind:open={relaunching} profileId={run.profile_id} />{/if}
{/if}

<style>
  .skeletons {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
</style>
