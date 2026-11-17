import type { components } from '$lib/api/schema';
import { templateNameLabel } from '$lib/config/templates';
import { endReason } from '$lib/events/summary';
import { ACTIVE_STATES, ENDED_STATES, stateLabel } from '$lib/state';

type Instance = components['schemas']['Instance'];

export const FINAL_STATES = ENDED_STATES.join(',');

export const isFinal = (run: Instance) => !ACTIVE_STATES.includes(run.state);

export const runHref = (run: Instance) => (isFinal(run) ? `/history/${run.id}` : `/browsers/${run.id}`);

export const NOT_RECORDED = 'Not recorded for runs before this version';

// A run keeps only its Template's name; the friendly title of a built-in applies by name.
export const templateLabel = templateNameLabel;

export const atLeastZero = (ms: number) => Math.max(0, ms);

// Detaching is not a stop, so it is worded the same wherever a run's result shows.
export const runResult = (run: Instance) => (run.state === 'detached' ? 'Detached' : run.end_reason ? endReason(run.end_reason) : stateLabel(run.state));
