<script lang="ts">
  import { page } from '$app/state';
  import { api, unwrap } from '$lib/api/client';
  import Badge from '$lib/ui/Badge.svelte';
  import Button from '$lib/ui/Button.svelte';
  import Card from '$lib/ui/Card.svelte';
  import CodeBlock from '$lib/ui/CodeBlock.svelte';
  import PageHeader from '$lib/ui/PageHeader.svelte';

  // Names the daemon expects; they come from crates/brr-daemon/src/wire.rs.
  const CLIENT_HEADER = 'Brr-Client';
  const CLIENT_KIND_HEADER = 'Brr-Client-Kind';

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

  let testing = $state(false);
  let result = $state<{ ok: boolean; text: string } | null>(null);

  async function test() {
    testing = true;
    try {
      const { profiles } = await unwrap(api.GET('/v1/profiles'));
      const names = profiles.filter((p) => !p.builtin).map((p) => p.name);
      result = { ok: true, text: `brr answered at ${origin}. ${names.length ? `Profiles agents can use: ${names.join(', ')}.` : 'You have no profiles yet; create one first.'}` };
    } catch (e) {
      result = { ok: false, text: e instanceof Error ? e.message : "Couldn't reach brr." };
    } finally {
      testing = false;
    }
  }
</script>

<PageHeader title="Connect an agent" description="Give an AI agent real browsers it can drive while you watch. Pick how it connects." />

<div class="stack">
  <Card title="Claude Code, Cursor and Codex" description="Add brr as an MCP server. The agent starts it on demand, so brr must be on its PATH.">
    <div class="blocks">
      <div>
        <h3>Claude Code (.mcp.json) and Cursor (.cursor/mcp.json)</h3>
        <CodeBlock code={mcpJson} label="MCP config for Claude Code and Cursor" maxHeight="none" />
      </div>
      <div>
        <h3>Codex (~/.codex/config.toml)</h3>
        <CodeBlock code={mcpToml} label="MCP config for Codex" maxHeight="none" />
      </div>
      <div>
        <h3>The server entry on its own</h3>
        <CodeBlock code={JSON.stringify(mcpEntry)} label="MCP server entry" maxHeight="none" />
      </div>
    </div>
  </Card>

  <Card title="Teach your agent brr" description="Installs a skill so Claude Code, Codex and similar agents know when and how to use brr. The skill tells them to run brr prime, which prints the guide and what is running now.">
    <CodeBlock code={skillInstall} label="Install the brr skill" maxHeight="none" />
  </Card>

  <Card title="MCP over HTTP" description="For agents that connect to a running server instead of starting one.">
    <div class="blocks">
      <div>
        <h3>Server URL</h3>
        <CodeBlock code="{origin}/mcp" label="HTTP MCP URL" maxHeight="none" />
      </div>
      <div>
        <h3>Example config</h3>
        <CodeBlock code={httpJson} label="HTTP MCP config" maxHeight="none" />
      </div>
    </div>
  </Card>

  <Card title="REST API" description="For your own code. Each caller registers once, then identifies itself on every request.">
    <div class="blocks">
      <div>
        <h3>Base URL</h3>
        <CodeBlock code="{origin}/v1" label="REST base URL" maxHeight="none" />
      </div>
      <div>
        <h3>Client header</h3>
        <CodeBlock code={CLIENT_HEADER} label="Client header name" maxHeight="none" />
        <p class="note">Optional <code>{CLIENT_KIND_HEADER}</code> says what kind of caller it is, such as <code>cli</code>.</p>
      </div>
      <div>
        <h3>Example</h3>
        <CodeBlock code={restExample} label="REST example" maxHeight="none" />
      </div>
    </div>
  </Card>

  <Card title="Playwright and Puppeteer" description="Drive a brr browser over the Chrome DevTools Protocol. Add ?profile=NAME to pick a profile.">
    <div class="blocks">
      <div>
        <h3>WebSocket URL</h3>
        <CodeBlock code={cdpUrl} label="CDP URL" maxHeight="none" />
      </div>
      <div>
        <h3>Example</h3>
        <CodeBlock code={playwright} label="Playwright example" maxHeight="none" />
      </div>
    </div>
  </Card>

  <Card title="Test the connection" description="Asks brr for your profiles, the way an agent would.">
    <div class="test">
      <Button variant="primary" loading={testing} onclick={test}>Test</Button>
      {#if result}
        <p role="status" class="result">
          <Badge tone={result.ok ? 'success' : 'danger'}>{result.ok ? 'Works' : 'Failed'}</Badge>
          {result.text}
        </p>
      {/if}
    </div>
  </Card>
</div>

<style>
  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .blocks {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  h3 {
    margin-bottom: var(--space-2);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
    font-weight: 500;
    letter-spacing: 0;
  }

  .note {
    margin-top: var(--space-2);
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
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
