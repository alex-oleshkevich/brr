import { afterNavigate, beforeNavigate, goto } from '$app/navigation';
import { confirm } from './confirm.svelte';

// Call during component init. In-app navigation asks first; closing or reloading the tab brings up the browser's own prompt.
export function guardUnsaved(dirty: () => boolean, description: string): { allow(): void } {
  let allowed = false;

  beforeNavigate(({ cancel, to }) => {
    if (!dirty() || allowed) return;
    cancel();
    if (!to) return;

    void confirm({ title: 'Discard unsaved changes?', description, confirmLabel: 'Discard', tone: 'danger' }).then((ok) => {
      if (!ok) return;
      allowed = true;
      void goto(to.url);
    });
  });
  afterNavigate(() => (allowed = false));

  return { allow: () => (allowed = true) };
}
