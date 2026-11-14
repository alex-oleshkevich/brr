<script lang="ts">
  import { stateLabel, stateTone, type StateKind } from '$lib/state';
  import Badge from './Badge.svelte';

  let { kind, state, label }: { kind: StateKind; state: string; label?: string } = $props();
</script>

<Badge dot tone={stateTone(kind, state)}>{label ?? stateLabel(state)}</Badge>
