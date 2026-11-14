import { api, unwrap } from '$lib/api/client';
import { toast } from '$lib/ui/toast.svelte';

// Asks the daemon to look for system browsers and tells the user what turned up; null when the request failed.
export async function detectBrowsers(known: readonly { id: string }[] = []) {
  try {
    const { builds } = await unwrap(api.POST('/v1/builds/detect', { body: {} }));
    const knownIds = new Set(known.map((b) => b.id));
    const fresh = builds.filter((b) => !knownIds.has(b.id)).length;
    toast.success(fresh > 0 ? `Found ${fresh} browser version${fresh === 1 ? '' : 's'}` : 'No new browser versions found');

    return builds;
  } catch (e) {
    toast.apiError(e, "Couldn't detect system browsers");

    return null;
  }
}
