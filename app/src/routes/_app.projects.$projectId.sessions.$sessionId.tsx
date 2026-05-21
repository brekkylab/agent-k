// Session — markup mirrors app-live SessionPage. Chat surface (head + messages
// + composer) + right side (members, references, access, artifact).

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
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
import { AI_USER } from '@/api/transformers';
import { formatMessageTime, formatMessageTimeFull } from '@/lib/formatMessageTime';
import type { Message, ShareMode, User } from '@/domain/types';
import { ApiError } from '@/api/client';
import { SessionTitleText } from '@/components/SessionTitleText';

export const Route = createFileRoute('/_app/projects/$projectId/sessions/$sessionId')({
  component: SessionPage,
});

const SUBAGENT_PREFIX = 'subagent_';
function stripSubagentPrefix(name: string): string {
  return name.startsWith(SUBAGENT_PREFIX) ? name.slice(SUBAGENT_PREFIX.length) : name;
}

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
    queryFn: () => listMessages(sessionId),
    enabled: Boolean(session.data && currentUser),
  });

  const messagesEndRef = useRef<HTMLDivElement>(null);

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

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [allMessages.length, streaming]);

  const send = useCallback(async () => {
    const text = composerText.trim();
    if (!text || streaming) return;
    setComposerText('');

    const nowIso = new Date().toISOString();
    const userMsg: Message = {
      id: `live-user-${Date.now()}`,
      sessionId,
      sender: { kind: 'user', userId: currentUser?.id ?? 'user' },
      createdAt: nowIso,
      body: text,
      status: 'done',
    };
    const aiId = `live-ai-${Date.now()}`;
    setLiveMessages((prev) => [...prev, userMsg, {
      id: aiId,
      sessionId,
      sender: { kind: 'agent' as const, name: 'agent-k' },
      createdAt: nowIso,
      body: '',
      status: 'streaming' as const,
    }]);
    setStreaming(true);

    const ctrl = new AbortController();

    try {
      for await (const update of streamMessage(sessionId, text, ctrl.signal)) {
        const isDone = update.status === 'done';
        const doneOrStreaming = isDone ? 'done' as const : 'streaming' as const;

        setLiveMessages((prev) => {
          // Update main agent-k bubble
          let next: Message[] = prev.map((m) => {
            if (m.id !== aiId) return m;
            const updatedToolCalls = update.toolCalls.length > 0
              ? update.toolCalls.map((tc) => ({ ...tc }))
              : m.toolCalls;
            return { ...m, body: update.text, status: doneOrStreaming, toolCalls: updatedToolCalls };
          });

          // Apply subagent bubble updates — stable id derived from sender name
          for (const sub of update.subagentUpdates) {
            const subId = `live-sub-${sub.sourceAgent}`;
            const exists = next.some((m) => m.id === subId);
            if (exists) {
              next = next.map((m) =>
                m.id === subId ? { ...m, body: sub.text, status: doneOrStreaming } : m,
              );
            } else {
              next = [...next, {
                id: subId,
                sessionId,
                sender: { kind: 'agent' as const, name: sub.sourceAgent },
                createdAt: nowIso,
                body: sub.text,
                status: 'streaming' as const,
              }];
            }
          }

          return next;
        });

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
      // Refetch and wait for new history data before clearing live messages to avoid flash.
      await queryClient.refetchQueries({ queryKey: ['messages', sessionId] });
      setLiveMessages([]);
      void queryClient.invalidateQueries({ queryKey: ['session', sessionId] });
      void queryClient.invalidateQueries({ queryKey: ['sessions', projectId] });
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
  const usersForRender: User[] = [...userList, AI_USER];



  return (
    <div className="cw-session-layout cw-page-enter">
      <section className="cw-chat-surface">
        <div className="cw-chat-head">
          <div>
            <h1><SessionTitleText title={sess?.title ?? '...'} /></h1>
            <p>
              {creator && <>Started by <Avatar user={creator} small /> {creator.name} · </>}
              {sess?.references.length ?? 0} files ·{' '}
              <Avatar user={AI_USER} small /> Cowork Default
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
              users={usersForRender}
              currentUserId={currentUser?.id ?? ''}
            />
          ))}
          {streaming && (
            <div className="cw-live"><span />AI 답변 중…</div>
          )}
          <div ref={messagesEndRef} />
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

function extractSubagentQuery(args: unknown): string {
  if (typeof args === 'string') {
    try {
      const parsed = JSON.parse(args) as unknown;
      if (parsed && typeof parsed === 'object') {
        const vals = Object.values(parsed as Record<string, unknown>);
        const first = vals[0];
        if (typeof first === 'string') return first;
      }
    } catch { /* ignore */ }
    return args;
  }
  if (args && typeof args === 'object') {
    const vals = Object.values(args as Record<string, unknown>);
    const first = vals[0];
    if (typeof first === 'string') return first;
  }
  return '';
}

function MessageBubble({
  message,
  users,
  currentUserId,
}: {
  message: Message;
  users: User[];
  currentUserId: string;
}) {
  const isAi = message.sender.kind === 'agent';
  const isSelf = message.sender.kind === 'user' && message.sender.userId === currentUserId;

  const displayUser: User = isAi
    ? (users.find((u) => u.id === 'ai') ?? AI_USER)
    : (users.find((u) => u.id === (message.sender as { userId: string }).userId)
      ?? { id: 'unknown', name: 'Member', roleLabel: 'Member', avatar: 'M', color: 'var(--cw-ink-3)' });

  const isStreaming = message.status === 'streaming';
  const timeLabel = formatMessageTime(message.createdAt);
  const agentLabel = isAi ? (message.sender as { name: string }).name : null;

  const isLive = message.id.startsWith('live-');

  return (
    <article className={`cw-message ${isAi ? 'is-ai' : isSelf ? 'is-self' : 'is-other'}${isLive ? ' is-entering' : ''}`}>
      {isAi ? <span className="cw-ai-chip">AI</span> : <Avatar user={displayUser} />}
      <div className="cw-message-body">
        <div className="cw-message-meta">
          <b>{isSelf ? `${displayUser.name.split(' ')[0]} · 나` : isAi ? (agentLabel ?? 'AI') : displayUser.name.split(' ')[0]}</b>
          <time dateTime={message.createdAt} data-tooltip={formatMessageTimeFull(message.createdAt)}>{timeLabel}</time>
        </div>
        <div className={isAi ? 'cw-ai-prose' : 'cw-message-bubble'}>
          {isAi
            ? <MarkdownRenderer text={message.body} />
            : message.body.split('\n').map((line, i) => <p key={`${message.id}-${i}`}>{line || ' '}</p>)}
        </div>
        {isAi && message.toolCalls?.map((tc) =>
          tc.name.startsWith(SUBAGENT_PREFIX) ? (
            <div key={tc.id} className="cw-subagent-call">
              <span className="cw-subagent-mention">@{stripSubagentPrefix(tc.name)}</span>
              {' '}{extractSubagentQuery(tc.arguments)}
            </div>
          ) : (
            <details key={tc.id} className="cw-toolcall">
              <summary>🔧 {tc.name}{tc.result === undefined && isStreaming ? ' · 실행 중…' : ''}</summary>
              {tc.arguments !== undefined && (
                <pre className="cw-toolcall-args">{typeof tc.arguments === 'string'
                  ? tc.arguments
                  : JSON.stringify(tc.arguments, null, 2)}</pre>
              )}
              {tc.result !== undefined && <pre className="cw-toolcall-result">{tc.result}</pre>}
            </details>
          )
        )}
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
