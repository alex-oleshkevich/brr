<script lang="ts">
  import { untrack } from 'svelte';
  import { ApiError, api, unwrap } from '$lib/api/client';
  import Button from '$lib/ui/Button.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import Input from '$lib/ui/Input.svelte';
  import Select from '$lib/ui/Select.svelte';
  import { toast } from '$lib/ui/toast.svelte';
  import { setHint } from '$lib/format';
  import { parseProxyUrl, type Proxy } from './format';

  let {
    open = $bindable(false),
    proxy,
    onsaved,
  }: { open?: boolean; proxy?: Proxy; onsaved: (saved: Proxy) => void } = $props();

  const schemes = [
    { value: 'http', label: 'HTTP' },
    { value: 'https', label: 'HTTPS' },
    { value: 'socks5', label: 'SOCKS5' },
  ];

  let scheme = $state('http');
  let host = $state('');
  let port = $state<number | null>(null);
  let username = $state('');
  let password = $state('');
  let errors = $state<Record<string, string>>({});
  let saving = $state(false);

  // Secrets never outlive the dialog: the password is cleared on open, on close and after each submit.
  $effect(() => {
    password = '';
    errors = {};
    if (!open) return;

    // Live health updates replace the proxy object while the dialog is open; they must not reset the form.
    untrack(() => {
      scheme = proxy?.scheme ?? 'http';
      host = proxy?.host ?? '';
      port = proxy?.port ?? null;
      username = proxy?.username ?? '';
    });
  });

  // Pasting a whole proxy URL into Host fills every field.
  $effect(() => {
    const parsed = parseProxyUrl(host);
    if (!parsed) return;
    scheme = parsed.scheme;
    host = parsed.host;
    port = parsed.port;
    username = parsed.username;
    password = parsed.password;
  });

  async function save(event: SubmitEvent) {
    event.preventDefault();
    errors = {};
    if (!host.trim()) errors.host = 'Enter a host.';
    if (port === null || !Number.isInteger(port) || port < 1 || port > 65535) errors.port = 'Enter a port from 1 to 65535.';
    if (Object.keys(errors).length) return;

    saving = true;
    const typed = password;
    try {
      const common = { scheme: scheme as 'http' | 'https' | 'socks5', host: host.trim(), port: port as number };
      const saved = proxy
        ? await unwrap(
            api.PATCH('/v1/proxies/{proxy_id}', {
              params: { path: { proxy_id: proxy.id } },
              // An empty field keeps the stored password; sending "" would wipe it.
              // The daemon clears the login only for ""; null means no change.
              body: { ...common, username: username.trim(), ...(typed && { password: typed }) },
            }),
          )
        : await unwrap(api.POST('/v1/proxies', { body: { ...common, username: username.trim() || null, password: typed || null } }));
      toast.success(proxy ? 'Proxy saved' : 'Proxy added');
      onsaved(saved);
      open = false;
    } catch (e) {
      const field = e instanceof ApiError && e.code === 'InvalidArgument' ? (e.details as { field?: string }).field : undefined;
      if (field && ['host', 'port', 'username', 'password', 'scheme'].includes(field)) errors = { [field]: (e as ApiError).message };
      else toast.apiError(e, "Couldn't save the proxy");
    } finally {
      password = '';
      saving = false;
    }
  }
</script>

<Dialog bind:open title={proxy ? 'Edit proxy' : 'Add proxy'} dismissible={!saving}>
  <form id="proxy-form" onsubmit={save} novalidate>
    <div class="fields">
      <Select label="Scheme" bind:value={scheme} options={schemes} error={errors.scheme} />
      <div class="row">
        <Input label="Host" bind:value={host} required placeholder="proxy.example.com" hint="Or paste a full URL like http://user:pass@host:3128" error={errors.host} autocomplete="off" />
        <Input label="Port" type="number" min={1} max={65535} step={1} bind:value={port} required error={errors.port} />
      </div>
      <Input label="Username (optional)" bind:value={username} error={errors.username} autocomplete="off" />
      <Input
        label="Password (optional)"
        type="password"
        bind:value={password}
        autocomplete="new-password"
        placeholder={proxy?.password?.set ? 'Leave empty to keep the current password' : undefined}
        hint={proxy?.password?.set ? setHint(proxy.password.hint) : undefined}
        error={errors.password}
      />
    </div>
  </form>
  {#snippet footer()}
    <Button onclick={() => (open = false)} disabled={saving}>Cancel</Button>
    <Button variant="primary" type="submit" form="proxy-form" loading={saving}>{proxy ? 'Save proxy' : 'Add proxy'}</Button>
  {/snippet}
</Dialog>

<style>
  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 120px;
    gap: var(--space-3);
  }
</style>
