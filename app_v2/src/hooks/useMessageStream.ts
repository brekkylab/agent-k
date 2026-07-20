import { useEffect, useRef, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { ApiError } from '@/api/client';
import { streamSessionMessages } from '@/api/messages';
import type { MessageItem, SessionResponse } from '@/api/types';

export interface MessageStreamState {
  messages: MessageItem[];
  running: boolean;
  runError: string | null;
  connected: boolean;
}

export function useMessageStream(sessionId: string): MessageStreamState {
  const [messages, setMessages] = useState<MessageItem[]>([]);
  const [running, setRunning] = useState(false);
  const [runError, setRunError] = useState<string | null>(null);
  const [connected, setConnected] = useState(false);
  const qc = useQueryClient();

  const lastSeqRef = useRef<number | undefined>(undefined);
  const abortRef = useRef<AbortController | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    // Reset all state when sessionId changes.
    setMessages([]);
    setRunning(false);
    setRunError(null);
    setConnected(false);
    lastSeqRef.current = undefined;

    let stopped = false;
    let backoffMs = 1000;

    function clearTimer() {
      if (timerRef.current !== null) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
    }

    async function connect() {
      if (stopped) return;
      const ctrl = new AbortController();
      abortRef.current = ctrl;

      try {
        setConnected(true);
        for await (const ev of streamSessionMessages(sessionId, lastSeqRef.current, ctrl.signal)) {
          // A live frame proves the connection is healthy — reset backoff.
          backoffMs = 1000;
          if (ev.kind === 'message') {
            const { item } = ev;
            // Dedup guard and ref mutation OUTSIDE the updater so that
            // React StrictMode's double-invocation of updaters does not
            // advance lastSeqRef on the discarded first call and then fail
            // the guard on the real second call, dropping every frame.
            if (item.seq <= (lastSeqRef.current ?? -1)) continue;
            lastSeqRef.current = item.seq;
            setMessages((prev) => {
              // Pure & idempotent: StrictMode may double-invoke updaters,
              // so guard against duplicates defensively here as well.
              if (prev.some((m) => m.seq === item.seq)) return prev;
              const next = [...prev, item];
              next.sort((a, b) => a.seq - b.seq);
              return next;
            });
          } else if (ev.kind === 'run') {
            const payload = ev.payload;
            if ('run' in payload) {
              if (payload.run === 'started') {
                setRunning(true);
                setRunError(null);
              } else if (payload.run === 'finished' || payload.run === 'idle') {
                setRunning(false);
              } else if (payload.run === 'error') {
                setRunning(false);
                setRunError((payload as { run: 'error'; message: string }).message);
              }
            }
          } else {
            // ev.kind === 'title' — an auto-generated title arrived. Patch the
            // cached session list in place so the sidebar (and anything else
            // reading ['sessions']) reflects it live, without a refetch.
            qc.setQueryData<SessionResponse[]>(['sessions'], (prev) =>
              prev?.map((s) => (s.id === sessionId ? { ...s, title: ev.title } : s)),
            );
          }
        }
        // Stream ended normally — reconnect with capped backoff so a server
        // that immediately closes doesn't get hammered every second.
        setConnected(false);
        if (!stopped) {
          const delay = backoffMs;
          backoffMs = Math.min(backoffMs * 2, 5000);
          timerRef.current = setTimeout(() => { void connect(); }, delay);
        }
      } catch (err) {
        if (stopped) return;
        setConnected(false);

        // 404 → session deleted, stop permanently.
        if (err instanceof ApiError && err.status === 404) {
          stopped = true;
          return;
        }

        // Abort signal → do not reconnect.
        if (err instanceof DOMException && err.name === 'AbortError') return;

        // Network or other error — reconnect with capped backoff.
        const delay = backoffMs;
        backoffMs = Math.min(backoffMs * 2, 5000);
        timerRef.current = setTimeout(() => { void connect(); }, delay);
      }
    }

    void connect();

    return () => {
      stopped = true;
      clearTimer();
      abortRef.current?.abort();
      abortRef.current = null;
    };
  }, [sessionId, qc]);

  return { messages, running, runError, connected };
}
