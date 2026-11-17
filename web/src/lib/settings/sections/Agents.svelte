<script lang="ts">
  import { page } from '$app/state';
  import { api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import { clock } from '$lib/clock.svelte';
  import { relative } from '$lib/format';
  import Badge from '$lib/ui/Badge.svelte';
  import Button from '$lib/ui/Button.svelte';
  import CodeBlock from '$lib/ui/CodeBlock.svelte';
  import Disclosure from '$lib/ui/Disclosure.svelte';
  import Checkbox from '$lib/ui/Checkbox.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Input from '$lib/ui/Input.svelte';
  import SaveBar from '$lib/ui/SaveBar.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import { SettingsForm } from '../form.svelte';

  let { dirty = $bindable(false) }: { dirty?: boolean } = $props();

  // Names the daemon expects; they come from crates/brr-daemon/src/wire.rs.
  const CLIENT_HEADER = 'Brr-Client';
  const CLIENT_KIND_HEADER = 'Brr-Client-Kind';
  const TOOLSET_HINTS = {
    browser: 'Tab tools (capture, navigation, input, cookies, network log), tab management and one-shot tools.',
    instances: 'Clients, browser instances, leases, limits, admission and usage.',
    profiles: 'Templates, profiles, backups, login checks and extensions.',
    builds: 'Browser versions: list, install and remove.',
    network: 'Proxies and proxy pools.',
    credentials: 'Saved logins in the vault (secrets are never returned).',
    captcha: 'Captcha settings, solve log, stats and balance.',
    storage: 'Storage backend, usage and retention.',
    system: 'Settings, events and system information.',
  } as const;
  const TOOLSETS = Object.keys(TOOLSET_HINTS) as (keyof typeof TOOLSET_HINTS)[];
  const IDLE_HINT = 'Its leases are released when it ends. 0 ends it as soon as it has no call or connection open.';

  const origin = page.url.origin;
  // Playwright needs a WebSocket address; /cdp speaks only WebSocket.
  const cdpUrl = `${origin.replace(/^http/, 'ws')}/cdp`;

  const mcpEntry = { command: 'brr', args: ['mcp'] };
  const mcpJson = JSON.stringify({ mcpServers: { brr: mcpEntry } }, null, 2);
  const mcpToml = `[mcp_servers.brr]\ncommand = "brr"\nargs = ["mcp"]`;
  const httpJson = JSON.stringify({ mcpServers: { brr: { url: `${origin}/mcp` } } }, null, 2);
  const restExample = `curl -s -X POST ${origin}/v1/clients -H 'Content-Type: application/json' -d '{"label":"my-agent"}'\n# then send the returned id on every call:\ncurl -s ${origin}/v1/profiles -H '${CLIENT_HEADER}: <client id>'`;
  const playwright = `import { chromium } from 'playwright';\n\nconst browser = await chromium.connectOverCDP('${cdpUrl}?profile=work');`;
  const skillInstall = 'brr skill install';

  const form = new SettingsForm(['oneshot.profile', 'oneshot.pool_size', 'oneshot.linger_s', 'client.mcp_http_idle_min', 'client.rest_idle_min', 'mcp.default_toolsets'], ['mcp.default_toolsets']);
  const clients = liveQuery(async () => (await unwrap(api.GET('/v1/clients'))).clients, { kinds: ['client.created', 'client.ended'] });

  const visibleClients = $derived((clients.data ?? []).filter((client) => client.kind !== 'ui'));

  const selectedToolsets = $derived(TOOLSETS.filter((toolset) => String(form.values['mcp.default_toolsets'] ?? '').split('\n').includes(toolset)));

  function toggleToolset(toolset: (typeof TOOLSETS)[number], checked: boolean) {
    const next = new Set(selectedToolsets);
    if (checked) next.add(toolset);
    else next.delete(toolset);
    form.values['mcp.default_toolsets'] = TOOLSETS.filter((t) => next.has(t)).join('\n');
  }

  let testing = $state(false);
  let result = $state<{ ok: boolean; text: string } | null>(null);

  $effect(() => {
    dirty = form.dirty;
  });

  async function test() {
    testing = true;
    try {
      const { profiles } = await unwrap(api.GET('/v1/profiles'));
      const names = profiles.filter((p) => !p.builtin).map((p) => p.name);
      result = { ok: true, text: `The REST API answered at ${origin}. ${names.length ? `Profiles agents can use: ${names.join(', ')}.` : 'You have no profiles yet; create one first.'}` };
    } catch (e) {
      result = { ok: false, text: e instanceof Error ? e.message : "Couldn't reach brr." };
    } finally {
      testing = false;
    }
  }
</script>

<div class="section">
  <p class="lead">Give an AI agent real browsers it can drive while you watch. Pick how it connects.</p>

  <section>
    <h3>Claude Code, Cursor and Codex</h3>
    <p class="note">Add brr as an MCP server. The agent starts it on demand, so brr must be on its PATH. To reach a daemon that is not at the default address, set <code>BRR_SERVER__BIND</code> (for example <code>127.0.0.1:9899</code>) in the server entry's environment.</p>
    <h4>Claude Code (.mcp.json) and Cursor (.cursor/mcp.json)</h4>
    <CodeBlock code={mcpJson} label="MCP config for Claude Code and Cursor" maxHeight="none" />
    <h4>Codex (~/.codex/config.toml)</h4>
    <CodeBlock code={mcpToml} label="MCP config for Codex" maxHeight="none" />
    <h4>The server entry on its own</h4>
    <CodeBlock code={JSON.stringify(mcpEntry)} label="MCP server entry" maxHeight="none" />
  </section>

  <section>
    <h3>Teach your agent brr</h3>
    <p class="note">Installs a skill so Claude Code, Codex and similar agents know when and how to use brr. The skill tells them to run brr prime, which prints the guide and what is running now.</p>
    <CodeBlock code={skillInstall} label="Install the brr skill" maxHeight="none" />
  </section>

  <section>
    <h3>MCP over HTTP</h3>
    <p class="note">For agents that connect to a running server instead of starting one.</p>
    <h4>Server URL</h4>
    <CodeBlock code="{origin}/mcp" label="HTTP MCP URL" maxHeight="none" />
    <h4>Example config</h4>
    <CodeBlock code={httpJson} label="HTTP MCP config" maxHeight="none" />
  </section>

  <section>
    <h3>REST API</h3>
    <p class="note">For your own code. Each caller registers once, then identifies itself on every request.</p>
    <h4>Base URL</h4>
    <CodeBlock code="{origin}/v1" label="REST base URL" maxHeight="none" />
    <h4>Client header</h4>
    <CodeBlock code={CLIENT_HEADER} label="Client header name" maxHeight="none" />
    <p class="note">Optional <code>{CLIENT_KIND_HEADER}</code> says what kind of caller it is, such as <code>cli</code>.</p>
    <h4>Example</h4>
    <CodeBlock code={restExample} label="REST example" maxHeight="none" />
  </section>

  <section>
    <h3>Playwright and Puppeteer</h3>
    <p class="note">Drive a brr browser over the Chrome DevTools Protocol. Add ?profile=NAME to pick a profile.</p>
    <h4>WebSocket URL</h4>
    <CodeBlock code={cdpUrl} label="CDP URL" maxHeight="none" />
    <h4>Example</h4>
    <CodeBlock code={playwright} label="Playwright example" maxHeight="none" />
  </section>

  <section>
    <h3>Test the REST API</h3>
    <p class="note">Asks the REST API for your profiles. This does not check MCP or the CDP connection.</p>
    <div class="test">
      <Button variant="primary" loading={testing} onclick={test}>Test</Button>
      {#if result}
        <p role="status" class="result">
          <Badge tone={result.ok ? 'success' : 'danger'}>{result.ok ? 'Works' : 'Failed'}</Badge>
          {result.text}
        </p>
      {/if}
    </div>
  </section>

  <section>
    <h3>Connected clients</h3>
    {#if clients.loading && !clients.data}
      <div aria-busy="true" class="stack"><Skeleton height="20px" /><Skeleton height="20px" /></div>
    {:else if clients.error && !clients.data}
      <EmptyState tone="danger" title="Couldn't load connected clients" description={clients.error.message}>
        {#snippet action()}<Button onclick={() => clients.reload()}>Retry</Button>{/snippet}
      </EmptyState>
    {:else if !visibleClients.length}
      <p class="note">No agents connected</p>
    {:else}
      <ul class="clients">
        {#each visibleClients as client (client.id)}
          <li>
            <span class="client-label">{client.label || client.id}</span>
            <Badge>{client.kind}</Badge>
            <span class="note">Connected {relative(client.created_at, clock.now)}</span>
            <span class="note">{client.lease_count} {client.lease_count === 1 ? 'lease' : 'leases'}</span>
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  <section>
    <h3>Tools for agents</h3>
    {#if form.loadError}
      <EmptyState tone="danger" title="Couldn't load settings" description={form.loadError.message}>
        {#snippet action()}<Button onclick={form.load}>Retry</Button>{/snippet}
      </EmptyState>
    {:else if form.loading && form.settings === undefined}
      <div aria-busy="true" class="stack"><Skeleton height="56px" /></div>
    {:else}
      <fieldset class="toolsets" name="mcp.default_toolsets" disabled={form.isEnv('mcp.default_toolsets')}>
        <legend class="field-label">Tools offered to agents by default</legend>
        {#each TOOLSETS as toolset (toolset)}
          <Checkbox label={toolset} hint={TOOLSET_HINTS[toolset]} checked={selectedToolsets.includes(toolset)} onchange={(checked) => toggleToolset(toolset, checked)} />
        {/each}
      </fieldset>
      {#if form.isEnv('mcp.default_toolsets')}<p class="note">Set by BRR_MCP__DEFAULT_TOOLSETS. Remove the environment variable to edit it here.</p>{/if}
      <p class="note">A client can still pass ?toolsets= to choose its own tools.</p>
      {#if selectedToolsets.length === 0}<p class="field-error">Pick at least one toolset.</p>{/if}
      {#if form.error('mcp.default_toolsets')}<p class="field-error">{form.error('mcp.default_toolsets')}</p>{/if}
    {/if}
  </section>

  <section>
    <h3>One-off jobs</h3>
    {#if form.loadError}
      <EmptyState tone="danger" title="Couldn't load settings" description={form.loadError.message}>
        {#snippet action()}<Button onclick={form.load}>Retry</Button>{/snippet}
      </EmptyState>
    {:else if form.loading && form.settings === undefined}
      <div aria-busy="true" class="stack"><Skeleton height="56px" /><Skeleton height="56px" /></div>
    {:else}
      <Input name="oneshot.profile" label="Profile for one-off jobs" bind:value={form.values['oneshot.profile']} readonly={form.isEnv('oneshot.profile')} badge={form.badge('oneshot.profile')} hint={form.hint('oneshot.profile', 'The profile one-off jobs run in.')} error={form.error('oneshot.profile')} />
      <div class="pair">
        <Input name="oneshot.pool_size" label="Pre-started browsers for one-off jobs" type="number" min={0} bind:value={form.values['oneshot.pool_size']} readonly={form.isEnv('oneshot.pool_size')} badge={form.badge('oneshot.pool_size')} hint={form.hint('oneshot.pool_size', 'Kept ready so one-off jobs start instantly.')} error={form.error('oneshot.pool_size')} />
        <Input name="oneshot.linger_s" label="Keep a one-off browser open after its job (s)" type="number" min={0} bind:value={form.values['oneshot.linger_s']} readonly={form.isEnv('oneshot.linger_s')} badge={form.badge('oneshot.linger_s')} hint={form.hint('oneshot.linger_s', 'Lets a quick follow-up job reuse it.')} error={form.error('oneshot.linger_s')} />
      </div>
      <Disclosure summary="Advanced">
        <div class="pair">
          <Input name="client.mcp_http_idle_min" label="End an idle MCP HTTP client after (min)" type="number" min={0} bind:value={form.values['client.mcp_http_idle_min']} readonly={form.isEnv('client.mcp_http_idle_min')} badge={form.badge('client.mcp_http_idle_min')} hint={form.hint('client.mcp_http_idle_min', IDLE_HINT)} error={form.error('client.mcp_http_idle_min')} />
          <Input name="client.rest_idle_min" label="End an idle REST client after (min)" type="number" min={0} bind:value={form.values['client.rest_idle_min']} readonly={form.isEnv('client.rest_idle_min')} badge={form.badge('client.rest_idle_min')} hint={form.hint('client.rest_idle_min', IDLE_HINT)} error={form.error('client.rest_idle_min')} />
        </div>
      </Disclosure>
    {/if}
  </section>
  <SaveBar dirty={form.dirty} saving={form.saving} disabled={form.invalid || selectedToolsets.length === 0} onsave={form.save} ondiscard={form.discard} />
</div>

<style>
  .section {
    display: flex;
    flex-direction: column;
    gap: var(--space-6);
  }

  section {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  h3 {
    font-size: var(--text-md-size);
    line-height: var(--text-md-line);
    letter-spacing: 0;
  }

  h4 {
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
    font-weight: 500;
    letter-spacing: 0;
  }

  .lead,
  .note {
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }

  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .pair {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-3);
  }

  .clients {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .clients li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .client-label {
    font-weight: 500;
  }

  .toolsets {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-inline-size: 0;
    margin: 0;
    padding: 0;
    border: 0;
  }

  .toolsets legend {
    margin-bottom: var(--space-2);
  }

  .test {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }

  .result {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
</style>
