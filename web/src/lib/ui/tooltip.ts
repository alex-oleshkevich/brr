// One shared tooltip bubble for every icon-only control: hover or keyboard focus shows its label.
const SHOW_DELAY_MS = 400;
const GAP_PX = 6;
const EDGE_PX = 8;

let bubble: HTMLDivElement | undefined;

function ensureBubble(): HTMLDivElement {
  if (bubble) return bubble;
  bubble = document.createElement('div');
  bubble.className = 'tooltip';
  bubble.setAttribute('popover', 'manual');
  bubble.setAttribute('aria-hidden', 'true');
  document.body.append(bubble);
  return bubble;
}

export function tooltip(node: HTMLElement, text: string | undefined) {
  let label = text;
  let timer: ReturnType<typeof setTimeout> | undefined;

  function show() {
    if (!label || node.matches(':disabled, [aria-expanded="true"]')) return;
    const el = ensureBubble();
    el.textContent = label;
    el.showPopover();
    const anchor = node.getBoundingClientRect();
    const width = el.offsetWidth;
    const left = Math.min(Math.max(EDGE_PX, anchor.left + anchor.width / 2 - width / 2), window.innerWidth - width - EDGE_PX);
    const below = anchor.bottom + GAP_PX + el.offsetHeight <= window.innerHeight;
    el.style.left = `${left}px`;
    el.style.top = `${below ? anchor.bottom + GAP_PX : anchor.top - GAP_PX - el.offsetHeight}px`;
  }

  function schedule() {
    clearTimeout(timer);
    timer = setTimeout(show, SHOW_DELAY_MS);
  }

  function hide() {
    clearTimeout(timer);
    if (bubble?.matches(':popover-open')) bubble.hidePopover();
  }

  const focus = () => node.matches(':focus-visible') && schedule();
  node.addEventListener('pointerenter', schedule);
  node.addEventListener('pointerleave', hide);
  node.addEventListener('pointerdown', hide);
  node.addEventListener('focus', focus);
  node.addEventListener('blur', hide);
  const escape = (e: KeyboardEvent) => e.key === 'Escape' && hide();
  node.addEventListener('keydown', escape);

  return {
    update(next: string | undefined) {
      label = next;
    },
    destroy() {
      hide();
      node.removeEventListener('pointerenter', schedule);
      node.removeEventListener('pointerleave', hide);
      node.removeEventListener('pointerdown', hide);
      node.removeEventListener('focus', focus);
      node.removeEventListener('blur', hide);
      node.removeEventListener('keydown', escape);
    },
  };
}
