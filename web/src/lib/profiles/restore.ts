export const MAX_NAME_LEN = 64;

const NAME_CHARS = /^[A-Za-z0-9._-]+$/;
const ULID = /^[0-9A-HJKMNP-TV-Za-hjkmnp-tv-z]{26}$/;

// An id-shaped name would be taken for an id by the id-or-name lookups, so the daemon rejects it too.
export function profileNameError(name: string): string | null {
  const valid = name.length >= 1 && name.length <= MAX_NAME_LEN && NAME_CHARS.test(name) && !ULID.test(name);
  return valid ? null : `Use 1 to ${MAX_NAME_LEN} characters from A-Z, a-z, 0-9, ".", "_" and "-".`;
}

// Profile names allow letters, digits and . _ - only, so the date goes in as an ISO stamp.
export function restoredName(profileName: string, createdAt: number): string {
  const suffix = `-restored-${new Date(createdAt).toISOString().slice(0, 10)}`;
  return `${profileName.slice(0, MAX_NAME_LEN - suffix.length)}${suffix}`;
}
