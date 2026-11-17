<script lang="ts">
  import { tick } from 'svelte';
  import { revealInvalid } from '$lib/scroll';
  import { ApiError, api, unwrap } from '$lib/api/client';
  import type { components } from '$lib/api/schema';
  import Icon from '$lib/ui/Icon.svelte';
  import Button from '$lib/ui/Button.svelte';
  import Checkbox from '$lib/ui/Checkbox.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import Input from '$lib/ui/Input.svelte';
  import Textarea from '$lib/ui/Textarea.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  const FIELDS = ['site', 'login_url', 'username', 'password', 'totp', 'notes'];

  type Credential = components['schemas']['Credential'];
  type Profile = components['schemas']['Profile'];

  let {
    open = $bindable(false),
    credential = null,
    profiles,
    onsaved,
  }: { open?: boolean; credential?: Credential | null; profiles: Profile[]; onsaved?: () => void } = $props();

  const editing = $derived(credential !== null);
  // Bitwarden logins keep their secrets in Bitwarden; only the site, selectors and links are edited here.
  const external = $derived(credential?.source === 'bitwarden');

  let site = $state('');
  let loginUrl = $state('');
  let username = $state('');
  let password = $state('');
  let totp = $state('');
  let removeTotp = $state(false);
  let notes = $state('');
  let selUsername = $state('');
  let selPassword = $state('');
  let selSubmit = $state('');
  let selTotp = $state('');
  let linked = $state<string[]>([]);
  let saving = $state(false);
  let errors = $state<Record<string, string>>({});

  function resetSecrets() {
    password = '';
    totp = '';
    notes = '';
  }

  $effect(() => {
    if (!open) return;
    site = credential?.site ?? '';
    loginUrl = credential?.login_url ?? '';
    username = credential?.username ?? '';
    resetSecrets();
    removeTotp = false;
    selUsername = credential?.selectors.username_selector ?? '';
    selPassword = credential?.selectors.password_selector ?? '';
    selSubmit = credential?.selectors.submit_selector ?? '';
    selTotp = credential?.selectors.totp_selector ?? '';
    linked = [...(credential?.profile_ids ?? [])];
    errors = {};
  });

  // A bare host is accepted; the daemon adds https://, this only tells the user.
  const siteHint = $derived(site.trim() && !site.includes('://') ? `Saved as https://${site.trim()}` : undefined);

  function toggleProfile(id: string, checked: boolean) {
    linked = checked ? [...linked, id] : linked.filter((p) => p !== id);
  }

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (saving) return;
    saving = true;
    errors = {};
    const selectors = {
      username_selector: selUsername || null,
      password_selector: selPassword || null,
      submit_selector: selSubmit || null,
      totp_selector: selTotp || null,
    };
    try {
      if (credential) {
        await unwrap(
          api.PATCH('/v1/credentials/{credential_id}', {
            params: { path: { credential_id: credential.id } },
            body: {
              site: site.trim(),
              username: username.trim(),
              login_url: loginUrl.trim() === '' ? null : loginUrl.trim(),
              selectors,
              profile_ids: linked,
              ...(password ? { password } : {}),
              ...(removeTotp ? { totp: null } : totp ? { totp } : {}),
              ...(notes ? { notes } : {}),
            },
          }),
        );
        toast.success('Login saved');
      } else {
        await unwrap(
          api.POST('/v1/credentials', {
            body: {
              site: site.trim(),
              username: username.trim(),
              password,
              login_url: loginUrl.trim() === '' ? null : loginUrl.trim(),
              selectors,
              profile_ids: linked,
              ...(totp ? { totp } : {}),
              ...(notes ? { notes } : {}),
            },
          }),
        );
        toast.success('Login added');
      }
      resetSecrets();
      open = false;
      onsaved?.();
    } catch (e) {
      const field = e instanceof ApiError && e.code === 'InvalidArgument' ? (e.details as { field?: string }).field : undefined;
      if (field && FIELDS.includes(field)) {
        errors = { [field]: (e as ApiError).message };
        await tick();
        revealInvalid(document.getElementById('credential-form') ?? document);
      } else toast.apiError(e, "Couldn't save the login");
    } finally {
      saving = false;
    }
  }
</script>

<Dialog bind:open title={editing ? 'Edit login' : 'Add login'} size="lg" onclose={resetSecrets}>
  <form id="credential-form" onsubmit={save}>
    <div class="fields">
      <Input label="Site" bind:value={site} required placeholder="github.com" hint={siteHint} error={errors.site} />
      <Input label="Login page (optional)" bind:value={loginUrl} placeholder={site || 'Same as the site'} error={errors.login_url} />
      <Input label="Username" bind:value={username} required autocomplete="off" error={errors.username} />
      {#if !external}
        <Input
          label="Password"
          type="password"
          bind:value={password}
          required={!editing}
          autocomplete="new-password"
          placeholder={editing ? 'Leave empty to keep the current password' : undefined}
          error={errors.password}
        />
        <Input
          label="One-time code secret (optional)"
          type="password"
          bind:value={totp}
          autocomplete="off"
          hint="The setup key shown when you enable two-factor sign-in, or its otpauth:// link."
          error={errors.totp}
          placeholder={editing && credential?.totp_set ? 'Leave empty to keep' : undefined}
        />
        {#if editing && credential?.totp_set}
          <Checkbox label="Remove the one-time code secret" bind:checked={removeTotp} />
        {/if}
      {/if}

      <details>
        <summary><span class="chevron"><Icon name="chevron-right" /></span>Custom form fields (optional)</summary>
        <div class="fields selectors">
          <Input label="Username selector" bind:value={selUsername} />
          <Input label="Password selector" bind:value={selPassword} />
          <Input label="Submit selector" bind:value={selSubmit} />
          <Input label="TOTP selector" bind:value={selTotp} />
        </div>
      </details>

      {#if !external}
        <Textarea label="Notes (optional)" bind:value={notes} rows={3} placeholder={editing ? 'Leave empty to keep the current notes' : undefined} />
      {/if}

      <div class="field">
        <span class="field-label">Use in profiles (optional)</span>
        <div class="profiles">
          {#each profiles as profile (profile.id)}
            <Checkbox label={profile.name} checked={linked.includes(profile.id)} onchange={(checked) => toggleProfile(profile.id, checked)} />
          {/each}
          {#if profiles.length === 0}<p class="field-hint">No profiles yet.</p>{/if}
        </div>
      </div>
    </div>
  </form>
  {#snippet footer()}
    <Button onclick={() => (open = false)} disabled={saving}>Cancel</Button>
    <Button
      variant="primary"
      type="submit"
      form="credential-form"
      loading={saving}
      disabled={!site.trim() || !username.trim() || (!editing && !password)}
    >
      {editing ? 'Save login' : 'Add login'}
    </Button>
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

  .profiles {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    max-height: 200px;
    overflow: auto;
    width: 100%;
  }

  details {
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }

  summary {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: var(--control-h-md);
    padding: 0 var(--space-3);
    color: var(--text-secondary);
    font-weight: 500;
    list-style: none;
    cursor: pointer;
  }

  summary::-webkit-details-marker {
    display: none;
  }

  summary:hover {
    color: var(--text);
  }

  .chevron {
    display: flex;
    transition: transform var(--duration-fast) var(--ease-standard);
  }

  details[open] .chevron {
    transform: rotate(90deg);
  }

  .selectors {
    padding: var(--space-3) var(--space-3) var(--space-4);
    border-top: 1px solid var(--border);
  }
</style>
