export type Tone = 'neutral' | 'accent' | 'success' | 'warning' | 'danger';
export type StateKind = 'instance' | 'tab' | 'build' | 'login' | 'proxy';

const tones: Record<StateKind, Record<string, Tone>> = {
  instance: {
    queued: 'accent',
    starting: 'accent',
    attaching: 'accent',
    running: 'success',
    attached: 'success',
    idle: 'neutral',
    stopping: 'neutral',
    stopped: 'neutral',
    detached: 'neutral',
    reaping: 'warning',
    killed: 'warning',
    crashed: 'danger',
    failed: 'danger',
    lost: 'danger',
  },
  tab: {
    opening: 'accent',
    loading: 'accent',
    ready: 'success',
    idle: 'neutral',
    closing: 'neutral',
    closed: 'neutral',
    reaping: 'warning',
    awaiting_human: 'warning',
    crashed: 'danger',
  },
  build: { installing: 'accent', installed: 'success', registered: 'neutral', failed: 'danger' },
  login: { ok: 'success', failed: 'danger', unknown: 'neutral' },
  proxy: { healthy: 'success', unhealthy: 'danger', unchecked: 'neutral' },
};

export const ACTIVE_STATES = ['queued', 'starting', 'running', 'idle', 'reaping', 'stopping', 'killed', 'crashed', 'attaching', 'attached'];
export const ENDED_STATES = ['stopped', 'failed', 'detached', 'lost'];

export const knownStates = (kind: StateKind): string[] => Object.keys(tones[kind]);

export function stateTone(kind: StateKind, state: string): Tone {
  return tones[kind][state] ?? 'neutral';
}

export function stateLabel(state: string): string {
  const text = state.replaceAll('_', ' ');
  return text.charAt(0).toUpperCase() + text.slice(1);
}
