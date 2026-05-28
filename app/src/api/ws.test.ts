import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { getMe } from './auth';

// Mock the entire client module — ws.ts imports getToken and notifyUnauthorized from it.
vi.mock('./client', () => ({
  getBaseUrl: () => 'http://localhost:8080',
  getToken: vi.fn(() => 'test-token'),
  notifyUnauthorized: vi.fn(),
  ApiError: class ApiError extends Error {
    constructor(public status: number, message: string, public body?: unknown) {
      super(message);
    }
  },
}));

// Mock the auth module so getMe can be controlled per test.
vi.mock('./auth', () => ({
  getMe: vi.fn(),
}));

import { getToken, notifyUnauthorized } from './client';
import { AppWebSocketManager } from './ws';

// WebSocket mock
class MockWebSocket {
  static OPEN = 1;
  static CONNECTING = 0;
  readyState = MockWebSocket.CONNECTING;
  onopen: (() => void) | null = null;
  onclose: ((evt: { code: number; reason: string }) => void) | null = null;
  onmessage: ((evt: { data: string }) => void) | null = null;
  onerror: (() => void) | null = null;
  close() { this.readyState = 3; }
}

let wsInstances: MockWebSocket[] = [];
const OriginalWebSocket = globalThis.WebSocket;

beforeEach(async () => {
  wsInstances = [];
  vi.useFakeTimers();
  vi.mocked(notifyUnauthorized).mockClear();
  vi.mocked(getToken).mockReturnValue('test-token');
  // Default: getMe rejects with 401 so rapid-close tests still see notifyUnauthorized called.
  const { ApiError } = await import('./client');
  vi.mocked(getMe).mockRejectedValue(new ApiError(401, 'invalid token', { error: 'invalid token' }));
  // @ts-expect-error mock
  globalThis.WebSocket = class extends MockWebSocket {
    constructor() {
      super();
      wsInstances.push(this);
    }
  };
  // @ts-expect-error assigning to readonly static for mock purposes
  globalThis.WebSocket.OPEN = 1;
  // @ts-expect-error assigning to readonly static for mock purposes
  globalThis.WebSocket.CONNECTING = 0;
});

afterEach(() => {
  globalThis.WebSocket = OriginalWebSocket;
  vi.useRealTimers();
});

describe('AppWebSocketManager rapid-close heuristic', () => {
  it('accumulates rapidCloseCount across reconnect cycles when connections are short-lived', async () => {
    const mgr = new AppWebSocketManager();
    mgr.connect('token');

    // 1st connection — closes after 500ms (below 2000ms threshold)
    expect(wsInstances).toHaveLength(1);
    vi.advanceTimersByTime(500);
    wsInstances[0].onclose?.({ code: 1006, reason: '' });

    // wait out the 3000ms reconnect timer
    vi.advanceTimersByTime(3000);
    expect(wsInstances).toHaveLength(2);

    // 2nd connection — closes after 500ms
    vi.advanceTimersByTime(500);
    wsInstances[1].onclose?.({ code: 1006, reason: '' });

    vi.advanceTimersByTime(3000);
    expect(wsInstances).toHaveLength(3);

    // 3rd connection — closes after 500ms → reaches RAPID_CLOSE_MAX (3)
    vi.advanceTimersByTime(500);
    wsInstances[2].onclose?.({ code: 1006, reason: '' });

    // flush microtask so the async getMe() rejection is processed
    await Promise.resolve();

    expect(notifyUnauthorized).toHaveBeenCalledOnce();
  });

  it('resets rapidCloseCount when a connection lives longer than threshold', () => {
    const mgr = new AppWebSocketManager();
    mgr.connect('token');

    // 1st connection — short-lived
    vi.advanceTimersByTime(500);
    wsInstances[0].onclose?.({ code: 1006, reason: '' });

    // 2nd connection — lives for 3000ms (> RAPID_CLOSE_THRESHOLD_MS=2000ms → counter resets)
    vi.advanceTimersByTime(3000); // exhaust the reconnect timer
    vi.advanceTimersByTime(3000); // this connection's lifetime = 3000ms > 2000ms threshold
    wsInstances[1].onclose?.({ code: 1006, reason: '' });

    // 3rd connection — short-lived again (count starts from 1, not 3)
    vi.advanceTimersByTime(3000);
    vi.advanceTimersByTime(500);
    wsInstances[2].onclose?.({ code: 1006, reason: '' });

    // counter is only 1 — notifyUnauthorized must not have been called
    expect(notifyUnauthorized).not.toHaveBeenCalled();
  });

  it('does NOT trigger logout for close code 1008 (proxy policy violation)', () => {
    const mgr = new AppWebSocketManager();
    mgr.connect('token');

    wsInstances[0].onclose?.({ code: 1008, reason: '' });

    // counter is 1, but RAPID_CLOSE_MAX not reached — notifyUnauthorized must not fire
    expect(notifyUnauthorized).not.toHaveBeenCalled();
  });

  it('stops reconnecting for close code 4401 (auth close code)', () => {
    const mgr = new AppWebSocketManager();
    mgr.connect('token');
    wsInstances[0].onclose?.({ code: 4401, reason: '' });
    // active is set to false — no further reconnect timer
    vi.advanceTimersByTime(10000);
    expect(wsInstances).toHaveLength(1); // no reconnect happened
  });
});

describe('AppWebSocketManager auth close classification', () => {
  it('calls notifyUnauthorized with expired body when getMe returns 401 "token has expired"', async () => {
    const { ApiError } = await import('./client');
    vi.mocked(getMe).mockRejectedValue(new ApiError(401, 'token has expired', { error: 'token has expired' }));

    const mgr = new AppWebSocketManager();
    mgr.connect('token');

    wsInstances[0].onclose?.({ code: 4401, reason: '' });

    await Promise.resolve();

    expect(notifyUnauthorized).toHaveBeenCalledWith(401, { error: 'token has expired' });
  });

  it('calls notifyUnauthorized with invalid body when getMe returns generic 401', async () => {
    const { ApiError } = await import('./client');
    vi.mocked(getMe).mockRejectedValue(new ApiError(401, 'unauthorized', { error: 'unauthorized' }));

    const mgr = new AppWebSocketManager();
    mgr.connect('token');

    wsInstances[0].onclose?.({ code: 4401, reason: '' });

    await Promise.resolve();

    expect(notifyUnauthorized).toHaveBeenCalledWith(401, { error: 'unauthorized' });
  });

  it('does NOT call notifyUnauthorized when getMe succeeds (WS flap, not auth failure)', async () => {
    vi.mocked(getMe).mockResolvedValue({ id: '1', username: 'test', displayName: 'Test' } as any);

    const mgr = new AppWebSocketManager();
    mgr.connect('token');

    wsInstances[0].onclose?.({ code: 4401, reason: '' });

    await Promise.resolve();

    expect(notifyUnauthorized).not.toHaveBeenCalled();
  });
});
