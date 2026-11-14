import { goto } from '$app/navigation';
import { ApiError, api, unwrap } from '$lib/api/client';
import { ACTIVE_STATES } from '$lib/state';
import { toast } from '$lib/ui/toast.svelte';

export type LaunchBody = {
  priority: number;
  admission: 'queue' | 'reject' | 'evict';
  exclusive: boolean;
  display?: 'headless' | 'visible' | 'virtual';
  url?: string;
};

// A launch that is still pending after this long is probably waiting in the admission queue.
const QUEUE_CHECK_MS = 1200;
const QUEUE_POLL_MS = 3000;
const WAITING_TOAST_MS = 120_000;
const LAST_PROFILE_KEY = 'brr.lastLaunchedProfile';

export const lastLaunchedProfile = (): string | null => localStorage.getItem(LAST_PROFILE_KEY);

// Toasts have no handle, so the id of the one just shown is read back from the stack.
function show(options: Parameters<typeof toast.show>[0]): number {
  toast.show(options);
  return toast.items.at(-1)!.id;
}

async function failureToast(error: unknown, name: string) {
  if (!(error instanceof ApiError)) return toast.apiError(error, `Couldn't launch ${name}`);

  const reason = (error.details as { reason?: string }).reason;
  if (error.code === 'CapExceeded') {
    const [limits, active] = await Promise.all([
      unwrap(api.GET('/v1/limits')).catch(() => null),
      unwrap(api.GET('/v1/instances', { params: { query: { state: ACTIVE_STATES.join(','), limit: 200 } } })).catch(() => null),
    ]);
    const atMax = limits && active && active.instances.length >= limits.max_instances;
    const description = atMax
      ? `Max browsers (${limits.max_instances}) reached. Stop a running browser or raise the limit in Capacity.`
      : `${error.message} Check the limits in Capacity.`;
    return toast.show({ tone: 'danger', title: `Couldn't launch ${name}`, description, action: { label: 'Open Capacity', onclick: () => void goto('/limits') } });
  }

  if (error.code === 'Conflict' && reason === 'profile_locked') {
    const running = await unwrap(api.GET('/v1/instances', { params: { query: { profile: name, state: ACTIVE_STATES.join(','), limit: 1 } } })).catch(() => null);
    const target = running?.instances[0];
    return toast.show({
      tone: 'danger',
      title: `${name} is already running`,
      description: 'A persistent profile can only run one browser at a time.',
      action: target ? { label: 'Open running browser', onclick: () => void goto('/instances/' + target.id) } : undefined,
    });
  }

  toast.apiError(error, `Couldn't launch ${name}`);
}

// Starts the launch and follows it with toasts, so the dialog never has to stay open while it waits in the queue.
export function launchProfile(profile: { id: string; name: string }, body: LaunchBody): void {
  localStorage.setItem(LAST_PROFILE_KEY, profile.id);
  const name = profile.name;
  const startedAt = Date.now();
  let toastId = show({ tone: 'neutral', title: `Starting ${name}…`, timeoutMs: WAITING_TOAST_MS });
  let queuedId: string | null = null;
  let cancelled = false;
  let position: number | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;

  const cancel = async () => {
    if (!queuedId) return;
    cancelled = true;
    try {
      await unwrap(api.POST('/v1/instances/{instance_id}/stop', { params: { path: { instance_id: queuedId } }, body: {} }));
      toast.success(`Cancelled the launch of ${name}`);
    } catch (e) {
      cancelled = false;
      toast.apiError(e, "Couldn't cancel the launch");
    }
  };

  async function poll() {
    try {
      const { queue } = await unwrap(api.GET('/v1/admission/queue'));
      const mine = queue.filter((q) => q.profile_name === name && q.queued_at >= startedAt - 1000).at(-1);
      if (mine && (mine.queue_pos !== position || mine.instance_id !== queuedId)) {
        queuedId = mine.instance_id;
        position = mine.queue_pos;
        toast.dismiss(toastId);
        toastId = show({
          tone: 'warning',
          title: `${name} is waiting to start (#${mine.queue_pos})`,
          description: 'It starts when a slot frees up. You can keep working.',
          action: { label: 'Cancel launch', onclick: () => void cancel() },
          timeoutMs: WAITING_TOAST_MS,
        });
      }
    } catch {
      // The queue view is a courtesy; the launch result below is what matters.
    }
    timer = setTimeout(poll, QUEUE_POLL_MS);
  }
  timer = setTimeout(poll, QUEUE_CHECK_MS);

  void unwrap(api.POST('/v1/profiles/{profile}/launch', { params: { path: { profile: profile.id } }, body })).then(
    (result) => {
      clearTimeout(timer);
      toast.dismiss(toastId);
      toast.show({
        tone: 'success',
        title: `Started ${name}`,
        description: "You're in control of its first tab. Release it so agents can use this browser.",
        action: { label: 'Open', onclick: () => void goto('/instances/' + result.instance.id) },
      });
    },
    async (error: unknown) => {
      clearTimeout(timer);
      toast.dismiss(toastId);
      if (!cancelled) await failureToast(error, name);
    },
  );
}
