const TICK_MS = 15_000;
const SECOND_MS = 1000;

class Clock {
  now = $state(Date.now());
}

export const clock = new Clock();

setInterval(() => {
  if (document.visibilityState === 'visible') clock.now = Date.now();
}, TICK_MS);

// A one-second clock for a countdown on screen; the shared one above ticks too slowly to count down.
export function everySecond(onTick: (now: number) => void): () => void {
  onTick(Date.now());
  const timer = setInterval(() => onTick(Date.now()), SECOND_MS);

  return () => clearInterval(timer);
}
