<script lang="ts">
  import { tick } from 'svelte';
  import { session } from '$lib/api/session';
  import Button from '$lib/ui/Button.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import PageDialog, { type PageDialogState } from './PageDialog.svelte';
  import {
    capped,
    keyMessage,
    modifiers,
    mouseButton,
    screencastUrl,
    toFramePoint,
    wheelDeltas,
    type ClientMsg,
    type FrameMetadata,
    type ServerMsg,
  } from './protocol';

  export type Status = 'connecting' | 'live' | 'closed' | 'unsupported' | 'error';

  let {
    tabId,
    interactive,
    title = '',
    focused = false,
    status = $bindable('connecting'),
  }: { tabId: string; interactive: boolean; title?: string; focused?: boolean; status?: Status } = $props();

  const RETRY_DELAYS_MS = [1000, 2000, 4000, 4000, 4000];
  const MULTI_CLICK_MS = 500;
  const MULTI_CLICK_PX = 5;
  const DEFAULT_ASPECT = 16 / 9;

  let errorMessage = $state('');
  let notice = $state('');
  let src = $state('');
  let aspect = $state(DEFAULT_ASPECT);
  let dialog = $state<PageDialogState | null>(null);
  let attempt = $state(0);

  let moveRaf = 0;
  let socket: WebSocket | null = null;
  // What the daemon said before it closed a socket that never showed a frame (no Tab, no access): such a close is final.
  // An error after frames is a refused input and leaves reconnecting alone.
  let serverError = '';
  let framed = false;
  // Keys pressed in the page and not yet released.
  const pressed = new Map<string, ClientMsg>();
  let retryTimer: ReturnType<typeof setTimeout> | undefined;
  let retries = 0;
  let pendingFrame: string | null = null;
  let frameRaf = 0;
  let img = $state<HTMLImageElement>();
  let region = $state<HTMLDivElement>();
  let sink = $state<HTMLTextAreaElement>();

  const hintId = $props.id();

  function send(message: ClientMsg) {
    if (socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify(message));
  }

  // Only the newest frame is decoded: a slow machine would otherwise queue decodes and lag behind.
  function showFrame(data: string, metadata: FrameMetadata) {
    pendingFrame = data;
    if (metadata.deviceWidth > 0 && metadata.deviceHeight > 0) aspect = metadata.deviceWidth / metadata.deviceHeight;
    if (frameRaf) return;
    frameRaf = requestAnimationFrame(() => {
      frameRaf = 0;
      if (pendingFrame !== null) src = 'data:image/jpeg;base64,' + pendingFrame;
      pendingFrame = null;
    });
  }

  function handle(message: ServerMsg) {
    switch (message.type) {
      case 'frame':
        framed = true;
        retries = 0;
        status = 'live';
        showFrame(message.data, message.metadata);
        break;
      case 'dialog':
        dialog = { type: message.dialog_type, message: capped(message.message), defaultPrompt: capped(message.default_prompt ?? '') };
        break;
      case 'dialog_closed':
        dialog = null;
        break;
      case 'closed':
        status = 'closed';
        break;
      case 'error':
        if (message.code === 'EngineUnsupported') {
          status = 'unsupported';
        } else {
          if (!framed) serverError = message.message;
          notice = message.message;
        }
        break;
    }
  }

  function connect() {
    clearTimeout(retryTimer);
    if (!session.clientId) {
      status = 'error';
      errorMessage = 'No session with the daemon.';
      return;
    }

    framed = false;
    serverError = '';
    const ws = new WebSocket(screencastUrl(tabId, session.clientId));
    socket = ws;
    ws.onmessage = (e) => {
      if (socket !== ws || typeof e.data !== 'string') return;
      try {
        handle(JSON.parse(e.data) as ServerMsg);
      } catch {
        // A message that is not JSON is not ours to act on.
      }
    };
    ws.onclose = () => {
      if (socket !== ws) return;
      socket = null;
      if (status === 'closed' || status === 'unsupported' || status === 'error') return;
      if (serverError) {
        errorMessage = serverError;
        status = 'error';
        return;
      }

      const delay = RETRY_DELAYS_MS[retries];
      if (delay === undefined) {
        errorMessage = 'The connection to the live view was lost.';
        status = 'error';
        return;
      }
      retries += 1;
      status = 'connecting';
      retryTimer = setTimeout(connect, delay);
    };
  }

  function disconnect() {
    clearTimeout(retryTimer);
    cancelAnimationFrame(frameRaf);
    frameRaf = 0;
    cancelAnimationFrame(moveRaf);
    const ws = socket;
    socket = null;
    ws?.close();
  }

  // A new Tab or a retry starts a clean connection.
  $effect(() => {
    void tabId;
    void attempt;
    status = 'connecting';
    src = '';
    dialog = null;
    notice = '';
    serverError = '';
    retries = 0;
    connect();

    return disconnect;
  });

  $effect(() => {
    if (!focused || !interactive || !sink) return;
    void tick().then(() => sink?.focus());
  });

  // The page behind a Tab that has just been given to someone else must not keep a stale notice.
  $effect(() => {
    void interactive;
    notice = '';
  });

  let lastMove: { x: number; y: number; mods: number } | null = null;
  let lastDown = { at: 0, x: 0, y: 0, button: -1, count: 0 };

  const natural = () => (img && img.naturalWidth > 0 ? { w: img.naturalWidth, h: img.naturalHeight } : null);

  function point(e: MouseEvent) {
    const size = natural();
    if (!img || !size) return null;

    return toFramePoint(e.clientX, e.clientY, img.getBoundingClientRect(), size);
  }

  // Pointer events carry no click count (detail is always 0), so double and triple clicks are counted here.
  function clickCount(e: PointerEvent): number {
    const near = Math.hypot(e.clientX - lastDown.x, e.clientY - lastDown.y) <= MULTI_CLICK_PX;
    const again = e.timeStamp - lastDown.at <= MULTI_CLICK_MS && e.button === lastDown.button && near;
    lastDown = { at: e.timeStamp, x: e.clientX, y: e.clientY, button: e.button, count: again ? lastDown.count + 1 : 1 };

    return lastDown.count;
  }

  function pointerdown(e: PointerEvent) {
    const at = point(e);
    if (!at) return;

    e.preventDefault();
    region?.setPointerCapture(e.pointerId);
    sink?.focus();
    send({ type: 'mouse', event: 'down', ...at, button: mouseButton(e.button), click_count: clickCount(e), modifiers: modifiers(e), delta_x: 0, delta_y: 0 });
  }

  function pointerup(e: PointerEvent) {
    const at = point(e);
    if (!at) return;

    if (region?.hasPointerCapture(e.pointerId)) region.releasePointerCapture(e.pointerId);
    send({ type: 'mouse', event: 'up', ...at, button: mouseButton(e.button), click_count: lastDown.count, modifiers: modifiers(e), delta_x: 0, delta_y: 0 });
  }

  function pointermove(e: PointerEvent) {
    const at = point(e);
    if (!at) return;

    lastMove = { ...at, mods: modifiers(e) };
    if (moveRaf) return;
    moveRaf = requestAnimationFrame(() => {
      moveRaf = 0;
      if (!lastMove) return;
      send({ type: 'mouse', event: 'move', x: lastMove.x, y: lastMove.y, button: 'none', click_count: 0, modifiers: lastMove.mods, delta_x: 0, delta_y: 0 });
      lastMove = null;
    });
  }

  function wheel(e: WheelEvent) {
    const at = point(e);
    if (!at) return;

    e.preventDefault();
    send({ type: 'mouse', event: 'wheel', ...at, button: 'none', click_count: 0, modifiers: modifiers(e), ...wheelDeltas(e) });
  }

  // Svelte attaches wheel listeners as passive, and a passive listener cannot stop the page from scrolling.
  $effect(() => {
    if (!region || !interactive) return;
    const target = region;
    target.addEventListener('wheel', wheel, { passive: false });

    return () => target.removeEventListener('wheel', wheel);
  });

  function keydown(e: KeyboardEvent) {
    if (e.isComposing || e.keyCode === 229) return;
    // Tab leaves the view (WCAG 2.1.2); "Send Tab" presses it in the page. Pasting is the paste event's.
    if (e.key === 'Tab') return;
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'v') return;

    e.preventDefault();
    const message = keyMessage(e, 'down');
    pressed.set(e.code, message);
    send(message);
  }

  function keyup(e: KeyboardEvent) {
    if (e.isComposing || e.keyCode === 229 || e.key === 'Tab') return;
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'v') return;

    e.preventDefault();
    pressed.delete(e.code);
    send(keyMessage(e, 'up'));
  }

  // Focus left (Tab, a click elsewhere): the keyup will go to someone else, so release what is held.
  function released() {
    for (const message of pressed.values()) {
      if (message.type === 'key') send({ ...message, event: 'up' });
    }
    pressed.clear();
  }

  function composed(e: CompositionEvent) {
    if (e.data) send({ type: 'text', text: e.data });
    if (sink) sink.value = '';
  }

  function paste(e: ClipboardEvent) {
    e.preventDefault();
    const text = e.clipboardData?.getData('text') ?? '';
    if (text) send({ type: 'text', text });
  }

  function sendTab() {
    const base = { type: 'key', key: 'Tab', code: 'Tab', key_code: 9, text: '', modifiers: 0 } as const;
    send({ ...base, event: 'down' });
    send({ ...base, event: 'up' });
  }

  function answerDialog(accept: boolean, promptText?: string) {
    send({ type: 'dialog_answer', accept, ...(promptText === undefined ? {} : { prompt_text: promptText }) });
  }
</script>

{#if status === 'closed'}
  <EmptyState title="This tab is closed" description="Nothing is left to show." />
{:else if status === 'unsupported'}
  <EmptyState title="Live view isn't available for this browser" description="This browser build can't stream its tabs." />
{:else if status === 'error'}
  <EmptyState tone="danger" title="Couldn't show the live view" description={errorMessage}>
    {#snippet action()}<Button onclick={() => attempt++}>Retry</Button>{/snippet}
  </EmptyState>
{:else}
  <div class="live">
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="frame"
      style:--aspect={aspect}
      bind:this={region}
      role={interactive ? 'application' : undefined}
      aria-label={interactive ? 'Live view' : undefined}
      onpointerdown={interactive ? pointerdown : undefined}
      onpointerup={interactive ? pointerup : undefined}
      onpointermove={interactive ? pointermove : undefined}
      oncontextmenu={interactive ? (e) => e.preventDefault() : undefined}
    >
      {#if src}
        <img bind:this={img} {src} alt="Live view of {title || 'the tab'}" draggable="false" />
      {:else}
        <div class="skeleton" aria-busy="true"><Skeleton width="100%" height="100%" radius="0" /></div>
      {/if}

      {#if interactive}
        <!-- The one tab stop of the view: it takes the keys, IME and paste. -->
        <textarea
          bind:this={sink}
          class="sink"
          aria-label="Live view, press keys to type into the page"
          aria-describedby={hintId}
          autocomplete="off"
          autocapitalize="off"
          spellcheck="false"
          onkeydown={keydown}
          onkeyup={keyup}
          onblur={released}
          oncompositionend={composed}
          onpaste={paste}
        ></textarea>
      {/if}
    </div>

    {#if interactive}
      <div class="hint-row">
        <p class="hint" id={hintId}>Tab moves focus out of the live view. Use Send Tab to press Tab in the page.</p>
        <Button variant="ghost" size="sm" onclick={sendTab}>Send Tab</Button>
      </div>
    {:else}
      <p class="hint">Take control to use this tab.</p>
    {/if}
    <p class="hint notice" role="status">{notice}</p>
  </div>

  <PageDialog {dialog} {interactive} onanswer={answerDialog} />
{/if}

<style>
  .live {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .frame {
    position: relative;
    /* Sized from the frame's own shape, so no bars surround the picture (a click in a bar would
       land on the page's edge). */
    width: min(100%, calc(70vh * var(--aspect)));
    aspect-ratio: var(--aspect);
    margin-inline: auto;
    overflow: hidden;
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    /* The frame is a picture of someone else's page: no selection, no drag, no touch scrolling. */
    user-select: none;
    touch-action: none;
  }

  .frame:focus-within {
    outline: 2px solid var(--focus-ring);
    outline-offset: 2px;
  }

  img {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }

  .skeleton {
    position: absolute;
    inset: 0;
  }

  .sink {
    position: absolute;
    top: 0;
    left: 0;
    width: 1px;
    height: 1px;
    padding: 0;
    border: 0;
    opacity: 0;
    resize: none;
    pointer-events: none;
  }

  .hint-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .hint {
    color: var(--text-tertiary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }

  .notice {
    color: var(--warning-fg);
  }

  /* Always mounted, so a screen reader announces text that arrives later. */
  .notice:empty {
    height: 0;
    margin-top: calc(var(--space-2) * -1);
    overflow: hidden;
  }
</style>
