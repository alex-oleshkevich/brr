import createClient, { type Middleware } from 'openapi-fetch';
import type { paths } from './schema';
import { humanize } from './errors';
import { session } from './session';

export type ErrorBody = { code: string; message: string; details: Record<string, unknown> };

export class ApiError extends Error {
  readonly field?: string;

  constructor(
    public status: number,
    public code: string,
    message: string,
    public details: Record<string, unknown>,
  ) {
    const { message: text, field } = humanize(code, message);
    super(text);
    this.field = field;
    // A malformed id in the URL is the same thing to the user as an id that doesn't exist. Only the
    // bare path-id fields (`id`, `client_id`) map; body fields such as settings.build.build_id keep
    // the server's message.
    if (code === 'InvalidArgument' && (field === 'id' || field === 'client_id') && /not a valid/.test(message)) {
      this.code = 'NotFound';
      this.status = 404;
      this.message = 'This item does not exist.';
    }
  }

  // The daemon is down or unreachable, as opposed to it answering with an error.
  get offline(): boolean {
    return this.code === 'Unreachable';
  }
}

const clientHeader: Middleware = {
  onRequest({ request }) {
    if (session.clientId) request.headers.set('Brr-Client', session.clientId);
    return request;
  },
};

const unreachable = () => new ApiError(0, 'Unreachable', "Can't reach the brr daemon.", {});

// For responses that bypass openapi-fetch (the raw archive routes): the same ErrorBody, or the status text for a non-JSON body.
export async function readError(response: Response): Promise<ApiError> {
  const body = (await response.json().catch(() => null)) as Partial<ErrorBody> | null;
  return new ApiError(response.status, body?.code ?? 'HttpError', body?.message ?? response.statusText, body?.details ?? {});
}

export const api = createClient<paths>({ baseUrl: '' });
api.use(clientHeader);

export async function unwrap<T>(call: Promise<{ data?: T; error?: unknown; response: Response }>): Promise<T> {
  let result;
  try {
    result = await call;
  } catch {
    throw unreachable();
  }

  if (result.error === undefined) return result.data as T;

  const body = result.error as Partial<ErrorBody> | undefined;
  if (typeof body?.code !== 'string') throw unreachable();

  throw new ApiError(result.response.status, body.code, body.message ?? '', (body.details ?? {}) as Record<string, unknown>);
}
