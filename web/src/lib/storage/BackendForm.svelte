<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { ApiError, api, unwrap } from '$lib/api/client';
  import type { components, paths } from '$lib/api/schema';
  import Button from '$lib/ui/Button.svelte';
  import Input from '$lib/ui/Input.svelte';
  import KeyValue from '$lib/ui/KeyValue.svelte';
  import SegmentedControl from '$lib/ui/SegmentedControl.svelte';
  import StatusDot from '$lib/ui/StatusDot.svelte';
  import Switch from '$lib/ui/Switch.svelte';
  import { setHint } from '$lib/format';
  import { toast } from '$lib/ui/toast.svelte';

  type Backend = components['schemas']['BackendOutput'];
  type Body = paths['/v1/storage/backend']['put']['requestBody']['content']['application/json'];

  let { backend, onsaved, dirty = $bindable(false) }: { backend: Backend; onsaved: () => void; dirty?: boolean } = $props();

  const kinds = [
    { value: 'local', label: 'Local disk' },
    { value: 's3', label: 'S3-compatible' },
    { value: 'r2', label: 'Cloudflare R2' },
  ];

  const FIELDS = ['endpoint', 'region', 'bucket', 'access_key_id', 'secret_access_key', 'public_endpoint', 'r2_account_id'];

  type Form = {
    kind: string;
    endpoint: string;
    region: string;
    bucket: string;
    access_key_id: string;
    path_style: boolean;
    public_endpoint: string;
    account_id: string;
  };

  const fromBackend = (b: Backend): Form => ({
    kind: b.backend,
    endpoint: b.endpoint ?? '',
    region: b.region ?? '',
    bucket: b.bucket ?? '',
    access_key_id: b.access_key_id ?? '',
    path_style: b.path_style,
    public_endpoint: b.public_endpoint ?? '',
    account_id: b.r2_account_id ?? '',
  });

  let form = $state<Form>(untrack(() => fromBackend(backend)));
  let baseline = $state(untrack(() => JSON.stringify(form)));
  // Typed only when the user replaces the key: the masked hint must never travel back to the server.
  let secret = $state('');
  let replacing = $state(false);
  let testing = $state(false);
  let saving = $state(false);
  let result = $state<{ ok: true; ms: number } | { ok: false; message: string } | null>(null);
  let errors = $state<Record<string, string>>({});

  const secretSet = $derived(backend.secret_access_key?.set === true);
  const isDirty = $derived(JSON.stringify(form) !== baseline || secret !== '');
  $effect(() => {
    dirty = isDirty;
  });
  const r2Endpoint = $derived(form.account_id.trim() ? `https://${form.account_id.trim()}.r2.cloudflarestorage.com` : null);

  $effect(() => {
    const next = backend;
    untrack(() => {
      if (JSON.stringify(form) !== baseline || secret !== '') return;
      form = fromBackend(next);
      baseline = JSON.stringify(form);
    });
  });

  // Bucket subdomains don't resolve on a bare IP, so path-style is the only addressing that works there.
  const ipEndpoint = $derived(isIpEndpoint(form.endpoint));
  let wasIp = untrack(() => ipEndpoint);

  $effect(() => {
    const isIp = ipEndpoint;
    untrack(() => {
      if (isIp && !wasIp) form.path_style = true;
      wasIp = isIp;
    });
  });

  // A result or server error describes the values that produced it; editing anything clears it.
  $effect(() => {
    void JSON.stringify(form);
    void secret;
    untrack(() => {
      result = null;
      errors = {};
    });
  });

  function isIpEndpoint(url: string): boolean {
    try {
      const { hostname } = new URL(url.trim());
      return /^\d{1,3}(\.\d{1,3}){3}$/.test(hostname) || hostname.startsWith('[');
    } catch {
      return false;
    }
  }

  async function replace() {
    replacing = true;
    await tick();
    document.getElementById('storage-secret')?.focus();
  }

  function body(): Body {
    const f = form;
    const b: Body = { backend: f.kind as 'local' | 's3' | 'r2' };
    if (f.kind === 's3') {
      b.endpoint = f.endpoint.trim();
      b.region = f.region.trim();
      b.bucket = f.bucket.trim();
      b.access_key_id = f.access_key_id.trim();
      b.path_style = f.path_style;
      b.public_endpoint = f.public_endpoint.trim() || null;
    } else if (f.kind === 'r2') {
      b.r2_account_id = f.account_id.trim();
      b.bucket = f.bucket.trim();
      b.access_key_id = f.access_key_id.trim();
    }
    if (f.kind !== 'local' && secret) b.secret_access_key = secret;
    return b;
  }

  function fail(e: unknown, fallback: string) {
    const field = e instanceof ApiError && e.code === 'InvalidArgument' ? (e.details as { field?: string }).field : undefined;
    if (field && FIELDS.includes(field)) errors = { [field]: (e as ApiError).message };
    else if (e instanceof ApiError && e.code === 'StorageUnavailable') result = { ok: false, message: e.message.replace(/^storage backend unavailable: /, '') };
    else toast.apiError(e, fallback);
  }

  async function test() {
    testing = true;
    result = null;
    try {
      const out = await unwrap(api.POST('/v1/storage/backend/test', { body: body() }));
      result = { ok: true, ms: out.elapsed_ms };
    } catch (e) {
      fail(e, "Couldn't test the connection");
    } finally {
      testing = false;
    }
  }

  async function save() {
    saving = true;
    try {
      await unwrap(api.PUT('/v1/storage/backend', { body: body() }));
      toast.success('Storage backend saved');
      secret = '';
      replacing = false;
      baseline = JSON.stringify(form);
      onsaved();
    } catch (e) {
      fail(e, "Couldn't save the storage backend");
    } finally {
      saving = false;
    }
  }
</script>

{#snippet secretField()}
  {#if secretSet && !replacing}
    <div class="field">
      <span class="field-label" id="secret-label">Secret access key</span>
      <div class="secret" role="group" aria-labelledby="secret-label">
        <span>{setHint(backend.secret_access_key?.hint)}</span>
        <Button variant="ghost" size="sm" onclick={replace}>Replace</Button>
      </div>
    </div>
  {:else}
    <Input id="storage-secret" label="Secret access key" type="password" autocomplete="new-password" bind:value={secret} error={errors.secret_access_key} />
  {/if}
{/snippet}

<div class="form">
  <SegmentedControl name="storage.backend" label="Backend" bind:value={form.kind} options={kinds} />

  {#if form.kind === 'local'}
    <div class="local">
      <KeyValue items={[{ label: 'Location', value: backend.local_root, mono: true }]} />
      <p class="note">Profile data always stays on local disk.</p>
    </div>
  {:else if form.kind === 's3'}
    <div class="grid">
      <div class="span"><Input name="storage.endpoint" label="Endpoint URL" type="url" placeholder="https://s3.example.com" bind:value={form.endpoint} error={errors.endpoint} autocomplete="off" /></div>
      <Input name="storage.region" label="Region" bind:value={form.region} error={errors.region} autocomplete="off" />
      <Input name="storage.bucket" label="Bucket" bind:value={form.bucket} error={errors.bucket} autocomplete="off" />
      <Input name="storage.access_key_id" label="Access key ID" bind:value={form.access_key_id} error={errors.access_key_id} autocomplete="off" />
      {@render secretField()}
      <div class="span"><Switch
          name="storage.path_style"
          label="Path-style requests"
          bind:checked={form.path_style}
          hint={ipEndpoint ? "An IP-address endpoint can only be reached path-style, so this is on." : "Address the bucket in the URL path, not as a subdomain. Needed by most self-hosted S3 servers."}
        /></div>
      <div class="span">
        <Input
          name="storage.public_endpoint"
          label="Public endpoint (optional)"
          type="url"
          bind:value={form.public_endpoint}
          error={errors.public_endpoint}
          hint="Used only to sign download URLs, e.g. a CDN or the R2 public host"
          autocomplete="off"
        />
      </div>
    </div>
  {:else}
    <div class="grid">
      <Input name="storage.r2_account_id" label="Account ID" bind:value={form.account_id} error={errors.r2_account_id} autocomplete="off" />
      <Input name="storage.bucket" label="Bucket" bind:value={form.bucket} error={errors.bucket} autocomplete="off" />
      <Input name="storage.access_key_id" label="Access key ID" bind:value={form.access_key_id} error={errors.access_key_id} autocomplete="off" />
      {@render secretField()}
      <div class="span"><KeyValue items={[{ label: 'Endpoint', value: r2Endpoint, mono: true }]} /></div>
    </div>
  {/if}

  {#if result}
    <div class="result">
      {#if result.ok}
        <div role="status"><StatusDot tone="success" label={`Connected · ${result.ms} ms`} /></div>
      {:else}
        <p class="failure" role="status">Couldn't connect to the storage backend.</p>
        <p class="detail" title={result.message}>{result.message}</p>
      {/if}
    </div>
  {/if}

  <div class="footer">
    {#if form.kind !== 'local'}<Button loading={testing} disabled={saving} onclick={test}>Test connection</Button>{/if}
    <Button variant="primary" loading={saving} disabled={!isDirty || testing} onclick={save}>Save backend</Button>
  </div>
</div>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--space-4);
  }

  .span {
    grid-column: 1 / -1;
  }

  .secret {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    min-height: var(--control-h-md);
  }

  .note {
    color: var(--text-tertiary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }

  .footer {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-3);
    padding-top: var(--space-4);
    border-top: 1px solid var(--border);
  }

  .result {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }

  .failure {
    color: var(--danger-fg);
    font-weight: 500;
  }

  .detail {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    color: var(--text-secondary);
    overflow-wrap: anywhere;
  }

  @media (max-width: 1023px) {
    .grid {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
