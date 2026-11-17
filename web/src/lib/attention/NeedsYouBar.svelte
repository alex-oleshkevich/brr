<script lang="ts">
  import { everySecond } from '$lib/clock.svelte';
  import { countdown } from '$lib/events/summary';
  import Button from '$lib/ui/Button.svelte';
  import Icon from '$lib/ui/Icon.svelte';
  import { attention, hostOf, REASON_VERBS } from './attention.svelte';

  // Mounted once the session is ready, so the shared queries start with the Brr-Client header.
  attention.init();

  const DOCUMENT_PREFIX = '(1) Needs you · ';

  const first = $derived(attention.needsYou[0]);
  const more = $derived(attention.needsYou.length - 1);

  let now = $state(Date.now());
  $effect(() => {
    if (!first) return;

    return everySecond((tick) => (now = tick));
  });

  // Pages set their own title; keep the prefix on whatever they set, and take it off again when nobody needs you.
  $effect(() => {
    if (!first) return;

    const prefix = () => {
      if (!document.title.startsWith(DOCUMENT_PREFIX)) document.title = DOCUMENT_PREFIX + document.title;
    };
    prefix();
    const observer = new MutationObserver(prefix);
    observer.observe(document.head, { childList: true, characterData: true, subtree: true });

    return () => {
      observer.disconnect();
      document.title = document.title.replace(DOCUMENT_PREFIX, '');
    };
  });
</script>

{#if first}
  <div class="bar" role="status">
    <Icon name="bell" />
    <p>
      <strong>{first.agent} needs you to {REASON_VERBS[first.reason]} on {hostOf(first.url)}</strong>
      <span class="num">· {countdown(first.deadline, now)}</span>
      {#if more > 0}<span>· and {more} more</span>{/if}
    </p>
    <Button variant="primary" size="sm" href="/browsers/{first.instanceId}?tab={first.tabId}">Step in</Button>
  </div>
{/if}

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-8);
    background: var(--warning-bg);
    border-bottom: 1px solid var(--warning-border);
    color: var(--warning-fg);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }

  p {
    flex: 1;
    min-width: 0;
  }
</style>
