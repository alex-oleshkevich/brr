const GAP = 4;

// Places a top-layer popover under its anchor, flipping above when it would overflow the viewport.
export function placePopover(anchor: HTMLElement, panel: HTMLElement, align: 'start' | 'end'): void {
  const anchorRect = anchor.getBoundingClientRect();
  const { width, height } = panel.getBoundingClientRect();
  const left = align === 'end' ? anchorRect.right - width : anchorRect.left;
  const below = anchorRect.bottom + GAP;
  const top = below + height > innerHeight ? anchorRect.top - height - GAP : below;

  panel.style.left = `${Math.max(GAP, Math.min(left, innerWidth - width - GAP))}px`;
  panel.style.top = `${Math.max(GAP, top)}px`;
}
