<script lang="ts">
  import type { Component } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { confirm } from '$lib/ui/confirm.svelte';
  import Button from '$lib/ui/Button.svelte';
  import Agents from './sections/Agents.svelte';
  import Capacity from './sections/Capacity.svelte';
  import Captcha from './sections/Captcha.svelte';
  import General from './sections/General.svelte';
  import Hosts from './sections/Hosts.svelte';
  import Logins from './sections/Logins.svelte';
  import Proxies from './sections/Proxies.svelte';
  import Resources from './sections/Resources.svelte';
  import Sandbox from './sections/Sandbox.svelte';
  import Storage from './sections/Storage.svelte';
  import Updates from './sections/Updates.svelte';
  import { settingsHref, settingsParam, settingsSections, type SettingsSectionId } from './sections';

  type SectionProps = { dirty?: boolean };
  const bodies: Record<SettingsSectionId, Component<SectionProps>> = {
    general: General,
    proxies: Proxies,
    captcha: Captcha,
    resources: Resources,
    logins: Logins,
    sandbox: Sandbox,
    hosts: Hosts,
    capacity: Capacity,
    storage: Storage,
    agents: Agents,
    updates: Updates,
  };

  let dialog: HTMLDialogElement;
  let dirty = $state(false);

  const current = $derived(settingsSections.find((s) => s.id === page.url.searchParams.get(settingsParam)));
  const Body = $derived(current ? bodies[current.id] : undefined);

  $effect(() => {
    if (current && !dialog.open) {
      dialog.showModal();
      dialog.focus();
    } else if (!current && dialog.open) dialog.close();
    if (!current) dirty = false;
  });

  $effect(() => {
    const requested = page.url.searchParams.get(settingsParam);
    if (requested !== null && !current) void navigate(null);
  });

  const navigate = (id: SettingsSectionId | null) => goto(settingsHref(id), { replaceState: true, keepFocus: true, noScroll: true });

  async function leave(next: SettingsSectionId | null) {
    if (dirty && current) {
      const ok = await confirm({
        title: 'Discard unsaved changes?',
        description: `${current.label} has unsaved changes. They will be lost.`,
        confirmLabel: 'Discard',
        tone: 'danger',
      });
      if (!ok) return;
    }
    dirty = false;
    await navigate(next);
  }

  const select = (id: SettingsSectionId) => id !== current?.id && leave(id);
</script>

<!-- Esc and focus handling are native to <dialog>; the click handler only adds backdrop dismissal. -->
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog
  bind:this={dialog}
  tabindex="-1"
  aria-labelledby="settings-title"
  oncancel={(e) => {
    e.preventDefault();
    void leave(null);
  }}
  onclick={(e) => e.target === dialog && void leave(null)}
>
  {#if current && Body}
    <nav aria-label="Settings sections">
      <h2 class="nav-title">Settings</h2>
      {#each settingsSections as section (section.id)}
        <button type="button" aria-current={section.id === current.id ? 'page' : undefined} onclick={() => select(section.id)}>{section.label}</button>
      {/each}
    </nav>

    <div class="pane">
      <header>
        <h2 id="settings-title" tabindex="-1">{current.label}</h2>
        <Button variant="ghost" size="sm" iconOnly icon="x" aria-label="Close settings" onclick={() => leave(null)} />
      </header>
      <div class="body">
        {#key current.id}
          <Body bind:dirty />
        {/key}
      </div>
    </div>
  {/if}
</dialog>

<style>
  dialog {
    width: min(1100px, 95vw);
    max-width: none;
    height: min(760px, 90vh);
    max-height: none;
    padding: 0;
    overflow: hidden;
    background: var(--surface-raised);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-xl);
    box-shadow: var(--shadow-lg);
    opacity: 0;
    transform: scale(0.98);
    transition:
      opacity var(--duration-slow) var(--ease-out),
      transform var(--duration-slow) var(--ease-out);
  }

  dialog:focus {
    outline: none;
  }

  dialog[open] {
    display: grid;
    grid-template-columns: 220px minmax(0, 1fr);
    opacity: 1;
    transform: none;
  }

  @starting-style {
    dialog[open] {
      opacity: 0;
      transform: scale(0.98);
    }
  }

  dialog::backdrop {
    background: var(--overlay);
  }

  nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--space-4) var(--space-3);
    overflow-y: auto;
    background: var(--bg-subtle);
    border-right: 1px solid var(--border);
  }

  .nav-title {
    padding: 0 var(--space-2) var(--space-2);
    font-size: var(--text-md-size);
    line-height: var(--text-md-line);
    letter-spacing: 0;
  }

  nav button {
    height: var(--control-h-md);
    padding: 0 var(--space-2);
    border: 0;
    border-radius: var(--radius-md);
    background: none;
    color: var(--text-secondary);
    font: inherit;
    font-weight: 500;
    text-align: left;
    cursor: pointer;
  }

  nav button:hover,
  nav button[aria-current='page'] {
    background: var(--bg-muted);
    color: var(--text);
  }

  .pane {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-4) var(--space-6);
    border-bottom: 1px solid var(--border);
  }

  header h2 {
    font-size: var(--text-lg-size);
    line-height: var(--text-lg-line);
    letter-spacing: 0;
  }

  header h2:focus {
    outline: none;
  }

  .body {
    flex: 1;
    min-height: 0;
    padding: var(--space-6) var(--space-6) var(--space-12);
    overflow-y: auto;
    overscroll-behavior: contain;
  }

  @media (max-width: 719px) {
    dialog[open] {
      grid-template-columns: minmax(0, 1fr);
      grid-template-rows: auto minmax(0, 1fr);
    }

    nav {
      flex-direction: row;
      padding: var(--space-2);
      overflow-x: auto;
      border-right: 0;
      border-bottom: 1px solid var(--border);
    }

    .nav-title {
      display: none;
    }

    nav button {
      flex: none;
    }
  }
</style>
