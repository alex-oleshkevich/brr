<script lang="ts">
  let {
    width = '100%',
    height = '12px',
    radius = 'var(--radius-sm)',
  }: { width?: string; height?: string; radius?: string } = $props();
</script>

<span class="skeleton" aria-hidden="true" style:width style:height style:border-radius={radius}></span>

<style>
  .skeleton {
    display: block;
    background-color: var(--bg-muted);
    background-image: linear-gradient(
      90deg,
      transparent,
      color-mix(in srgb, var(--bg-emphasis) 60%, transparent),
      transparent
    );
    background-size: 200% 100%;
    background-repeat: no-repeat;
    animation: shimmer 1.4s linear infinite;
  }

  @keyframes shimmer {
    from {
      background-position: 200% 0;
    }
    to {
      background-position: -200% 0;
    }
  }
</style>
