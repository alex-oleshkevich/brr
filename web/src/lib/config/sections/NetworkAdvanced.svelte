<script lang="ts">
  import { untrack } from 'svelte';
  import Select from '$lib/ui/Select.svelte';
  import type { SectionProps } from '../form';

  let { settings, readonly = false }: SectionProps = $props();

  let webrtc = $state(untrack(() => (settings.webrtc)));

  $effect(() => {
    settings.webrtc = webrtc;
  });
</script>

<Select
  label="WebRTC policy"
  bind:value={webrtc}
  disabled={readonly}
  options={[
    { value: 'default', label: 'Default' },
    { value: 'default_public_and_private_interfaces', label: 'Public and private interfaces' },
    { value: 'default_public_interface_only', label: 'Public interface only' },
    { value: 'disable_non_proxied_udp', label: 'Disable non-proxied UDP' },
  ]}
/>
