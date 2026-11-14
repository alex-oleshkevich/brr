export type FrameMetadata = { deviceWidth: number; deviceHeight: number; pageScaleFactor: number; offsetTop: number };
export type DialogType = 'alert' | 'confirm' | 'prompt' | 'beforeunload';

export type ServerMsg =
  | { type: 'frame'; data: string; metadata: FrameMetadata }
  | { type: 'dialog'; dialog_type: DialogType; message: string; default_prompt: string }
  | { type: 'dialog_closed' }
  | { type: 'closed' }
  | { type: 'error'; code: string; message: string };

export type MouseButton = 'left' | 'middle' | 'right' | 'none';

export type MouseMsg = {
  type: 'mouse';
  event: 'down' | 'up' | 'move' | 'wheel';
  x: number;
  y: number;
  button: MouseButton;
  click_count: number;
  modifiers: number;
  delta_x: number;
  delta_y: number;
};

export type KeyMsg = { type: 'key'; event: 'down' | 'up'; key: string; code: string; key_code: number; text: string; modifiers: number };
export type TextMsg = { type: 'text'; text: string };
export type DialogAnswerMsg = { type: 'dialog_answer'; accept: boolean; prompt_text?: string };
export type ClientMsg = MouseMsg | KeyMsg | TextMsg | DialogAnswerMsg;

const ALT = 1;
const CTRL = 2;
const META = 4;
const SHIFT = 8;
const LINE_PX = 16;

const MAX_DIALOG_CHARS = 2000;

// The frame's pixels under a pointer. The image is drawn with object-fit: contain, so a box that
// is wider or taller than the frame leaves bars the pointer must not count.
export function toFramePoint(
  clientX: number,
  clientY: number,
  rect: { left: number; top: number; width: number; height: number },
  natural: { w: number; h: number },
): { x: number; y: number } {
  const scale = Math.min(rect.width / natural.w, rect.height / natural.h);
  const offsetX = (rect.width - natural.w * scale) / 2;
  const offsetY = (rect.height - natural.h * scale) / 2;
  const x = (clientX - rect.left - offsetX) / scale;
  const y = (clientY - rect.top - offsetY) / scale;

  return { x: Math.min(Math.max(x, 0), natural.w), y: Math.min(Math.max(y, 0), natural.h) };
}

export function modifiers(e: KeyboardEvent | MouseEvent | WheelEvent): number {
  return (e.altKey ? ALT : 0) | (e.ctrlKey ? CTRL : 0) | (e.metaKey ? META : 0) | (e.shiftKey ? SHIFT : 0);
}

export function keyMessage(e: KeyboardEvent, event: 'down' | 'up'): KeyMsg {
  // A shortcut types nothing: Ctrl+A must select, not insert an "a". AltGr reports Ctrl and Alt together.
  const shortcut = (e.ctrlKey || e.metaKey) && !e.altKey;
  let text = '';
  if (e.key === 'Enter') text = '\r';
  else if (e.key.length === 1 && !shortcut) text = e.key;

  return { type: 'key', event, key: e.key, code: e.code, key_code: e.keyCode, text, modifiers: modifiers(e) };
}

export function mouseButton(button: number): MouseButton {
  return button === 0 ? 'left' : button === 1 ? 'middle' : button === 2 ? 'right' : 'none';
}

export function wheelDeltas(e: WheelEvent): { delta_x: number; delta_y: number } {
  const unit = e.deltaMode === WheelEvent.DOM_DELTA_LINE ? LINE_PX : 1;

  return { delta_x: e.deltaX * unit, delta_y: e.deltaY * unit };
}

export const capped = (text: string): string => text.slice(0, MAX_DIALOG_CHARS);

export function screencastUrl(tabId: string, clientId: string): string {
  const scheme = location.protocol === 'https:' ? 'wss' : 'ws';

  return `${scheme}://${location.host}/v1/tabs/${tabId}/screencast?client_id=${encodeURIComponent(clientId)}`;
}
