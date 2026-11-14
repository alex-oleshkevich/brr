<script lang="ts">
  import type { components } from '$lib/api/schema';
  import Button from '$lib/ui/Button.svelte';
  import CodeBlock from '$lib/ui/CodeBlock.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import { profileLabel } from './actions';

  type Instance = components['schemas']['Instance'];

  let { open = $bindable(false), instance }: { open?: boolean; instance: Instance } = $props();

  const MCP_CONFIG = JSON.stringify({ mcpServers: { brr: { command: 'brr', args: ['mcp'] } } }, null, 2);

  const name = $derived(profileLabel(instance));
  // /cdp attaches to the running browser of a persistent profile; a temporary one would launch a fresh browser instead.
  const shared = $derived(instance.kind === 'managed' && !instance.ephemeral && instance.profile_name !== null);
  const cdpUrl = $derived(`${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/cdp?profile=${encodeURIComponent(instance.profile_name ?? '')}`);
</script>

<Dialog bind:open size="md" title="Connect to {name}" description="Let an agent or a script use this browser.">
  <div class="sections">
    <section>
      <h3>An AI agent (MCP)</h3>
      <p>Add brr to your agent's MCP servers, then ask it to open a tab in {name}.</p>
      <CodeBlock code={MCP_CONFIG} label="MCP configuration" maxHeight="160px" />
    </section>
    <section>
      <h3>A script (Playwright, Puppeteer)</h3>
      {#if shared}
        <p>Connect over CDP at this address to drive this browser:</p>
        <CodeBlock code={cdpUrl} label="CDP address for this browser" maxHeight="80px" />
      {:else}
        <p>A temporary browser can't be shared over CDP: connecting starts a new one. Promote it to a saved profile first, or use MCP.</p>
      {/if}
    </section>
  </div>
  {#snippet footer()}
    <Button href="/connect">More ways to connect</Button>
    <Button variant="primary" onclick={() => (open = false)}>Close</Button>
  {/snippet}
</Dialog>

<style>
  .sections {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }

  h3 {
    font-size: var(--text-md-size);
    font-weight: 600;
    margin-bottom: var(--space-1);
  }

  p {
    margin-bottom: var(--space-2);
    color: var(--text-secondary);
  }
</style>
