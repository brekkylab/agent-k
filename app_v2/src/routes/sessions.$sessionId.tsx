import { useState, useEffect, useRef } from 'react';
import { createFileRoute } from '@tanstack/react-router';
import { useMessageStream } from '@/hooks/useMessageStream';
import { buildTranscript, type TranscriptEntry } from '@/lib/transcript';
import { sendMessage } from '@/api/messages';
import { ApiError } from '@/api/client';
import { MessageList } from '@/components/chat/MessageList';
import { Composer } from '@/components/chat/Composer';
import { takePendingMessage } from '@/stores/pendingMessage';

export const Route = createFileRoute('/sessions/$sessionId')({
  component: SessionPage,
});

function SessionPage() {
  const { sessionId } = Route.useParams();
  const { messages, running, runError, connected } = useMessageStream(sessionId);
  const [sendError, setSendError] = useState<string | null>(null);

  // Optimistically-rendered user message: shown the instant it is sent, before
  // the server echoes it back over SSE (which only happens after the run's
  // sandbox restore). Cleared once a new user message actually arrives.
  const [optimistic, setOptimistic] = useState<string | null>(null);
  const baselineUsers = useRef(0);

  const transcript = buildTranscript(messages);
  const userMsgCount = messages.reduce(
    (n, m) => (m.message.role === 'user' ? n + 1 : n),
    0,
  );

  // The "Run already in progress" hint describes an active run; once the run
  // ends (Send returns), it is stale — clear it so it doesn't linger.
  useEffect(() => {
    if (!running) setSendError(null);
  }, [running]);

  // First message handed over from the home composer — render it immediately on
  // arrival. Read-and-clear is StrictMode-safe (2nd invoke gets null → no-op).
  useEffect(() => {
    const first = takePendingMessage();
    if (first !== null) {
      baselineUsers.current = 0;
      setOptimistic(first);
    }
  }, []);

  // Drop the optimistic bubble once the server has persisted a new user message
  // (its count grew past the baseline captured at send time).
  useEffect(() => {
    if (optimistic !== null && userMsgCount > baselineUsers.current) {
      setOptimistic(null);
    }
  }, [userMsgCount, optimistic]);

  async function handleSend(text: string): Promise<boolean> {
    setSendError(null);
    baselineUsers.current = userMsgCount;
    setOptimistic(text);
    try {
      await sendMessage(sessionId, text);
      return true;
    } catch (err) {
      setOptimistic(null);
      if (err instanceof ApiError && err.status === 409) {
        setSendError('Run already in progress — please wait');
      } else {
        setSendError(err instanceof Error ? err.message : 'Send failed');
      }
      return false;
    }
  }

  // Append the optimistic user bubble only while the server hasn't echoed it yet
  // (guards the one-render window between the message arriving and the effect).
  const showOptimistic = optimistic !== null && userMsgCount <= baselineUsers.current;
  const entries: TranscriptEntry[] = showOptimistic
    ? [...transcript, { kind: 'user', text: optimistic, toolCalls: [] }]
    : transcript;

  return (
    <div className="cw-chat-surface">
      <div className="cw-chat-head">
        <div className="cw-chat-status">
          {connected ? (
            <span className="cw-status-dot cw-status-dot--connected" title="Connected" />
          ) : (
            <span className="cw-status-dot cw-status-dot--disconnected" title="Connecting…" />
          )}
          {running && <span className="cw-status-running">AI is responding…</span>}
        </div>
      </div>

      {runError && (
        <div className="cw-chat-banner cw-chat-banner--error">Agent error: {runError}</div>
      )}

      {sendError && (
        <div className="cw-chat-banner cw-chat-banner--warn">{sendError}</div>
      )}

      <MessageList entries={entries} running={running} />

      <Composer
        running={running}
        sessionId={sessionId}
        onSend={handleSend}
      />
    </div>
  );
}
