// Turns the daemon's developer-facing error text into copy for the UI.

export const sentence = (text: string): string => text.charAt(0).toUpperCase() + text.slice(1);

const BACKTICKS = /`/g;
const FIELD_PREFIX = /^invalid `([^`]+)`: /;
// The daemon's CLI hints ("run brr build install …") mean nothing in the UI.
const CLI_HINT = /[;,:.]?\s*(?:run|use|with)\s+`?brr\s[^;]*$/i;
const NOT_A_NUMBER = /invalid value: (?:integer|float|string) [^,]*, expected [iu]\d+/;
const OFFLINE = "Can't reach the brr daemon.";

// "idle.tab_min" -> "Tab min"; the last path segment is the one the user sees.
export function fieldLabel(field: string): string {
  const name = field.split('.').pop() ?? field;
  return sentence(name.replaceAll('_', ' '));
}

export type Humanized = { message: string; field?: string };

// Strips the nested `invalid `a`: invalid `b`: …` prefixes and keeps the innermost field and reason.
export function humanize(code: string, raw: string): Humanized {
  let message = raw;
  let field: string | undefined;
  for (let match = FIELD_PREFIX.exec(message); match; match = FIELD_PREFIX.exec(message)) {
    field = match[1];
    message = message.slice(match[0].length);
  }

  if (code === 'BuildMissing') {
    const reason = message.replace(/^build missing: /, '').replace(CLI_HINT, '').trim();
    message = `${reason ? sentence(reason.replace(/[.]$/, '')) : 'No matching browser build is installed'}. Install one on the Builds page.`;
  } else if (NOT_A_NUMBER.test(message)) {
    message = 'Enter a whole number of 0 or more.';
  } else if (code === 'StorageUnavailable') {
    message = message.replace(/^storage backend unavailable: /, '');
  }

  message = sentence(message.replace(BACKTICKS, '').trim());
  if (code === 'Unreachable') message = OFFLINE;
  return { message, field };
}
