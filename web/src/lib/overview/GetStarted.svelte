<script lang="ts">
  import { settingsHref } from '$lib/settings/sections';
  import Button from '$lib/ui/Button.svelte';
  import Card from '$lib/ui/Card.svelte';
  import Icon from '$lib/ui/Icon.svelte';
  import type { OverviewData } from './data.svelte';

  const DISMISSED_KEY = 'brr.overview.get-started-dismissed';

  let { data, onlaunch }: { data: OverviewData; onlaunch: () => void } = $props();

  const AGENT_KEY = 'brr.overview.agent-seen';

  let dismissed = $state(localStorage.getItem(DISMISSED_KEY) === '1');

  // An agent that disconnected was still connected once; keep the step done.
  let agentSeen = $state(localStorage.getItem(AGENT_KEY) === '1');
  $effect(() => {
    if (!data.hasAgent() || agentSeen) return;
    agentSeen = true;
    localStorage.setItem(AGENT_KEY, '1');
  });

  const loaded = $derived(data.builds.data && data.profiles.data && data.firstRun.data !== undefined && data.clients.data);
  const steps = $derived([
    {
      label: 'Add a browser version',
      done: data.builds.data?.some((b) => ['installed', 'registered'].includes(b.state)) ?? false,
      action: { label: 'Open versions', href: '/versions' },
    },
    {
      label: 'Create a profile',
      done: data.profiles.data?.some((p) => !p.builtin) ?? false,
      action: { label: 'Open profiles', href: '/profiles' },
    },
    { label: 'Launch your first browser', done: data.firstRun.data ?? false, action: { label: 'Launch', onclick: onlaunch } },
    {
      label: 'Connect an agent',
      done: agentSeen || data.hasAgent(),
      action: { label: 'Connect', href: settingsHref('agents') },
    },
  ]);
  const complete = $derived(steps.every((s) => s.done));

  function dismiss() {
    dismissed = true;
    localStorage.setItem(DISMISSED_KEY, '1');
  }
</script>

{#if loaded && !complete && !dismissed}
  <Card title="Get started" description="brr runs real browsers for your AI agents and lets you watch and step in.">
    {#snippet actions()}<Button variant="ghost" size="sm" onclick={dismiss}>Dismiss</Button>{/snippet}
    <ol class="steps">
      {#each steps as step (step.label)}
        <li class:done={step.done}>
          <span class="mark" aria-hidden="true">{#if step.done}<Icon name="check" size={16} />{/if}</span>
          <span class="label">{step.label}{#if step.done}<span class="sr-only"> (done)</span>{/if}</span>
          {#if !step.done}
            {#if 'href' in step.action}
              <Button size="sm" href={step.action.href}>{step.action.label}</Button>
            {:else}
              <Button size="sm" onclick={step.action.onclick}>{step.action.label}</Button>
            {/if}
          {/if}
        </li>
      {/each}
    </ol>
  </Card>
{/if}

<style>
  .steps {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--space-3);
    min-height: 32px;
  }

  .mark {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-full);
  }

  .done .mark {
    background: var(--success-bg);
    border-color: var(--success-border);
    color: var(--success-fg);
  }

  .done .label {
    color: var(--text-secondary);
    text-decoration: line-through;
  }
</style>
