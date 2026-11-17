<script lang="ts">
  import Button from '$lib/ui/Button.svelte';
  import Disclosure from '$lib/ui/Disclosure.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Input from '$lib/ui/Input.svelte';
  import SaveBar from '$lib/ui/SaveBar.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import Textarea from '$lib/ui/Textarea.svelte';
  import { SettingsForm } from '../form.svelte';

  let { dirty = $bindable(false) }: { dirty?: boolean } = $props();

  const LIST_KEY = 'content.adblock_lists';
  const URL_KEYS = ['builds.github_api', 'builds.edge_repo', 'extensions.cws_update_url', 'extensions.amo_api'];

  const isHttpUrl = (text: string) => {
    try {
      return ['http:', 'https:'].includes(new URL(text).protocol);
    } catch {
      return false;
    }
  };

  function validate(key: string, value: string | number | null): string | null {
    const text = String(value ?? '').trim();
    if (key === LIST_KEY) {
      const bad = text.split('\n').map((line) => line.trim()).filter((line) => line && !isHttpUrl(line));
      return bad.length ? `Each line must be an http:// or https:// URL. Check: ${bad[0]}` : null;
    }
    if (URL_KEYS.includes(key)) return isHttpUrl(text) ? null : 'Enter an http:// or https:// URL.';
    return null;
  }

  const form = new SettingsForm(['builds.check_at', LIST_KEY, ...URL_KEYS], [LIST_KEY], validate);

  $effect(() => {
    dirty = form.dirty;
  });
</script>

{#if form.loadError}
  <EmptyState tone="danger" title="Couldn't load settings" description={form.loadError.message}>
    {#snippet action()}<Button onclick={form.load}>Retry</Button>{/snippet}
  </EmptyState>
{:else if form.loading && form.settings === undefined}
  <div aria-busy="true" class="section"><Skeleton height="56px" /><Skeleton height="120px" /></div>
{:else}
  <div class="section">
    <Input name="builds.check_at" label="Check for new browser versions at" type="time" bind:value={form.values['builds.check_at']} readonly={form.isEnv('builds.check_at')} badge={form.badge('builds.check_at')} hint={form.hint('builds.check_at', 'Local time of the daily check.')} error={form.error('builds.check_at')} />
    <Textarea name="content.adblock_lists" label="Ad-block lists" rows={4} bind:value={() => String(form.values['content.adblock_lists']), (v) => (form.values['content.adblock_lists'] = v)} readonly={form.isEnv('content.adblock_lists')} hint={form.hint('content.adblock_lists', 'One list URL per line. Profiles with ad blocking use these.')} error={form.error('content.adblock_lists')} />
    <Disclosure summary="Advanced">
      <Input name="builds.github_api" label="GitHub API URL" type="url" bind:value={form.values['builds.github_api']} readonly={form.isEnv('builds.github_api')} badge={form.badge('builds.github_api')} error={form.error('builds.github_api')} />
      <Input name="builds.edge_repo" label="Edge package repository" type="url" bind:value={form.values['builds.edge_repo']} readonly={form.isEnv('builds.edge_repo')} badge={form.badge('builds.edge_repo')} error={form.error('builds.edge_repo')} />
      <Input name="extensions.cws_update_url" label="Chrome Web Store update URL" type="url" bind:value={form.values['extensions.cws_update_url']} readonly={form.isEnv('extensions.cws_update_url')} badge={form.badge('extensions.cws_update_url')} error={form.error('extensions.cws_update_url')} />
      <Input name="extensions.amo_api" label="Firefox add-ons API URL" type="url" bind:value={form.values['extensions.amo_api']} readonly={form.isEnv('extensions.amo_api')} badge={form.badge('extensions.amo_api')} error={form.error('extensions.amo_api')} />
    </Disclosure>
    <SaveBar dirty={form.dirty} saving={form.saving} disabled={form.invalid} onsave={form.save} ondiscard={form.discard} />
  </div>
{/if}

<style>
  .section {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }
</style>
