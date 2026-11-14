// A confirm dialog returns focus to the row menu that opened it; when the row then disappears, focus would drop to <body>.
const SETTLE_MS = 600;

export function refocusIfLost(id: string): void {
  setTimeout(() => {
    if (document.activeElement === document.body) document.getElementById(id)?.focus({ preventScroll: true });
  }, SETTLE_MS);
}
