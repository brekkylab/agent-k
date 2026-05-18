// Session — markup mirrors app-live SessionPage. Chat surface (head + messages
// + composer) + right side (members, references, access, artifact).

import { useCallback, useEffect, useMemo, useState } from 'react';
import { createFileRoute } from '@tanstack/react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { getSession, updateSessionShareMode } from '@/api/sessions';
import { listMessages, streamMessage } from '@/api/messages';
import { getProject, listMembers } from '@/api/projects';
import { Icon } from '@/components/Icon';
import { Avatar, IconPocket, IntentBadge, SharePill, ShareSelect } from '@/components/uiPrimitives';
import { useAuthStore } from '@/stores/auth';
import { useToastStore } from '@/components/Toast';
import { shareMeta } from '@/domain/metadata';
import { MarkdownRenderer } from '@/components/chat/MarkdownRenderer';
import type { Message, ShareMode } from '@/domain/types';
import { ApiError } from '@/api/client';
import { SessionTitleText } from '@/components/SessionTitleText';

export const Route = createFileRoute('/_app/projects/$projectId/sessions/$sessionId')({
  component: SessionPage,
});

function SessionPage() {
  const { projectId, sessionId } = Route.useParams();
  const queryClient = useQueryClient();
  const showToast = useToastStore((s) => s.show);
  const currentUser = useAuthStore((s) => s.currentUser);

  const project = useQuery({ queryKey: ['project', projectId], queryFn: () => getProject(projectId) });
  const session = useQuery({ queryKey: ['session', sessionId], queryFn: () => getSession(sessionId) });
  const members = useQuery({ queryKey: ['members', projectId], queryFn: () => listMembers(projectId) });
  const history = useQuery({
    queryKey: ['messages', sessionId],
    queryFn: () => listMessages(sessionId, session.data?.creatorId ?? currentUser?.id ?? 'user'),
    enabled: Boolean(session.data && currentUser),
  });

  const [composerText, setComposerText] = useState('');
  const [liveMessages, setLiveMessages] = useState<Message[]>([]);
  const [streaming, setStreaming] = useState(false);

  useEffect(() => {
    setLiveMessages([]);
    setComposerText('');
    setStreaming(false);
  }, [sessionId]);

  // After messages load, mark-read side effect has run on the backend — sync badge in session list.
  useEffect(() => {
    if (history.isSuccess) {
      void queryClient.invalidateQueries({ queryKey: ['sessions', projectId] });
    }
  }, [history.isSuccess, history.dataUpdatedAt, projectId, queryClient]);

  const allMessages = useMemo<Message[]>(() => [
    ...(history.data ?? []),
    ...liveMessages,
  ], [history.data, liveMessages]);

  const send = useCallback(async () => {
    const text = composerText.trim();
    if (!text || streaming) return;
    setComposerText('');

    const userMsg: Message = {
      id: `live-user-${Date.now()}`,
      sessionId,
      senderId: currentUser?.id ?? 'user',
      createdAt: '방금 전',
      body: text,
      status: 'done',
    };
    const aiId = `live-ai-${Date.now()}`;
    setLiveMessages((prev) => [...prev, userMsg, {
      id: aiId,
      sessionId,
      senderId: 'ai',
      createdAt: '응답 중',
      body: '',
      status: 'streaming',
    }]);
    setStreaming(true);

    const ctrl = new AbortController();
    try {
      for await (const update of streamMessage(sessionId, text, ctrl.signal)) {
        setLiveMessages((prev) => prev.map((m) => (
          m.id === aiId ? { ...m, body: update.text, status: update.status === 'done' ? 'done' : 'streaming' } : m
        )));
        if (update.status === 'error') {
          showToast(`스트리밍 실패: ${update.errorText ?? 'unknown'}`);
          break;
        }
      }
    } catch (err) {
      const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'stream failed';
      showToast(`전송 실패: ${msg}`);
    } finally {
      setStreaming(false);
      await queryClient.invalidateQueries({ queryKey: ['messages', sessionId] });
      void queryClient.invalidateQueries({ queryKey: ['session', sessionId] });
      void queryClient.invalidateQueries({ queryKey: ['sessions', projectId] });
      setLiveMessages([]);
    }
  }, [composerText, streaming, sessionId, projectId, currentUser, queryClient, showToast]);

  const shareMutation = useMutation({
    mutationFn: (mode: ShareMode) => updateSessionShareMode(sessionId, mode),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['session', sessionId] });
      await queryClient.invalidateQueries({ queryKey: ['sessions', projectId] });
      showToast('공유 모드가 변경되었습니다');
    },
    onError: (err) => {
      const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'update failed';
      showToast(`공유 변경 실패: ${msg}`);
    },
  });

  const sess = session.data;
  const userList = members.data ?? [];
  const creator = userList.find((u) => u.id === sess?.creatorId);
  const aiUser = { id: 'ai', name: 'Cowork', roleLabel: 'Agent', avatar: 'CW', color: 'var(--cw-ink)' };

  return (
    <div className="cw-session-layout cw-page-enter">
      <section className="cw-chat-surface">
        <div className="cw-chat-head">
          <div>
            <h1><SessionTitleText title={sess?.title ?? '...'} /></h1>
            <p>
              {creator && <>Started by <Avatar user={creator} small /> {creator.name} · </>}
              {sess?.references.length ?? 0} files ·{' '}
              <Avatar user={aiUser} small /> Cowork Default
            </p>
          </div>
          <div className="cw-session-head-actions">
            {sess && <IntentBadge intent={sess.intent} />}
            {sess && (
              <ShareSelect mode={sess.shareMode} onChange={(mode) => shareMutation.mutate(mode)} />
            )}
          </div>
        </div>

        <div className="cw-messages">
          {allMessages.map((msg) => (
            <MessageBubble
              key={msg.id}
              message={msg}
              users={[...userList, aiUser]}
              currentUserId={currentUser?.id ?? ''}
            />
          ))}
          {streaming && (
            <div className="cw-live"><span />응답 받는 중…</div>
          )}
        </div>

        <form className="cw-composer" onSubmit={(e) => { e.preventDefault(); void send(); }}>
          <div className="cw-composer-box">
            <input
              value={composerText}
              onChange={(e) => setComposerText(e.target.value)}
              placeholder="Message Cowork and the team…"
              disabled={streaming}
            />
            <button type="submit" className="cw-send-button" aria-label="Send" disabled={!composerText.trim() || streaming}>
              <Icon name="send" size={12} />
            </button>
          </div>
          <button
            type="button"
            className="cw-btn-secondary cw-artifact-button"
            onClick={() => showToast('Artifacts 기능은 곧 추가됩니다.')}
            disabled={streaming}
          >
            <IconPocket tone="trust" icon="artifact" compact /> Artifact
          </button>
          <small>Enter to send · Reference files with @filename</small>
        </form>
      </section>

      <aside className="cw-session-side">
        <h3>Members</h3>
        {userList.map((user) => (
          <div className="cw-side-row" key={user.id}>
            <Avatar user={user} small />
            {user.name}
          </div>
        ))}
        <h3>Referenced files</h3>
        {sess?.references.length
          ? <p style={{ fontFamily: 'var(--cw-font-mono)', fontSize: 11 }}>{sess.references.join(', ')}</p>
          : <p>No pinned files yet.</p>}
        <h3>Access</h3>
        {sess && <SharePill mode={sess.shareMode} />}
        {sess && <p>{shareMeta[sess.shareMode].desc}</p>}
        <h3>Session</h3>
        <p style={{ fontFamily: 'var(--cw-font-mono)', fontSize: 10.5, color: 'var(--cw-ink-4)' }}>{sessionId}</p>
        <p style={{ fontFamily: 'var(--cw-font-mono)', fontSize: 10.5, color: 'var(--cw-ink-4)' }}>
          project · {project.data?.name ?? '...'}
        </p>
      </aside>
    </div>
  );
}

function MessageBubble({
  message,
  users,
  currentUserId,
}: {
  message: Message;
  users: Array<{ id: string; name: string; color: string; avatar: string; roleLabel: string }>;
  currentUserId: string;
}) {
  const isAi = message.senderId === 'ai';
  const isSelf = message.senderId === currentUserId;
  const user = users.find((u) => u.id === message.senderId);
  const fallbackUser = !user && !isAi
    ? { id: message.senderId, name: isSelf ? '나' : 'Member', roleLabel: 'Member', avatar: isSelf ? '나' : 'M', color: 'var(--cw-ink-3)' }
    : null;
  const displayUser = user ?? fallbackUser;
  const userName = displayUser?.name ?? (isSelf ? '나' : 'Member');
  const isStreaming = message.status === 'streaming';

  return (
    <article className={`cw-message ${isAi ? 'is-ai' : isSelf ? 'is-self' : 'is-other'}`}>
      {isAi ? <span className="cw-ai-chip">CW</span> : (displayUser && <Avatar user={displayUser} />)}
      <div className="cw-message-body">
        <div className="cw-message-meta">
          <b>{isSelf ? `${userName.split(' ')[0]} · 나` : isAi ? 'AI' : userName.split(' ')[0]}</b>
          {isAi && <span>Cowork Default</span>}
          <time>{message.createdAt}</time>
        </div>
        <div className={isAi ? 'cw-ai-prose' : 'cw-message-bubble'}>
          {isAi
            ? <><MarkdownRenderer text={message.body} />{isStreaming && <span className="cw-thinking-cursor" />}</>
            : message.body.split('\n').map((line, i) => <p key={`${message.id}-${i}`}>{line || ' '}</p>)}
        </div>
        {isAi && message.status === 'done' && (
          <div className="cw-ai-actions">
            <button>Copy</button>
            <button>Regenerate</button>
            <button>Good</button>
          </div>
        )}
      </div>
    </article>
  );
}
