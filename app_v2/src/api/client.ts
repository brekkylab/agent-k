// Single typed fetch wrapper for backend_v2.
// Concerns: base URL configuration, auth header injection, JSON parse, typed errors.
// All endpoint modules call request<T>() — there is no other fetch in the app.

const TOKEN_KEY = 'agentk.v2.token';
export const BASE_URL = import.meta.env.VITE_BACKEND_URL ?? 'http://127.0.0.1:8080';

// Surgery (a): reauth-once-and-retry handler instead of unauthorized banner.
// Concurrent-401 dedup lives in auth.ts's shared inflight promise.
let reauthHandler: (() => Promise<boolean>) | null = null;

export function setReauthHandler(fn: () => Promise<boolean>): void {
  reauthHandler = fn;
}

/** Trigger the reauth handler once and return its result. Used by workspace.ts. */
export async function reauthOnce(): Promise<boolean> {
  if (!reauthHandler) return false;
  setToken(null);
  const ok = await reauthHandler();
  return ok;
}

export class ApiError extends Error {
  constructor(public status: number, message: string, public body?: unknown) {
    super(message);
    this.name = 'ApiError';
  }
}

function readStored(key: string): string | null {
  try { return window.localStorage.getItem(key); } catch { return null; }
}

function writeStored(key: string, value: string | null): void {
  try {
    if (value == null) window.localStorage.removeItem(key);
    else window.localStorage.setItem(key, value);
  } catch { /* noop */ }
}

export function getToken(): string | null {
  return readStored(TOKEN_KEY);
}

export function setToken(token: string | null): void {
  writeStored(TOKEN_KEY, token);
}

interface RequestOptions extends Omit<RequestInit, 'body'> {
  body?: BodyInit | object | null;
  skipAuth?: boolean;
  isForm?: boolean;
  /** Internal flag to prevent infinite retry loops. */
  _isRetry?: boolean;
}

export async function request<T = unknown>(path: string, options: RequestOptions = {}): Promise<T> {
  const { body, skipAuth, isForm, _isRetry, headers: headerInit, ...rest } = options;
  const headers = new Headers(headerInit);

  if (!skipAuth) {
    const token = getToken();
    if (token) headers.set('Authorization', `Bearer ${token}`);
  }

  let resolvedBody: BodyInit | null | undefined;
  if (body == null) {
    resolvedBody = undefined;
  } else if (isForm || body instanceof FormData) {
    resolvedBody = body as BodyInit;
  } else if (body instanceof ArrayBuffer || body instanceof Blob || typeof body === 'string') {
    resolvedBody = body as BodyInit;
  } else {
    if (!headers.has('Content-Type')) headers.set('Content-Type', 'application/json');
    resolvedBody = JSON.stringify(body);
  }

  const response = await fetch(`${BASE_URL}${path}`, {
    ...rest,
    headers,
    body: resolvedBody,
  });

  if (!response.ok) {
    const raw = await response.text().catch(() => '');
    let parsed: unknown;
    try { parsed = raw ? JSON.parse(raw) : undefined; } catch { parsed = raw; }
    const msg = typeof parsed === 'object' && parsed && 'error' in parsed
      ? String((parsed as Record<string, unknown>).error)
      : (raw || `${response.status} ${response.statusText}`);

    // Surgery (a): on 401, attempt reauth once and retry.
    if (response.status === 401 && !skipAuth && !_isRetry && reauthHandler) {
      setToken(null);
      const ok = await reauthHandler();
      if (ok) {
        return request<T>(path, { ...options, _isRetry: true });
      }
    }

    throw new ApiError(response.status, msg, parsed);
  }

  if (response.status === 204) return undefined as T;
  const text = await response.text();
  if (!text) return undefined as T;
  return JSON.parse(text) as T;
}

// SSE streaming helper — Surgery (b): generalized for GET and POST.
export interface SseEvent {
  event: string;
  data: string;
}

export interface StreamSseOptions {
  method?: 'GET' | 'POST';  // default 'GET'
  body?: object;             // when set: Content-Type: application/json + JSON.stringify
  signal?: AbortSignal;
}

export async function* streamSse(
  path: string,
  options: StreamSseOptions = {},
): AsyncGenerator<SseEvent, void, void> {
  const { method = 'GET', body, signal } = options;
  const headers = new Headers({ Accept: 'text/event-stream' });

  const token = getToken();
  if (token) headers.set('Authorization', `Bearer ${token}`);

  // Only set Content-Type when there is a body to send.
  let resolvedBody: string | undefined;
  if (body != null) {
    headers.set('Content-Type', 'application/json');
    resolvedBody = JSON.stringify(body);
  }

  const doFetch = (overrideToken?: string): Promise<Response> => {
    if (overrideToken) headers.set('Authorization', `Bearer ${overrideToken}`);
    return fetch(`${BASE_URL}${path}`, { method, headers, body: resolvedBody, signal });
  };

  let response = await doFetch();

  // On 401, reauth once and retry.
  if (response.status === 401 && reauthHandler) {
    setToken(null);
    const ok = await reauthHandler();
    if (!ok) {
      const raw = await response.text().catch(() => '');
      throw new ApiError(401, raw || '401 Unauthorized');
    }
    const newToken = getToken();
    response = await doFetch(newToken ?? undefined);
  }

  if (!response.ok || !response.body) {
    const raw = await response.text().catch(() => '');
    throw new ApiError(response.status, raw || `${response.status} ${response.statusText}`);
  }

  const reader = response.body.getReader();
  const decoder = new TextDecoder('utf-8');
  let buffer = '';

  while (true) {
    const { value, done } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });

    // SSE frames are separated by a blank line.
    let separator = buffer.indexOf('\n\n');
    while (separator !== -1) {
      const frame = buffer.slice(0, separator);
      buffer = buffer.slice(separator + 2);
      const parsed = parseFrame(frame);
      if (parsed) yield parsed;
      separator = buffer.indexOf('\n\n');
    }
  }
}

function parseFrame(frame: string): SseEvent | null {
  const lines = frame.split('\n');
  let event = 'message';
  const dataLines: string[] = [];
  for (const line of lines) {
    if (!line || line.startsWith(':')) continue;
    if (line.startsWith('event:')) event = line.slice(6).trim();
    else if (line.startsWith('data:')) dataLines.push(line.slice(5).trimStart());
  }
  if (dataLines.length === 0) return null;
  return { event, data: dataLines.join('\n') };
}
