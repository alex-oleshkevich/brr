import { redirect } from '@sveltejs/kit';

// Templates are a tab inside Profiles; this path keeps old links working.
export function load() {
  redirect(307, '/profiles?tab=templates');
}
