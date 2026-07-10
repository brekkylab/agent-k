import { useState, useEffect, useRef } from 'react';
import { createFileRoute } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';
import { useMessageStream } from '@/hooks/useMessageStream';
import { buildTranscript, type TranscriptEntry } from '@/lib/transcript';
import { sendMessage } from '@/api/messages';
import { listSessions } from '@/api/sessions';
import { ApiError } from '@/api/client';
import { MessageList } from '@/components/chat/MessageList';
import { Composer } from '@/components/chat/Composer';
import { SessionTitle } from '@/components/SessionTitle';
import { takePendingMessage } from '@/stores/pendingMessage';

export const Route = createFileRoute('/sessions/$sessionId')({
  component: SessionPage,
});

function SessionPage() {
  const { sessionId } = Route.useParams();
  const { messages, running, runError } = useMessageStream(sessionId);
  const [sendError, setSendError] = useState<string | null>(null);

  // Session title, read from the shared ['sessions'] cache (populated by the
  // sidebar and live-patched by useMessageStream when the auto-title arrives).
  const { data: sessions } = useQuery({
    queryKey: ['sessions'],
    queryFn: listSessions,
    staleTime: 30_000,
  });
  const session = sessions?.find((s) => s.id === sessionId);
  const title = session?.title ?? null;

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
        {/* Key by sessionId so switching sessions remounts the title: the
            router reuses this component across param changes, and without a
            fresh mount an already-titled session would inherit the previous
            session's null→value transition and appear to "type" itself. */}
        <SessionTitle
          key={sessionId}
          title={title}
          createdAt={session?.created_at}
          className="cw-chat-title"
          skeletonClassName="cw-title-skeleton--head"
          fallback={sessionId.slice(0, 8)}
        />
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
