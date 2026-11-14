<script lang="ts">
  import Button from '$lib/ui/Button.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import Input from '$lib/ui/Input.svelte';
  import type { DialogType } from './protocol';

  export type PageDialogState = { type: DialogType; message: string; defaultPrompt: string };

  let {
    dialog,
    interactive,
    onanswer,
  }: {
    dialog: PageDialogState | null;
    interactive: boolean;
    onanswer: (accept: boolean, promptText?: string) => void;
  } = $props();

  const TITLES: Record<DialogType, string> = { alert: 'Alert', confirm: 'Confirm', prompt: 'Prompt', beforeunload: 'Leave page?' };

  let answer = $state('');
  let open = $state(false);
  // Set while an answer closes the dialog, so closing is not mistaken for dismissing it.
  let answering = false;

  // Also when control is gained: a dialog hidden by a viewer is still blocking the page.
  $effect(() => {
    void interactive;
    open = dialog !== null;
    answer = dialog?.defaultPrompt ?? '';
  });

  function reply(accept: boolean) {
    answering = true;
    onanswer(accept, dialog?.type === 'prompt' && accept ? answer : undefined);
    open = false;
  }

  // Esc, the close button and the backdrop: an interactive viewer cancels, because a page stays
  // blocked until its dialog is answered. Anyone else only hides it; the daemon's dialog_closed
  // clears it.
  function closed() {
    if (answering) {
      answering = false;
      return;
    }
    if (dialog !== null && interactive) onanswer(false);
  }
</script>

<Dialog bind:open size="sm" title={dialog ? TITLES[dialog.type] : ''} onclose={closed}>
  {#if dialog}
    <p class="message">{dialog.message}</p>
    {#if dialog.type === 'prompt' && interactive}
      <div class="field"><Input bind:value={answer} label="Your answer" /></div>
    {/if}
    {#if !interactive}<p class="note">Whoever is in control of the tab answers this dialog.</p>{/if}
  {/if}
  {#snippet footer()}
    {#if interactive}
      {#if dialog?.type !== 'alert'}<Button onclick={() => reply(false)}>Cancel</Button>{/if}
      <Button variant="primary" onclick={() => reply(true)} autofocus>OK</Button>
    {:else}
      <Button onclick={() => (open = false)}>Close</Button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .message {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .field {
    margin-top: var(--space-4);
  }

  .note {
    margin-top: var(--space-3);
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }
</style>
