import { api, unwrap } from '$lib/api/client';
import type { components } from '$lib/api/schema';
import { endReason, plainError } from '$lib/events/summary';
import { ACTIVE_STATES, ENDED_STATES, type Tone } from '$lib/state';
import { refocusIfLost } from '$lib/ui/focus';
import { toast } from '$lib/ui/toast.svelte';

type Instance = components['schemas']['Instance'];

export const profileLabel = (instance: Instance) => (instance.kind === 'live' ? 'Live browser' : (instance.profile_name ?? 'Browser'));

// brr never stops a Live browser (spec §10 V3).
export const canStop = (instance: Instance) => instance.kind === 'managed' && ACTIVE_STATES.includes(instance.state);

export const canPromote = (instance: Instance) =>
  instance.kind === 'managed' && instance.ephemeral && ['running', 'idle'].includes(instance.state);

export const hasEnded = (instance: Instance) => ENDED_STATES.includes(instance.state);

// A one-off browser's profile is brr's own, so only a managed browser of a user's profile can be started again.
export const canRelaunch = (instance: Instance) =>
  instance.kind === 'managed' && !instance.ephemeral && instance.profile_id != null && (hasEnded(instance) || instance.state === 'crashed');

const START_FAILURES = ['launch_error', 'start_timeout'];

// What the badge says when the state alone would mislead: a queued launch has not started, a cancelled or timed-out one never did, a crash or failed start is red.
export function outcome(instance: Instance): { label: string; tone: Tone } | null {
  if (instance.state === 'queued') return { label: 'Waiting to start', tone: 'accent' };
  if (instance.state === 'crashed' || instance.end_reason === 'crashed') return { label: 'Crashed', tone: 'danger' };
  if (instance.state === 'failed' || (instance.end_reason && START_FAILURES.includes(instance.end_reason))) {
    return { label: 'Failed to start', tone: 'danger' };
  }
  if (instance.end_reason === 'cancel') return { label: 'Cancelled', tone: 'neutral' };
  if (instance.end_reason === 'queue_timeout') return { label: 'Never started', tone: 'neutral' };

  return null;
}

// The Ended list's Reason column: why it ended, and for a failure the specific cause.
export function whyEnded(instance: Instance): string {
  if (!instance.end_reason) return '—';

  const reason = endReason(instance.end_reason);

  return instance.error ? `${reason}: ${plainError(instance.error)}` : reason;
}

// The confirmation lives in StopDialog; this only stops.
export async function stopInstance(instance: Instance): Promise<boolean> {
  try {
    await unwrap(api.POST('/v1/instances/{instance_id}/stop', { params: { path: { instance_id: instance.id } }, body: {} }));
    toast.success(`Stopping ${profileLabel(instance)}`);
    refocusIfLost('main');
    return true;
  } catch (e) {
    toast.apiError(e, "Couldn't stop the browser");
    return false;
  }
}
