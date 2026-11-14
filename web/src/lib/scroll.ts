import { tick } from 'svelte';

const TOPBAR_PX = 48; // --topbar-h in tokens.css
const TOPBAR_GAP_PX = 16;
const BOTTOM_BAR_PX = 80;

// A page that renders a skeleton first has no anchor target when SvelteKit looks for it,
// so it calls this once its content is in.
export async function scrollToHash(): Promise<void> {
  await tick();
  if (location.hash) document.getElementById(decodeURIComponent(location.hash.slice(1)))?.scrollIntoView();
}

// Brings the first invalid field into view only when it isn't already, so a failed save doesn't throw the page around.
export function revealInvalid(root: ParentNode = document): void {
  const field = root.querySelector<HTMLElement>('[aria-invalid="true"]');
  if (!field) return;
  const { top, bottom } = field.getBoundingClientRect();
  if (top < TOPBAR_PX + TOPBAR_GAP_PX || bottom > innerHeight - BOTTOM_BAR_PX) field.scrollIntoView({ block: 'nearest' });
}
