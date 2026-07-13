import { describe, it, expect, vi, beforeEach } from 'vitest';

// Mock the client module before importing messages.
vi.mock('@/api/client', () => {
  const ApiError = class ApiError extends Error {
    status: number;
    body?: unknown;
    constructor(status: number, message: string, body?: unknown) {
      super(message);
      this.name = 'ApiError';
      this.status = status;
      this.body = body;
    }
  };
  return {
    ApiError,
    request: vi.fn(),
    streamSse: vi.fn(),
    getToken: vi.fn(() => null),
    setToken: vi.fn(),
  };
});

import { request, streamSse, ApiError } from '@/api/client';
import { sendMessage, stopRun, streamSessionMessages } from '@/api/messages';
import type { MessageItem, RunEventPayload } from '@/api/types';

beforeEach(() => {
  vi.resetAllMocks();
});

describe('sendMessage', () => {
  it('posts exact query body', async () => {
    vi.mocked(request).mockResolvedValue(undefined);
    await sendMessage('sess-1', 'hello');
    expect(request).toHaveBeenCalledWith('/sessions/sess-1/messages', {
      method: 'POST',
      body: { query: [{ type: 'text', text: 'hello' }] },
    });
  });
});

describe('stopRun', () => {
  it('swallows 404', async () => {
    vi.mocked(request).mockRejectedValue(new ApiError(404, 'Not Found'));
    await expect(stopRun('sess-1')).resolves.toBeUndefined();
  });

  it('rethrows non-404 errors', async () => {
    vi.mocked(request).mockRejectedValue(new ApiError(500, 'Server Error'));
    await expect(stopRun('sess-1')).rejects.toBeInstanceOf(ApiError);
  });
});

describe('streamSessionMessages', () => {
  async function* makeStream(events: Array<{ event: string; data: string }>) {
    for (const ev of events) yield ev;
  }

  it('maps message frames correctly', async () => {
    const item: MessageItem = { seq: 1, message: { role: 'user', contents: [{ type: 'text', text: 'hi' }] }, created_at: '2026-01-01T00:00:00Z' };
    vi.mocked(streamSse).mockReturnValue(makeStream([
      { event: 'message', data: JSON.stringify(item) },
    ]));

    const ctrl = new AbortController();
    const results = [];
    for await (const ev of streamSessionMessages('s1', undefined, ctrl.signal)) {
      results.push(ev);
    }

    expect(results).toHaveLength(1);
    expect(results[0]).toEqual({ kind: 'message', item });
  });

  it('maps run frames correctly', async () => {
    const payload: RunEventPayload = { run: 'started' };
    vi.mocked(streamSse).mockReturnValue(makeStream([
      { event: 'run', data: JSON.stringify(payload) },
    ]));

    const ctrl = new AbortController();
    const results = [];
    for await (const ev of streamSessionMessages('s1', undefined, ctrl.signal)) {
      results.push(ev);
    }

    expect(results).toHaveLength(1);
    expect(results[0]).toEqual({ kind: 'run', payload });
  });

  it('maps title frames correctly', async () => {
    vi.mocked(streamSse).mockReturnValue(makeStream([
      { event: 'title', data: JSON.stringify({ title: 'My Chat' }) },
    ]));

    const ctrl = new AbortController();
    const results = [];
    for await (const ev of streamSessionMessages('s1', undefined, ctrl.signal)) {
      results.push(ev);
    }

    expect(results).toHaveLength(1);
    expect(results[0]).toEqual({ kind: 'title', title: 'My Chat' });
  });

  it('ignores unknown event names', async () => {
    vi.mocked(streamSse).mockReturnValue(makeStream([
      { event: 'unknown_type', data: '{"foo":"bar"}' },
    ]));

    const ctrl = new AbortController();
    const results = [];
    for await (const ev of streamSessionMessages('s1', undefined, ctrl.signal)) {
      results.push(ev);
    }
    expect(results).toHaveLength(0);
  });

  it('includes last_seq in path when defined', async () => {
    vi.mocked(streamSse).mockReturnValue(makeStream([]));

    const ctrl = new AbortController();
    // eslint-disable-next-line @typescript-eslint/no-unused-vars
    for await (const _ of streamSessionMessages('s1', 42, ctrl.signal)) { /* empty */ }

    expect(streamSse).toHaveBeenCalledWith(
      '/sessions/s1/messages/stream?last_seq=42',
      expect.objectContaining({ method: 'GET' }),
    );
  });
});
