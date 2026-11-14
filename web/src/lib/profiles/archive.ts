import { ApiError, readError } from '$lib/api/client';
import { session } from '$lib/api/session';
import type { components } from '$lib/api/schema';

// Archives go over the raw routes: the body is streamed, so they are not capped like the JSON operations.
// A header cannot carry arbitrary Unicode, so the passphrase travels as base64 of its UTF-8 bytes.
function headers(passphrase: string, extra: Record<string, string> = {}): Record<string, string> {
  const bytes = new TextEncoder().encode(passphrase);
  let binary = '';
  for (const byte of bytes) binary += String.fromCharCode(byte);
  const out: Record<string, string> = { 'X-Brr-Passphrase': btoa(binary), ...extra };
  if (session.clientId) out['Brr-Client'] = session.clientId;
  return out;
}

async function send(path: string, init: RequestInit): Promise<Response> {
  let response: Response;
  try {
    response = await fetch(path, { method: 'POST', ...init });
  } catch {
    throw new ApiError(0, 'Unreachable', "Can't reach the brr daemon.", {});
  }
  if (response.ok) return response;

  throw await readError(response);
}

export async function exportArchive(profileId: string, passphrase: string): Promise<{ blob: Blob; filename: string | null }> {
  const response = await send(`/v1/profiles/${encodeURIComponent(profileId)}/export/raw`, { headers: headers(passphrase) });
  const filename = response.headers.get('Content-Disposition')?.match(/filename="([^"]+)"/)?.[1] ?? null;
  return { blob: await response.blob(), filename };
}

export async function importArchive(file: File, passphrase: string): Promise<components['schemas']['Imported']> {
  const response = await send('/v1/profiles/import/raw', {
    headers: headers(passphrase, { 'Content-Type': 'application/octet-stream' }),
    body: file,
  });
  return (await response.json()) as components['schemas']['Imported'];
}
