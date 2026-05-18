// Single typed fetch wrapper for backend-v2.
// Concerns: base URL configuration, auth header injection, JSON parse, typed errors.
// All endpoint modules call request<T>() — there is no other fetch in the app.

const BASE_URL_KEY = 'cowork.v2.baseUrl';
const TOKEN_KEY = 'cowork.v2.token';
const DEFAULT_BASE_URL = import.meta.env.VITE_BACKEND_V2_URL ?? 'http://127.0.0.1:8080';

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

export function getBaseUrl(): string {
  return readStored(BASE_URL_KEY) ?? DEFAULT_BASE_URL;
}

export function setBaseUrl(url: string): string {
  const trimmed = url.replace(/\/+$/, '') || DEFAULT_BASE_URL;
  writeStored(BASE_URL_KEY, trimmed);
  return trimmed;
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
}

export async function request<T = unknown>(path: string, options: RequestOptions = {}): Promise<T> {
  const { body, skipAuth, isForm, headers: headerInit, ...rest } = options;
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

  const response = await fetch(`${getBaseUrl()}${path}`, {
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
    throw new ApiError(response.status, msg, parsed);
  }

  if (response.status === 204) return undefined as T;
  const text = await response.text();
  if (!text) return undefined as T;
  return JSON.parse(text) as T;
}

// SSE streaming helper for POST /sessions/{id}/messages/stream.
// Returns an async iterable of parsed events. Caller controls lifetime via AbortController.
export interface SseEvent {
  event: string;
  data: string;
}

export async function* streamSse(
  path: string,
  body: object,
  signal?: AbortSignal,
): AsyncGenerator<SseEvent, void, void> {
  const headers = new Headers({ 'Content-Type': 'application/json', Accept: 'text/event-stream' });
  const token = getToken();
  if (token) headers.set('Authorization', `Bearer ${token}`);

  const response = await fetch(`${getBaseUrl()}${path}`, {
    method: 'POST',
    headers,
    body: JSON.stringify(body),
    signal,
  });

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
