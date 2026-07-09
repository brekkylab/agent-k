// Home (`/`) — a "new chat" surface. Typing the first message creates a session,
// sends the message, then jumps into the chat where SSE catch-up shows the user
// turn + streaming reply. Browsing past sessions lives in the sidebar Recents
// list, not here.

import { useState, useEffect } from 'react';
import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { createSession } from '@/api/sessions';
import { sendMessage } from '@/api/messages';
import { ApiError } from '@/api/client';
import {
  ProjectHomeComposer,
  type ProjectHomeComposerSubmission,
} from '@/components/chat/ProjectHomeComposer';
import type { AgentType } from '@/api/types';
import { takePendingAttachment, type PendingAttachment } from '@/stores/pendingAttachment';

export const Route = createFileRoute('/')({
  component: HomePage,
});

// Exported for unit testing (rendered directly with mocked router/api).
export function HomePage() {
  const { t } = useTranslation('session');
  const navigate = useNavigate();
  const qc = useQueryClient();

  const [composerText, setComposerText] = useState('');
  const [agentType, setAgentType] = useState<AgentType>('coworker');
  const [model, setModel] = useState('');
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // StrictMode-safe read-and-clear: useState initializer must NOT call
  // takePendingAttachment() because StrictMode double-invokes initializers in
  // dev, consuming the value on the discarded first call so the chip never
  // shows. useEffect runs after commit; the second (real) effect run gets null
  // from the now-empty store and is a no-op — the chip set by the first run
  // survives.
  const [attachment, setAttachment] = useState<PendingAttachment | null>(null);
  useEffect(() => {
    const a = takePendingAttachment();
    if (a) setAttachment(a);
  }, []);

  // create → send → navigate. On any failure, surface an inline error on the
  // composer and do NOT navigate.
  async function handleSubmit({ text }: ProjectHomeComposerSubmission) {
    if (pending) return;
    setPending(true);
    setError(null);
    try {
      const session = await createSession({
        agentType,
        model: model.trim() || undefined,
      });
      // Prepend the shared-mount path so the agent can read the file.
      const finalText = attachment
        ? '[첨부 파일: ' + attachment.sharedPath + ']\n' + text
        : text;
      await sendMessage(session.id, finalText);
      setAttachment(null);
      void qc.invalidateQueries({ queryKey: ['sessions'] });
      setComposerText('');
      void navigate({ to: '/sessions/$sessionId', params: { sessionId: session.id } });
    } catch (err) {
      const message =
        err instanceof ApiError || err instanceof Error ? err.message : t('home.error_generic');
      setError(t('home.error', { message }));
    } finally {
      setPending(false);
    }
  }

  return (
    <section className="cw-page cw-page-enter">
      <div className="cw-home-blank">
        <p className="cw-home-greeting">{t('home.greeting')}</p>

        <div className="cw-agent-composer-wrap" data-agent={agentType}>
          {/* Attachment chip — shown when a workspace file is pending. */}
          {attachment && (
            <div className="cw-home-attachment-chip">
              <span className="cw-home-attachment-icon" aria-hidden="true">📄</span>
              <span className="cw-home-attachment-name">{attachment.name}</span>
              <button
                type="button"
                className="cw-home-attachment-remove"
                aria-label={t('home.removeAttachment')}
                onClick={() => setAttachment(null)}
              >
                ✕
              </button>
            </div>
          )}
          {attachment && (
            <p className="cw-home-attachment-hint">{t('home.attachmentHint')}</p>
          )}
          <ProjectHomeComposer
            value={composerText}
            onChange={setComposerText}
            onSubmit={handleSubmit}
            agentType={agentType}
            onAgentTypeChange={setAgentType}
            model={model}
            onModelChange={setModel}
            disabled={pending}
            pending={pending}
            error={error}
          />
        </div>
      </div>
    </section>
  );
}
