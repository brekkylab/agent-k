import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { request, setToken, setReauthHandler, streamSse } from '../client';

// Reset module-level state between tests.
beforeEach(() => {
  localStorage.clear();
  setToken(null);
  setReauthHandler(async () => false);
  vi.restoreAllMocks();
});

afterEach(() => {
  vi.restoreAllMocks();
});

function makeJsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

describe('request(): 401 retry with reauth', () => {
  it('calls reauth handler on 401, retries with new token, succeeds', async () => {
    const newToken = 'fresh-token';
    const reauth = vi.fn(async () => {
      setToken(newToken);
      return true;
    });
    setReauthHandler(reauth);

    const fetchMock = vi.fn()
      .mockResolvedValueOnce(makeJsonResponse({ error: 'unauthorized' }, 401))
      .mockResolvedValueOnce(makeJsonResponse({ ok: true }, 200));
    vi.stubGlobal('fetch', fetchMock);

    const result = await request<{ ok: boolean }>('/test');
    expect(result.ok).toBe(true);
    expect(reauth).toHaveBeenCalledOnce();
    expect(fetchMock).toHaveBeenCalledTimes(2);
    // Second call should have the new token.
    const secondCallHeaders = fetchMock.mock.calls[1][1].headers as Headers;
    expect(secondCallHeaders.get('Authorization')).toBe(`Bearer ${newToken}`);
  });

  it('throws ApiError(401) when reauth returns false', async () => {
    setReauthHandler(async () => false);
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(makeJsonResponse({ error: 'bad' }, 401)));

    await expect(request('/test')).rejects.toMatchObject({ status: 401 });
  });
});

describe('streamSse()', () => {
  it('yields named events, skips comment-only frames', async () => {
    const sseBody =
      'event: message\ndata: {"hello":"world"}\n\n' +
      ': ping\n\n' +
      'event: run\ndata: {"run":"idle"}\n\n';

    const encoder = new TextEncoder();
    const stream = new ReadableStream({
      start(controller) {
        controller.enqueue(encoder.encode(sseBody));
        controller.close();
      },
    });

    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(
      new Response(stream, { status: 200, headers: { 'Content-Type': 'text/event-stream' } }),
    ));

    const events: Array<{ event: string; data: string }> = [];
    for await (const ev of streamSse('/stream')) {
      events.push(ev);
    }

    expect(events).toHaveLength(2);
    expect(events[0]).toEqual({ event: 'message', data: '{"hello":"world"}' });
    expect(events[1]).toEqual({ event: 'run', data: '{"run":"idle"}' });
  });

  it('sends Accept: text/event-stream and Bearer header, no Content-Type when no body', async () => {
    setToken('my-token');
    const stream = new ReadableStream({ start(c) { c.close(); } });
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(stream, { status: 200 }),
    );
    vi.stubGlobal('fetch', fetchMock);

    // Consume the generator.
    // eslint-disable-next-line @typescript-eslint/no-unused-vars
    for await (const _ of streamSse('/stream')) { /* noop */ }

    const [, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    const headers = init.headers as Headers;
    expect(headers.get('Accept')).toBe('text/event-stream');
    expect(headers.get('Authorization')).toBe('Bearer my-token');
    expect(headers.get('Content-Type')).toBeNull();
  });

  it('retries once after 401 via reauth handler', async () => {
    const newToken = 'fresh-stream-token';
    const reauth = vi.fn(async () => {
      setToken(newToken);
      return true;
    });
    setReauthHandler(reauth);

    const sseBody = 'event: message\ndata: {"seq":1}\n\n';
    const encoder = new TextEncoder();
    const successStream = new ReadableStream({
      start(controller) {
        controller.enqueue(encoder.encode(sseBody));
        controller.close();
      },
    });

    const fetchMock = vi.fn()
      .mockResolvedValueOnce(new Response('Unauthorized', { status: 401 }))
      .mockResolvedValueOnce(new Response(successStream, { status: 200 }));
    vi.stubGlobal('fetch', fetchMock);

    const events: Array<{ event: string; data: string }> = [];
    for await (const ev of streamSse('/stream')) {
      events.push(ev);
    }

    expect(events).toHaveLength(1);
    expect(events[0]).toEqual({ event: 'message', data: '{"seq":1}' });
    expect(reauth).toHaveBeenCalledOnce();
    expect(fetchMock).toHaveBeenCalledTimes(2);
    // Second call should have the new token.
    const secondCallHeaders = fetchMock.mock.calls[1][1].headers as Headers;
    expect(secondCallHeaders.get('Authorization')).toBe(`Bearer ${newToken}`);
  });
});
