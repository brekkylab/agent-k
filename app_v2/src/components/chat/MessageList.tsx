import { useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { MarkdownRenderer } from './MarkdownRenderer';
import { ToolCallDetails } from './ToolCallDetails';
import type { TranscriptEntry } from '@/lib/transcript';
import { formatMessageDate, formatMessageDateFull } from '@/lib/formatMessageDate';

interface MessageListProps {
  entries: TranscriptEntry[];
  running: boolean;
}

export function MessageList({ entries, running }: MessageListProps) {
  const bottomRef = useRef<HTMLDivElement>(null);
  // Subscribe to language changes so timestamps re-render on toggle (not just
  // after a refresh); the value is passed into the date formatters below.
  const { t, i18n } = useTranslation('session');
  const lng = i18n.language;

  // Auto-scroll to bottom when new entries arrive or the run status toggles
  // (so the "responding" indicator is scrolled into view when it appears).
  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [entries.length, running]);

  return (
    <div className="cw-messages-scroll">
      <div className="cw-messages">
        {entries.map((entry, idx) => (
          <div
            key={idx}
            className={`cw-message ${entry.kind === 'user' ? 'is-self' : 'is-ai'}`}
          >
            {entry.kind === 'assistant' && (
              <span className="cw-ai-chip" aria-hidden="true">AI</span>
            )}
            <div className="cw-message-body">
              {entry.kind === 'user' ? (
                <div className="cw-message-bubble">
                  <pre className="cw-message-text">{entry.text}</pre>
                </div>
              ) : (
                <div className="cw-message-bubble">
                  {entry.text && <MarkdownRenderer text={entry.text} />}
                  {entry.toolCalls.map((tc) => (
                    <ToolCallDetails key={tc.id} tc={tc} isStreaming={running && idx === entries.length - 1} />
                  ))}
                </div>
              )}
              {/* Timestamp revealed on hover of the message. */}
              {entry.createdAt && (
                <time
                  className="cw-message-time"
                  dateTime={entry.createdAt}
                  title={formatMessageDateFull(entry.createdAt, lng)}
                >
                  {formatMessageDate(entry.createdAt, lng)}
                </time>
              )}
            </div>
          </div>
        ))}
        {/* Typing indicator below the last message — only while a run is in
            flight, so it (and its dot) disappear the moment the run ends. */}
        {running && (
          <div className="cw-typing" aria-live="polite">
            <span className="cw-status-dot cw-status-dot--connected" aria-hidden="true" />
            <span className="cw-status-running">{t('ui.ai_responding')}</span>
          </div>
        )}
        <div ref={bottomRef} />
      </div>
    </div>
  );
}
