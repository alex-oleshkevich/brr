<script lang="ts">
  import { untrack } from 'svelte';
  import { ApiError, api, unwrap } from '$lib/api/client';
  import type { components } from '$lib/api/schema';
  import Button from '$lib/ui/Button.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import Input from '$lib/ui/Input.svelte';
  import SegmentedControl from '$lib/ui/SegmentedControl.svelte';
  import Select from '$lib/ui/Select.svelte';
  import Switch from '$lib/ui/Switch.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  type LoginCheck = components['schemas']['LoginCheck'];
  type Credential = components['schemas']['Credential'];

  let {
    open = $bindable(false),
    profileId,
    check,
    credentials,
    onsaved,
  }: { open?: boolean; profileId: string; check?: LoginCheck; credentials: Credential[]; onsaved: () => void } = $props();

  let site = $state('');
  let url = $state('');
  let proof = $state<'element' | 'text'>('element');
  let expected = $state('');
  let credentialId = $state('');
  let autoRelogin = $state(false);
  let saving = $state(false);
  let errors = $state<Record<string, string>>({});

  $effect(() => {
    if (!open) return;
    untrack(() => {
      site = check?.site ?? '';
      url = check?.url ?? '';
      proof = check?.expect_text ? 'text' : 'element';
      expected = (check?.expect_text ? check.expect_text : check?.expect_selector) ?? '';
      credentialId = check?.credential_id ?? '';
      autoRelogin = check?.auto_relogin ?? false;
      errors = {};
    });
  });

  async function save(event: SubmitEvent) {
    event.preventDefault();
    errors = {};
    if (!site.trim()) errors.site = 'Enter the site.';
    if (!url.trim()) errors.url = 'Enter the URL to check.';
    if (!expected.trim()) errors.expected = proof === 'element' ? 'Enter a CSS selector.' : 'Enter the text to look for.';
    if (Object.keys(errors).length) return;

    saving = true;
    const body = {
      site: site.trim(),
      url: url.trim(),
      expect_selector: proof === 'element' ? expected.trim() : null,
      expect_text: proof === 'text' ? expected.trim() : null,
      credential_id: credentialId || null,
      auto_relogin: autoRelogin,
    };
    try {
      if (check) await unwrap(api.PATCH('/v1/login-checks/{login_check_id}', { params: { path: { login_check_id: check.id } }, body }));
      else await unwrap(api.POST('/v1/profiles/{profile}/login-checks', { params: { path: { profile: profileId } }, body }));
      toast.success(check ? 'Login check saved' : 'Login check added');
      onsaved();
      open = false;
    } catch (e) {
      const field = e instanceof ApiError && e.code === 'InvalidArgument' ? (e.details as { field?: string }).field : undefined;
      if (field && ['site', 'url'].includes(field)) errors = { [field]: (e as ApiError).message };
      else if (field && field.startsWith('expect_')) errors = { expected: (e as ApiError).message };
      else toast.apiError(e, "Couldn't save the login check");
    } finally {
      saving = false;
    }
  }
</script>

<Dialog bind:open title={check ? 'Edit login check' : 'Add login check'} dismissible={!saving}>
  <form id="login-check-form" onsubmit={save} novalidate>
    <div class="fields">
      <Input label="Site" bind:value={site} required error={errors.site} placeholder="https://example.com" autocomplete="off" />
      <Input label="URL" bind:value={url} required error={errors.url} placeholder="https://example.com/account" autocomplete="off" />
      <div class="field">
        <span class="field-label">Prove login by</span>
        <SegmentedControl
          label="Prove login by"
          bind:value={proof}
          options={[
            { value: 'element', label: 'Element' },
            { value: 'text', label: 'Text' },
          ]}
        />
      </div>
      <Input
        label={proof === 'element' ? 'CSS selector' : 'Text on the page'}
        bind:value={expected}
        required
        error={errors.expected}
        autocomplete="off"
      />
      <Select
        label="Credential"
        bind:value={credentialId}
        options={[{ value: '', label: 'None' }, ...credentials.map((c) => ({ value: c.id, label: `${c.site} · ${c.username}` }))]}
      />
      <Switch label="Re-login automatically when the check fails" bind:checked={autoRelogin} />
    </div>
  </form>
  {#snippet footer()}
    <Button onclick={() => (open = false)} disabled={saving}>Cancel</Button>
    <Button variant="primary" type="submit" form="login-check-form" loading={saving}>Save</Button>
  {/snippet}
</Dialog>

<style>
  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    align-items: flex-start;
  }
</style>
