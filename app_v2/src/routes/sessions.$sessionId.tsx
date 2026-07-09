import { useState } from 'react';
import { createFileRoute } from '@tanstack/react-router';
import { useMessageStream } from '@/hooks/useMessageStream';
import { buildTranscript } from '@/lib/transcript';
import { sendMessage } from '@/api/messages';
import { ApiError } from '@/api/client';
import { MessageList } from '@/components/chat/MessageList';
import { Composer } from '@/components/chat/Composer';

export const Route = createFileRoute('/sessions/$sessionId')({
  component: SessionPage,
});

function SessionPage() {
  const { sessionId } = Route.useParams();
  const { messages, running, runError, connected } = useMessageStream(sessionId);
  const [sendError, setSendError] = useState<string | null>(null);

  const transcript = buildTranscript(messages);

  async function handleSend(text: string) {
    setSendError(null);
    try {
      await sendMessage(sessionId, text);
    } catch (err) {
      if (err instanceof ApiError && err.status === 409) {
        setSendError('Run already in progress — please wait');
      } else {
        setSendError(err instanceof Error ? err.message : 'Send failed');
      }
    }
  }

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

      <MessageList entries={transcript} running={running} />

      <Composer
        running={running}
        sessionId={sessionId}
        onSend={(text) => { void handleSend(text); }}
      />
    </div>
  );
}
