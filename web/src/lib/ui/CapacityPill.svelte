<script lang="ts">
  import { events } from '$lib/api/events.svelte';
  import { capacity } from '$lib/capacity.svelte';
  import Button from './Button.svelte';
  import StatusDot from './StatusDot.svelte';

  // Mounted once the session is ready, so the shared polls start with the Brr-Client header.
  capacity.init();

  const pressure = $derived(capacity.pressure);
  const tone = $derived(pressure.state === 'ok' ? 'success' : 'warning');
  const known = $derived(capacity.info && capacity.limits && capacity.usage && !capacity.error);
  // The numbers are the last ones received; they say nothing about the daemon while the stream is down.
  const offline = $derived(events.status === 'reconnecting');
</script>

{#if offline}
  <Button variant="ghost" size="sm" href="/limits" aria-label="Capacity: unknown, daemon offline">
    <StatusDot tone="neutral" label="Capacity unknown" showLabel={false} />
    <span class="text">Offline</span>
  </Button>
{:else if known}
  <Button variant="ghost" size="sm" href="/limits" aria-label="Capacity: {pressure.text}">
    <StatusDot {tone} label={pressure.text} showLabel={false} pulse={pressure.state === 'pressure'} />
    <span class="text">{pressure.text}</span>
  </Button>
{/if}

<style>
  @media (max-width: 639px) {
    .text {
      position: absolute;
      width: 1px;
      height: 1px;
      overflow: hidden;
      clip-path: inset(50%);
      white-space: nowrap;
    }
  }
</style>
