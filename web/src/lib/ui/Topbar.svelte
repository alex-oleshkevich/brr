<script lang="ts">
  import type { Snippet } from 'svelte';
  import Button from './Button.svelte';

  let { onmenu, menuExpanded, children }: { onmenu: () => void; menuExpanded: boolean; children?: Snippet } = $props();
</script>

<header>
  <div class="menu">
    <Button variant="ghost" iconOnly icon="menu" aria-label="Open navigation" aria-expanded={menuExpanded} onclick={onmenu} />
  </div>
  <div class="actions">{@render children?.()}</div>
</header>

<style>
  header {
    position: sticky;
    top: 0;
    z-index: 10;
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: var(--topbar-h);
    padding: 0 var(--space-6);
    background: var(--bg);
    border-bottom: 1px solid var(--border);
  }

  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    margin-left: auto;
  }

  @media (min-width: 1024px) {
    .menu {
      display: none;
    }
  }
</style>
