<script lang="ts">
  import { untrack } from 'svelte';
  import Input from '$lib/ui/Input.svelte';
  import Textarea from '$lib/ui/Textarea.svelte';
  import Select from '$lib/ui/Select.svelte';
  import { engineOf, fieldError, halfPair, sourceOf, type SectionProps } from '../form';

  let { settings, readonly = false, errors, builds, reportInvalid }: SectionProps = $props();

  let latitude = $state<number | null>(untrack(() => (settings.geolocation?.latitude ?? null)));
  let longitude = $state<number | null>(untrack(() => (settings.geolocation?.longitude ?? null)));
  let accuracy = $state<number | null>(untrack(() => (settings.geolocation?.accuracy ?? null)));
  let viewportWidth = $state<number | null>(untrack(() => (settings.viewport?.width ?? null)));
  let viewportHeight = $state<number | null>(untrack(() => (settings.viewport?.height ?? null)));
  let userAgent = $state(untrack(() => (settings.user_agent ?? '')));

  let fpOs = $state<'windows' | 'macos' | 'linux' | ''>(untrack(() => (settings.fingerprint?.os ?? '')));
  let fpScreenWidth = $state<number | null>(untrack(() => (settings.fingerprint?.screen?.width ?? null)));
  let fpScreenHeight = $state<number | null>(untrack(() => (settings.fingerprint?.screen?.height ?? null)));
  let fpFonts = $state(untrack(() => ((settings.fingerprint?.fonts ?? []).join('\n'))));
  let fpVendor = $state(untrack(() => (settings.fingerprint?.webgl?.vendor ?? '')));
  let fpRenderer = $state(untrack(() => (settings.fingerprint?.webgl?.renderer ?? '')));

  const showFingerprint = $derived(engineOf(settings, builds) === 'camoufox' || sourceOf(settings, builds) === 'fingerprint_chromium');

  const geoHalf = $derived(halfPair(latitude, longitude));
  const viewportHalf = $derived(halfPair(viewportWidth, viewportHeight));
  const fpHalf = $derived(halfPair(fpScreenWidth, fpScreenHeight));
  const pairHint = (missing: string, other: string) => `Enter a ${missing} too, or clear the ${other}.`;
  const negative = (v: number | null) => v !== null && v < 0;
  const sizeError = (v: number | null, half: boolean, missing: string, other: string) =>
    negative(v) ? 'Enter 0 or more.' : half ? pairHint(missing, other) : null;

  $effect(() => {
    reportInvalid('geolocation', geoHalf.first || geoHalf.second);
    reportInvalid('viewport', viewportHalf.first || viewportHalf.second || negative(viewportWidth) || negative(viewportHeight));
    reportInvalid('fingerprint-screen', showFingerprint && (fpHalf.first || fpHalf.second || negative(fpScreenWidth) || negative(fpScreenHeight)));
    return () => {
      reportInvalid('geolocation', false);
      reportInvalid('viewport', false);
      reportInvalid('fingerprint-screen', false);
    };
  });
  $effect(() => {
    settings.geolocation = latitude !== null && longitude !== null ? { latitude, longitude, accuracy } : null;
  });
  $effect(() => {
    settings.viewport = viewportWidth !== null && viewportHeight !== null ? { width: viewportWidth, height: viewportHeight } : null;
  });
  $effect(() => {
    settings.user_agent = userAgent.trim() === '' ? null : userAgent;
  });
  $effect(() => {
    const fonts = fpFonts.split('\n').map((f) => f.trim()).filter(Boolean);
    const has = fpOs !== '' || fpScreenWidth !== null || fonts.length > 0 || fpVendor !== '' || fpRenderer !== '';
    settings.fingerprint = has
      ? {
          os: fpOs === '' ? null : fpOs,
          screen: fpScreenWidth !== null && fpScreenHeight !== null ? { width: fpScreenWidth, height: fpScreenHeight } : null,
          fonts,
          webgl: fpVendor !== '' || fpRenderer !== '' ? { vendor: fpVendor, renderer: fpRenderer } : null,
        }
      : null;
  });
</script>

<div class="fields">
  <div class="trio">
    <Input label="Latitude" type="number" step="any" bind:value={latitude} readonly={readonly} error={fieldError(errors, 'settings.geolocation', 'geolocation') ?? (geoHalf.first ? pairHint('latitude', 'longitude') : null)} />
    <Input label="Longitude" type="number" step="any" bind:value={longitude} readonly={readonly} error={geoHalf.second ? pairHint('longitude', 'latitude') : null} />
    <Input label="Accuracy (m)" type="number" min={0} step="any" bind:value={accuracy} readonly={readonly} />
  </div>
  <div class="pair">
    <Input label="Viewport width" type="number" min={0} bind:value={viewportWidth} readonly={readonly} error={fieldError(errors, 'settings.viewport', 'viewport') ?? sizeError(viewportWidth, viewportHalf.first, 'width', 'height')} />
    <Input label="Viewport height" type="number" min={0} bind:value={viewportHeight} readonly={readonly} error={sizeError(viewportHeight, viewportHalf.second, 'height', 'width')} />
  </div>
  <Input label="User agent" bind:value={userAgent} readonly={readonly} placeholder={readonly ? 'Not set' : 'Browser default'} />

  {#if showFingerprint}
    <h3>Fingerprint</h3>
    <Select
      label="Operating system"
      bind:value={fpOs}
      disabled={readonly}
      options={[
        { value: '', label: 'Automatic' },
        { value: 'windows', label: 'Windows' },
        { value: 'macos', label: 'macOS' },
        { value: 'linux', label: 'Linux' },
      ]}
    />
    <div class="pair">
      <Input label="Fingerprint screen width" type="number" min={0} bind:value={fpScreenWidth} readonly={readonly} error={fieldError(errors, 'settings.fingerprint.screen', 'fingerprint.screen') ?? sizeError(fpScreenWidth, fpHalf.first, 'width', 'height')} />
      <Input label="Fingerprint screen height" type="number" min={0} bind:value={fpScreenHeight} readonly={readonly} error={sizeError(fpScreenHeight, fpHalf.second, 'height', 'width')} />
    </div>
    <Textarea label="Fonts" bind:value={fpFonts} readonly={readonly} rows={3} hint="One font per line." />
    <div class="pair">
      <Input label="WebGL vendor" bind:value={fpVendor} readonly={readonly} />
      <Input label="WebGL renderer" bind:value={fpRenderer} readonly={readonly} />
    </div>
  {/if}
</div>

<style>
  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .pair {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-3);
  }

  .trio {
    display: grid;
    grid-template-columns: 1fr 1fr 1fr;
    gap: var(--space-3);
  }

  h3 {
    font-size: var(--text-base-size);
    line-height: var(--text-base-line);
    margin-top: var(--space-2);
  }
</style>
