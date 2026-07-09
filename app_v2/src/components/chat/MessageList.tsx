import { useEffect, useRef } from 'react';
import { MarkdownRenderer } from './MarkdownRenderer';
import { ToolCallDetails } from './ToolCallDetails';
import type { TranscriptEntry } from '@/lib/transcript';

interface MessageListProps {
  entries: TranscriptEntry[];
  running: boolean;
}

export function MessageList({ entries, running }: MessageListProps) {
  const bottomRef = useRef<HTMLDivElement>(null);

  // Auto-scroll to bottom when new entries arrive.
  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [entries.length]);

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
            </div>
          </div>
        ))}
        <div ref={bottomRef} />
      </div>
    </div>
  );
}
