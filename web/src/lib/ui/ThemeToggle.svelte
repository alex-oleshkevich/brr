<script lang="ts">
  import { theme, type ThemePref } from '$lib/theme.svelte';
  import Button from './Button.svelte';
  import Menu from './Menu.svelte';
  import type { IconName } from './Icon.svelte';

  const icons: Record<ThemePref, IconName> = { system: 'monitor', light: 'sun', dark: 'moon' };
  const names: Record<ThemePref, string> = { system: 'System', light: 'Light', dark: 'Dark' };

  const items = $derived(
    (['system', 'light', 'dark'] as const).map((pref) => ({
      label: names[pref],
      checked: theme.pref === pref,
      disabled: theme.lockedByEnv,
      hint: theme.lockedByEnv ? 'Set by BRR_UI__THEME' : undefined,
      onselect: () => theme.set(pref),
    })),
  );
</script>

<Menu label="Theme" {items}>
  {#snippet trigger(props)}
    <Button variant="ghost" iconOnly icon={icons[theme.pref]} {...props} aria-label="Theme: {names[theme.pref]}" />
  {/snippet}
</Menu>
