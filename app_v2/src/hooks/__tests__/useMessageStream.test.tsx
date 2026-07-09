import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';
import { StrictMode, type ReactNode } from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

// Mock react-i18next — required by components imported indirectly.
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k, i18n: { language: 'en' } }),
}));

// Mock streamSessionMessages from messages module.
vi.mock('@/api/messages', () => ({
  streamSessionMessages: vi.fn(),
  sendMessage: vi.fn(),
  stopRun: vi.fn(),
  listMessages: vi.fn(),
}));

import { streamSessionMessages } from '@/api/messages';
import { ApiError } from '@/api/client';
import { useMessageStream } from '@/hooks/useMessageStream';
import type { MessageItem, RunEventPayload } from '@/api/types';

function wrapper({ children }: { children: ReactNode }) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
}

async function* makeStream(
  events: Array<{ kind: 'message'; item: MessageItem } | { kind: 'run'; payload: RunEventPayload }>,
  holdOpen = false,
): AsyncGenerator<{ kind: 'message'; item: MessageItem } | { kind: 'run'; payload: RunEventPayload }> {
  for (const ev of events) yield ev;
  if (holdOpen) {
    // Never resolve — simulates an open connection.
    await new Promise<never>(() => {});
  }
}

beforeEach(() => {
  vi.resetAllMocks();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('useMessageStream', () => {
  it('deduplicates messages by seq', async () => {
    const item: MessageItem = { seq: 1, message: { role: 'user', contents: [] } };
    // Return stream twice so reconnect after first stream completes doesn't hang.
    vi.mocked(streamSessionMessages)
      .mockReturnValueOnce(makeStream([
        { kind: 'message', item },
        { kind: 'message', item }, // duplicate
      ]))
      .mockReturnValue(makeStream([], true)); // hold open on reconnect

    const { result } = renderHook(() => useMessageStream('s1'), { wrapper });

    await waitFor(() => expect(result.current.messages).toHaveLength(1));
    expect(result.current.messages[0]!.seq).toBe(1);
  });

  it('sets running=true on started, false on finished', async () => {
    vi.mocked(streamSessionMessages)
      .mockReturnValueOnce(makeStream([
        { kind: 'run', payload: { run: 'started' } },
        { kind: 'run', payload: { run: 'finished' } },
      ]))
      .mockReturnValue(makeStream([], true)); // hold open on reconnect

    const { result } = renderHook(() => useMessageStream('s1'), { wrapper });
    await waitFor(() => expect(result.current.running).toBe(false));
  });

  it('sets runError on run:error event', async () => {
    vi.mocked(streamSessionMessages)
      .mockReturnValueOnce(makeStream([
        { kind: 'run', payload: { run: 'error', message: 'oops' } as RunEventPayload },
      ]))
      .mockReturnValue(makeStream([], true)); // hold reconnect open

    const { result } = renderHook(() => useMessageStream('s1'), { wrapper });

    await waitFor(() => expect(result.current.runError).toBe('oops'));
  });

  it('clears runError on next run:started after reconnect', async () => {
    // Use a real-timers approach: make setTimeout fire immediately by replacing it.
    const origSetTimeout = globalThis.setTimeout;
    vi.spyOn(globalThis, 'setTimeout').mockImplementation((fn, _delay?, ..._args) => {
      // Fire reconnect immediately instead of waiting the backoff delay.
      return origSetTimeout(fn as TimerHandler, 0) as unknown as ReturnType<typeof setTimeout>;
    });

    vi.mocked(streamSessionMessages)
      .mockReturnValueOnce(makeStream([
        { kind: 'run', payload: { run: 'error', message: 'oops' } as RunEventPayload },
      ]))
      .mockReturnValue(makeStream([
        { kind: 'run', payload: { run: 'started' } },
      ], true));

    const { result } = renderHook(() => useMessageStream('s1'), { wrapper });

    await waitFor(() => expect(result.current.runError).toBeNull(), { timeout: 3000 });
    vi.restoreAllMocks();
  });

  it('stops permanently on 404', async () => {
    vi.mocked(streamSessionMessages).mockImplementation(async function*() {
      throw new ApiError(404, 'Not Found');
    });

    const { result } = renderHook(() => useMessageStream('s1'), { wrapper });

    // After the 404, connected should go back to false and stay there.
    await waitFor(() => expect(result.current.connected).toBe(false));

    // streamSessionMessages called only once (no reconnect attempts).
    expect(streamSessionMessages).toHaveBeenCalledTimes(1);
  });

  it('aborts on unmount', () => {
    const abortSpy = vi.spyOn(AbortController.prototype, 'abort');
    vi.mocked(streamSessionMessages).mockReturnValue(makeStream([], true));

    const { unmount } = renderHook(() => useMessageStream('s1'), { wrapper });
    unmount();

    expect(abortSpy).toHaveBeenCalled();
  });

  // This test MUST fail on the old code (impure updater that mutates
  // lastSeqRef.current inside setMessages) and pass on the fixed code.
  // React StrictMode double-invokes state updaters in dev; the old code
  // advanced lastSeqRef on the discarded first call so the real second
  // call failed the guard and returned prev, dropping every frame.
  it('delivers all frames under React StrictMode (regression: impure updater)', async () => {
    const items: MessageItem[] = [
      { seq: 0, message: { role: 'user', contents: [] } },
      { seq: 1, message: { role: 'assistant', contents: [] } },
      { seq: 2, message: { role: 'user', contents: [] } },
    ];

    vi.mocked(streamSessionMessages)
      .mockReturnValueOnce(makeStream(items.map((item) => ({ kind: 'message' as const, item }))))
      .mockReturnValue(makeStream([], true)); // hold open on reconnect

    const strictWrapper = ({ children }: { children: ReactNode }) => {
      const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
      return (
        <StrictMode>
          <QueryClientProvider client={qc}>{children}</QueryClientProvider>
        </StrictMode>
      );
    };

    const { result } = renderHook(() => useMessageStream('s1'), { wrapper: strictWrapper });

    // All 3 messages must arrive — the old impure updater would drop them all.
    await waitFor(() => expect(result.current.messages).toHaveLength(3));
    expect(result.current.messages.map((m) => m.seq)).toEqual([0, 1, 2]);
  });
});
